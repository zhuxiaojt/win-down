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

const MIN_SPLIT_FLOOR: i64 = 128 * 1024;

// ---------------- scheduler ----------------

impl Engine {
    pub(super) fn scheduler_loop(&self) {
        let mut spawned_need = 0usize;
        loop {
            let mut st = self.state.lock().unwrap();
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            self.process_commands(&mut st);
            self.schedule(&mut st);
            let demand = (st.jobs.len() + st.running_jobs).min(st.effective);
            drop(st);
            if demand > spawned_need {
                self.ensure_workers(demand);
                spawned_need = demand;
            }
            let st = self.state.lock().unwrap();
            let result = self.tick.wait_timeout(st, Duration::from_millis(20)).unwrap();
            drop(result.0);
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
                    let mut reverify: Option<usize> = None;
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
                                    if t.verify_requested && t.segs.iter().all(|s| s.done) {
                                        t.verify_requested = false;
                                        reverify = Some(idx);
                                    }
                                } else {
                                    t.single_ready = true;
                                    t.single_retry_at = 0;
                                    t.single_queued = false;
                                }
                            }
                            if reverify.is_none() {
                                events.push(state_event(t));
                            }
                        }
                    }
                    for e in events {
                        self.emit(e);
                    }
                    if let Some(vidx) = reverify {
                        let mut rev_events = Vec::new();
                        self.begin_verify_locked(st, vidx, &mut rev_events);
                        for e in rev_events {
                            self.emit(e);
                        }
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
                    let mut reverify: Vec<usize> = Vec::new();
                    for idx in 0..st.tasks.len() {
                        let t = &mut st.tasks[idx];
                        if t.state != TaskState::Paused {
                            continue;
                        }
                        let mut do_reverify = false;
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
                                if t.verify_requested && t.segs.iter().all(|s| s.done) {
                                    t.verify_requested = false;
                                    reverify.push(idx);
                                    do_reverify = true;
                                }
                            } else {
                                t.single_ready = true;
                                t.single_retry_at = 0;
                                t.single_queued = false;
                            }
                        }
                        if !do_reverify {
                            events.push(state_event(t));
                        }
                    }
                    for e in events {
                        self.emit(e);
                    }
                    for vidx in reverify {
                        let mut rev_events = Vec::new();
                        self.begin_verify_locked(st, vidx, &mut rev_events);
                        for e in rev_events {
                            self.emit(e);
                        }
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
                    let removable = st.tasks[idx].inflight == 0
                        && (st.tasks[idx].state.terminal()
                            || st.tasks[idx].state == TaskState::Paused);
                    if !removable {
                        self.emit_done(cmd.token, false, "ERR_TASK_ACTIVE", "task is still running");
                        continue;
                    }
                    let dest = PathBuf::from(&st.tasks[idx].destination);
                    let sink = st.tasks[idx].sink.take();
                    if let Some(sink) = &sink {
                        sink.close();
                    }
                    drop(sink);
                    let _ = fs::remove_file(meta_path(&dest));
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
        eligible.sort_by(|&a, &b| st.tasks[b].priority.cmp(&st.tasks[a].priority));
        let mut in_flight: i64 = eligible
            .iter()
            .map(|&i| st.tasks[i].inflight.max(0) as i64)
            .sum();
        let budget = st.effective as i64;

        for &i in &eligible {
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

        while in_flight < budget {
            let Some((ti, si)) = self.pick_split_candidate(st, &host_use, max_per_host) else {
                break;
            };
            let gen = st.tasks[ti].generation;
            let host = st.tasks[ti].host.clone();
            let new_idx = {
                let t = &mut st.tasks[ti];
                let s = &t.segs[si];
                let rem = (s.end - s.start + 1) - s.written;
                let keep = rem / 2;
                let split_at = s.start + s.written + keep;
                let old_end = s.end;
                t.segs[si].end = split_at - 1;
                t.segs.push(Segment {
                    start: split_at,
                    end: old_end,
                    ..Default::default()
                });
                t.segs.len() - 1
            };
            st.tasks[ti].segs[new_idx].active = true;
            st.tasks[ti].inflight += 1;
            if !host.is_empty() {
                *host_use.entry(host).or_insert(0) += 1;
            }
            st.jobs.push_back(Job {
                kind: JobKind::Segment,
                task: ti,
                seg: new_idx,
                gen,
                token: 0,
                url: String::new(),
            });
            in_flight += 1;
            self.cv.notify_one();
        }

        let interval = self.opts.progress_interval_ms.max(20);
        if now - st.last_progress_at >= interval {
            st.last_progress_at = now;
            let mut events = Vec::new();
            for idx in 0..st.tasks.len() {
                let t = &mut st.tasks[idx];
                if t.state.terminal() {
                    continue;
                }
                let win_dt = now - t.last_progress_at;
                if win_dt >= 500 {
                    let delta = (t.session_bytes - t.last_session).max(0) as f64;
                    t.speed = delta * 1000.0 / win_dt as f64;
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

    fn pick_split_candidate(
        &self,
        st: &State,
        host_use: &HashMap<String, i64>,
        max_per_host: i64,
    ) -> Option<(usize, usize)> {
        let max_segs = self.opts.max_segments_per_task.max(1);
        let mut best: Option<(usize, usize, i64)> = None;
        for (ti, t) in st.tasks.iter().enumerate() {
            if t.mode != Mode::Segmented
                || !matches!(t.state, TaskState::Downloading | TaskState::Retrying)
                || t.sink.is_none()
                || t.pause_pending
                || t.cancel_pending
                || t.verify_requested
                || t.segs.len() >= max_segs
                || !host_has_room(host_use, &t.host, max_per_host)
            {
                continue;
            }
            let limit = if t.opt.max_concurrency > 0 {
                t.opt.max_concurrency
            } else {
                st.max_concurrency
            };
            if t.inflight as usize >= limit {
                continue;
            }
            let alive = t.segs.iter().filter(|s| !s.done).count();
            if alive >= limit {
                continue;
            }
            if !st.split_ok && alive * 2 > limit {
                continue;
            }
            let per_conn = if t.inflight > 0 {
                t.speed / t.inflight as f64
            } else {
                0.0
            };
            let mut best_idle: Option<(usize, usize, i64)> = None;
            let mut best_running: Option<(usize, usize, i64)> = None;
            for (si, s) in t.segs.iter().enumerate() {
                if !s.active || s.done {
                    continue;
                }
                let rem = (s.end - s.start + 1) - s.written;
                if rem < MIN_SPLIT_FLOOR * 2 {
                    continue;
                }
                if per_conn > 0.0 {
                    let need = if s.running { 1.0 } else { 0.5 };
                    if (rem as f64) < per_conn * need {
                        continue;
                    }
                } else if s.running {
                    continue;
                } else if rem < MIN_SPLIT_FLOOR * 4 {
                    continue;
                }
                let slot = if s.running {
                    &mut best_running
                } else {
                    &mut best_idle
                };
                if slot.map_or(true, |(_, _, r)| rem > r) {
                    *slot = Some((ti, si, rem));
                }
            }
            if let Some(p) = best_idle.or(best_running) {
                if best.map_or(true, |(_, _, r)| p.2 > r) {
                    best = Some(p);
                }
            }
        }
        best.map(|(ti, si, _)| (ti, si))
    }

    fn update_adaptive(&self, st: &mut State, now: i64) {
        let mut speed_total = 0.0;
        for t in &st.tasks {
            if !t.state.terminal() {
                speed_total += t.speed;
            }
        }
        let (cpu_hi, mem_lo, lat_hi) = match st.policy {
            Policy::FavorOtherApps => (65.0, 20.0, 30.0),
            Policy::MaxPerformance => (98.0, 6.0, 80.0),
            Policy::Balanced => (90.0, 10.0, 50.0),
        };
        let saturated = st.cpu_pct > cpu_hi
            || st.mem_avail_pct < mem_lo
            || st.disk_latency_ms > lat_hi
            || st.error_rate() > 0.05;

        let resource = self.resource_target(st);
        let limit = st.max_concurrency.max(1).min(MAX_WORKERS);
        let mut want = resource.min(limit);

        let speed_improved = st.last_adapt_speed > 0.0 && speed_total > st.last_adapt_speed * 1.02;
        let inflight_now = st
            .tasks
            .iter()
            .filter(|t| !t.state.terminal())
            .map(|t| t.inflight.max(0) as usize)
            .sum::<usize>();
        let conn_dropped = st.last_adapt_inflight > inflight_now;

        if saturated {
            want = (want * 4 / 5).max(1);
            st.split_ok = false;
            st.split_off_at = now;
        } else if speed_improved || conn_dropped {
            st.split_ok = true;
        } else if st.last_adapt_speed > 0.0 {
            if st.split_ok {
                st.split_off_at = now;
            }
            st.split_ok = false;
        }
        st.effective = want;

        if !st.split_ok && st.split_off_at > 0 && now - st.split_off_at >= 5_000 {
            st.split_ok = true;
            st.split_off_at = 0;
        }

        st.last_adapt_at = now;
        st.last_adapt_speed = speed_total;
        st.last_adapt_inflight = inflight_now;
    }
}

fn continue_after_done(engine: &Engine, token: i32) {
    engine.emit_done(token, true, "", "");
}
