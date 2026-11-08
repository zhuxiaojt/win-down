use std::fs;
use std::path::{Path, PathBuf};

use crate::file;
use super::resume::{meta_path, save_meta};
use super::*;

// ---------------- state transitions ----------------

impl Engine {
    pub(super) fn start_single(&self, idx: usize, reason: DegradeReason) -> bool {
        let mut events = Vec::new();
        let ok;
        {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.mode != Mode::Segmented || t.state.terminal() || t.verify_requested {
                ok = false;
            } else {
                t.mode = Mode::Single;
                t.generation += 1;
                t.degraded = true;
                t.reason = reason;
                t.single_ready = true;
                t.single_queued = false;
                t.single_retry_at = 0;
                t.single_attempts = 0;
                t.downloaded = 0;
                t.session_bytes = 0;
                t.cursor = 0;
                for s in t.segs.iter_mut() {
                    s.active = false;
                    s.retry_at = 0;
                }
                events.push(degraded_event(t, reason));
                ok = true;
            }
        }
        for e in events {
            self.emit(e);
        }
        self.tick.notify_all();
        ok
    }

    pub(super) fn finalize_pause(&self, st: &mut State, idx: usize) {
        let mut events = Vec::new();
        let token;
        {
            let t = &mut st.tasks[idx];
            if !t.pause_pending {
                return;
            }
            t.pause_pending = false;
            if let Some(sink) = &t.sink {
                let _ = sink.flush();
            }
            if resume_enabled(&self.opts, t) && t.mode == Mode::Segmented && !t.segs.is_empty() {
                let dest = PathBuf::from(&t.destination);
                save_meta(&dest, t.total, &t.etag, &t.last_modified, &t.segs);
            }
            t.state = TaskState::Paused;
            t.speed = 0.0;
            events.push(state_event(t));
            token = t.pause_token;
            t.pause_token = 0;
        }
        for e in events {
            self.emit(e);
        }
        if token != 0 {
            self.emit_done(token, true, "", "");
        }
    }

    pub(super) fn finalize_cancel(&self, st: &mut State, idx: usize) {
        let mut events = Vec::new();
        let token;
        {
            let t = &mut st.tasks[idx];
            if !t.cancel_pending {
                return;
            }
            t.cancel_pending = false;
            let dest = PathBuf::from(&t.destination);
            let _ = fs::remove_file(meta_path(&dest));
            // 先释放句柄，否则 Windows 上删除目标文件会因占用而失败。
            let sink = t.sink.take();
            if let Some(sink) = &sink {
                sink.close();
            }
            drop(sink);
            if t.cancel_remove_partial {
                let _ = fs::remove_file(&dest);
            }
            t.state = TaskState::Cancelled;
            t.finished_at = now_ms();
            t.speed = 0.0;
            events.push(state_event(t));
            events.push(progress_event(t));
            token = t.cancel_token;
            t.cancel_token = 0;
        }
        for e in events {
            self.emit(e);
        }
        if token != 0 {
            self.emit_done(token, true, "", "");
        }
    }

    pub(super) fn fail_task(&self, idx: usize, code: &str, message: &str) {
        let mut events = Vec::new();
        {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.state.terminal() {
                return;
            }
            t.error_code = code.to_string();
            t.error_message = message.to_string();
            t.state = TaskState::Failed;
            t.finished_at = now_ms();
            t.speed = 0.0;
            if let Some(sink) = &t.sink {
                let _ = sink.flush();
                sink.close();
            }
            let dest = PathBuf::from(&t.destination);
            if !resume_enabled(&self.opts, t) {
                let _ = fs::remove_file(meta_path(&dest));
            }
            events.push(state_event(t));
            events.push(progress_event(t));
            events.push(failed_event(t));
        }
        for e in events {
            self.emit(e);
        }
        self.emit_log("error", message, None);
        self.tick.notify_all();
    }

    pub(super) fn finish_skipped(&self, idx: usize) {
        let mut events = Vec::new();
        {
            let mut st = self.state.lock().unwrap();
            let t = &mut st.tasks[idx];
            if t.state.terminal() {
                return;
            }
            t.skipped = true;
            t.finished_at = now_ms();
            t.state = TaskState::Completed;
            t.total = file::path_size(Path::new(&t.destination)).max(0);
            t.downloaded = t.total;
            events.push(state_event(t));
            events.push(progress_event(t));
            events.push(completed_event(t));
        }
        for e in events {
            self.emit(e);
        }
    }
}
