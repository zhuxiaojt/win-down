pub type HeaderList = Vec<(String, String)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Policy {
    Balanced,
    FavorOtherApps,
    MaxPerformance,
}

impl Policy {
    pub fn parse(s: &str) -> Option<Policy> {
        match s.to_ascii_lowercase().as_str() {
            "balanced" => Some(Policy::Balanced),
            "favor-other-apps" | "favorotherapps" => Some(Policy::FavorOtherApps),
            "max-performance" | "maxperformance" => Some(Policy::MaxPerformance),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Policy::Balanced => 0,
            Policy::FavorOtherApps => 1,
            Policy::MaxPerformance => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    Queued,
    Probing,
    Downloading,
    Retrying,
    Paused,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn name(&self) -> &'static str {
        match self {
            TaskState::Queued => "queued",
            TaskState::Probing => "probing",
            TaskState::Downloading => "downloading",
            TaskState::Retrying => "retrying",
            TaskState::Paused => "paused",
            TaskState::Verifying => "verifying",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
            TaskState::Cancelled => "cancelled",
        }
    }

    pub fn terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Cancelled
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DegradeReason {
    None,
    NoRangeSupport,
    UnknownLength,
    SmallFile,
    RangeIgnored,
    RangeMismatch,
    ContentEncoding,
    ValidatorChanged,
    RemoteVolume,
    ServerErrorStorm,
}

impl DegradeReason {
    pub fn name(&self) -> &'static str {
        match self {
            DegradeReason::None => "none",
            DegradeReason::NoRangeSupport => "no-range-support",
            DegradeReason::UnknownLength => "unknown-length",
            DegradeReason::SmallFile => "small-file",
            DegradeReason::RangeIgnored => "range-ignored",
            DegradeReason::RangeMismatch => "range-mismatch",
            DegradeReason::ContentEncoding => "content-encoding",
            DegradeReason::ValidatorChanged => "validator-changed",
            DegradeReason::RemoteVolume => "remote-volume",
            DegradeReason::ServerErrorStorm => "server-error-storm",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Segmented,
    Single,
    UnknownLength,
}

#[derive(Clone)]
pub struct Options {
    pub max_concurrency: usize,
    pub max_tasks_in_flight: usize,
    pub policy: Policy,
    pub segment_size: i64,
    pub max_segments_per_task: usize,
    pub small_file_threshold: i64,
    pub max_per_host: usize,
    pub connect_timeout_ms: i32,
    pub idle_timeout_ms: i32,
    pub max_retries: u32,
    pub base_retry_ms: i64,
    pub max_retry_ms: i64,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub verify_tls: bool,
    pub proxy: String,
    pub user_agent: String,
    pub headers: HeaderList,
    pub resume: bool,
    pub preallocate: bool,
    pub progress_interval_ms: i64,
    pub io_buffer_size: usize,
    pub create_directories: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_concurrency: 16,
            max_tasks_in_flight: 4,
            policy: Policy::Balanced,
            segment_size: 8 << 20,
            max_segments_per_task: 32,
            small_file_threshold: 2 << 20,
            max_per_host: 6,
            connect_timeout_ms: 10_000,
            idle_timeout_ms: 30_000,
            max_retries: 5,
            base_retry_ms: 250,
            max_retry_ms: 30_000,
            follow_redirects: true,
            max_redirects: 10,
            verify_tls: true,
            proxy: "system".to_string(),
            user_agent: "win-down/1.0".to_string(),
            headers: Vec::new(),
            resume: false,
            preallocate: true,
            progress_interval_ms: 100,
            io_buffer_size: 512 << 10,
            create_directories: true,
        }
    }
}

#[derive(Clone)]
pub struct TaskOptions {
    pub url: String,
    pub destination: String,
    pub mirrors: Vec<String>,
    pub headers: HeaderList,
    pub expected_size: i64,
    pub checksum_alg: String,
    pub checksum_value: String,
    pub priority: i32,
    pub max_concurrency: usize,
    pub overwrite: String,
    pub resume: Option<bool>,
    pub segment_size: i64,
}

impl Default for TaskOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            destination: String::new(),
            mirrors: Vec::new(),
            headers: Vec::new(),
            expected_size: -1,
            checksum_alg: String::new(),
            checksum_value: String::new(),
            priority: 0,
            max_concurrency: 0,
            overwrite: "error".to_string(),
            resume: None,
            segment_size: 0,
        }
    }
}

#[derive(Clone, Default)]
pub struct ProbeResult {
    pub status: i64,
    pub content_length: i64,
    pub accept_ranges: bool,
    pub range_supported: bool,
    pub etag: String,
    pub last_modified: String,
    pub content_type: String,
    pub content_encoding: String,
    pub content_range: String,
    pub final_url: String,
    pub error: String,
}

pub const EV_PROGRESS: u8 = 0;
pub const EV_STATE: u8 = 1;
pub const EV_DEGRADED: u8 = 2;
pub const EV_RESET: u8 = 3;
pub const EV_COMPLETED: u8 = 4;
pub const EV_FAILED: u8 = 5;
pub const EV_DONE: u8 = 6;
pub const EV_LOG: u8 = 7;
pub const EV_STATS: u8 = 8;
pub const EV_PROBE: u8 = 9;

#[derive(Clone, Default)]
pub struct Event {
    pub kind: u8,

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
    pub policy: i64,
    pub cpu_usage_pct: f64,
    pub mem_available_pct: f64,
    pub disk_write_latency_ms: f64,
    pub total_speed_bps: f64,
    pub total_downloaded: i64,
    pub total_remaining: i64,
    pub degraded_tasks: i64,
}

impl Event {
    pub fn new(kind: u8) -> Self {
        Self {
            kind,
            total_bytes: -1,
            content_length: -1,
            percent: -1.0,
            eta_seconds: -1.0,
            ok: true,
            ..Default::default()
        }
    }
}

pub fn now_ms() -> i64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() as i64 }
}

pub fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

pub fn backoff_ms(base: i64, max: i64, retries: u32) -> i64 {
    if retries == 0 {
        return base;
    }
    let shift = (retries - 1).min(20);
    (base.saturating_mul(1i64 << shift)).min(max)
}
