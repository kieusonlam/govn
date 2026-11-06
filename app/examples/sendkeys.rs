//! Test helper: brings a window of one process to the front and types keys into it.
//! A key is sent only while that process owns the foreground window; otherwise it stops with exit
//! code 3, so nothing can land in another program.
//!
//! usage: sendkeys <pid> <keys>
//! `{L}` is Ctrl+L, `{ENTER}` Enter, `{ESC}` Esc, `{BS}` Backspace, `{A}` Ctrl+A, `{DEL}` Delete, `{TAB}` Tab, `{CEND}` Ctrl+End.
use std::ptr::null_mut;
use std::time::Duration;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_BACK, VK_CONTROL, VK_DELETE,
    VK_END, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    SetForegroundWindow, ShowWindow, SW_RESTORE,
};

fn owner(h: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    pid
}

struct Find {
    pid: u32,
    found: HWND,
}

unsafe extern "system" fn enum_cb(h: HWND, l: LPARAM) -> BOOL {
    let f = &mut *(l as *mut Find);
    if owner(h) == f.pid && IsWindowVisible(h) != 0 && GetWindowTextLengthW(h) > 0 {
        f.found = h;
        return 0;
    }
    1
}

fn raw(vk: u16, up: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
        },
    };
    unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pid: u32 = args[1].parse().expect("pid");
    let keys = &args[2];

    let mut f = Find { pid, found: null_mut() };
    unsafe { EnumWindows(Some(enum_cb), &mut f as *mut Find as LPARAM) };
    if f.found.is_null() {
        eprintln!("no window for pid {pid}");
        std::process::exit(3);
    }
    let mut ok = false;
    for _ in 0..40 {
        unsafe {
            ShowWindow(f.found, SW_RESTORE);
            SetForegroundWindow(f.found);
        }
        std::thread::sleep(Duration::from_millis(50));
        if owner(unsafe { GetForegroundWindow() }) == pid {
            ok = true;
            break;
        }
    }
    if !ok {
        eprintln!("could not take focus; nothing typed");
        std::process::exit(3);
    }

    let guard = || {
        if owner(unsafe { GetForegroundWindow() }) != pid {
            eprintln!("lost focus; stopping");
            std::process::exit(3);
        }
    };
    // Roughly human speed, but fast enough for autocomplete to race with it.
    let tap = |vk: u16| {
        guard();
        raw(vk, false);
        raw(vk, true);
        std::thread::sleep(Duration::from_millis(90));
    };
    let chord = |vk: u16| {
        guard();
        raw(VK_CONTROL, false);
        raw(vk, false);
        raw(vk, true);
        raw(VK_CONTROL, true);
        std::thread::sleep(Duration::from_millis(250));
    };

    let mut rest = keys.as_str();
    while let Some(ch) = rest.chars().next() {
        if ch == '{' {
            if let Some(end) = rest.find('}') {
                match &rest[1..end] {
                    "L" => chord(0x4C),
                    "A" => chord(0x41),
                    "ENTER" => tap(VK_RETURN),
                    "ESC" => tap(VK_ESCAPE),
                    "BS" => tap(VK_BACK),
                    "DEL" => tap(VK_DELETE),
                    "TAB" => tap(VK_TAB),
                    "CEND" => chord(VK_END),
                    other => panic!("unknown {other}"),
                }
                rest = &rest[end + 1..];
                continue;
            }
        }
        let scan = unsafe { VkKeyScanW(ch as u16) };
        assert_ne!(scan, -1, "no key for {ch:?}");
        let shift = scan & 0x100 != 0;
        if shift {
            raw(VK_SHIFT, false);
        }
        tap((scan & 0xFF) as u16);
        if shift {
            raw(VK_SHIFT, true);
        }
        rest = &rest[ch.len_utf8()..];
    }
}
