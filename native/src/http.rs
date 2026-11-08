use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Networking::WinHttp::*;

use crate::core::{HeaderList, Options, ProbeResult};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Stop,
}

#[derive(Clone, Default)]
pub struct ResponseHead {
    pub status: u32,
    pub final_url: String,
    pub content_length: i64,
    pub content_range: String,
    pub accept_ranges: String,
    pub etag: String,
    pub last_modified: String,
    pub content_encoding: String,
    pub content_type: String,
}

impl ResponseHead {
    pub fn range_supported(&self) -> bool {
        self.status == 206
            || self
                .accept_ranges
                .to_ascii_lowercase()
                .contains("bytes")
    }
}

pub struct AbortRegistry {
    inner: Mutex<HashMap<u64, usize>>,
    next: AtomicU64,
    closed: AtomicBool,
}

impl AbortRegistry {
    pub fn new() -> Arc<AbortRegistry> {
        Arc::new(AbortRegistry {
            inner: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        })
    }

    pub fn register(&self, handle: usize) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut map = self.inner.lock().unwrap();
        if !self.closed.load(Ordering::Relaxed) {
            map.insert(id, handle);
        }
        id
    }

    pub fn take(&self, id: u64) -> Option<usize> {
        self.inner.lock().unwrap().remove(&id)
    }

    pub fn abort_all(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let handles: Vec<usize> = {
            let mut map = self.inner.lock().unwrap();
            map.drain().map(|(_, v)| v).collect()
        };
        for h in handles {
            unsafe {
                WinHttpCloseHandle(h as *mut core::ffi::c_void);
            }
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

struct HandleGuard<'a> {
    registry: &'a AbortRegistry,
    id: u64,
}

impl Drop for HandleGuard<'_> {
    fn drop(&mut self) {
        if let Some(h) = self.registry.take(self.id) {
            unsafe {
                WinHttpCloseHandle(h as *mut core::ffi::c_void);
            }
        }
    }
}

struct UrlParts {
    host: String,
    port: u16,
    secure: bool,
    path: String,
}

fn parse_url(url: &str) -> Result<UrlParts, String> {
    let (scheme, rest) = url
        .split_once("://")
        .filter(|(s, _)| s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
        .ok_or_else(|| format!("unsupported url: {url}"))?;
    let secure = scheme.eq_ignore_ascii_case("https");
    let (authority, path) = match rest.find(|c| c == '/' || c == '?') {
        Some(i) => {
            let (a, tail) = rest.split_at(i);
            let p = if tail.starts_with('?') {
                format!("/{}", tail)
            } else {
                tail.to_string()
            };
            (a, p)
        }
        None => (rest, "/".to_string()),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);

    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (h, tail) = stripped
            .split_once(']')
            .ok_or_else(|| format!("invalid ipv6 host in url: {url}"))?;
        let p = tail
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(if secure { 443 } else { 80 });
        (h.to_string(), p)
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        if let Ok(p) = p.parse::<u16>() {
            (h.to_string(), p)
        } else {
            (authority.to_string(), if secure { 443 } else { 80 })
        }
    } else {
        (
            authority.to_string(),
            if secure { 443 } else { 80 },
        )
    };
    if host.is_empty() {
        return Err(format!("missing host in url: {url}"));
    }
    Ok(UrlParts {
        host,
        port,
        secure,
        path,
    })
}

pub fn host_of(url: &str) -> String {
    parse_url(url).map(|p| p.host).unwrap_or_default()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn basic_auth(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest
        .split(|c| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or(rest);
    let (userinfo, _) = authority.rsplit_once('@')?;
    let (user, pass) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    let raw = format!("{}:{}", percent_decode(user), percent_decode(pass));
    Some(format!("Basic {}", base64_encode(raw.as_bytes())))
}

pub struct RequestSpec {
    pub method: String,
    pub url: String,
    pub headers: HeaderList,
    pub range: Option<(i64, i64)>,
    pub if_range: Option<String>,
}

pub struct StreamCtx<'a> {
    pub on_head: &'a mut dyn FnMut(&ResponseHead) -> Flow,
    pub on_data: &'a mut dyn FnMut(&[u8]) -> Flow,
}

fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn last_error() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

fn query_header(req: usize, id: u32) -> Option<String> {
    let h = req as *mut core::ffi::c_void;
    // WinHTTP 以字节为单位返回缓冲区长度，这里按 WCHAR 数量换算。
    let mut bytes: u32 = 8192;
    let mut buf = vec![0u16; bytes as usize / 2];
    let mut index: u32 = 0;
    let ok = unsafe {
        WinHttpQueryHeaders(
            h,
            id,
            std::ptr::null(),
            buf.as_mut_ptr().cast(),
            &mut bytes,
            &mut index,
        )
    };
    if ok == 0 {
        if last_error() != windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        let need = bytes as usize / 2 + 1;
        buf = vec![0u16; need];
        bytes = (need * 2) as u32;
        index = 0;
        let ok2 = unsafe {
            WinHttpQueryHeaders(
                h,
                id,
                std::ptr::null(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
                &mut index,
            )
        };
        if ok2 == 0 {
            return None;
        }
    }
    let chars = ((bytes as usize) / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..chars]);
    Some(text.trim_end_matches('\0').to_string())
}

fn query_number(req: usize, id: u32) -> Option<u32> {
    let mut value: u32 = 0;
    let mut len: u32 = 4;
    let mut index: u32 = 0;
    let ok = unsafe {
        WinHttpQueryHeaders(
            req as *mut core::ffi::c_void,
            id | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut value as *mut u32).cast(),
            &mut len,
            &mut index,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(value)
    }
}

fn query_final_url(req: usize) -> String {
    let mut len: u32 = 4096;
    let mut buf = vec![0u16; (len as usize) / 2];
    let ok = unsafe {
        WinHttpQueryOption(
            req as *mut core::ffi::c_void,
            WINHTTP_OPTION_URL,
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if ok == 0 {
        return String::new();
    }
    let chars = (len as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..chars]);
    text.trim_end_matches('\0').to_string()
}

fn parse_content_length(raw: &Option<String>) -> i64 {
    raw.as_ref()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(-1)
}

fn parse_total_from_content_range(value: &str) -> i64 {
    value
        .rsplit_once('/')
        .and_then(|(_, total)| total.trim().parse::<i64>().ok())
        .unwrap_or(-1)
}

fn build_head(req: usize) -> ResponseHead {
    let status = query_number(req, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
    let content_length_raw = query_header(req, WINHTTP_QUERY_CONTENT_LENGTH);
    let content_range = query_header(req, WINHTTP_QUERY_CONTENT_RANGE).unwrap_or_default();
    let accept_ranges = query_header(req, WINHTTP_QUERY_ACCEPT_RANGES).unwrap_or_default();
    let mut content_length = parse_content_length(&content_length_raw);
    if status == 206 {
        // 206 的 Content-Length 是"分段长度"而非资源总长（如 Range: bytes=0-0 时为 1），
        // 总长只能从 Content-Range 的 total 取；解析失败（如 */）则保持 -1 表示未知。
        content_length = parse_total_from_content_range(&content_range);
    }
    ResponseHead {
        status,
        final_url: query_final_url(req),
        content_length,
        content_range,
        accept_ranges,
        etag: query_header(req, WINHTTP_QUERY_ETAG).unwrap_or_default(),
        last_modified: query_header(req, WINHTTP_QUERY_LAST_MODIFIED).unwrap_or_default(),
        content_encoding: query_header(req, WINHTTP_QUERY_CONTENT_ENCODING).unwrap_or_default(),
        content_type: query_header(req, WINHTTP_QUERY_CONTENT_TYPE).unwrap_or_default(),
    }
}

pub struct HttpWorker {
    opts: Options,
    abort: Arc<AbortRegistry>,
    session: usize,
    connect: Option<(String, u16, usize)>,
}

impl Drop for HttpWorker {
    fn drop(&mut self) {
        unsafe {
            if let Some((_, _, h)) = self.connect.take() {
                WinHttpCloseHandle(h as *mut core::ffi::c_void);
            }
            if self.session != 0 {
                WinHttpCloseHandle(self.session as *mut core::ffi::c_void);
            }
        }
    }
}

impl HttpWorker {
    pub fn new(opts: Options, abort: Arc<AbortRegistry>) -> HttpWorker {
        HttpWorker {
            opts,
            abort,
            session: 0,
            connect: None,
        }
    }

    fn ensure_session(&mut self) -> Result<usize, String> {
        if self.session != 0 {
            return Ok(self.session);
        }
        let agent = wide(&self.opts.user_agent);
        let access = match self.opts.proxy.to_ascii_lowercase().as_str() {
            "none" => WINHTTP_ACCESS_TYPE_NO_PROXY,
            "system" => WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            _ => WINHTTP_ACCESS_TYPE_NAMED_PROXY,
        };
        let proxy_wide = if access == WINHTTP_ACCESS_TYPE_NAMED_PROXY {
            Some(wide(&self.opts.proxy))
        } else {
            None
        };
        let proxy_ptr = proxy_wide
            .as_ref()
            .map(|v| v.as_ptr())
            .unwrap_or(std::ptr::null());
        let mut handle = unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                access,
                proxy_ptr,
                std::ptr::null(),
                0,
            )
        };
        if handle.is_null() && access == WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY {
            handle = unsafe {
                WinHttpOpen(
                    agent.as_ptr(),
                    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                )
            };
        }
        if handle.is_null() {
            return Err(format!("WinHttpOpen failed: {}", last_error()));
        }
        let h = handle as usize;
        self.session = h;
        unsafe {
            WinHttpSetTimeouts(
                h as *mut core::ffi::c_void,
                self.opts.connect_timeout_ms,
                self.opts.connect_timeout_ms,
                self.opts.idle_timeout_ms,
                self.opts.idle_timeout_ms,
            );
        }
        Ok(h)
    }

    fn ensure_connect(&mut self, parts: &UrlParts) -> Result<usize, String> {
        if let Some((host, port, h)) = &self.connect {
            if host == &parts.host && *port == parts.port {
                return Ok(*h);
            }
        }
        if let Some((_, _, old)) = self.connect.take() {
            unsafe {
                WinHttpCloseHandle(old as *mut core::ffi::c_void);
            }
        }
        let session = self.ensure_session()?;
        let host = wide(&parts.host);
        let h = unsafe {
            WinHttpConnect(
                session as *mut core::ffi::c_void,
                host.as_ptr(),
                parts.port,
                0,
            )
        };
        if h.is_null() {
            return Err(format!("WinHttpConnect failed: {}", last_error()));
        }
        self.connect = Some((parts.host.clone(), parts.port, h as usize));
        Ok(h as usize)
    }

    fn open_request(&mut self, parts: &UrlParts, method: &str) -> Result<usize, String> {
        let connect = self.ensure_connect(parts)?;
        let verb = wide(method);
        let path = wide(&parts.path);
        let flags = if parts.secure {
            WINHTTP_FLAG_SECURE
        } else {
            0
        };
        let req = unsafe {
            WinHttpOpenRequest(
                connect as *mut core::ffi::c_void,
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                flags,
            )
        };
        if req.is_null() {
            return Err(format!("WinHttpOpenRequest failed: {}", last_error()));
        }
        let req_usize = req as usize;

        unsafe {
            let redirect: u32 = if self.opts.follow_redirects {
                WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS
            } else {
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER
            };
            WinHttpSetOption(
                req,
                WINHTTP_OPTION_REDIRECT_POLICY,
                (&redirect as *const u32).cast(),
                4,
            );
            let max_redirects = self.opts.max_redirects.max(1);
            WinHttpSetOption(
                req,
                WINHTTP_OPTION_MAX_HTTP_AUTOMATIC_REDIRECTS,
                (&max_redirects as *const u32).cast(),
                4,
            );
            if !self.opts.verify_tls {
                let flags: u32 = SECURITY_FLAG_IGNORE_UNKNOWN_CA
                    | SECURITY_FLAG_IGNORE_CERT_CN_INVALID
                    | SECURITY_FLAG_IGNORE_CERT_DATE_INVALID
                    | SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE;
                WinHttpSetOption(
                    req,
                    WINHTTP_OPTION_SECURITY_FLAGS,
                    (&flags as *const u32).cast(),
                    4,
                );
            }
        }
        Ok(req_usize)
    }

    fn perform(
        &mut self,
        spec: &RequestSpec,
        ctx: &mut StreamCtx<'_>,
    ) -> Result<ResponseHead, String> {
        if self.abort.is_closed() {
            return Err("aborted".to_string());
        }
        let parts = parse_url(&spec.url)?;
        let req = self.open_request(&parts, &spec.method)?;

        // 注册请求句柄，强制关停时由其它线程关闭以中断阻塞读取。
        let id = self.abort.register(req);
        let _guard = HandleGuard {
            registry: &self.abort,
            id,
        };

        let mut header_text = String::new();
        let mut has_auth = false;
        for (k, v) in &spec.headers {
            let key = k.trim();
            if key.is_empty()
                || key.eq_ignore_ascii_case("range")
                || key.eq_ignore_ascii_case("if-range")
                || key.eq_ignore_ascii_case("accept-encoding")
            {
                continue;
            }
            if key.eq_ignore_ascii_case("authorization") {
                has_auth = true;
            }
            header_text.push_str(key);
            header_text.push_str(": ");
            header_text.push_str(v);
            header_text.push_str("\r\n");
        }
        // Range 请求依赖未压缩的字节流，强制 identity。
        header_text.push_str("Accept-Encoding: identity\r\n");
        if !has_auth {
            if let Some(auth) = basic_auth(&spec.url) {
                header_text.push_str("Authorization: ");
                header_text.push_str(&auth);
                header_text.push_str("\r\n");
            }
        }
        if let Some((start, end)) = spec.range {
            header_text.push_str(&format!("Range: bytes={}-{}\r\n", start, end));
        }
        if let Some(if_range) = &spec.if_range {
            if !if_range.is_empty() {
                header_text.push_str(&format!("If-Range: {}\r\n", if_range));
            }
        }
        let headers_wide = if header_text.is_empty() {
            None
        } else {
            Some(wide(&header_text))
        };
        let hp = headers_wide
            .as_ref()
            .map(|v| v.as_ptr())
            .unwrap_or(std::ptr::null());
        let hl = if headers_wide.is_some() {
            u32::MAX
        } else {
            0
        };

        let sent = unsafe {
            WinHttpSendRequest(
                req as *mut core::ffi::c_void,
                hp,
                hl,
                std::ptr::null(),
                0,
                0,
                0,
            )
        };
        if sent == 0 {
            return Err(format!("WinHttpSendRequest failed: {}", last_error()));
        }
        if self.abort.is_closed() {
            return Err("aborted".to_string());
        }
        let received = unsafe {
            WinHttpReceiveResponse(req as *mut core::ffi::c_void, std::ptr::null_mut())
        };
        if received == 0 {
            return Err(format!("WinHttpReceiveResponse failed: {}", last_error()));
        }

        let head = build_head(req);
        if (ctx.on_head)(&head) == Flow::Stop || self.abort.is_closed() {
            return Ok(head);
        }

        let buf_size = self.opts.io_buffer_size.clamp(64 * 1024, 4 * 1024 * 1024);
        let mut buf = vec![0u8; buf_size];
        loop {
            if self.abort.is_closed() {
                return Ok(head);
            }
            let mut read: u32 = 0;
            let ok = unsafe {
                WinHttpReadData(
                    req as *mut core::ffi::c_void,
                    buf.as_mut_ptr().cast(),
                    buf.len() as u32,
                    &mut read,
                )
            };
            if ok == 0 {
                let err = last_error();
                if self.abort.is_closed() {
                    return Ok(head);
                }
                return Err(format!("WinHttpReadData failed: {err}"));
            }
            if read == 0 {
                break;
            }
            if (ctx.on_data)(&buf[..read as usize]) == Flow::Stop {
                break;
            }
        }
        Ok(head)
    }

    pub fn probe(&mut self, url: &str, headers: &HeaderList) -> ProbeResult {
        let mut result = ProbeResult {
            content_length: -1,
            ..Default::default()
        };

        let head_spec = RequestSpec {
            method: "HEAD".to_string(),
            url: url.to_string(),
            headers: headers.clone(),
            range: None,
            if_range: None,
        };
        let mut noop = StreamCtx {
            on_head: &mut |_: &ResponseHead| Flow::Continue,
            on_data: &mut |_: &[u8]| Flow::Continue,
        };
        let mut head_failed = false;
        match self.perform(&head_spec, &mut noop) {
            Ok(head) => {
                // 只有 200/206 的 HEAD 响应才是资源元数据的可信来源；
                // 405 等响应也可能带 Content-Length: 0，误用会把文件当成 0 字节。
                if head.status == 200 || head.status == 206 {
                    result.status = head.status as i64;
                    result.content_length = head.content_length;
                    result.accept_ranges = head.range_supported();
                    result.etag = head.etag.clone();
                    result.last_modified = head.last_modified.clone();
                    result.content_type = head.content_type.clone();
                    result.content_encoding = head.content_encoding.clone();
                    result.content_range = head.content_range.clone();
                    result.final_url = head.final_url.clone();
                    result.range_supported = head.range_supported();
                } else {
                    result.status = head.status as i64;
                    head_failed = true;
                }
            }
            Err(err) => {
                // HEAD 可能被服务器拒绝，继续尝试 GET Range 探测。
                result.error = err;
                head_failed = true;
            }
        }

        // 长度已知且为小文件时无需再探测 Range 支持。
        let enough_info = !head_failed
            && result.content_length >= 0
            && (result.accept_ranges || result.content_length < self.opts.small_file_threshold);
        if !enough_info {
            let get_spec = RequestSpec {
                method: "GET".to_string(),
                url: url.to_string(),
                headers: headers.clone(),
                range: Some((0, 0)),
                if_range: None,
            };
            let mut noop = StreamCtx {
                // 只探测头部，拿到响应后立即结束，避免把整个文件读一遍。
                on_head: &mut |_: &ResponseHead| Flow::Stop,
                on_data: &mut |_: &[u8]| Flow::Continue,
            };
            match self.perform(&get_spec, &mut noop) {
                Ok(head) => {
                    result.error.clear();
                    result.status = head.status as i64;
                    if head.status == 206 {
                        result.range_supported = true;
                        result.accept_ranges = true;
                        // build_head 已把 206 的长度换算成 Content-Range 的资源总长。
                        if head.content_length >= 0 {
                            result.content_length = head.content_length;
                        }
                    } else if head.status == 200 {
                        result.range_supported = false;
                        result.accept_ranges = false;
                        result.content_length = head.content_length;
                    }
                    if result.final_url.is_empty() {
                        result.final_url = head.final_url.clone();
                    }
                    if result.etag.is_empty() {
                        result.etag = head.etag.clone();
                    }
                    if result.last_modified.is_empty() {
                        result.last_modified = head.last_modified.clone();
                    }
                    if result.content_type.is_empty() {
                        result.content_type = head.content_type.clone();
                    }
                    if result.content_encoding.is_empty() {
                        result.content_encoding = head.content_encoding.clone();
                    }
                }
                Err(err) => {
                    if result.error.is_empty() {
                        result.error = err;
                    }
                }
            }
        }
        result
    }

    pub fn fetch(
        &mut self,
        method: &str,
        url: &str,
        headers: &HeaderList,
        range: Option<(i64, i64)>,
        if_range: Option<&str>,
        ctx: &mut StreamCtx<'_>,
    ) -> Result<ResponseHead, String> {
        let spec = RequestSpec {
            method: method.to_string(),
            url: url.to_string(),
            headers: headers.clone(),
            range,
            if_range: if_range.map(|s| s.to_string()),
        };
        self.perform(&spec, ctx)
    }
}
