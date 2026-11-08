use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// 直接写入目标文件的落盘器：使用位置写（seek_write），多个线程可并发写不同偏移。
pub struct FileSink {
    file: File,
    latency_micros: AtomicU64,
    closed: AtomicBool,
}

impl FileSink {
    pub fn open(
        path: &Path,
        total: i64,
        preallocate: bool,
        truncate: bool,
        create_dirs: bool,
    ) -> io::Result<Arc<FileSink>> {
        if create_dirs {
            ensure_parent_dir(path)?;
        }
        let mut opt = OpenOptions::new();
        opt.write(true).create(true);
        if truncate {
            opt.truncate(true);
        }
        let file = opt.open(path)?;
        if preallocate && total > 0 {
            file.set_len(total as u64)?;
        }
        Ok(Arc::new(FileSink {
            file,
            latency_micros: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        }))
    }

    pub fn write_at(&self, offset: i64, data: &[u8]) -> io::Result<()> {
        let start = Instant::now();
        let mut pos = offset as u64;
        let mut rest = data;
        while !rest.is_empty() {
            let n = self.file.seek_write(rest, pos)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "zero bytes written",
                ));
            }
            pos += n as u64;
            rest = &rest[n..];
        }
        let micros = start.elapsed().as_micros() as u64;
        let prev = self.latency_micros.load(Ordering::Relaxed);
        let next = if prev == 0 {
            micros
        } else {
            (prev * 7 + micros * 3) / 10
        };
        self.latency_micros.store(next, Ordering::Relaxed);
        Ok(())
    }

    pub fn flush(&self) -> io::Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.file.sync_data()
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    pub fn latency_ms(&self) -> f64 {
        self.latency_micros.load(Ordering::Relaxed) as f64 / 1000.0
    }
}

pub fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub fn path_exists(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

pub fn path_size(path: &Path) -> i64 {
    fs::metadata(path).map(|m| m.len() as i64).unwrap_or(-1)
}

/// 目标文件存在时生成 "name (n).ext" 形式的新路径。
pub fn unique_rename(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    let ext = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    for i in 1..1000 {
        let candidate = parent.join(format!("{} ({}){}", stem, i, ext));
        if !path_exists(&candidate) {
            return candidate;
        }
    }
    parent.join(format!("{}-{}", stem, crate::core::now_ms()))
}

pub fn meta_path(dest: &Path) -> PathBuf {
    let mut os = dest.as_os_str().to_owned();
    os.push(".wdl");
    PathBuf::from(os)
}
