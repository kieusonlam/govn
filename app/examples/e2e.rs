//! End-to-end test: types real key events into a real window through a running `govn.exe`.
//!
//! Safety: it creates its own window, the keyboard only acts on this process (`--only-pid`) and exits
//! by itself (`--duration-secs`), and a key is sent only while our window is in the foreground. If
//! focus is lost, the test stops rather than typing into someone else's window.
//!
//! usage: e2e <path to govn.exe>
#[path = "../src/tray_ids.rs"]
mod tray_ids;

use std::process::{Child, Command};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SendInput, SetFocus, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VK_BACK, VK_CAPITAL, VK_CONTROL, VK_LEFT, VK_SHIFT,
};
use tray_ids::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const TEST_RUN_KEY: &str = "Software\\govn-e2e-test\\Run";

unsafe extern "system" fn wnd_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, w, l)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

struct Ui {
    top: HWND,
    edit: HWND,
}

impl Ui {
    fn new() -> Ui {
        unsafe {
            let inst = GetModuleHandleW(null());
            let class = wide("GovnE2E");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(wnd_proc);
            wc.hInstance = inst;
            wc.lpszClassName = class.as_ptr();
            RegisterClassW(&wc);
            let top = CreateWindowExW(
                WS_EX_TOPMOST,
                class.as_ptr(),
                wide("govn e2e test (tu dong dong)").as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                80,
                80,
                640,
                200,
                null_mut(),
                null_mut(),
                inst,
                null(),
            );
            let edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | 0x4, // ES_MULTILINE
                0,
                0,
                620,
                160,
                top,
                null_mut(),
                inst,
                null(),
            );
            Ui { top, edit }
        }
    }

    fn focus(&self) -> bool {
        unsafe {
            for _ in 0..50 {
                ShowWindow(self.top, SW_RESTORE);
                SetForegroundWindow(self.top);
                pump(40);
                if GetForegroundWindow() == self.top {
                    SetFocus(self.edit);
                    pump(20);
                    return true;
                }
            }
        }
        false
    }

    fn has_focus(&self) -> bool {
        unsafe { GetForegroundWindow() == self.top }
    }

    fn clear(&self) {
        unsafe {
            SetWindowTextW(self.edit, wide("").as_ptr());
        }
        pump(10);
    }

    fn text(&self) -> String {
        unsafe {
            let mut buf = [0u16; 4096];
            let n = GetWindowTextW(self.edit, buf.as_mut_ptr(), buf.len() as i32);
            String::from_utf16_lossy(&buf[..n as usize])
        }
    }

    fn vk(&self, vk: u16, up: bool) {
        assert!(self.has_focus(), "mất focus: dừng để không gõ vào cửa sổ khác");
        let flags = if up { KEYEVENTF_KEYUP } else { 0 };
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
    }

    fn tap(&self, vk: u16) {
        self.vk(vk, false);
        self.vk(vk, true);
        pump(25);
    }

    /// `\u{8}` is backspace, `\u{1}` the left arrow, `\u{2}` Ctrl+Shift (toggle), `\u{3}` Caps Lock.
    fn type_keys(&self, keys: &str) {
        for ch in keys.chars() {
            match ch {
                '\u{8}' => self.tap(VK_BACK),
                '\u{1}' => self.tap(VK_LEFT),
                '\u{3}' => self.tap(VK_CAPITAL),
                '\u{2}' => {
                    self.vk(VK_CONTROL, false);
                    self.vk(VK_SHIFT, false);
                    pump(25);
                    self.vk(VK_SHIFT, true);
                    self.vk(VK_CONTROL, true);
                    pump(40);
                }
                _ => {
                    let scan = unsafe { VkKeyScanW(ch as u16) };
                    assert_ne!(scan, -1, "phím không có trên layout hiện tại: {ch:?}");
                    let vk = (scan & 0xFF) as u16;
                    let shift = scan & 0x100 != 0;
                    if shift {
                        self.vk(VK_SHIFT, false);
                    }
                    self.tap(vk);
                    if shift {
                        self.vk(VK_SHIFT, true);
                    }
                }
            }
        }
        pump(60);
    }
}

fn start_keyboard(exe: &str, method: &str) -> Child {
    let pid = unsafe { GetCurrentProcessId() }.to_string();
    let child = Command::new(exe)
        .args(["--method", method, "--only-pid", &pid, "--accept-injected", "--duration-secs", "45"])
        .spawn()
        .expect("không chạy được govn.exe");
    pump(400);
    child
}

fn start_with_config(exe: &str, cfg: &std::path::Path) -> Child {
    let pid = unsafe { GetCurrentProcessId() }.to_string();
    let child = Command::new(exe)
        .args(["--only-pid", &pid, "--accept-injected", "--duration-secs", "60", "--autostart-key", TEST_RUN_KEY, "--config"])
        .arg(cfg)
        .spawn()
        .expect("không chạy được govn.exe");
    pump(500);
    child
}

fn tray_window() -> HWND {
    unsafe { FindWindowW(wide(WINDOW_CLASS).as_ptr(), null()) }
}

fn command(cmd: usize) {
    unsafe { PostMessageW(tray_window(), WM_COMMAND, cmd, 0) };
    pump(150);
}

fn click_icon() {
    unsafe { PostMessageW(tray_window(), WM_TRAY, 0, WM_LBUTTONUP as isize) };
    pump(150);
}

fn wait_exit(child: &mut Child) -> bool {
    for _ in 0..100 {
        if let Ok(Some(_)) = child.try_wait() {
            return true;
        }
        pump(30);
    }
    false
}

fn autostart_value() -> Option<String> {
    let key = format!("HKCU\\{TEST_RUN_KEY}");
    let out = Command::new("reg").args(["query", &key, "/v", "govn"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Default)]
struct Tally {
    failed: u32,
    total: u32,
}

impl Tally {
    fn ok(&mut self, name: &str, passed: bool) {
        self.total += 1;
        if passed {
            println!("ok    {name}");
        } else {
            self.failed += 1;
            println!("FAIL  {name}");
        }
    }

    /// Types "go", puts a selected "ogle" after the caret as a browser would, then types the tone key.
    fn check_suggestion(&mut self, ui: &Ui, name: &str, want: &str) {
        const EM_SETSEL: u32 = 0xB1;
        const EM_REPLACESEL: u32 = 0xC2;
        ui.clear();
        ui.type_keys("go");
        unsafe {
            SendMessageW(ui.edit, EM_REPLACESEL, 0, wide("ogle").as_ptr() as LPARAM);
            SendMessageW(ui.edit, EM_SETSEL, 2, 6);
        }
        ui.type_keys("x");
        let got = ui.text();
        self.ok(&format!("{name}: {got:?}"), got == want);
        ui.type_keys(" ");
    }

    fn check(&mut self, ui: &Ui, name: &str, keys: &str, want: &str) {
        ui.clear();
        ui.type_keys(keys);
        let got = ui.text();
        self.total += 1;
        if got == want {
            println!("ok    {name}: {got:?}");
        } else {
            self.failed += 1;
            println!("FAIL  {name}: keys {keys:?}\n      want {want:?}\n      got  {got:?}");
        }
        // leave a word boundary so tests do not leak into each other
        ui.type_keys(" ");
    }
}

fn main() {
    let exe = std::env::args().nth(1).expect("usage: e2e <govn.exe>");
    let ui = Ui::new();
    if !ui.focus() {
        eprintln!("KHÔNG LẤY ĐƯỢC FOCUS: chưa gõ phím nào, test không chạy.");
        std::process::exit(3);
    }
    if unsafe { GetKeyState(i32::from(VK_CAPITAL)) } & 1 != 0 {
        eprintln!("Caps Lock đang bật: bỏ qua để test có kết quả xác định.");
        std::process::exit(3);
    }

    let mut t = Tally::default();

    // Baseline: without the keyboard, keys must arrive untouched. Otherwise another IME is rewriting them.
    t.check(&ui, "baseline (chưa bật govn)", "tieengs", "tieengs");

    let mut kb = start_keyboard(&exe, "telex");
    t.check(&ui, "telex câu", "tieengs vieetj", "tiếng việt");
    t.check(&ui, "telex ươ", "nguoiwf dduwowngf", "người đường");
    t.check(&ui, "telex hoa", "Vieetj Nam", "Việt Nam");
    t.check(&ui, "telex in hoa", "VIEETJ", "VIỆT");
    t.check(&ui, "telex hoàn tác dấu", "ass", "as");
    t.check(&ui, "telex không phải tiếng Việt", "https", "https");
    t.check(&ui, "backspace thường", "abc\u{8}", "ab");
    t.check(&ui, "backspace xóa ký tự có dấu", "tieengs\u{8}", "tiến");
    t.check(&ui, "backspace dời dấu", "toaf\u{8}", "tò");
    t.check(&ui, "dấu câu kết thúc từ", "vieetj, nam.", "việt, nam.");
    t.check(&ui, "phím mũi tên ngắt từ", "a\u{1}s", "sa");
    t.check(&ui, "Caps Lock", "\u{3}tieengs\u{3}", "TIẾNG");
    t.check(&ui, "tắt tiếng Việt bằng Ctrl+Shift", "\u{2}tieengs\u{2}", "tieengs");
    t.check(&ui, "bật lại sau khi tắt", "tieengs", "tiếng");
    let _ = kb.kill();
    let _ = kb.wait();
    pump(300);

    let mut kb = start_keyboard(&exe, "vni");
    for i in 0..6 {
        t.check(&ui, &format!("vni câu lần {i}"), "tie61ng vie65t", "tiếng việt");
    }
    t.check(&ui, "vni từng chữ", "tie6", "tiê");
    t.check(&ui, "vni từng chữ 2", "tie61", "tiế");
    t.check(&ui, "vni từng chữ 3", "tie16", "tiế");
    t.check(&ui, "vni đ ươ", "d9uo72ng", "đường");
    let _ = kb.kill();
    let _ = kb.wait();
    pump(300);

    // Tray: menu commands, the icon click, persistence and autostart, checked by their real effect.
    let cfg = std::env::temp_dir().join(format!("govn-e2e-{}.ini", std::process::id()));
    let _ = std::fs::remove_file(&cfg);
    let mut kb = start_with_config(&exe, &cfg);
    t.ok("có cửa sổ khay", !tray_window().is_null());

    command(CMD_VNI);
    t.check(&ui, "menu: chọn VNI", "a1", "á");
    command(CMD_TELEX);
    t.check(&ui, "menu: chọn Telex", "as", "á");
    click_icon();
    t.check(&ui, "nhấp icon: tắt", "as", "as");
    click_icon();
    t.check(&ui, "nhấp icon: bật lại", "as", "á");
    command(CMD_TOGGLE);
    t.check(&ui, "menu: tắt tiếng Việt", "as", "as");
    command(CMD_TOGGLE);
    command(CMD_MODERN);
    t.check(&ui, "menu: bỏ dấu kiểu mới", "hoaf", "hoà");
    command(CMD_MODERN);
    t.check(&ui, "menu: bỏ dấu kiểu cũ", "hoaf", "hòa");
    t.check(&ui, "chính tả bật", "kas", "kas");
    command(CMD_SPELL);
    t.check(&ui, "menu: tắt chính tả", "kas", "ká");
    command(CMD_SPELL);
    command(CMD_RESTORE);
    t.check(&ui, "menu: bật tự khôi phục", "cofnputer ", "cofnputer ");
    command(CMD_RESTORE);
    t.check(&ui, "menu: tắt tự khôi phục", "cofnputer ", "cònputer ");

    // An inline suggestion is selected text after the caret; only the placeholder keeps "go" intact.
    t.check_suggestion(&ui, "gợi ý, Tự động: chương trình này không thuộc danh sách", "goõ");
    command(CMD_FIX_ALWAYS);
    t.check_suggestion(&ui, "menu: gợi ý Luôn bật", "gõ");
    command(CMD_FIX_NEVER);
    t.check_suggestion(&ui, "menu: gợi ý Tắt", "goõ");
    command(CMD_FIX_AUTO);

    let second = Command::new(&exe).args(["--only-pid", "0"]).status().ok().and_then(|s| s.code());
    t.ok("chạy lần 2 bị từ chối", second == Some(1));
    t.check(&ui, "chạy lần 2 không gây gõ đôi", "tieengs", "tiếng");

    t.ok("khóa test autostart lúc đầu trống", autostart_value().is_none());
    command(CMD_AUTOSTART);
    t.ok("autostart bật", autostart_value().is_some_and(|v| v.contains("govn.exe")));
    command(CMD_AUTOSTART);
    t.ok("autostart tắt", autostart_value().is_none());

    // Ctrl+Shift has to reach the settings file too, and a restart must keep everything.
    command(CMD_VNI);
    command(CMD_MODERN);
    ui.type_keys("\u{2}");
    pump(200);
    let saved = std::fs::read_to_string(&cfg).unwrap_or_default();
    t.ok("Ctrl+Shift lưu vào file", saved.contains("viet=0"));
    t.ok("file ghi VNI và kiểu mới", saved.contains("method=vni") && saved.contains("modern=1"));
    ui.type_keys("\u{2}");
    pump(200);
    command(CMD_QUIT);
    let exited = wait_exit(&mut kb);
    t.ok("menu: thoát", exited);
    if !exited {
        let _ = kb.kill();
    }
    t.check(&ui, "sau menu thoát phím về bình thường", "as", "as");

    let mut kb = start_with_config(&exe, &cfg);
    t.check(&ui, "mở lại vẫn nhớ VNI", "a1", "á");
    t.check(&ui, "mở lại vẫn nhớ kiểu mới", "hoa2", "hoà");
    command(CMD_QUIT);
    wait_exit(&mut kb);
    let _ = std::fs::remove_file(&cfg);
    let _ = Command::new("reg").args(["delete", "HKCU\\Software\\govn-e2e-test", "/f"]).output();
    pump(300);

    t.check(&ui, "sau khi thoát phím về bình thường", "tieengs", "tieengs");

    unsafe { DestroyWindow(ui.top) };
    println!("\n{}/{} đạt", t.total - t.failed, t.total);
    std::process::exit(i32::from(t.failed != 0));
}
