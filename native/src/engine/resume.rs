use std::fs;
use std::path::{Path, PathBuf};

use crate::file;
use super::*;

// ---------------- resume metadata & segment helpers ----------------

pub(super) fn meta_path(dest: &Path) -> PathBuf {
    file::meta_path(dest)
}

pub(super) fn save_meta(dest: &Path, total: i64, etag: &str, last_modified: &str, segs: &[Segment]) {
    let path = meta_path(dest);
    let mut text = String::new();
    text.push_str("# win-down resume v1\n");
    text.push_str(&format!("total\t{}\n", total));
    text.push_str(&format!("etag\t{}\n", etag));
    text.push_str(&format!("last-modified\t{}\n", last_modified));
    for seg in segs {
        text.push_str(&format!(
            "seg\t{}\t{}\t{}\n",
            seg.start, seg.end, seg.written
        ));
    }
    let _ = fs::write(path, text);
}

pub(super) fn load_meta(
    dest: &Path,
    total: i64,
    etag: &str,
    last_modified: &str,
    file_size: i64,
) -> Option<Vec<Segment>> {
    if !file::path_exists(dest) {
        return None;
    }
    let text = fs::read_to_string(meta_path(dest)).ok()?;
    let mut file_total = -1i64;
    let mut file_etag = String::new();
    let mut file_lm = String::new();
    let mut segs = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        match parts.next() {
            Some("total") => file_total = parts.next().and_then(|v| v.parse().ok()).unwrap_or(-1),
            Some("etag") => file_etag = parts.next().unwrap_or("").to_string(),
            Some("last-modified") => file_lm = parts.next().unwrap_or("").to_string(),
            Some("seg") => {
                let start = parts.next().and_then(|v| v.parse::<i64>().ok());
                let end = parts.next().and_then(|v| v.parse::<i64>().ok());
                let written = parts.next().and_then(|v| v.parse::<i64>().ok());
                if let (Some(start), Some(end), Some(written)) = (start, end, written) {
                    segs.push(Segment {
                        start,
                        end,
                        written,
                        done: written >= end - start + 1,
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }
    if file_total != total || segs.is_empty() {
        return None;
    }
    // 服务器提供了校验器时必须严格匹配，不再要求两边都非空。
    if !etag.is_empty() && etag != file_etag {
        return None;
    }
    if !last_modified.is_empty() && last_modified != file_lm {
        return None;
    }
    // 磁盘文件必须已经覆盖所有记录的写入偏移，否则会产生空洞。
    let need = segs
        .iter()
        .map(|s| s.start.saturating_add(s.written))
        .max()
        .unwrap_or(0);
    if file_size < need {
        return None;
    }
    Some(segs)
}

pub(super) fn build_segments(total: i64, segment_size: i64, max_segs: usize) -> Vec<Segment> {
    if total <= 0 {
        return Vec::new();
    }
    let mut size = if segment_size > 0 {
        segment_size
    } else {
        8 << 20
    };
    size = size.max(1);
    let max_segs = max_segs.max(1);
    loop {
        let count = ((total + size - 1) / size).max(1) as usize;
        if count <= max_segs {
            break;
        }
        size *= 2;
    }
    let mut segs = Vec::new();
    let mut offset = 0i64;
    while offset < total {
        let end = (offset + size - 1).min(total - 1);
        segs.push(Segment {
            start: offset,
            end,
            ..Default::default()
        });
        offset = end + 1;
    }
    segs
}

pub(super) fn take_segment(t: &mut Task, now: i64) -> Option<usize> {
    for (i, s) in t.segs.iter().enumerate() {
        if !s.done && !s.active && s.retry_at > 0 && s.retry_at <= now {
            return Some(i);
        }
    }
    for i in t.next_seg..t.segs.len() {
        if !t.segs[i].done && !t.segs[i].active && t.segs[i].retry_at == 0 {
            t.next_seg = i + 1;
            return Some(i);
        }
    }
    for i in 0..t.next_seg.min(t.segs.len()) {
        if !t.segs[i].done && !t.segs[i].active && t.segs[i].retry_at == 0 {
            return Some(i);
        }
    }
    None
}
