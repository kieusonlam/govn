//! Running with administrator rights, so the keyboard also works in elevated programs.
//!
//! Windows does not let a normal process see or send keys for an elevated window. A scheduled task
//! with the highest run level starts govn elevated at logon without a UAC prompt each time.
use crate::wide;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED, HANDLE};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

pub const DEFAULT_TASK: &str = "govn";
pub const RESULT_OK: u32 = 0;
pub const RESULT_FAILED: u32 = 1;
pub const RESULT_CANCELLED: u32 = ERROR_CANCELLED;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> Option<Output> {
    // By full path: an elevated process must not pick up a schtasks.exe from PATH.
    let mut buf = [0u16; 260];
    let n = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let exe = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])).join("schtasks.exe");
    Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
}

fn succeeded(args: &[&str]) -> bool {
    schtasks(args).is_some_and(|o| o.status.success())
}

pub fn task_exists(name: &str) -> bool {
    succeeded(&["/query", "/tn", name])
}

pub fn run_task(name: &str) -> bool {
    succeeded(&["/run", "/tn", name])
}

/// The task starts this very file, not a copy that was moved or deleted since.
pub fn task_points_here(name: &str) -> bool {
    let Some(exe) = exe_path() else { return false };
    schtasks(&["/query", "/tn", name, "/xml"]).is_some_and(|o| {
        o.status.success() && String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&xml_escape(&exe).to_lowercase())
    })
}

fn exe_path() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.display().to_string())
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut info).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && info.TokenIsElevated != 0
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Written as XML because the `schtasks /create` flags cannot express the settings that matter here:
/// by default a task does not start on battery, stops when the power cable is pulled, is killed after
/// three days, and runs at a low priority that would make typing lag.
fn task_xml(exe: &str, args: &str, user: &str) -> String {
    let (exe, args, user) = (xml_escape(exe), xml_escape(args), xml_escape(user));
    let arguments = if args.is_empty() { String::new() } else { format!("<Arguments>{args}</Arguments>") };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>govn: Vietnamese keyboard, running elevated</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command>{arguments}</Exec></Actions>
</Task>
"#
    )
}

/// Needs administrator rights.
pub fn set_task(name: &str, on: bool, args: &str) -> bool {
    if !on {
        return succeeded(&["/delete", "/tn", name, "/f"]);
    }
    let Some(exe) = exe_path() else { return false };
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    // schtasks wants UTF-16 with a byte order mark.
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(task_xml(&exe, args, &user).encode_utf16().flat_map(u16::to_le_bytes));
    let file = std::env::temp_dir().join(format!("govn-task-{}.xml", std::process::id()));
    if std::fs::write(&file, bytes).is_err() {
        return false;
    }
    let ok = succeeded(&["/create", "/tn", name, "/xml", &file.display().to_string(), "/f"]);
    let _ = std::fs::remove_file(&file);
    ok
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Changes the task from a normal process by running this program elevated, which shows the UAC prompt.
pub fn set_task_elevated(name: &str, on: bool, args: &str) -> u32 {
    let Some(exe) = exe_path() else { return RESULT_FAILED };
    let params = format!(
        "--set-admin-task {} --task-name {} --task-args {}",
        if on { "on" } else { "off" },
        quote(name),
        quote(args)
    );
    let (verb, exe, params) = (wide("runas"), wide(&exe), wide(&params));
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = exe.as_ptr();
        info.lpParameters = params.as_ptr();
        info.lpDirectory = null();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 {
            return if GetLastError() == ERROR_CANCELLED { RESULT_CANCELLED } else { RESULT_FAILED };
        }
        if info.hProcess.is_null() {
            return RESULT_FAILED;
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = RESULT_FAILED;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_keeps_running_on_a_laptop() {
        let x = task_xml(r"C:\Program Files\govn\govn.exe", "", r"PC\An");
        for needed in [
            "<RunLevel>HighestAvailable</RunLevel>",
            "<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>",
            "<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>",
            "<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>",
            r"<Command>C:\Program Files\govn\govn.exe</Command>",
        ] {
            assert!(x.contains(needed), "{needed}");
        }
        assert!(!x.contains("<Arguments>"));
    }

    #[test]
    fn task_xml_escapes_paths() {
        let x = task_xml(r"C:\R&D\govn.exe", "--a <b>", r"PC\A&B");
        assert!(x.contains(r"<Command>C:\R&amp;D\govn.exe</Command>"));
        assert!(x.contains("<Arguments>--a &lt;b&gt;</Arguments>"));
        assert!(x.contains(r"<UserId>PC\A&amp;B</UserId>"));
    }
}
