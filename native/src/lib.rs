#![allow(clippy::too_many_arguments)]

mod core;
mod engine;
mod file;
mod hash;
mod http;
mod win;

use std::collections::HashMap;
use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

use crate::core::{Event, Options, Policy, TaskOptions};
use crate::engine::{Engine, Sink};

// ---------------- JS-facing types ----------------

#[napi(object)]
#[derive(Default)]
pub struct JsOptions {
    pub max_concurrency: Option<u32>,
    pub max_tasks_in_flight: Option<u32>,
    pub policy: Option<String>,
    pub segment_size: Option<i64>,
    pub max_segments_per_task: Option<u32>,
    pub small_file_threshold: Option<i64>,
    pub max_per_host: Option<u32>,
    pub connect_timeout_ms: Option<i32>,
    pub idle_timeout_ms: Option<i32>,
    pub max_retries: Option<u32>,
    pub base_retry_ms: Option<i64>,
    pub max_retry_ms: Option<i64>,
    pub follow_redirects: Option<bool>,
    pub max_redirects: Option<u32>,
    pub verify_tls: Option<bool>,
    pub proxy: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub resume: Option<bool>,
    pub preallocate: Option<bool>,
    pub progress_interval_ms: Option<i64>,
    pub io_buffer_size: Option<u32>,
    pub create_directories: Option<bool>,
}

#[napi(object)]
#[derive(Default)]
pub struct JsChecksum {
    pub algorithm: Option<String>,
    pub value: Option<String>,
}

#[napi(object)]
#[derive(Default)]
pub struct JsTaskOptions {
    pub url: String,
    pub destination: String,
    pub mirrors: Option<Vec<String>>,
    pub headers: Option<HashMap<String, String>>,
    pub expected_size: Option<i64>,
    pub checksum: Option<JsChecksum>,
    pub priority: Option<i32>,
    pub max_concurrency: Option<u32>,
    pub overwrite: Option<String>,
    pub resume: Option<bool>,
    pub segment_size: Option<i64>,
}

#[napi(object)]
#[derive(Default)]
pub struct JsStats {
    pub active_tasks: i64,
    pub queued_tasks: i64,
    pub effective_concurrency: i64,
    pub max_concurrency: i64,
    pub policy: String,
    pub cpu_usage_pct: f64,
    pub mem_available_pct: f64,
    pub disk_write_latency_ms: f64,
    pub total_speed_bps: f64,
    pub total_downloaded: i64,
    pub total_remaining: i64,
    pub degraded_tasks: i64,
}

#[napi(object)]
#[derive(Default)]
pub struct JsEvent {
    pub kind: String,
    pub task_id: String,
    pub url: String,
    pub destination: String,
    pub state: String,
    pub degrade_reason: String,
    pub code: String,
    pub message: String,
    pub level: String,
    pub scope: String,

    pub total_bytes: i64,
    pub downloaded_bytes: i64,
    pub session_bytes: i64,
    pub resumed_bytes: i64,
    pub percent: f64,
    pub speed_bps: f64,
    pub average_speed_bps: f64,
    pub eta_seconds: f64,
    pub active_connections: i64,
    pub segments_total: i64,
    pub segments_completed: i64,
    pub degraded: bool,
    pub elapsed_ms: i64,

    pub token: i32,
    pub ok: bool,
    pub status_code: i64,
    pub accept_ranges: bool,
    pub content_length: i64,
    pub final_url: String,
    pub etag: String,
    pub last_modified: String,
    pub content_type: String,

    pub attempts: i64,
    pub connections_used: i64,
    pub checksum_verified: bool,
    pub skipped: bool,

    pub active_tasks: i64,
    pub queued_tasks: i64,
    pub effective_concurrency: i64,
    pub max_concurrency: i64,
    pub policy: String,
    pub cpu_usage_pct: f64,
    pub mem_available_pct: f64,
    pub disk_write_latency_ms: f64,
    pub total_speed_bps: f64,
    pub total_downloaded: i64,
    pub total_remaining: i64,
    pub degraded_tasks: i64,
}

// ---------------- conversions ----------------

fn kind_name(kind: u8) -> &'static str {
    match kind {
        0 => "progress",
        1 => "state",
        2 => "degraded",
        3 => "reset",
        4 => "completed",
        5 => "failed",
        6 => "done",
        7 => "log",
        8 => "stats",
        9 => "probe",
        _ => "log",
    }
}

fn to_js_event(ev: Event) -> JsEvent {
    JsEvent {
        kind: kind_name(ev.kind).to_string(),
        task_id: ev.task_id,
        url: ev.url,
        destination: ev.destination,
        state: ev.state,
        degrade_reason: ev.degrade_reason,
        code: ev.code,
        message: ev.message,
        level: ev.level,
        scope: ev.scope,
        total_bytes: ev.total_bytes,
        downloaded_bytes: ev.downloaded_bytes,
        session_bytes: ev.session_bytes,
        resumed_bytes: ev.resumed_bytes,
        percent: ev.percent,
        speed_bps: ev.speed_bps,
        average_speed_bps: ev.average_speed_bps,
        eta_seconds: ev.eta_seconds,
        active_connections: ev.active_connections,
        segments_total: ev.segments_total,
        segments_completed: ev.segments_completed,
        degraded: ev.degraded,
        elapsed_ms: ev.elapsed_ms,
        token: ev.token,
        ok: ev.ok,
        status_code: ev.status_code,
        accept_ranges: ev.accept_ranges,
        content_length: ev.content_length,
        final_url: ev.final_url,
        etag: ev.etag,
        last_modified: ev.last_modified,
        content_type: ev.content_type,
        attempts: ev.attempts,
        connections_used: ev.connections_used,
        checksum_verified: ev.checksum_verified,
        skipped: ev.skipped,
        active_tasks: ev.active_tasks,
        queued_tasks: ev.queued_tasks,
        effective_concurrency: ev.effective_concurrency,
        max_concurrency: ev.max_concurrency,
        policy: match ev.policy {
            1 => "favor-other-apps".to_string(),
            2 => "max-performance".to_string(),
            _ => "balanced".to_string(),
        },
        cpu_usage_pct: ev.cpu_usage_pct,
        mem_available_pct: ev.mem_available_pct,
        disk_write_latency_ms: ev.disk_write_latency_ms,
        total_speed_bps: ev.total_speed_bps,
        total_downloaded: ev.total_downloaded,
        total_remaining: ev.total_remaining,
        degraded_tasks: ev.degraded_tasks,
    }
}

fn validate_headers(headers: &HashMap<String, String>) -> std::result::Result<(), String> {
    for (name, value) in headers {
        if name.trim().is_empty() || name.contains(':') || name.contains('\r') || name.contains('\n') {
            return Err(format!("invalid header name: {name:?}"));
        }
        if value.contains('\r') || value.contains('\n') {
            return Err(format!("invalid header value for {name:?}: CR/LF is not allowed"));
        }
    }
    Ok(())
}

fn convert_options(input: Option<JsOptions>) -> std::result::Result<Options, String> {
    let mut opts = Options::default();
    let Some(o) = input else {
        return Ok(opts);
    };
    if let Some(v) = o.max_concurrency {
        opts.max_concurrency = (v as usize).clamp(1, 256);
    }
    if let Some(v) = o.max_tasks_in_flight {
        opts.max_tasks_in_flight = (v as usize).clamp(1, 64);
    }
    if let Some(v) = o.policy {
        match Policy::parse(&v) {
            Some(p) => opts.policy = p,
            None => return Err(format!("invalid policy: {v}")),
        }
    }
    if let Some(v) = o.segment_size {
        opts.segment_size = v;
    }
    if let Some(v) = o.max_segments_per_task {
        opts.max_segments_per_task = (v as usize).clamp(1, 1024);
    }
    if let Some(v) = o.small_file_threshold {
        opts.small_file_threshold = v;
    }
    if let Some(v) = o.max_per_host {
        opts.max_per_host = (v as usize).max(1);
    }
    if let Some(v) = o.connect_timeout_ms {
        opts.connect_timeout_ms = v.max(1);
    }
    if let Some(v) = o.idle_timeout_ms {
        opts.idle_timeout_ms = v.max(1);
    }
    if let Some(v) = o.max_retries {
        opts.max_retries = v;
    }
    if let Some(v) = o.base_retry_ms {
        opts.base_retry_ms = v.max(0);
    }
    if let Some(v) = o.max_retry_ms {
        opts.max_retry_ms = v.max(0);
    }
    if let Some(v) = o.follow_redirects {
        opts.follow_redirects = v;
    }
    if let Some(v) = o.max_redirects {
        opts.max_redirects = v.max(1);
    }
    if let Some(v) = o.verify_tls {
        opts.verify_tls = v;
    }
    if let Some(v) = o.proxy {
        opts.proxy = v;
    }
    if let Some(v) = o.user_agent {
        opts.user_agent = v;
    }
    if let Some(v) = o.headers {
        validate_headers(&v)?;
        opts.headers = v.into_iter().collect();
    }
    if let Some(v) = o.resume {
        opts.resume = v;
    }
    if let Some(v) = o.preallocate {
        opts.preallocate = v;
    }
    if let Some(v) = o.progress_interval_ms {
        opts.progress_interval_ms = v.max(20);
    }
    if let Some(v) = o.io_buffer_size {
        opts.io_buffer_size = (v as usize).clamp(64 << 10, 4 << 20);
    }
    if let Some(v) = o.create_directories {
        opts.create_directories = v;
    }
    Ok(opts)
}

fn convert_task(input: JsTaskOptions) -> std::result::Result<TaskOptions, String> {
    if let Some(h) = &input.headers {
        validate_headers(h)?;
    }
    let mut task = TaskOptions {
        url: input.url,
        destination: input.destination,
        mirrors: input.mirrors.unwrap_or_default(),
        headers: input
            .headers
            .map(|h| h.into_iter().collect())
            .unwrap_or_default(),
        expected_size: input.expected_size.unwrap_or(-1),
        priority: input.priority.unwrap_or(0),
        max_concurrency: input.max_concurrency.unwrap_or(0) as usize,
        overwrite: input.overwrite.unwrap_or_else(|| "error".to_string()),
        resume: input.resume,
        segment_size: input.segment_size.unwrap_or(0),
        ..Default::default()
    };
    if let Some(cs) = input.checksum {
        task.checksum_alg = cs.algorithm.unwrap_or_default();
        task.checksum_value = cs.value.unwrap_or_default();
    }
    Ok(task)
}

// ---------------- downloader handle ----------------

#[napi]
pub struct NativeDownloader {
    engine: Arc<Engine>,
    _tsfn: Arc<ThreadsafeFunction<JsEvent>>,
}

#[napi]
impl NativeDownloader {
    #[napi(constructor)]
    pub fn new(options: Option<JsOptions>, callback: ThreadsafeFunction<JsEvent>) -> Result<Self> {
        let tsfn = Arc::new(callback);
        let sink: Sink = {
            let tsfn = Arc::clone(&tsfn);
            Arc::new(move |event: Event| {
                let _ = tsfn.call(Ok(to_js_event(event)), ThreadsafeFunctionCallMode::NonBlocking);
            })
        };
        let engine = Engine::new(
            convert_options(options).map_err(|e| Error::new(Status::InvalidArg, e))?,
            sink,
        );
        engine.start();
        Ok(NativeDownloader {
            engine,
            _tsfn: tsfn,
        })
    }

    #[napi]
    pub fn add_task(&self, task: JsTaskOptions) -> Result<String> {
        let task = convert_task(task).map_err(|e| Error::new(Status::InvalidArg, e))?;
        self.engine
            .add_task(task)
            .map_err(|e| Error::new(Status::InvalidArg, e))
    }

    #[napi]
    pub fn invoke(&self, command: String, token: i32, arg: Option<String>, flag: Option<bool>) {
        self.engine
            .invoke(&command, token, arg.as_deref().unwrap_or(""), flag.unwrap_or(false));
    }

    #[napi]
    pub fn get_stats(&self) -> JsStats {
        let e = self.engine.stats_snapshot();
        JsStats {
            active_tasks: e.active_tasks,
            queued_tasks: e.queued_tasks,
            effective_concurrency: e.effective_concurrency,
            max_concurrency: e.max_concurrency,
            policy: match e.policy {
                1 => "favor-other-apps".to_string(),
                2 => "max-performance".to_string(),
                _ => "balanced".to_string(),
            },
            cpu_usage_pct: e.cpu_usage_pct,
            mem_available_pct: e.mem_available_pct,
            disk_write_latency_ms: e.disk_write_latency_ms,
            total_speed_bps: e.total_speed_bps,
            total_downloaded: e.total_downloaded,
            total_remaining: e.total_remaining,
            degraded_tasks: e.degraded_tasks,
        }
    }

    #[napi]
    pub fn set_max_concurrency(&self, value: u32) {
        self.engine.set_max_concurrency(value as usize);
    }

    #[napi]
    pub fn set_max_tasks_in_flight(&self, value: u32) {
        self.engine.set_max_tasks_in_flight(value as usize);
    }

    #[napi]
    pub fn set_policy(&self, policy: String) -> Result<()> {
        let parsed = Policy::parse(&policy)
            .ok_or_else(|| Error::new(Status::InvalidArg, format!("invalid policy: {policy}")))?;
        self.engine.set_policy(parsed);
        Ok(())
    }

    #[napi]
    pub fn shutdown(&self) {
        self.engine.shutdown();
    }
}

impl Drop for NativeDownloader {
    fn drop(&mut self) {
        self.engine.shutdown();
    }
}
