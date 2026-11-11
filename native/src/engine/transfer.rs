use std::fs;
use std::path::PathBuf;

use crate::file;
use crate::hash;
use crate::http::{Flow, HttpWorker, ResponseHead, StreamCtx};
use super::resume::meta_path;
use super::*;

fn parse_content_range_start(value: &str) -> Option<i64> {
    let value = value.trim();
    let rest = value.strip_prefix("bytes")?.trim();
    let (range, _) = rest.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    start.trim().parse().ok()
}

// ---------------- segments ----------------

impl Engine {
    pub(super) fn run_segment(&self, idx: usize, seg_idx: usize, gen: u64, http: &mut HttpWorker) {
        let snapshot = {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.generation != gen
                || !matches!(t.state, TaskState::Downloading | TaskState::Retrying)
                || t.pause_pending
                || t.cancel_pending
            {
                if t.generation == gen && seg_idx < t.segs.len() {
                    t.segs[seg_idx].active = false;
                }
                return;
            }
            let s = &t.segs[seg_idx];
            if s.done {
                return;
            }
            let pos = s.start + s.written;
            (
                if t.final_url.is_empty() {
                    t.url.clone()
                } else {
                    t.final_url.clone()
                },
                t.request_headers.clone(),
                s.start,
                s.end,
                pos,
                t.etag.clone(),
                t.mode,
            )
        };
        let (url, headers, start, end, pos, etag, mode) = snapshot;
        if mode != Mode::Segmented {
            return;
        }
        if pos > end {
            self.complete_segment(idx, seg_idx, gen);
            return;
        }

        let mut head_status: u32 = 0;
        let mut range_ignored = false;
        let mut range_mismatch = false;
        let mut bad_encoding = false;
        let mut written: i64 = 0;
        let mut write_error: Option<String> = None;

        let result = {
            let mut on_head = |head: &ResponseHead| -> Flow {
                head_status = head.status;
                if head.status == 200 {
                    range_ignored = true;
                    return Flow::Stop;
                }
                if head.status != 206 {
                    return Flow::Stop;
                }
                match parse_content_range_start(&head.content_range) {
                    Some(cr_start) if cr_start == pos => {}
                    _ => {
                        range_mismatch = true;
                        return Flow::Stop;
                    }
                }
                let enc = head.content_encoding.to_ascii_lowercase();
                if !enc.is_empty() && enc != "identity" {
                    bad_encoding = true;
                    return Flow::Stop;
                }
                Flow::Continue
            };
            let mut on_data = |chunk: &[u8]| -> Flow {
                let sink = {
                    let st = self.state.lock().unwrap();
                    let t = &st.tasks[idx];
                    if t.generation != gen
                        || !matches!(t.state, TaskState::Downloading | TaskState::Retrying)
                        || t.pause_pending
                        || t.cancel_pending
                    {
                        return Flow::Stop;
                    }
                    t.sink.clone()
                };
                let Some(sink) = sink else {
                    write_error = Some("sink closed".to_string());
                    return Flow::Stop;
                };
                let offset = pos + written;
                let remaining = end + 1 - offset;
                if remaining <= 0 {
                    return Flow::Continue;
                }
                let take = (chunk.len() as i64).min(remaining) as usize;
                if let Err(err) = sink.write_at(offset, &chunk[..take]) {
                    write_error = Some(err.to_string());
                    return Flow::Stop;
                }
                let len = take as i64;
                written += len;
                let mut st = self.state.lock().unwrap();
                let t = &mut st.tasks[idx];
                t.downloaded += len;
                t.session_bytes += len;
                Flow::Continue
            };
            let mut ctx = StreamCtx {
                on_head: &mut on_head,
                on_data: &mut on_data,
            };
            http.fetch(
                "GET",
                &url,
                &headers,
                Some((pos, end)),
                if etag.is_empty() {
                    None
                } else {
                    Some(etag.as_str())
                },
                &mut ctx,
            )
        };

        {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.generation != gen {
                if seg_idx < t.segs.len() {
                    t.segs[seg_idx].active = false;
                }
                return;
            }
        }

        let seg_len = end - start + 1;
        let initial = {
            let st = self.state.lock().unwrap();
            st.tasks[idx].segs[seg_idx].written
        };

        if let Some(err) = write_error {
            let mut st = self.state.lock().unwrap();
            st.tasks[idx].segs[seg_idx].active = false;
            drop(st);
            self.fail_task(idx, "ERR_IO", &format!("write failed: {err}"));
            return;
        }

        if range_ignored {
            let mut st = self.state.lock().unwrap();
            st.tasks[idx].segs[seg_idx].active = false;
            drop(st);
            if !self.start_single(idx, DegradeReason::RangeIgnored) {
                let mut st = self.state.lock().unwrap();
                st.tasks[idx].segs[seg_idx].retry_at = now_ms() + self.opts.base_retry_ms;
            }
            return;
        }
        if range_mismatch {
            let mut st = self.state.lock().unwrap();
            st.tasks[idx].segs[seg_idx].active = false;
            drop(st);
            if !self.start_single(idx, DegradeReason::RangeMismatch) {
                let mut st = self.state.lock().unwrap();
                st.tasks[idx].segs[seg_idx].retry_at = now_ms() + self.opts.base_retry_ms;
            }
            return;
        }
        if bad_encoding {
            let mut st = self.state.lock().unwrap();
            st.tasks[idx].segs[seg_idx].active = false;
            drop(st);
            self.start_single(idx, DegradeReason::ContentEncoding);
            return;
        }

        let new_written = initial + written;
        let status = head_status;

        let success = match &result {
            Ok(head) => !range_ignored && !bad_encoding && head.status == 206,
            Err(_) => false,
        };

        if success && new_written >= seg_len {
            let mut st = self.state.lock().unwrap();
            st.success_count += 1;
            let t = &mut st.tasks[idx];
            t.segs[seg_idx].active = false;
            t.segs[seg_idx].written = seg_len;
            t.segs[seg_idx].done = true;
            t.segs[seg_idx].retries = 0;
            t.max_conn_used = t.max_conn_used.max(t.inflight);
            let all_done = t.segs.iter().all(|s| s.done);
            drop(st);
            if all_done {
                self.begin_verify(idx);
            } else {
                self.tick.notify_all();
            }
            return;
        }

        let mut st = self.state.lock().unwrap();
        let t = &mut st.tasks[idx];
        if t.state.terminal() {
            return;
        }
        t.segs[seg_idx].active = false;
        t.segs[seg_idx].written = new_written.min(seg_len);
        if t.pause_pending || t.cancel_pending {
            t.downloaded = t.segs.iter().map(|s| s.written).sum();
            return;
        }
        st.error_count += 1;
        let retryable = status == 0
            || status == 408
            || status == 425
            || status == 429
            || status >= 500
            || matches!(&result, Ok(_));
        if !retryable && status != 0 {
            let msg = format!("server returned HTTP {status}");
            drop(st);
            self.fail_task(idx, "ERR_HTTP_STATUS", &msg);
            return;
        }
        let seg_retries = {
            let t = &mut st.tasks[idx];
            t.segs[seg_idx].retries += 1;
            t.attempts += 1;
            t.segs[seg_idx].retries
        };
        if seg_retries > self.opts.max_retries {
            drop(st);
            self.start_single(idx, DegradeReason::ServerErrorStorm);
            return;
        }
        let ev = {
            let t = &mut st.tasks[idx];
            t.segs[seg_idx].retry_at = now_ms()
                + backoff_ms(
                    self.opts.base_retry_ms,
                    self.opts.max_retry_ms,
                    seg_retries,
                );
            t.state = TaskState::Retrying;
            t.error_message = match &result {
                Err(err) => err.clone(),
                Ok(_) => format!("incomplete segment ({} bytes)", new_written),
            };
            state_event(t)
        };
        drop(st);
        self.emit(ev);
    }

    fn complete_segment(&self, idx: usize, seg_idx: usize, _gen: u64) {
        let all_done = {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if seg_idx < t.segs.len() {
                t.segs[seg_idx].done = true;
                t.segs[seg_idx].written = t.segs[seg_idx].end - t.segs[seg_idx].start + 1;
                t.segs[seg_idx].active = false;
            }
            t.segs.iter().all(|s| s.done)
        };
        if all_done {
            self.begin_verify(idx);
        }
    }

    fn begin_verify(&self, idx: usize) {
        let mut events = Vec::new();
        {
            let mut st = self.state.lock().unwrap();
            self.begin_verify_locked(&mut st, idx, &mut events);
        }
        for e in events {
            self.emit(e);
        }
    }

    pub(super) fn begin_verify_locked(&self, st: &mut State, idx: usize, events: &mut Vec<Event>) {
        let t = &mut st.tasks[idx];
        if t.verify_requested || t.state.terminal() {
            return;
        }
        t.verify_requested = true;
        t.state = TaskState::Verifying;
        t.inflight += 1;
        events.push(state_event(t));
        st.jobs.push_back(Job {
            kind: JobKind::Verify,
            task: idx,
            seg: 0,
            gen: t.generation,
            token: 0,
            url: String::new(),
        });
        self.cv.notify_one();
    }

    // ---------------- single stream ----------------

    pub(super) fn run_single(&self, idx: usize, gen: u64, http: &mut HttpWorker) {
        let snapshot = {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.generation != gen || t.state != TaskState::Downloading {
                t.single_queued = false;
                return;
            }
            t.single_queued = false;
            t.downloaded = 0;
            t.session_bytes = 0;
            t.cursor = 0;
            t.last_session = 0;
            t.last_progress_at = now_ms();
            t.segs.clear();
            let ev = reset_event(t);
            let url = if t.final_url.is_empty() {
                t.url.clone()
            } else {
                t.final_url.clone()
            };
            (url, t.request_headers.clone(), t.total, ev)
        };
        let (url, headers, known_total, reset) = snapshot;
        self.emit(reset);

        let mut head_status: u32 = 0;
        let mut head_length: i64 = -1;
        let mut bad_encoding = false;
        let mut written: i64 = 0;
        let mut write_error: Option<String> = None;

        let result = {
            let mut on_head = |head: &ResponseHead| -> Flow {
                head_status = head.status;
                head_length = head.content_length;
                if head.status != 200 && head.status != 206 {
                    return Flow::Stop;
                }
                let enc = head.content_encoding.to_ascii_lowercase();
                if !enc.is_empty() && enc != "identity" {
                    bad_encoding = true;
                    return Flow::Stop;
                }
                Flow::Continue
            };
            let mut on_data = |chunk: &[u8]| -> Flow {
                let sink = {
                    let st = self.state.lock().unwrap();
                    let t = &st.tasks[idx];
                    if t.generation != gen
                        || t.state != TaskState::Downloading
                        || t.pause_pending
                        || t.cancel_pending
                    {
                        return Flow::Stop;
                    }
                    t.sink.clone()
                };
                let Some(sink) = sink else {
                    write_error = Some("sink closed".to_string());
                    return Flow::Stop;
                };
                if let Err(err) = sink.write_at(written, chunk) {
                    write_error = Some(err.to_string());
                    return Flow::Stop;
                }
                let len = chunk.len() as i64;
                written += len;
                let mut st = self.state.lock().unwrap();
                let t = &mut st.tasks[idx];
                t.downloaded = written;
                t.cursor = written;
                t.session_bytes += len;
                Flow::Continue
            };
            let mut ctx = StreamCtx {
                on_head: &mut on_head,
                on_data: &mut on_data,
            };
            http.fetch("GET", &url, &headers, None, None, &mut ctx)
        };

        {
            let st = self.state.lock().unwrap();
            if st.tasks[idx].generation != gen {
                return;
            }
        }

        if let Some(err) = write_error {
            self.fail_task(idx, "ERR_IO", &format!("write failed: {err}"));
            return;
        }
        if bad_encoding {
            self.fail_task(
                idx,
                "ERR_CONTENT_ENCODING",
                "server returned a non-identity content encoding",
            );
            return;
        }

        let complete = match &result {
            Ok(_) => {
                if known_total > 0 {
                    written >= known_total
                } else if head_length > 0 {
                    written >= head_length
                } else {
                    true
                }
            }
            Err(_) => false,
        };

        if complete {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.total <= 0 {
                t.total = written;
            }
            t.downloaded = t.total.max(written);
            t.cursor = written;
            t.max_conn_used = t.max_conn_used.max(1);
            drop(st);
            self.begin_verify(idx);
            return;
        }

        let status = head_status;
        let retryable = status == 0
            || status == 408
            || status == 425
            || status == 429
            || status >= 500
            || matches!(&result, Ok(_));
        let mut st = self.state.lock().unwrap();
        if st.tasks[idx].state.terminal() {
            return;
        }
        if st.tasks[idx].pause_pending || st.tasks[idx].cancel_pending {
            return;
        }
        st.error_count += 1;
        let t = &mut st.tasks[idx];
        t.single_attempts += 1;
        t.attempts += 1;
        if !retryable && status != 0 {
            let msg = format!("server returned HTTP {status}");
            drop(st);
            self.fail_task(idx, "ERR_HTTP_STATUS", &msg);
            return;
        }
        if t.single_attempts > self.opts.max_retries {
            drop(st);
            self.fail_task(idx, "ERR_RETRY_EXHAUSTED", "retry limit exceeded");
            return;
        }
        t.error_message = match &result {
            Err(err) => err.clone(),
            Ok(_) => format!("incomplete stream ({written} bytes)"),
        };
        t.single_ready = true;
        t.single_retry_at = now_ms()
            + backoff_ms(
                self.opts.base_retry_ms,
                self.opts.max_retry_ms,
                t.single_attempts,
            );
        t.state = TaskState::Retrying;
        let ev = state_event(t);
        drop(st);
        self.emit(ev);
    }

    // ---------------- verify ----------------

    pub(super) fn run_verify(&self, idx: usize) {
        let (path, alg, value, total, expected) = {
            let st = self.state.lock().unwrap();
            let t = &st.tasks[idx];
            (
                PathBuf::from(&t.destination),
                t.opt.checksum_alg.clone(),
                t.opt.checksum_value.clone(),
                t.total,
                t.opt.expected_size,
            )
        };

        let mut actual_size = file::path_size(&path);
        if total > 0 && actual_size > total {
            if let Ok(file) = fs::OpenOptions::new().write(true).open(&path) {
                let _ = file.set_len(total as u64);
            }
            actual_size = file::path_size(&path);
        }
        if expected > 0 && actual_size != expected {
            self.fail_task(
                idx,
                "ERR_SIZE_MISMATCH",
                &format!("expected {expected} bytes, got {actual_size}"),
            );
            return;
        }

        let mut verified = false;
        if !alg.is_empty() && !value.is_empty() {
            match hash::hash_file(&path, &alg) {
                Ok(actual) => {
                    if actual.eq_ignore_ascii_case(value.trim()) {
                        verified = true;
                    } else {
                        self.fail_task(
                            idx,
                            "ERR_CHECKSUM_MISMATCH",
                            &format!("checksum mismatch: expected {value}, got {actual}"),
                        );
                        return;
                    }
                }
                Err(err) => {
                    self.fail_task(idx, "ERR_CHECKSUM", &err);
                    return;
                }
            }
        }

        let sink = {
            let st = self.state.lock().unwrap();
            if st.tasks[idx].state.terminal() {
                return;
            }
            st.tasks[idx].sink.clone()
        };
        if let Some(sink) = sink {
            let _ = sink.flush();
            sink.close();
        }

        let mut events = Vec::new();
        {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.state.terminal() {
                return;
            }
            let dest = PathBuf::from(&t.destination);
            let _ = fs::remove_file(meta_path(&dest));
            t.checksum_verified = verified;
            t.finished_at = now_ms();
            t.speed = 0.0;
            t.state = TaskState::Completed;
            if t.total <= 0 {
                t.total = file::path_size(&dest).max(0);
            }
            events.push(state_event(t));
            events.push(progress_event(t));
            events.push(completed_event(t));
            st.success_count += 1;
        }
        for e in events {
            self.emit(e);
        }
        self.tick.notify_all();
    }
}
