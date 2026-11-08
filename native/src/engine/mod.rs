use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::thread::{self, JoinHandle};

use crate::core::*;
use crate::file::{self, FileSink};
use crate::http::AbortRegistry;

mod lifecycle;
mod monitor;
mod resume;
mod scheduler;
mod transfer;
mod worker;

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Default, Clone)]
struct Segment {
    start: i64,
    end: i64,
    written: i64,
    done: bool,
    active: bool,
    retries: u32,
    retry_at: i64,
}

#[derive(Clone)]
struct Task {
    id: String,
    url: String,
    final_url: String,
    host: String,
    destination: String,
    opt: TaskOptions,
    request_headers: HeaderList,

    state: TaskState,
    mode: Mode,
    total: i64,
    downloaded: i64,
    session_bytes: i64,
    resumed_bytes: i64,
    inflight: i32,
    generation: u64,
    degraded: bool,
    reason: DegradeReason,

    segs: Vec<Segment>,
    next_seg: usize,

    cursor: i64,
    single_attempts: u32,
    probe_attempts: u32,
    probe_requested: bool,
    probe_retry_at: i64,
    single_ready: bool,
    single_queued: bool,
    single_retry_at: i64,
    pause_pending: bool,
    cancel_pending: bool,
    cancel_remove_partial: bool,
    pause_token: i32,
    cancel_token: i32,
    verify_requested: bool,

    etag: String,
    last_modified: String,
    content_encoding: String,
    content_type: String,
    accept_ranges: bool,

    created_at: i64,
    started_at: i64,
    finished_at: i64,
    attempts: i64,
    max_conn_used: i32,
    error_code: String,
    error_message: String,
    priority: i32,
    skipped: bool,
    checksum_verified: bool,

    last_session: i64,
    last_progress_at: i64,
    speed: f64,

    sink: Option<Arc<FileSink>>,
}

#[derive(Clone)]
struct Job {
    kind: JobKind,
    task: usize,
    seg: usize,
    gen: u64,
    token: i32,
    url: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Probe,
    Segment,
    Single,
    Verify,
    StandaloneProbe,
}

#[derive(Clone)]
struct Command {
    kind: CmdKind,
    token: i32,
    arg: String,
    // 命令附带布尔参数：Cancel/CancelAll/RemoveTask 用它表示是否删除不完整的目标文件。
    flag: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CmdKind {
    Pause,
    Resume,
    Cancel,
    PauseAll,
    ResumeAll,
    CancelAll,
    Flush,
    RemoveTask,
    ClearCompleted,
}

struct State {
    tasks: Vec<Task>,
    jobs: VecDeque<Job>,
    commands: VecDeque<Command>,

    effective: usize,
    max_concurrency: usize,
    max_tasks_in_flight: usize,
    policy: Policy,

    pause_all_tokens: Vec<i32>,
    cancel_all_tokens: Vec<i32>,
    next_task_id: u64,

    cpu_pct: f64,
    mem_avail_pct: f64,
    disk_latency_ms: f64,
    error_count: u64,
    success_count: u64,

    last_adapt_at: i64,
    last_adapt_speed: f64,
    last_stats_at: i64,
    last_progress_at: i64,
}

impl State {
    fn error_rate(&self) -> f64 {
        let total = self.error_count + self.success_count;
        if total == 0 {
            0.0
        } else {
            self.error_count as f64 / total as f64
        }
    }
}

pub struct Engine {
    opts: Options,
    sink: Sink,
    state: Mutex<State>,
    cv: Condvar,
    tick: Condvar,
    abort: Arc<AbortRegistry>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    workers: Mutex<usize>,
    self_weak: OnceLock<Weak<Engine>>,
    stop: AtomicBool,
    joined: AtomicBool,
    started: AtomicBool,
}

fn task_base_event(kind: u8, t: &Task) -> Event {
    let mut e = Event::new(kind);
    e.task_id = t.id.clone();
    e.url = t.url.clone();
    e.destination = t.destination.clone();
    e.state = t.state.name().to_string();
    e.total_bytes = t.total;
    e.downloaded_bytes = t.downloaded;
    e.session_bytes = t.session_bytes;
    e.resumed_bytes = t.resumed_bytes;
    e.percent = if t.total > 0 {
        (t.downloaded as f64 * 100.0 / t.total as f64).min(100.0)
    } else {
        -1.0
    };
    e.speed_bps = t.speed;
    let end = if t.finished_at > 0 {
        t.finished_at
    } else {
        now_ms()
    };
    let begin = if t.started_at > 0 {
        t.started_at
    } else {
        t.created_at
    };
    let elapsed = (end - begin).max(0);
    e.elapsed_ms = elapsed;
    e.average_speed_bps = if elapsed > 0 {
        t.session_bytes as f64 * 1000.0 / elapsed as f64
    } else {
        0.0
    };
    e.eta_seconds = if t.speed > 0.0 && t.total > 0 {
        ((t.total - t.downloaded) as f64 / t.speed).max(0.0)
    } else {
        -1.0
    };
    e.active_connections = t.inflight as i64;
    e.segments_total = if t.mode == Mode::Segmented {
        t.segs.len() as i64
    } else if t.downloaded > 0 || t.total == 0 {
        1
    } else {
        0
    };
    e.segments_completed = if t.mode == Mode::Segmented {
        t.segs.iter().filter(|s| s.done).count() as i64
    } else if t.state == TaskState::Completed {
        1
    } else {
        0
    };
    e.content_type = t.content_type.clone();
    e.degraded = t.degraded;
    e.degrade_reason = t.reason.name().to_string();
    e
}

fn state_event(t: &Task) -> Event {
    task_base_event(EV_STATE, t)
}

fn progress_event(t: &Task) -> Event {
    task_base_event(EV_PROGRESS, t)
}

fn completed_event(t: &Task) -> Event {
    let mut e = task_base_event(EV_COMPLETED, t);
    e.attempts = t.attempts;
    e.connections_used = t.max_conn_used as i64;
    e.checksum_verified = t.checksum_verified;
    e.skipped = t.skipped;
    e.final_url = t.final_url.clone();
    e.etag = t.etag.clone();
    e.last_modified = t.last_modified.clone();
    e
}

fn failed_event(t: &Task) -> Event {
    let mut e = task_base_event(EV_FAILED, t);
    e.code = t.error_code.clone();
    e.message = t.error_message.clone();
    e.attempts = t.attempts;
    e
}

fn reset_event(t: &Task) -> Event {
    task_base_event(EV_RESET, t)
}

fn degraded_event(t: &Task, reason: DegradeReason) -> Event {
    let mut e = Event::new(EV_DEGRADED);
    e.task_id = t.id.clone();
    e.url = t.url.clone();
    e.destination = t.destination.clone();
    e.state = t.state.name().to_string();
    e.degrade_reason = reason.name().to_string();
    e.degraded = true;
    e.message = "falling back to single connection".to_string();
    e
}

fn resume_enabled(opts: &Options, t: &Task) -> bool {
    t.opt.resume.unwrap_or(opts.resume)
}

fn policy_cap(policy: Policy, max_concurrency: usize) -> usize {
    match policy {
        Policy::FavorOtherApps => (max_concurrency / 2).max(1),
        _ => max_concurrency.max(1),
    }
}

fn find_task(st: &State, id: &str) -> Option<usize> {
    st.tasks.iter().position(|t| t.id == id)
}

// 移除任务并修正排队 Job 中保存的索引。
fn detach_task(st: &mut State, idx: usize) {
    st.tasks.remove(idx);
    st.jobs.retain(|j| j.kind == JobKind::StandaloneProbe || j.task != idx);
    for job in st.jobs.iter_mut() {
        if job.kind != JobKind::StandaloneProbe && job.task > idx {
            job.task -= 1;
        }
    }
}

// ---------------- engine lifecycle & public API ----------------

impl Engine {
    pub fn new(opts: Options, sink: Sink) -> Arc<Engine> {
        let effective = policy_cap(opts.policy, opts.max_concurrency.max(1));
        let now = now_ms();
        Arc::new(Engine {
            state: Mutex::new(State {
                tasks: Vec::new(),
                jobs: VecDeque::new(),
                commands: VecDeque::new(),
                effective,
                max_concurrency: opts.max_concurrency.max(1),
                max_tasks_in_flight: opts.max_tasks_in_flight.max(1),
                policy: opts.policy,
                pause_all_tokens: Vec::new(),
                cancel_all_tokens: Vec::new(),
                next_task_id: 1,
                cpu_pct: 0.0,
                mem_avail_pct: 100.0,
                disk_latency_ms: 0.0,
                error_count: 0,
                success_count: 0,
                last_adapt_at: now,
                last_adapt_speed: 0.0,
                last_stats_at: now,
                last_progress_at: now,
            }),
            opts,
            sink,
            cv: Condvar::new(),
            tick: Condvar::new(),
            abort: AbortRegistry::new(),
            handles: Mutex::new(Vec::new()),
            workers: Mutex::new(0),
            self_weak: OnceLock::new(),
            stop: AtomicBool::new(false),
            joined: AtomicBool::new(false),
            started: AtomicBool::new(false),
        })
    }

    pub fn start(self: &Arc<Engine>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut handles = Vec::new();
        // 先只起少量 worker：一次性探针不会因此创建整套线程；有任务时再按并发上限扩容。
        let worker_count = self.opts.max_concurrency.min(2).max(1);
        for _ in 0..worker_count {
            let me = Arc::clone(self);
            handles.push(thread::spawn(move || me.worker_loop()));
        }
        *self.workers.lock().unwrap() = worker_count;
        let _ = self.self_weak.set(Arc::downgrade(self));
        let me = Arc::clone(self);
        handles.push(thread::spawn(move || me.scheduler_loop()));
        let me = Arc::clone(self);
        handles.push(thread::spawn(move || me.monitor_loop()));
        *self.handles.lock().unwrap() = handles;
    }

    pub fn add_task(&self, opt: TaskOptions) -> Result<String, String> {
        if opt.url.is_empty() {
            return Err("url is required".to_string());
        }
        if opt.destination.is_empty() {
            return Err("destination is required".to_string());
        }
        if !opt.url.starts_with("http://") && !opt.url.starts_with("https://") {
            return Err("only http/https urls are supported".to_string());
        }
        let overwrite = lower(&opt.overwrite);
        if !matches!(overwrite.as_str(), "overwrite" | "rename" | "skip" | "error") {
            return Err(format!("invalid overwrite mode: {}", opt.overwrite));
        }
        let mut st = self.state.lock().unwrap();
        let id = format!("t{}-{}", st.next_task_id, now_ms());
        st.next_task_id += 1;
        let created = now_ms();
        let mut destination = PathBuf::from(&opt.destination);
        if overwrite == "rename" && file::path_exists(&destination) {
            destination = file::unique_rename(&destination);
        }
        let mut request_headers = self.opts.headers.clone();
        request_headers.extend(opt.headers.iter().cloned());
        let task = Task {
            id: id.clone(),
            url: opt.url.clone(),
            final_url: String::new(),
            host: String::new(),
            destination: destination.to_string_lossy().to_string(),
            priority: opt.priority,
            opt,
            request_headers,
            state: TaskState::Queued,
            mode: Mode::Segmented,
            total: -1,
            downloaded: 0,
            session_bytes: 0,
            resumed_bytes: 0,
            inflight: 0,
            generation: 0,
            degraded: false,
            reason: DegradeReason::None,
            segs: Vec::new(),
            next_seg: 0,
            cursor: 0,
            single_attempts: 0,
            probe_attempts: 0,
            probe_requested: false,
            probe_retry_at: 0,
            single_ready: false,
            single_queued: false,
            single_retry_at: 0,
            pause_pending: false,
            cancel_pending: false,
            cancel_remove_partial: false,
            pause_token: 0,
            cancel_token: 0,
            verify_requested: false,
            etag: String::new(),
            last_modified: String::new(),
            content_encoding: String::new(),
            content_type: String::new(),
            accept_ranges: false,
            created_at: created,
            started_at: 0,
            finished_at: 0,
            attempts: 0,
            max_conn_used: 0,
            error_code: String::new(),
            error_message: String::new(),
            skipped: false,
            checksum_verified: false,
            last_session: 0,
            last_progress_at: created,
            speed: 0.0,
            sink: None,
        };
        let ev = state_event(&task);
        st.tasks.push(task);
        let want_workers = st.effective;
        drop(st);
        self.emit(ev);
        self.ensure_workers(want_workers);
        self.tick.notify_all();
        Ok(id)
    }

    pub fn invoke(&self, name: &str, token: i32, arg: &str, flag: bool) {
        let kind = match name {
            "pause" => CmdKind::Pause,
            "resume" => CmdKind::Resume,
            "cancel" => CmdKind::Cancel,
            "pauseAll" => CmdKind::PauseAll,
            "resumeAll" => CmdKind::ResumeAll,
            "cancelAll" => CmdKind::CancelAll,
            "flush" => CmdKind::Flush,
            "removeTask" => CmdKind::RemoveTask,
            "clearCompleted" => CmdKind::ClearCompleted,
            "probe" => {
                let mut st = self.state.lock().unwrap();
                st.jobs.push_back(Job {
                    kind: JobKind::StandaloneProbe,
                    task: usize::MAX,
                    seg: 0,
                    gen: 0,
                    token,
                    url: arg.to_string(),
                });
                drop(st);
                self.cv.notify_one();
                return;
            }
            _ => {
                self.emit_done(token, false, "ERR_INVALID_ARG", &format!("unknown command: {name}"));
                return;
            }
        };
        let mut st = self.state.lock().unwrap();
        st.commands.push_back(Command {
            kind,
            token,
            arg: arg.to_string(),
            flag,
        });
        drop(st);
        self.tick.notify_all();
    }

    pub fn stats_snapshot(&self) -> Event {
        let st = self.state.lock().unwrap();
        self.stats_from_state(&st)
    }

    fn stats_from_state(&self, st: &State) -> Event {
        let mut e = Event::new(EV_STATS);
        e.max_concurrency = st.max_concurrency as i64;
        e.effective_concurrency = st.effective as i64;
        e.policy = st.policy.as_i32() as i64;
        e.cpu_usage_pct = st.cpu_pct;
        e.mem_available_pct = st.mem_avail_pct;
        e.disk_write_latency_ms = st.disk_latency_ms;
        for t in &st.tasks {
            if t.state.terminal() {
                continue;
            }
            if t.state == TaskState::Queued {
                e.queued_tasks += 1;
            } else {
                e.active_tasks += 1;
            }
            e.total_downloaded += t.downloaded;
            if t.total > 0 {
                e.total_remaining += (t.total - t.downloaded).max(0);
            }
            e.total_speed_bps += t.speed;
            if t.degraded {
                e.degraded_tasks += 1;
            }
        }
        e
    }

    pub fn set_max_concurrency(&self, n: usize) {
        let mut st = self.state.lock().unwrap();
        let value = n.clamp(1, 256);
        let raised = value > st.max_concurrency;
        st.max_concurrency = value;
        let cap = policy_cap(st.policy, value);
        if raised {
            // 用户显式调大上限时立即放开到策略上限，后续自适应若检测到饱和会自动回落。
            st.effective = cap;
        } else {
            st.effective = st.effective.min(cap).max(1);
        }
        drop(st);
        self.ensure_workers(value);
        self.tick.notify_all();
    }

    fn ensure_workers(&self, need: usize) {
        if !self.started.load(Ordering::SeqCst) || self.stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(me) = self.self_weak.get().and_then(|w| w.upgrade()) else {
            return;
        };
        let mut count = self.workers.lock().unwrap();
        if *count >= need {
            return;
        }
        let mut handles = self.handles.lock().unwrap();
        while *count < need && !self.stop.load(Ordering::SeqCst) {
            let me = Arc::clone(&me);
            handles.push(thread::spawn(move || me.worker_loop()));
            *count += 1;
        }
        drop(handles);
        drop(count);
        self.cv.notify_all();
    }

    pub fn set_max_tasks_in_flight(&self, n: usize) {
        let mut st = self.state.lock().unwrap();
        st.max_tasks_in_flight = n.clamp(1, 64);
        drop(st);
        self.tick.notify_all();
    }

    pub fn set_policy(&self, policy: Policy) {
        let mut st = self.state.lock().unwrap();
        st.policy = policy;
        st.effective = policy_cap(policy, st.max_concurrency);
        let want_workers = st.effective;
        drop(st);
        self.ensure_workers(want_workers);
        self.tick.notify_all();
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // 必须中断阻塞中的 WinHTTP 调用，否则等待 join 可能长达 idle 超时。
        self.abort.abort_all();
        self.cv.notify_all();
        self.tick.notify_all();
        self.join_threads();
    }

    fn join_threads(&self) {
        if self.joined.swap(true, Ordering::SeqCst) {
            return;
        }
        let handles = {
            let mut lock = self.handles.lock().unwrap();
            std::mem::take(&mut *lock)
        };
        for handle in handles {
            let _ = handle.join();
        }
    }

    fn emit(&self, event: Event) {
        (self.sink)(event);
    }

    fn emit_done(&self, token: i32, ok: bool, code: &str, message: &str) {
        if token <= 0 {
            return;
        }
        let mut e = Event::new(EV_DONE);
        e.token = token;
        e.ok = ok;
        e.code = code.to_string();
        e.message = message.to_string();
        self.emit(e);
    }

    fn emit_log(&self, level: &str, message: &str, task: Option<&Task>) {
        let mut e = Event::new(EV_LOG);
        e.level = level.to_string();
        e.message = message.to_string();
        e.scope = if task.is_some() { "task" } else { "engine" }.to_string();
        if let Some(t) = task {
            e.task_id = t.id.clone();
            e.url = t.url.clone();
        }
        self.emit(e);
    }
}
