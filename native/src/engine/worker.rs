use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::file::{self, FileSink};
use crate::http::{host_of, HttpWorker};
use crate::win;
use super::resume::{build_segments, load_meta, meta_path};
use super::*;

// ---------------- workers ----------------

impl Engine {
    pub(super) fn worker_loop(&self) {
        let mut http = HttpWorker::new(self.opts.clone(), Arc::clone(&self.abort));
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let policy = {
                let st = self.state.lock().unwrap();
                st.policy
            };
            win::apply_thread_priority(policy);

            let job = {
                let mut st = self.state.lock().unwrap();
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(j) = st.jobs.pop_front() {
                        break Some(j);
                    }
                    let result = self.cv.wait_timeout(st, Duration::from_millis(250)).unwrap();
                    st = result.0;
                    if result.1.timed_out() && self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                }
            };
            let Some(job) = job else {
                return;
            };

            match job.kind {
                JobKind::Probe => {
                    if let Some(idx) = self.valid_task(&job) {
                        self.run_probe(idx, &mut http);
                    }
                }
                JobKind::Segment => {
                    if let Some(idx) = self.valid_task(&job) {
                        if job.seg < {
                            let st = self.state.lock().unwrap();
                            st.tasks[idx].segs.len()
                        } {
                            self.run_segment(idx, job.seg, job.gen, &mut http);
                        }
                    }
                }
                JobKind::Single => {
                    if let Some(idx) = self.valid_task(&job) {
                        self.run_single(idx, job.gen, &mut http);
                    }
                }
                JobKind::Verify => {
                    if let Some(idx) = self.valid_task(&job) {
                        self.run_verify(idx);
                    }
                }
                JobKind::StandaloneProbe => {
                    self.run_standalone_probe(job.token, &job.url, &mut http);
                }
            }

            if job.kind != JobKind::StandaloneProbe && job.task != usize::MAX {
                let mut st = self.state.lock().unwrap();
                let t = &mut st.tasks[job.task];
                if t.inflight > 0 {
                    t.inflight -= 1;
                }
                drop(st);
            }
            self.tick.notify_all();
        }
    }

    fn valid_task(&self, job: &Job) -> Option<usize> {
        let st = self.state.lock().unwrap();
        if job.task >= st.tasks.len() {
            return None;
        }
        Some(job.task)
    }

    // ---------------- probes ----------------

    fn run_probe(&self, idx: usize, http: &mut HttpWorker) {
        let (urls, headers) = {
            let st = self.state.lock().unwrap();
            let t = &st.tasks[idx];
            if t.cancel_pending || t.pause_pending {
                return;
            }
            let mut urls = vec![t.url.clone()];
            for m in &t.opt.mirrors {
                if !m.is_empty() {
                    urls.push(m.clone());
                }
            }
            (urls, t.request_headers.clone())
        };

        let mut probe: Option<ProbeResult> = None;
        for url in &urls {
            let pr = http.probe(url, &headers);
            if pr.error.is_empty() && pr.status != 0 {
                probe = Some(pr);
                break;
            }
            probe = Some(pr);
        }
        let Some(pr) = probe else {
            return;
        };

        enum After {
            Nothing,
            Retry(String),
            Fail(String, String),
            Completed,
            Ready,
        }
        let mut after = After::Nothing;
        let mut events: Vec<Event> = Vec::new();
        let mut finish_skipped = false;

        let status_retryable = matches!(pr.status, 408 | 425 | 429) || pr.status >= 500;
        let status_fatal = pr.status >= 400 && !status_retryable;
        let failed = !pr.error.is_empty() || pr.status >= 400;
        let failure_msg = if pr.error.is_empty() {
            format!("server returned HTTP {}", pr.status)
        } else {
            pr.error.clone()
        };

        {
            let mut st = self.state.lock().unwrap();
            if failed {
                st.error_count += 1;
            } else {
                st.success_count += 1;
            }
            let t = &mut st.tasks[idx];
            if t.cancel_pending || t.pause_pending || t.state == TaskState::Paused {
                return;
            }
            t.attempts += 1;
            if failed {
                t.probe_attempts += 1;
                if status_fatal {
                    after = After::Fail("ERR_HTTP_STATUS".to_string(), failure_msg);
                } else if t.probe_attempts > self.opts.max_retries {
                    after = After::Fail("ERR_PROBE_FAILED".to_string(), failure_msg);
                } else {
                    t.state = TaskState::Queued;
                    t.probe_retry_at = now_ms()
                        + backoff_ms(
                            self.opts.base_retry_ms,
                            self.opts.max_retry_ms,
                            t.probe_attempts,
                        );
                    events.push(state_event(t));
                    after = After::Retry(failure_msg);
                }
            } else {
                t.final_url = if pr.final_url.is_empty() {
                    t.url.clone()
                } else {
                    pr.final_url.clone()
                };
                t.etag = pr.etag.clone();
                t.last_modified = pr.last_modified.clone();
                t.content_encoding = pr.content_encoding.clone();
                t.content_type = pr.content_type.clone();
                t.accept_ranges = pr.range_supported;
                t.host = host_of(&t.final_url);
                let mut total = pr.content_length;
                if total < 0 && t.opt.expected_size > 0 {
                    total = t.opt.expected_size;
                }
                t.total = total;

                let dest = PathBuf::from(&t.destination);
                let exists = file::path_exists(&dest);
                let resume_on = resume_enabled(&self.opts, t);
                let meta_exists = resume_on && file::path_exists(&meta_path(&dest));
                let overwrite = lower(&t.opt.overwrite);

                if exists && !meta_exists && overwrite == "skip" {
                    finish_skipped = true;
                } else if exists && !meta_exists && overwrite == "error" {
                    after = After::Fail(
                        "ERR_FILE_EXISTS".to_string(),
                        "destination already exists".to_string(),
                    );
                } else if total == 0 {
                    match FileSink::open(&dest, 0, false, true, self.opts.create_directories) {
                        Ok(sink) => sink.close(),
                        Err(err) => {
                            after = After::Fail(
                                "ERR_IO".to_string(),
                                format!("create destination failed: {err}"),
                            );
                        }
                    }
                    if matches!(after, After::Nothing) {
                        if t.started_at == 0 {
                            t.started_at = now_ms();
                        }
                        t.finished_at = now_ms();
                        t.state = TaskState::Completed;
                        events.push(state_event(t));
                        events.push(progress_event(t));
                        events.push(completed_event(t));
                        after = After::Completed;
                    }
                } else {
                    let mut mode = Mode::Segmented;
                    let mut reason = DegradeReason::None;
                    if total < 0 {
                        mode = Mode::UnknownLength;
                        reason = DegradeReason::UnknownLength;
                    } else if total < self.opts.small_file_threshold {
                        mode = Mode::Single;
                        reason = DegradeReason::SmallFile;
                    } else if !pr.range_supported {
                        mode = Mode::Single;
                        reason = DegradeReason::NoRangeSupport;
                    } else if win::is_remote_volume(&dest) {
                        mode = Mode::Single;
                        reason = DegradeReason::RemoteVolume;
                    }
                    t.mode = mode;
                    t.degraded = mode != Mode::Segmented;
                    t.reason = reason;

                    let resume_on = resume_enabled(&self.opts, t);
                    let mut truncate = true;
                    let mut loaded_meta = false;
                    if mode == Mode::Segmented && resume_on {
                        let had_meta = file::path_exists(&meta_path(&dest));
                        if let Some(mut segs) =
                            load_meta(&dest, total, &t.etag, &t.last_modified, file::path_size(&dest))
                        {
                            segs.iter_mut().for_each(|s| {
                                s.active = false;
                                s.retry_at = 0;
                                if s.written > s.end - s.start {
                                    s.written = s.end - s.start + 1;
                                    s.done = true;
                                }
                            });
                            t.downloaded = segs.iter().map(|s| s.written).sum();
                            t.resumed_bytes = t.downloaded;
                            t.segs = segs;
                            t.next_seg = 0;
                            truncate = false;
                            loaded_meta = true;
                        } else if had_meta {
                            t.degraded = true;
                            t.reason = DegradeReason::ValidatorChanged;
                        }
                    }
                    if !loaded_meta {
                        t.downloaded = 0;
                        t.resumed_bytes = 0;
                        t.segs.clear();
                        t.next_seg = 0;
                        if mode == Mode::Segmented {
                            t.segs = build_segments(
                                total,
                                if t.opt.segment_size > 0 {
                                    t.opt.segment_size
                                } else {
                                    self.opts.segment_size
                                },
                                self.opts.max_segments_per_task,
                            );
                        }
                    }

                    match FileSink::open(
                        &dest,
                        total,
                        self.opts.preallocate,
                        truncate,
                        self.opts.create_directories,
                    ) {
                        Ok(sink) => {
                            t.sink = Some(sink);
                        }
                        Err(err) => {
                            after = After::Fail(
                                "ERR_IO".to_string(),
                                format!("open destination failed: {err}"),
                            );
                        }
                    }

                    if matches!(after, After::Nothing) {
                        t.state = TaskState::Downloading;
                        if t.started_at == 0 {
                            t.started_at = now_ms();
                        }
                        if t.mode != Mode::Segmented {
                            t.single_ready = true;
                            t.single_queued = false;
                            t.single_retry_at = 0;
                        }
                        t.last_progress_at = now_ms();
                        t.last_session = t.session_bytes;
                        events.push(state_event(t));
                        if t.degraded
                            && t.reason != DegradeReason::SmallFile
                            && t.reason != DegradeReason::UnknownLength
                        {
                            events.push(degraded_event(t, t.reason));
                        }
                        if t.segs.iter().all(|s| s.done) && !t.segs.is_empty() {
                            self.begin_verify_locked(&mut st, idx, &mut events);
                        }
                        after = After::Ready;
                    }
                }
            }
        }

        for e in events {
            self.emit(e);
        }
        if finish_skipped {
            self.finish_skipped(idx);
            return;
        }
        match after {
            After::Retry(err) => {
                self.emit_log("warn", &format!("probe failed, retrying: {err}"), None);
            }
            After::Fail(code, message) => {
                self.fail_task(idx, &code, &message);
            }
            _ => {}
        }
    }

    fn run_standalone_probe(&self, token: i32, url: &str, http: &mut HttpWorker) {
        let pr = http.probe(url, &self.opts.headers);
        let mut e = Event::new(EV_PROBE);
        e.token = token;
        e.ok = pr.error.is_empty() && pr.status > 0 && pr.status < 400;
        e.status_code = pr.status;
        e.content_length = pr.content_length;
        e.accept_ranges = pr.range_supported;
        e.final_url = pr.final_url;
        e.etag = pr.etag;
        e.last_modified = pr.last_modified;
        e.content_type = pr.content_type;
        e.message = if pr.error.is_empty() && pr.status >= 400 {
            format!("server returned HTTP {}", pr.status)
        } else {
            pr.error
        };
        self.emit(e);
    }
}
