//! Minimal HTTPS GET over WinHTTP (system proxy, TLS by the OS). Used only for this repository's GitHub
//! releases: no telemetry, no other hosts are contacted by bitBybit. Responses are size-capped.

use bitbybit_core::{Error, ErrorCode, Result};
use std::ffi::c_void;
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR};

struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: WinHTTP handle owned by this guard.
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last(context: &str) -> Error {
    let e = std::io::Error::last_os_error();
    Error::platform(format!("{context}: {e}"), e.raw_os_error().unwrap_or(0))
}

/// `https://host/path` → (host, path). Other schemes, ports and user info are refused.
pub fn split_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
    (!host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
        .then(|| (host.to_owned(), path))
}

pub fn get(url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let (host, path) = split_url(url).ok_or_else(|| Error::invalid("Only plain https:// URLs are fetched."))?;
    // SAFETY: each handle is owned by a guard and closed in reverse order; buffers outlive the calls.
    unsafe {
        let agent = HSTRING::from(format!("bitBybit/{}", bitbybit_core::version()));
        let session =
            Handle(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err(last("WinHttpOpen"));
        }
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 30_000);
        let connection =
            Handle(WinHttpConnect(session.0, &HSTRING::from(host.as_str()), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if connection.0.is_null() {
            return Err(last("WinHttpConnect"));
        }
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            &HSTRING::from("GET"),
            &HSTRING::from(path.as_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(last("WinHttpOpenRequest"));
        }
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)
            .map_err(|e| Error::platform(format!("WinHttpSendRequest: {}", e.message()), e.code().0))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| Error::platform(format!("WinHttpReceiveResponse: {}", e.message()), e.code().0))?;
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(std::ptr::from_mut(&mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| Error::platform(format!("WinHttpQueryHeaders: {}", e.message()), e.code().0))?;
        if status != 200 {
            return Err(Error::new(
                if status == 404 { ErrorCode::NotFound } else { ErrorCode::PlatformError },
                format!("HTTP {status} for {url}"),
            ));
        }
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read)
                .map_err(|e| Error::platform(format!("WinHttpReadData: {}", e.message()), e.code().0))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > max_bytes {
                return Err(Error::new(
                    ErrorCode::PreconditionFailed,
                    format!("Response larger than {max_bytes} bytes refused."),
                ));
            }
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_https_urls() {
        assert_eq!(
            split_url("https://api.github.com/repos/a/b/releases?per_page=30"),
            Some(("api.github.com".into(), "/repos/a/b/releases?per_page=30".into()))
        );
        for bad in [
            "http://github.com/x",
            "https://user@github.com/x",
            "https://github.com:8443/x",
            "file:///c:/x",
            "https:///x",
        ] {
            assert!(split_url(bad).is_none(), "{bad}");
        }
    }
}
