use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use crate::win;
use super::*;

// ---------------- monitor ----------------

impl Engine {
    pub(super) fn monitor_loop(&self) {
        let (mut last_idle, mut last_kernel, mut last_user) = win::system_times();
        loop {
            for _ in 0..5 {
                if self.stop.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(200));
            }
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let (idle, kernel, user) = win::system_times();
            let d_idle = idle.saturating_sub(last_idle);
            let d_kernel = kernel.saturating_sub(last_kernel);
            let d_user = user.saturating_sub(last_user);
            last_idle = idle;
            last_kernel = kernel;
            last_user = user;
            let total = d_kernel + d_user;
            let cpu = if total > 0 {
                ((total.saturating_sub(d_idle)) as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            let mem = win::mem_available_pct();

            let mut st = self.state.lock().unwrap();
            st.cpu_pct = cpu.clamp(0.0, 100.0);
            st.mem_avail_pct = mem;
            let mut max_latency = 0.0f64;
            for t in &st.tasks {
                if let Some(sink) = &t.sink {
                    max_latency = max_latency.max(sink.latency_ms());
                }
            }
            st.disk_latency_ms = max_latency;
        }
    }
}
