//! "Check for updates": asks GitHub for the newest released version.
//!
//! This is the only network code in govn. It runs when the user picks the menu item, sends a plain
//! GET with no user data, and never downloads or runs anything: installing stays in the user's hands.
use crate::wide;
use std::ffi::c_void;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::process::Command;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};
use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;

const HOST: &str = "api.github.com";
/// All releases rather than `/latest`, which skips pre-releases.
const RELEASES_API: &str = "/repos/kieusonlam/govn/releases?per_page=20";
const RELEASES_PAGE: &str = "https://github.com/kieusonlam/govn/releases";
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const TIMEOUT_MS: i32 = 10_000;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

type Version = (u32, u32, u32);

#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    UpToDate,
    Available(String),
    Failed,
}

/// `v1.2.3` or `1.2.3`. Anything else, such as `v1.2.3-beta`, is not a version we offer.
fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.').map(|p| {
        // parse() alone would accept "+1"
        p.bytes().all(|b| b.is_ascii_digit()).then(|| p.parse::<u32>().ok()).flatten()
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The response comes from the network, so it only ever yields numbers: what the dialog shows is
/// rebuilt from them, and the page it opens is the fixed `RELEASES_PAGE`.
fn newest_tag(json: &str) -> Option<Version> {
    json.split("\"tag_name\"")
        .skip(1)
        .filter_map(|rest| {
            let value = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
            parse_version(value.split('"').next()?)
        })
        .max()
}

fn decide(json: &str, current: &str) -> Check {
    let (Some(newest), Some(current)) = (newest_tag(json), parse_version(current)) else {
        return Check::Failed;
    };
    if newest > current {
        Check::Available(format!("{}.{}.{}", newest.0, newest.1, newest.2))
    } else {
        Check::UpToDate
    }
}

struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> Option<Handle> {
        (!raw.is_null()).then_some(Handle(raw))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// HTTPS only, with WinHTTP's normal certificate checks.
fn fetch_releases() -> Option<String> {
    unsafe {
        let agent = wide(&format!("govn/{CURRENT_VERSION}"));
        let session = Handle::new(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0))?;
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);
        let connection = Handle::new(WinHttpConnect(session.0, wide(HOST).as_ptr(), 443, 0))?;
        let request = Handle::new(WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(RELEASES_API).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        let headers: Vec<u16> = "Accept: application/vnd.github+json".encode_utf16().collect();
        if WinHttpSendRequest(request.0, headers.as_ptr(), headers.len() as u32, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, null_mut()) == 0
        {
            return None;
        }
        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let queried = WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&raw mut status).cast(),
            &mut size,
            null_mut(),
        );
        if queried == 0 || status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read) == 0 {
                return None;
            }
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > MAX_RESPONSE {
                return None;
            }
        }
        String::from_utf8(body).ok()
    }
}

pub fn check() -> Check {
    fetch_releases().map_or(Check::Failed, |json| decide(&json, CURRENT_VERSION))
}

/// Through Explorer: opened directly, the browser would inherit administrator rights in admin mode.
pub fn open_releases_page() -> bool {
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return false;
    }
    let explorer = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])).join("explorer.exe");
    Command::new(explorer).arg(RELEASES_PAGE).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_version("10.20.30"), Some((10, 20, 30)));
        for bad in ["", "v", "1.2", "1.2.3.4", "v1.2.3-beta", "1.+2.3", "1..3", "x.y.z", "99999999999.0.0"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn compares_as_numbers_not_text() {
        let json = r#"[{"tag_name":"v0.9.0"},{"tag_name":"v0.10.0"}]"#;
        assert_eq!(decide(json, "0.9.0"), Check::Available("0.10.0".into()));
        assert_eq!(decide(json, "0.10.0"), Check::UpToDate);
        assert_eq!(decide(json, "0.11.0"), Check::UpToDate);
    }

    #[test]
    fn newest_wins_wherever_it_is_listed() {
        let json = r#"[{"tag_name": "v0.1.0"}, {"tag_name" : "v0.3.0"}, {"tag_name":"v0.2.0"}]"#;
        assert_eq!(decide(json, "0.1.0"), Check::Available("0.3.0".into()));
    }

    #[test]
    fn ignores_text_that_only_looks_like_a_tag() {
        // A release note quoting the field name is escaped in JSON, and a pre-release suffix is skipped.
        let json = r#"[{"tag_name":"v0.1.0","body":"see \"tag_name\":\"v9.9.9\""},{"tag_name":"v9.0.0-beta"}]"#;
        assert_eq!(decide(json, "0.1.0"), Check::UpToDate);
    }

    #[test]
    fn hostile_tag_cannot_reach_the_dialog() {
        let json = r#"[{"tag_name":"v9.9.9 Tải tại http://evil.example"}]"#;
        assert_eq!(decide(json, "0.1.0"), Check::Failed);
    }

    #[test]
    fn unusable_answers_are_failures_not_up_to_date() {
        for json in ["", "[]", "{\"message\":\"rate limit\"}", "<html>", "\"tag_name\"", "\"tag_name\":"] {
            assert_eq!(decide(json, "0.1.0"), Check::Failed, "{json}");
        }
        assert_eq!(decide(r#"[{"tag_name":"v1.0.0"}]"#, "bản lạ"), Check::Failed);
    }

    #[test]
    #[ignore = "calls api.github.com"]
    fn real_github_answers() {
        let json = fetch_releases().expect("no answer from GitHub");
        assert!(newest_tag(&json).is_some_and(|v| v >= (0, 1, 1)), "{json}");
    }
}
