use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::resume::{meta_path, save_meta, take_segment};
use super::*;

fn host_has_room(map: &HashMap<String, i64>, host: &str, max: i64) -> bool {
    host.is_empty() || map.get(host).copied().unwrap_or(0) < max
}

// ---------------- scheduler ----------------

impl Engine {
    pub(super) fn scheduler_loop(&self) {
        loop {
            let mut st = self.state.lock().unwrap();
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            self.process_commands(&mut st);
            self.schedule(&mut st);
            let result = self.tick.wait_timeout(st, Duration::from_millis(20)).unwrap();
            st = result.0;
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
        }
    }

    fn process_commands(&self, st: &mut State) {
        while let Some(cmd) = st.commands.pop_front() {
            match cmd.kind {
                CmdKind::Pause => {
                    let Some(idx) = find_task(st, &cmd.arg) else {
                        self.emit_done(cmd.token, false, "ERR_NOT_FOUND", "task not found");
                        continue;
                    };
                    let mut finalize = false;
                    let mut immediate: Option<(bool, &str, &str)> = None;
                    {
                        let t = &mut st.tasks[idx];
                        if t.state.terminal() {
                            immediate = Some((false, "ERR_TASK_TERMINAL", "task already finished"));
                        } else if t.state == TaskState::Paused {
                            immediate = Some((true, "", ""));
                        } else {
                            t.generation += 1;
                            t.pause_pending = true;
                            t.pause_token = cmd.token;
                            finalize = t.inflight == 0;
                        }
                    }
                    if let Some((ok, code, message)) = immediate {
                        self.emit_done(cmd.token, ok, code, message);
                    } else if finalize {
                        self.finalize_pause(st, idx);
                    }
                }
                CmdKind::Resume => {
                    let Some(idx) = find_task(st, &cmd.arg) else {
                        self.emit_done(cmd.token, false, "ERR_NOT_FOUND", "task not found");
                        continue;
                    };
                    let mut events = Vec::new();
                    {
                        let t = &mut st.tasks[idx];
                        if t.state.terminal() {
                            self.emit_done(cmd.token, false, "ERR_TASK_TERMINAL", "task already finished");
                            continue;
                        }
                        if t.state == TaskState::Paused {
                            if t.sink.is_none() {
                                t.state = TaskState::Queued;
                                t.probe_requested = false;
                                t.probe_retry_at = 0;
                            } else {
                                t.state = TaskState::Downloading;
                                t.last_progress_at = now_ms();
                                if t.mode == Mode::Segmented {
                                    t.next_seg = 0;
                                    for s in t.segs.iter_mut() {
                                        if !s.done {
                                            s.active = false;
                                            s.retry_at = 0;
                                        }
                                    }
                                } else {
                                    t.single_ready = true;
                                    t.single_retry_at = 0;
                                }
                            }
                            events.push(state_event(t));
                        }
                    }
                    for e in events {
                        self.emit(e);
                    }
                    self.emit_done(cmd.token, true, "", "");
                }
                CmdKind::Cancel => {
                    let Some(idx) = find_task(st, &cmd.arg) else {
                        self.emit_done(cmd.token, false, "ERR_NOT_FOUND", "task not found");
                        continue;
                    };
                    let mut finalize = false;
                    {
                        let t = &mut st.tasks[idx];
                        if t.state.terminal() {
                            continue_after_done(self, cmd.token);
                        } else {
                            t.generation += 1;
                            t.cancel_pending = true;
                            t.cancel_remove_partial = cmd.flag;
                            t.cancel_token = cmd.token;
                            finalize = t.inflight == 0;
                        }
                    }
                    if finalize {
                        self.finalize_cancel(st, idx);
                    }
                }
                CmdKind::PauseAll => {
                    let mut any = false;
                    for idx in 0..st.tasks.len() {
                        let t = &mut st.tasks[idx];
                        if t.state.terminal() || t.state == TaskState::Paused {
                            continue;
                        }
                        t.generation += 1;
                        t.pause_pending = true;
                        t.pause_token = 0;
                        any = true;
                    }
                    for idx in 0..st.tasks.len() {
                        if st.tasks[idx].pause_pending && st.tasks[idx].inflight == 0 {
                            self.finalize_pause(st, idx);
                        }
                    }
                    if any {
                        st.pause_all_tokens.push(cmd.token);
                    } else {
                        self.emit_done(cmd.token, true, "", "");
                    }
                }
                CmdKind::ResumeAll => {
                    let mut events = Vec::new();
                    for idx in 0..st.tasks.len() {
                        let t = &mut st.tasks[idx];
                        if t.state != TaskState::Paused {
                            continue;
                        }
                        if t.sink.is_none() {
                            t.state = TaskState::Queued;
                            t.probe_requested = false;
                            t.probe_retry_at = 0;
                        } else {
                            t.state = TaskState::Downloading;
                            t.last_progress_at = now_ms();
                            if t.mode == Mode::Segmented {
                                t.next_seg = 0;
                                for s in t.segs.iter_mut() {
                                    if !s.done {
                                        s.active = false;
                                        s.retry_at = 0;
                                    }
                                }
                            } else {
                                t.single_ready = true;
                                t.single_retry_at = 0;
                            }
                        }
                        events.push(state_event(t));
                    }
                    for e in events {
                        self.emit(e);
                    }
                    self.emit_done(cmd.token, true, "", "");
                }
                CmdKind::CancelAll => {
                    let mut any = false;
                    for idx in 0..st.tasks.len() {
                        let t = &mut st.tasks[idx];
                        if t.state.terminal() {
                            continue;
                        }
                        t.generation += 1;
                        t.cancel_pending = true;
                        t.cancel_remove_partial = cmd.flag;
                        t.cancel_token = 0;
                        any = true;
                    }
                    for idx in 0..st.tasks.len() {
                        if st.tasks[idx].cancel_pending && st.tasks[idx].inflight == 0 {
                            self.finalize_cancel(st, idx);
                        }
                    }
                    if any {
                        st.cancel_all_tokens.push(cmd.token);
                    } else {
                        self.emit_done(cmd.token, true, "", "");
                    }
                }
                CmdKind::RemoveTask => {
                    let Some(idx) = find_task(st, &cmd.arg) else {
                        self.emit_done(cmd.token, false, "ERR_NOT_FOUND", "task not found");
                        continue;
                    };
                    // 终态任务和已暂停任务都视为“不活动”，可以移除；暂停任务需要先关句柄并清掉断点元数据。
                    let removable = st.tasks[idx].inflight == 0
                        && (st.tasks[idx].state.terminal()
                            || st.tasks[idx].state == TaskState::Paused);
                    if !removable {
                        self.emit_done(cmd.token, false, "ERR_TASK_ACTIVE", "task is still running");
                        continue;
                    }
                    let dest = PathBuf::from(&st.tasks[idx].destination);
                    // 先把文件句柄移出任务，确保 Windows 上后续删除不会因占用而失败。
                    let sink = st.tasks[idx].sink.take();
                    if let Some(sink) = &sink {
                        sink.close();
                    }
                    drop(sink);
                    let _ = fs::remove_file(meta_path(&dest));
                    // 可选：连同不完整的目标文件一起删除（终态任务则为最终文件）。
                    if cmd.flag {
                        let _ = fs::remove_file(&dest);
                    }
                    detach_task(st, idx);
                    self.emit_done(cmd.token, true, "", "");
                }
                CmdKind::ClearCompleted => {
                    let mut removed = 0;
                    for idx in (0..st.tasks.len()).rev() {
                        if st.tasks[idx].state.terminal() && st.tasks[idx].inflight == 0 {
                            detach_task(st, idx);
                            removed += 1;
                        }
                    }
                    let mut ev = Event::new(EV_LOG);
                    ev.level = "debug".to_string();
                    ev.message = format!("removed {removed} finished task(s)");
                    ev.scope = "engine".to_string();
                    self.emit(ev);
                    self.emit_done(cmd.token, true, "", "");
                }
                CmdKind::Flush => {
                    for t in st.tasks.iter() {
                        if let Some(sink) = &t.sink {
                            let _ = sink.flush();
                        }
                        if !t.state.terminal() && resume_enabled(&self.opts, t) {
                            let dest = PathBuf::from(&t.destination);
                            save_meta(&dest, t.total, &t.etag, &t.last_modified, &t.segs);
                        }
                    }
                    self.emit_done(cmd.token, true, "", "");
                }
            }
        }
    }

    fn schedule(&self, st: &mut State) {
        let now = now_ms();

        // 完成挂起的暂停/取消（无在途连接时）。
        for idx in 0..st.tasks.len() {
            if st.tasks[idx].pause_pending && st.tasks[idx].inflight == 0 {
                self.finalize_pause(st, idx);
            }
            if st.tasks[idx].cancel_pending && st.tasks[idx].inflight == 0 {
                self.finalize_cancel(st, idx);
            }
        }

        if !st.pause_all_tokens.is_empty() && !st.tasks.iter().any(|t| t.pause_pending) {
            for token in std::mem::take(&mut st.pause_all_tokens) {
                self.emit_done(token, true, "", "");
            }
        }
        if !st.cancel_all_tokens.is_empty() && !st.tasks.iter().any(|t| t.cancel_pending) {
            for token in std::mem::take(&mut st.cancel_all_tokens) {
                self.emit_done(token, true, "", "");
            }
        }

        // 为排队任务启动探针。
        let mut probing = st
            .tasks
            .iter()
            .filter(|t| t.state == TaskState::Probing)
            .count();
        let active_downloads = st
            .tasks
            .iter()
            .filter(|t| {
                t.sink.is_some() && matches!(t.state, TaskState::Downloading | TaskState::Retrying)
            })
            .count();
        for idx in 0..st.tasks.len() {
            if st.tasks[idx].state == TaskState::Queued
                && st.tasks[idx].probe_requested
                && st.tasks[idx].probe_retry_at > 0
                && st.tasks[idx].probe_retry_at <= now
                && st.tasks[idx].inflight == 0
            {
                st.tasks[idx].probe_retry_at = 0;
                st.tasks[idx].state = TaskState::Probing;
                st.tasks[idx].inflight += 1;
                let gen = st.tasks[idx].generation;
                let mut ev = state_event(&st.tasks[idx]);
                ev.state = "probing".to_string();
                st.jobs.push_back(Job {
                    kind: JobKind::Probe,
                    task: idx,
                    seg: 0,
                    gen,
                    token: 0,
                    url: String::new(),
                });
                self.emit(ev);
                self.cv.notify_one();
                continue;
            }
            if st.tasks[idx].state == TaskState::Queued && !st.tasks[idx].probe_requested {
                if probing + active_downloads >= st.max_tasks_in_flight {
                    continue;
                }
                st.tasks[idx].probe_requested = true;
                st.tasks[idx].state = TaskState::Probing;
                st.tasks[idx].inflight += 1;
                probing += 1;
                let gen = st.tasks[idx].generation;
                let ev = state_event(&st.tasks[idx]);
                st.jobs.push_back(Job {
                    kind: JobKind::Probe,
                    task: idx,
                    seg: 0,
                    gen,
                    token: 0,
                    url: String::new(),
                });
                self.emit(ev);
                self.cv.notify_one();
            }
        }

        // 单流任务：等待所有在途连接结束后启动。
        let mut host_use: HashMap<String, i64> = HashMap::new();
        for t in st.tasks.iter() {
            if !t.host.is_empty() {
                *host_use.entry(t.host.clone()).or_insert(0) += t.inflight.max(0) as i64;
            }
        }
        let max_per_host = self.opts.max_per_host.max(1) as i64;
        for idx in 0..st.tasks.len() {
            let t = &st.tasks[idx];
            if t.mode != Mode::Segmented
                && matches!(t.state, TaskState::Downloading | TaskState::Retrying)
                && t.single_ready
                && !t.single_queued
                && t.inflight == 0
                && !t.pause_pending
                && !t.cancel_pending
                && t.single_retry_at <= now
                && host_has_room(&host_use, &t.host, max_per_host)
            {
                let gen = t.generation;
                let host = t.host.clone();
                let t = &mut st.tasks[idx];
                t.single_queued = true;
                t.single_ready = false;
                t.state = TaskState::Downloading;
                t.inflight += 1;
                if !host.is_empty() {
                    *host_use.entry(host).or_insert(0) += 1;
                }
                st.jobs.push_back(Job {
                    kind: JobKind::Single,
                    task: idx,
                    seg: 0,
                    gen,
                    token: 0,
                    url: String::new(),
                });
                self.cv.notify_one();
            }
        }

        // 到期重试的分段。
        for idx in 0..st.tasks.len() {
            if st.tasks[idx].mode != Mode::Segmented
                || !matches!(
                    st.tasks[idx].state,
                    TaskState::Downloading | TaskState::Retrying
                )
                || st.tasks[idx].pause_pending
                || st.tasks[idx].cancel_pending
                || st.tasks[idx].verify_requested
            {
                continue;
            }
            let mut push: Vec<usize> = Vec::new();
            for (si, seg) in st.tasks[idx].segs.iter().enumerate() {
                if !seg.done && !seg.active && seg.retry_at > 0 && seg.retry_at <= now {
                    push.push(si);
                }
            }
            for si in push {
                let host = st.tasks[idx].host.clone();
                if !host_has_room(&host_use, &host, max_per_host) {
                    break;
                }
                let gen = st.tasks[idx].generation;
                st.tasks[idx].state = TaskState::Downloading;
                st.tasks[idx].segs[si].active = true;
                st.tasks[idx].segs[si].retry_at = 0;
                st.tasks[idx].inflight += 1;
                if !host.is_empty() {
                    *host_use.entry(host).or_insert(0) += 1;
                }
                st.jobs.push_back(Job {
                    kind: JobKind::Segment,
                    task: idx,
                    seg: si,
                    gen,
                    token: 0,
                    url: String::new(),
                });
                self.cv.notify_one();
            }
        }

        // 连接预算分配（基础 1 + 轮询追加）。
        let mut eligible: Vec<usize> = Vec::new();
        for idx in 0..st.tasks.len() {
            let t = &st.tasks[idx];
            if t.mode == Mode::Segmented
                && matches!(t.state, TaskState::Downloading | TaskState::Retrying)
                && t.sink.is_some()
                && !t.pause_pending
                && !t.cancel_pending
                && !t.verify_requested
            {
                eligible.push(idx);
            }
        }
        // 连接预算优先给高优先级任务（数值越大越优先），同优先级按提交顺序。
        eligible.sort_by(|&a, &b| st.tasks[b].priority.cmp(&st.tasks[a].priority));
        let mut in_flight: i64 = eligible
            .iter()
            .map(|&i| st.tasks[i].inflight.max(0) as i64)
            .sum();
        let budget = st.effective as i64;

        for idx in 0..eligible.len() {
            let i = eligible[idx];
            if in_flight >= budget {
                break;
            }
            if st.tasks[i].inflight == 0 {
                let host = st.tasks[i].host.clone();
                if !host_has_room(&host_use, &host, max_per_host) {
                    continue;
                }
                if let Some(si) = take_segment(&mut st.tasks[i], now) {
                    let gen = st.tasks[i].generation;
                    st.tasks[i].segs[si].active = true;
                    st.tasks[i].inflight += 1;
                    if !host.is_empty() {
                        *host_use.entry(host).or_insert(0) += 1;
                    }
                    st.jobs.push_back(Job {
                        kind: JobKind::Segment,
                        task: i,
                        seg: si,
                        gen,
                        token: 0,
                        url: String::new(),
                    });
                    in_flight += 1;
                    self.cv.notify_one();
                }
            }
        }

        loop {
            let mut progressed = false;
            for &i in &eligible {
                if in_flight >= budget {
                    break;
                }
                let limit = if st.tasks[i].opt.max_concurrency > 0 {
                    st.tasks[i].opt.max_concurrency
                } else {
                    st.max_concurrency
                };
                if st.tasks[i].inflight as usize >= limit {
                    continue;
                }
                let host = st.tasks[i].host.clone();
                if !host_has_room(&host_use, &host, max_per_host) {
                    continue;
                }
                if let Some(si) = take_segment(&mut st.tasks[i], now) {
                    let gen = st.tasks[i].generation;
                    st.tasks[i].segs[si].active = true;
                    st.tasks[i].inflight += 1;
                    if !host.is_empty() {
                        *host_use.entry(host).or_insert(0) += 1;
                    }
                    st.jobs.push_back(Job {
                        kind: JobKind::Segment,
                        task: i,
                        seg: si,
                        gen,
                        token: 0,
                        url: String::new(),
                    });
                    in_flight += 1;
                    progressed = true;
                    self.cv.notify_one();
                }
            }
            if !progressed || in_flight >= budget {
                break;
            }
        }

        // 进度与统计。
        let interval = self.opts.progress_interval_ms.max(20);
        if now - st.last_progress_at >= interval {
            st.last_progress_at = now;
            let mut events = Vec::new();
            for idx in 0..st.tasks.len() {
                let t = &mut st.tasks[idx];
                if t.state.terminal() {
                    continue;
                }
                let dt = (now - t.last_progress_at) as f64;
                if dt > 0.0 {
                    let delta = (t.session_bytes - t.last_session) as f64;
                    let inst = delta * 1000.0 / dt;
                    t.speed = if t.speed <= 0.0 {
                        inst
                    } else {
                        t.speed * 0.7 + inst * 0.3
                    };
                    t.last_session = t.session_bytes;
                    t.last_progress_at = now;
                }
                events.push(progress_event(t));
            }
            for e in events {
                self.emit(e);
            }
        }

        if now - st.last_stats_at >= 1000 {
            st.last_stats_at = now;
            self.update_adaptive(st, now);
            let stats = self.stats_from_state(st);
            self.emit(stats);
        }
    }

    fn update_adaptive(&self, st: &mut State, now: i64) {
        let mut speed_total = 0.0;
        for t in &st.tasks {
            if !t.state.terminal() {
                speed_total += t.speed;
            }
        }
        let (cpu_hi, mem_lo, lat_hi) = match st.policy {
            Policy::FavorOtherApps => (60.0, 25.0, 25.0),
            Policy::MaxPerformance => (98.0, 8.0, 80.0),
            Policy::Balanced => (85.0, 12.0, 40.0),
        };
        let saturated = st.cpu_pct > cpu_hi
            || st.mem_avail_pct < mem_lo
            || st.disk_latency_ms > lat_hi
            || st.error_rate() > 0.05;
        if saturated {
            st.effective = (st.effective * 3 / 4).max(1);
        } else if st.last_adapt_speed > 0.0 && speed_total > st.last_adapt_speed * 1.05 {
            let step = match st.policy {
                Policy::FavorOtherApps => 1,
                Policy::Balanced => 2,
                Policy::MaxPerformance => 4,
            };
            st.effective = (st.effective + step).min(policy_cap(st.policy, st.max_concurrency));
        } else if st.last_adapt_speed > 0.0 && speed_total < st.last_adapt_speed * 0.5 {
            st.effective = st.effective.saturating_sub(1).max(1);
        }
        st.effective = st
            .effective
            .clamp(1, policy_cap(st.policy, st.max_concurrency));
        st.last_adapt_at = now;
        st.last_adapt_speed = speed_total;
    }
}

fn continue_after_done(engine: &Engine, token: i32) {
    engine.emit_done(token, true, "", "");
}
