use std::fs::File;
use std::io::Read;
use std::path::Path;

use windows_sys::Win32::Security::Cryptography::{
    BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptGetProperty,
    BCryptHashData, BCryptOpenAlgorithmProvider, BCRYPT_ALG_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_HASH_LENGTH,
    BCRYPT_MD5_ALGORITHM, BCRYPT_OBJECT_LENGTH, BCRYPT_SHA256_ALGORITHM,
};

// ---------------- bcrypt hashing ----------------

fn nt_ok(status: i32) -> bool {
    status >= 0
}

struct Provider {
    handle: BCRYPT_ALG_HANDLE,
}

impl Drop for Provider {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                BCryptCloseAlgorithmProvider(self.handle, 0);
            }
        }
    }
}

struct HashHandle {
    handle: BCRYPT_HASH_HANDLE,
}

impl Drop for HashHandle {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                BCryptDestroyHash(self.handle);
            }
        }
    }
}

fn bcrypt_hash(path: &Path, algorithm: *const u16, out_len: usize) -> Result<String, String> {
    let mut provider = Provider {
        handle: std::ptr::null_mut(),
    };
    let status = unsafe {
        BCryptOpenAlgorithmProvider(&mut provider.handle, algorithm, std::ptr::null(), 0)
    };
    if !nt_ok(status) {
        return Err(format!("BCryptOpenAlgorithmProvider failed: 0x{status:08X}"));
    }

    let mut obj_len: u32 = 0;
    let mut got: u32 = 0;
    let status = unsafe {
        BCryptGetProperty(
            provider.handle,
            BCRYPT_OBJECT_LENGTH,
            (&mut obj_len as *mut u32).cast(),
            4,
            &mut got,
            0,
        )
    };
    if !nt_ok(status) {
        return Err(format!("BCryptGetProperty(ObjectLength) failed: 0x{status:08X}"));
    }

    let mut hash_len: u32 = 0;
    let status = unsafe {
        BCryptGetProperty(
            provider.handle,
            BCRYPT_HASH_LENGTH,
            (&mut hash_len as *mut u32).cast(),
            4,
            &mut got,
            0,
        )
    };
    if !nt_ok(status) {
        return Err(format!("BCryptGetProperty(HashLength) failed: 0x{status:08X}"));
    }

    let mut object = vec![0u8; obj_len as usize];
    let mut hash = HashHandle {
        handle: std::ptr::null_mut(),
    };
    let status = unsafe {
        BCryptCreateHash(
            provider.handle,
            &mut hash.handle,
            object.as_mut_ptr(),
            obj_len,
            std::ptr::null(),
            0,
            0,
        )
    };
    if !nt_ok(status) {
        return Err(format!("BCryptCreateHash failed: 0x{status:08X}"));
    }

    let mut file = File::open(path).map_err(|e| format!("open {} failed: {e}", path.display()))?;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {} failed: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        let status = unsafe { BCryptHashData(hash.handle, buf.as_ptr(), n as u32, 0) };
        if !nt_ok(status) {
            return Err(format!("BCryptHashData failed: 0x{status:08X}"));
        }
    }

    let mut digest = vec![0u8; hash_len as usize];
    let status = unsafe { BCryptFinishHash(hash.handle, digest.as_mut_ptr(), hash_len, 0) };
    if !nt_ok(status) {
        return Err(format!("BCryptFinishHash failed: 0x{status:08X}"));
    }

    let take = out_len.min(digest.len().saturating_mul(2));
    let hex: String = digest
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
        .chars()
        .take(take)
        .collect();
    Ok(hex)
}

// ---------------- crc32 ----------------

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

fn crc32_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("open {} failed: {e}", path.display()))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut crc: u32 = 0xFFFF_FFFF;
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {} failed: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            let idx = ((crc ^ b as u32) & 0xFF) as usize;
            crc = (crc >> 8) ^ CRC_TABLE[idx];
        }
    }
    Ok(format!("{:08x}", !crc))
}

// ---------------- entry point ----------------

pub fn hash_file(path: &Path, algorithm: &str) -> Result<String, String> {
    match algorithm.trim().to_ascii_lowercase().as_str() {
        "" => Err("empty checksum algorithm".to_string()),
        "sha256" | "sha-256" | "sha2-256" => bcrypt_hash(path, BCRYPT_SHA256_ALGORITHM, 64),
        "md5" => bcrypt_hash(path, BCRYPT_MD5_ALGORITHM, 32),
        "crc32" | "crc-32" => crc32_file(path),
        other => Err(format!("unsupported checksum algorithm: {other}")),
    }
}
