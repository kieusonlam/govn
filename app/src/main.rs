//! Vietnamese keyboard for Windows: a low-level keyboard hook feeding `govn-engine`, with results
//! typed back through `SendInput`, and a notification-area icon for control.
//!
//! Hotkeys: Ctrl+Shift (press and release) toggles Vietnamese, Ctrl+Alt+Shift+Q quits.
//! All hook callbacks run on the main thread, inside its message loop.
#![windows_subsystem = "windows"]

mod admin;
mod config;
mod tray;
mod tray_ids;
mod update;

use config::{Settings, SuggestionFix};
use govn_engine::{Engine, InputMethod, Options};
use std::cell::RefCell;
use std::path::PathBuf;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, SendInput, ToUnicodeEx, INPUT, INPUT_0, INPUT_KEYBOARD,
    KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_ADD, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DECIMAL, VK_DIVIDE,
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_MULTIPLY, VK_NUMPAD0, VK_NUMPAD9, VK_PACKET, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SUBTRACT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, FindWindowW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    PostMessageW, PostQuitMessage, SetTimer, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    MSG, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN,
};

/// Marks the input we inject ourselves, so the hook lets it through untouched.
const OWN_TAG: usize = 0x474F_564E; // "GOVN"
const VK_Q: u16 = 0x51;
const DEFAULT_AUTOSTART_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Programs whose text fields show an inline suggestion as selected text after the caret, such as
/// `go|ogle.com` in a browser address bar. There, a backspace only removes the suggestion.
const SUGGESTION_APPS: &[&str] = &[
    "chrome.exe", "msedge.exe", "brave.exe", "firefox.exe", "opera.exe", "vivaldi.exe", "browser.exe", "excel.exe",
    "et.exe",
];
/// Typed over a selected suggestion so the real backspaces hit the real text.
const PLACEHOLDER: u16 = 0x202F;

struct State {
    engine: Engine,
    last_foreground: HWND,
    /// Key-ups of swallowed key-downs are swallowed too.
    swallowed: [bool; 256],
    toggle_armed: bool,
    /// Test mode: only act while this process owns the foreground window.
    only_pid: Option<u32>,
    /// Test mode: treat other programs' injected keys as typing. Normally they are left alone,
    /// so password managers and remote-control tools are not rewritten.
    accept_injected: bool,
    suggestion_fix: SuggestionFix,
    /// The foreground program is one where the placeholder is needed.
    use_placeholder: bool,
    tray_window: HWND,
    /// `None`: settings are not saved (automated tests).
    config_path: Option<PathBuf>,
    autostart_key: String,
    own_pid: u32,
    task_name: String,
    /// Arguments of the scheduled task; only automated tests set them.
    task_args: String,
    elevated: bool,
}

fn current_settings(st: &State) -> Settings {
    Settings {
        viet: st.engine.viet_key,
        method: st.engine.input_method(),
        modern_style: st.engine.options.modern_style,
        spell_check: st.engine.options.spell_check,
        auto_restore: st.engine.options.auto_non_vn_restore,
        suggestion_fix: st.suggestion_fix,
    }
}

/// What the hook decided while `State` was borrowed. Injection happens after the borrow ends,
/// because `SendInput` re-enters the hook.
enum Action {
    Pass,
    Swallow,
    Replace { backs: usize, text: Vec<u16>, placeholder: bool },
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn is_down(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

fn is_modifier(vk: u16) -> bool {
    matches!(
        vk,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU
            | VK_LWIN | VK_RWIN
    )
}

fn is_numpad(vk: u16) -> bool {
    (VK_NUMPAD0..=VK_NUMPAD9).contains(&vk) || matches!(vk, VK_MULTIPLY | VK_ADD | VK_SUBTRACT | VK_DECIMAL | VK_DIVIDE)
}

/// File name of the program owning a window, in lower case.
fn window_exe_name(h: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);
        ok
    };
    if ok == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().map(str::to_lowercase)
}

fn set_viet(st: &mut State, on: bool) {
    st.engine.reset();
    st.engine.viet_key = on;
    // The icon and the settings file are updated outside the hook, which has to return quickly.
    unsafe { PostMessageW(st.tray_window, tray_ids::WM_STATE_CHANGED, 0, 0) };
}

/// The character this key produces with the current modifiers and the foreground keyboard layout.
fn key_to_char(k: &KBDLLHOOKSTRUCT, fg: HWND, caps: bool) -> Option<u32> {
    let mut state = [0u8; 256];
    if is_down(VK_SHIFT) {
        state[VK_SHIFT as usize] = 0x80;
    }
    if caps {
        state[VK_CAPITAL as usize] = 0x01;
    }
    let mut buf = [0u16; 4];
    // Flag 4: do not change kernel keyboard state, so dead keys typed in the app keep working.
    let n = unsafe {
        let thread = GetWindowThreadProcessId(fg, null_mut());
        ToUnicodeEx(k.vkCode, k.scanCode, state.as_ptr(), buf.as_mut_ptr(), buf.len() as i32, 4, GetKeyboardLayout(thread))
    };
    (n == 1).then_some(u32::from(buf[0]))
}

fn handle_key(st: &mut State, k: &KBDLLHOOKSTRUCT, msg: u32) -> Action {
    let vk = k.vkCode as u16;
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;

    if !down {
        if st.toggle_armed && matches!(vk, VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL) {
            st.toggle_armed = false;
            let on = !st.engine.viet_key;
            set_viet(st, on);
        }
        let idx = usize::from(vk & 0xFF);
        if st.swallowed[idx] {
            st.swallowed[idx] = false;
            return Action::Swallow;
        }
        return Action::Pass;
    }

    if is_modifier(vk) {
        // GetAsyncKeyState does not include the key being processed yet, so count it explicitly.
        let ctrl = is_down(VK_CONTROL) || matches!(vk, VK_CONTROL | VK_LCONTROL | VK_RCONTROL);
        let shift = is_down(VK_SHIFT) || matches!(vk, VK_SHIFT | VK_LSHIFT | VK_RSHIFT);
        st.toggle_armed = ctrl && shift && !is_down(VK_MENU);
        return Action::Pass;
    }
    st.toggle_armed = false;

    let (ctrl, alt, win) = (is_down(VK_CONTROL), is_down(VK_MENU), is_down(VK_LWIN) || is_down(VK_RWIN));
    if vk == VK_Q && ctrl && alt && is_down(VK_SHIFT) {
        unsafe { PostQuitMessage(0) };
        return Action::Swallow;
    }

    let fg = unsafe { GetForegroundWindow() };
    let mut owner = 0u32;
    unsafe { GetWindowThreadProcessId(fg, &mut owner) };
    // Our own menu is in front: keys navigate it, they are not text.
    if owner == st.own_pid || st.only_pid.is_some_and(|pid| owner != pid) {
        st.engine.reset();
        return Action::Pass;
    }
    if fg != st.last_foreground {
        st.last_foreground = fg;
        st.engine.reset();
        st.use_placeholder = match st.suggestion_fix {
            SuggestionFix::Always => true,
            SuggestionFix::Never => false,
            SuggestionFix::Auto => window_exe_name(fg).is_some_and(|n| SUGGESTION_APPS.contains(&n.as_str())),
        };
    }

    let injected = k.flags & LLKHF_INJECTED != 0;
    if (injected && !st.accept_injected) || vk == VK_PACKET || ctrl || alt || win || is_numpad(vk) {
        st.engine.reset();
        return Action::Pass;
    }
    if !st.engine.viet_key {
        return Action::Pass;
    }

    let out = if vk == VK_BACK {
        st.engine.process_backspace()
    } else {
        let caps = unsafe { GetKeyState(i32::from(VK_CAPITAL)) } & 1 != 0;
        st.engine.caps_lock = caps;
        match key_to_char(k, fg, caps) {
            Some(ch) => st.engine.process(ch),
            None => {
                // arrows, Home, Delete, function keys, dead keys...
                st.engine.reset();
                return Action::Pass;
            }
        }
    };
    // A plain backspace is normally left to the app. With a suggestion showing it would only clear the
    // suggestion while the engine drops a character, so here it goes through the placeholder path too.
    let backspace_in_word = vk == VK_BACK && st.use_placeholder && out.backs > 0;
    if !out.handled && !backspace_in_word {
        return Action::Pass;
    }
    st.swallowed[usize::from(vk & 0xFF)] = true;
    Action::Replace { placeholder: st.use_placeholder && out.backs > 0, backs: out.backs, text: out.text }
}

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: OWN_TAG },
        },
    }
}

fn send(backs: usize, text: &[u16], placeholder: bool) {
    let mut inputs = Vec::with_capacity((backs + text.len() + 2) * 2);
    if placeholder {
        inputs.push(key_input(0, PLACEHOLDER, KEYEVENTF_UNICODE));
        inputs.push(key_input(0, PLACEHOLDER, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    for _ in 0..backs + usize::from(placeholder) {
        inputs.push(key_input(VK_BACK, 0, 0));
        inputs.push(key_input(VK_BACK, 0, KEYEVENTF_KEYUP));
    }
    for &unit in text {
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE));
        inputs.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    if !inputs.is_empty() {
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let k = &*(lparam as *const KBDLLHOOKSTRUCT);
        let msg = wparam as u32;
        if k.dwExtraInfo != OWN_TAG && matches!(msg, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP) {
            // A panic must never cross into the OS or leave the keyboard dead: pass the key on instead.
            let action = catch_unwind(AssertUnwindSafe(|| {
                STATE.with_borrow_mut(|s| s.as_mut().map_or(Action::Pass, |st| handle_key(st, k, msg)))
            }))
            .unwrap_or(Action::Pass);
            match action {
                Action::Pass => {}
                Action::Swallow => return 1,
                Action::Replace { backs, text, placeholder } => {
                    send(backs, &text, placeholder);
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) {
        // The caret may have moved, so the buffered word no longer matches the text field.
        let _ = catch_unwind(AssertUnwindSafe(|| {
            STATE.with_borrow_mut(|s| {
                if let Some(st) = s.as_mut() {
                    st.engine.reset();
                }
            })
        }));
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const USAGE: &str = "govn [--method telex|vni|simple|simple2] [--modern] [--no-spell-check] [--auto-restore] [--off] [--suggestion-fix auto|always|never] [--config <file>]";

/// Command-line values override the saved settings.
#[derive(Default)]
struct Args {
    method: Option<InputMethod>,
    modern: bool,
    no_spell_check: bool,
    auto_restore: bool,
    off: bool,
    suggestion_fix: Option<SuggestionFix>,
    config: Option<PathBuf>,
    autostart_key: Option<String>,
    set_admin_task: Option<bool>,
    task_name: Option<String>,
    task_args: Option<String>,
    only_pid: Option<u32>,
    accept_injected: bool,
    duration_secs: Option<u32>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--method" => {
                let v = it.next().unwrap_or_default();
                a.method = Some(config::parse_method(&v).ok_or(format!("unknown method: {v}"))?);
            }
            "--modern" => a.modern = true,
            "--no-spell-check" => a.no_spell_check = true,
            "--auto-restore" => a.auto_restore = true,
            "--off" => a.off = true,
            "--suggestion-fix" => {
                let v = it.next().unwrap_or_default();
                a.suggestion_fix = Some(config::parse_fix(&v).ok_or(format!("unknown --suggestion-fix value: {v}"))?);
            }
            "--config" => a.config = it.next().map(PathBuf::from),
            // for automated tests
            "--autostart-key" => a.autostart_key = it.next(),
            "--task-name" => a.task_name = it.next(),
            "--task-args" => a.task_args = it.next(),
            // what the UAC prompt runs, see admin::set_task_elevated
            "--set-admin-task" => a.set_admin_task = it.next().map(|v| v == "on"),
            "--only-pid" => a.only_pid = it.next().and_then(|v| v.parse().ok()),
            "--accept-injected" => a.accept_injected = true,
            "--duration-secs" => a.duration_secs = it.next().and_then(|v| v.parse().ok()),
            other => return Err(format!("unknown argument: {other}\n\nusage: {USAGE}")),
        }
    }
    Ok(a)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            tray::message_box(&e);
            std::process::exit(2);
        }
    };
    let task_name = args.task_name.clone().unwrap_or_else(|| admin::DEFAULT_TASK.to_string());
    let task_args = args.task_args.clone().unwrap_or_default();
    if let Some(on) = args.set_admin_task {
        std::process::exit(if admin::set_task(&task_name, on, &task_args) { 0 } else { 1 });
    }

    // Automated tests (--only-pid) stay out of the notification area and the user's settings,
    // unless they ask for a config file of their own.
    let testing = args.only_pid.is_some();
    let config_path = args.config.clone().or_else(|| if testing { None } else { config::default_path() });
    let show_icon = !testing || args.config.is_some();
    let autostart_key = args.autostart_key.clone().unwrap_or_else(|| DEFAULT_AUTOSTART_KEY.to_string());

    let mut settings = config_path.as_deref().map(Settings::load).unwrap_or_default();
    settings.method = args.method.unwrap_or(settings.method);
    settings.modern_style |= args.modern;
    settings.spell_check &= !args.no_spell_check;
    settings.auto_restore |= args.auto_restore;
    settings.viet &= !args.off;
    settings.suggestion_fix = args.suggestion_fix.unwrap_or(settings.suggestion_fix);

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);

        // Two instances would convert every key twice.
        let mutex_name = wide("Local\\govn-keyboard");
        let mut mutex = CreateMutexW(null(), 0, mutex_name.as_ptr());
        // Access denied: the instance holding it runs elevated, and this one does not.
        if matches!(GetLastError(), ERROR_ALREADY_EXISTS | ERROR_ACCESS_DENIED) {
            if !testing {
                tray::message_box("govn đang chạy rồi. Xem biểu tượng V/E ở khay hệ thống.");
            }
            std::process::exit(1);
        }

        let elevated = admin::is_elevated();
        // Opened by hand or from an old shortcut while admin mode is on: hand over to the elevated task.
        if !testing && !elevated && admin::task_points_here(&task_name) {
            CloseHandle(mutex);
            if admin::run_task(&task_name) {
                std::process::exit(0);
            }
            mutex = CreateMutexW(null(), 0, mutex_name.as_ptr());
        }

        let mut engine = Engine::new();
        engine.set_input_method(settings.method);
        engine.options = Options {
            modern_style: settings.modern_style,
            spell_check: settings.spell_check,
            auto_non_vn_restore: settings.auto_restore,
            ..Options::default()
        };
        engine.viet_key = settings.viet;
        STATE.with_borrow_mut(|s| {
            *s = Some(State {
                engine,
                last_foreground: null_mut(),
                swallowed: [false; 256],
                toggle_armed: false,
                only_pid: args.only_pid,
                accept_injected: args.accept_injected,
                suggestion_fix: settings.suggestion_fix,
                use_placeholder: false,
                tray_window: null_mut(),
                config_path,
                autostart_key: autostart_key.clone(),
                own_pid: GetCurrentProcessId(),
                task_name: task_name.clone(),
                task_args,
                elevated,
            })
        });

        let window = tray::create(show_icon);
        STATE.with_borrow_mut(|s| {
            if let Some(st) = s.as_mut() {
                st.tray_window = window;
            }
        });
        if show_icon {
            tray::autostart_sync(&autostart_key);
            tray::set_admin_enabled(admin::task_exists(&task_name));
        }
        if !testing && !FindWindowW(wide("EVKey_MainWnd").as_ptr(), null()).is_null() {
            tray::show_warning("EVKey đang chạy. Hãy chuyển nó sang chế độ E để tránh gõ bị đôi.");
        }

        let module = GetModuleHandleW(null());
        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
        if kb.is_null() || mouse.is_null() {
            let error = GetLastError();
            tray::remove_icon();
            tray::message_box(&format!("Không cài được hook bàn phím (lỗi {error})."));
            std::process::exit(1);
        }

        let timer = args.duration_secs.map(|secs| SetTimer(null_mut(), 0, secs.saturating_mul(1000), None));

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // Only our own timer: closing a menu can leave stray WM_TIMER messages in the queue.
            if msg.message == WM_TIMER && msg.hwnd.is_null() && timer == Some(msg.wParam) {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(kb);
        UnhookWindowsHookEx(mouse);
        tray::remove_icon();
        if tray::restart_requested() {
            // The elevated instance refuses to start while this one holds the mutex.
            CloseHandle(mutex);
            admin::run_task(&task_name);
        }
    }
}
