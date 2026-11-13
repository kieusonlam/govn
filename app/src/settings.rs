//! The settings window, a separate process started with `--settings`.
//!
//! Plain Win32 controls on a tab control, laid out by hand in 96-DPI units and scaled for the
//! monitor. Each change is written to the settings file (read-modify-write, since the keyboard
//! writes it too) and the keyboard is told to read it again. Admin mode is still switched by the
//! keyboard process, which owns the UAC flow.
use crate::admin;
use crate::config::{Hotkey, Settings};
use crate::tray;
use crate::tray_ids::{CMD_ADMIN, WINDOW_CLASS, WM_RELOAD};
use crate::update::{self, Check};
use crate::{wide, window_exe_path};
use govn_engine::InputMethod;
use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    GetLastError, BOOL, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, InvalidateRect, GetSysColor, GetSysColorBrush, MonitorFromPoint, SetBkColor, SetTextColor,
    COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetMonitorInfoW, HDC, HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, BST_CHECKED, ICC_STANDARD_CLASSES, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, NMHDR, TCIF_TEXT, TCITEMW,
    TCM_ADJUSTRECT, TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL, TCN_SELCHANGE, WC_TABCONTROLW,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForMonitor, GetDpiForWindow, SetProcessDpiAwarenessContext,
    SystemParametersInfoForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetKeyState, VK_CONTROL, VK_SHIFT, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const TITLE: &str = "govn - Cài đặt";
const CLASS: &str = "govn-settings";
/// One white child window per tab, holding that tab's controls.
const PAGE_CLASS: &str = "govn-settings-page";
const ADMIN_NOTE: &str =
    "Để gõ được trong ứng dụng chạy quyền admin. Chỉ bật khi govn.exe nằm ở thư mục chỉ admin sửa được, như Program Files.";
const HINT_TELEX: &str = "Gõ đầy đủ như trên điện thoại: uw → ư, ow → ơ, aw → ă, dd → đ.";
const HINT_VNI: &str = "a1 → á, a6 → â, a8 → ă, o7 → ơ, d9 → đ.";
const PLACEHOLDER: &str = "Chọn ứng dụng…";

/// Client area in 96-DPI pixels; the pages below are laid out to fit it.
const CLIENT_W: i32 = 500;
const CLIENT_H: i32 = 376;

/// Posted by the update-check thread: wParam is 0 up to date, 1 available, 2 failed;
/// lParam is a `Box<String>` with the new version when one is available.
const WM_UPDATE_DONE: u32 = WM_APP + 1;
const TIMER_ADMIN: usize = 1;
// Static control styles (SystemServices in windows-sys).
const SS_LEFT: u32 = 0;
const SS_NOPREFIX: u32 = 0x80;

// Control IDs; above IDOK/IDCANCEL so Esc arrives as IDCANCEL.
const ID_TELEX: u16 = 100;
const ID_VNI: u16 = 101;
const ID_MODERN: u16 = 102;
const ID_SPELL: u16 = 103;
const ID_RESTORE: u16 = 104;
const ID_LIST: u16 = 110;
const ID_REMOVE: u16 = 111;
const ID_COMBO: u16 = 112;
const ID_ADD: u16 = 113;
const ID_REFRESH: u16 = 114;
const ID_REMEMBER: u16 = 115;
const ID_HOTKEY: u16 = 120;
const ID_AUTOSTART: u16 = 121;
const ID_ADMIN: u16 = 122;
const ID_CHECK: u16 = 123;
const ID_DOWNLOAD: u16 = 124;

fn keyboard_window() -> HWND {
    unsafe { FindWindowW(wide(WINDOW_CLASS).as_ptr(), null()) }
}

/// Read-modify-write, then tell the keyboard. Returns false if the file could not be written.
fn edit(path: &Path, change: impl FnOnce(&mut Settings)) -> bool {
    let mut s = Settings::load(path);
    change(&mut s);
    if s.save(path).is_err() {
        tray::message_box("Không lưu được cài đặt.");
        return false;
    }
    let window = keyboard_window();
    if !window.is_null() {
        unsafe { PostMessageW(window, WM_RELOAD, 0, 0) };
    }
    true
}

/// The "File description" in a program's version information, such as "Visual Studio Code".
fn file_description(path: &str) -> Option<String> {
    let file = wide(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(file.as_ptr(), null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(file.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let block = data.as_ptr().cast();
        let mut value: *mut c_void = null_mut();
        let mut len = 0u32;
        let mut tables = Vec::new();
        if VerQueryValueW(block, wide("\\VarFileInfo\\Translation").as_ptr(), &mut value, &mut len) != 0 && len >= 4 {
            let pair = value as *const u16;
            tables.push(format!("{:04x}{:04x}", *pair, *pair.add(1)));
        }
        tables.extend(["040904b0".to_string(), "040904e4".to_string()]);
        for table in tables {
            let key = wide(&format!("\\StringFileInfo\\{table}\\FileDescription"));
            if VerQueryValueW(block, key.as_ptr(), &mut value, &mut len) != 0 && len > 1 {
                let text = std::slice::from_raw_parts(value as *const u16, len as usize - 1);
                let text = String::from_utf16_lossy(text).trim().to_string();
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

/// "Visual Studio Code (code.exe)", or just the file name when the description adds nothing.
fn app_label(exe: &str, description: Option<&str>) -> String {
    match description {
        Some(d) if !d.eq_ignore_ascii_case(exe.trim_end_matches(".exe")) && !d.eq_ignore_ascii_case(exe) => {
            let d: String = d.chars().take(60).collect();
            format!("{d} ({exe})")
        }
        _ => exe.to_string(),
    }
}

/// A running program: file name and the label shown for it.
struct App {
    exe: String,
    label: String,
}

/// Programs with a visible top-level window, sorted by label.
fn running_apps() -> Vec<App> {
    unsafe extern "system" fn collect(window: HWND, list: LPARAM) -> BOOL {
        let list = &mut *(list as *mut Vec<HWND>);
        if IsWindowVisible(window) != 0 && GetWindowTextLengthW(window) > 0 && GetWindow(window, GW_OWNER).is_null() {
            list.push(window);
        }
        1
    }
    let mut windows: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(collect), (&raw mut windows) as LPARAM) };
    let own = unsafe { GetCurrentProcessId() };
    let paths: Vec<String> = windows
        .into_iter()
        .filter(|&w| {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(w, &mut pid) };
            pid != own
        })
        .filter_map(window_exe_path)
        .collect();
    let mut apps: Vec<App> = Vec::new();
    for path in paths {
        let Some(exe) = path.rsplit('\\').next().map(str::to_lowercase) else { continue };
        if apps.iter().any(|a| a.exe == exe) {
            continue;
        }
        let label = app_label(&exe, file_description(&path).as_deref());
        apps.push(App { exe, label });
    }
    apps.sort_by_key(|a| a.label.to_lowercase());
    apps
}

/// The combo box items: a placeholder first, so nothing is picked until the user picks.
fn running_labels(apps: &[App]) -> Vec<String> {
    std::iter::once(PLACEHOLDER.to_string()).chain(apps.iter().map(|a| a.label.clone())).collect()
}

/// The list names running programs like the combo box does; the others by file name.
fn list_labels(english: &[String], running: &[App]) -> Vec<String> {
    english.iter().map(|e| running.iter().find(|a| &a.exe == e).map_or(e.clone(), |a| a.label.clone())).collect()
}

/// A control and where it goes, in 96-DPI pixels relative to its page's top-left corner.
struct Placed {
    hwnd: HWND,
    rect: (i32, i32, i32, i32),
}

/// The controls, created once.
struct Ui {
    window: HWND,
    tab: HWND,
    pages: [HWND; 3],
    placed: Vec<Placed>,
    /// Drawn in grey.
    hints: Vec<HWND>,
    telex: HWND,
    vni: HWND,
    method_hint: HWND,
    modern: HWND,
    spell: HWND,
    restore: HWND,
    list: HWND,
    remove: HWND,
    combo: HWND,
    add: HWND,
    remember: HWND,
    hotkey: HWND,
    autostart: HWND,
    autostart_note: HWND,
    admin: HWND,
    admin_note: HWND,
    check: HWND,
    download: HWND,
    status: HWND,
}

/// What changes while the window is open.
struct Data {
    path: PathBuf,
    task: String,
    autostart_key: String,
    apps: Vec<App>,
    /// The file names behind the rows of the list, which shows labels.
    english: Vec<String>,
    /// Set while waiting for the keyboard process to switch admin mode: the wanted state and since when.
    admin_target: Option<(bool, Instant)>,
    update_available: bool,
}

thread_local! {
    static UI: OnceCell<Ui> = const { OnceCell::new() };
    static DATA: RefCell<Option<Data>> = const { RefCell::new(None) };
    static FONT: Cell<HFONT> = const { Cell::new(null_mut()) };
}

fn ui<R>(f: impl FnOnce(&Ui) -> R) -> Option<R> {
    UI.with(|u| u.get().map(f))
}

/// Borrows the data briefly; never call anything that can show a message box from inside `f`.
fn data<R>(f: impl FnOnce(&mut Data) -> R) -> Option<R> {
    DATA.with(|d| d.try_borrow_mut().ok().and_then(|mut d| d.as_mut().map(f)))
}

fn scale(v: i32, dpi: u32) -> i32 {
    (v as i64 * dpi as i64 / 96) as i32
}

unsafe fn send(w: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    SendMessageW(w, msg, wp, lp)
}

unsafe fn set_text(w: HWND, text: &str) {
    SetWindowTextW(w, wide(text).as_ptr());
}

unsafe fn checked(w: HWND) -> bool {
    send(w, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
}

unsafe fn set_checked(w: HWND, on: bool) {
    send(w, BM_SETCHECK, if on { BST_CHECKED as usize } else { 0 }, 0);
}

unsafe fn enable(w: HWND, on: bool) {
    EnableWindow(w, i32::from(on));
}

unsafe fn fill(w: HWND, items: &[String], add: u32, reset: u32) {
    send(w, reset, 0, 0);
    for item in items {
        send(w, add, 0, wide(item).as_ptr() as isize);
    }
}

/// The message font at the window's DPI (Segoe UI 9 pt on a default Windows).
unsafe fn make_font(dpi: u32) -> HFONT {
    let mut m: NONCLIENTMETRICSW = std::mem::zeroed();
    m.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    if SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, m.cbSize, (&raw mut m).cast(), 0, dpi) == 0 {
        return null_mut();
    }
    CreateFontIndirectW(&m.lfMessageFont)
}

/// Creates the controls of all pages. Positions come later, in `layout`.
unsafe fn create_controls(window: HWND) -> Ui {
    let instance = GetModuleHandleW(null());
    let tab = CreateWindowExW(
        0,
        WC_TABCONTROLW,
        null(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_TABSTOP,
        0,
        0,
        0,
        0,
        window,
        null_mut(),
        instance,
        null(),
    );
    for (i, title) in ["Gõ phím", "Ứng dụng", "Hệ thống"].iter().enumerate() {
        let mut title = wide(title);
        let mut item: TCITEMW = std::mem::zeroed();
        item.mask = TCIF_TEXT;
        item.pszText = title.as_mut_ptr();
        send(tab, TCM_INSERTITEMW, i, (&raw const item) as isize);
    }
    let page_class = wide(PAGE_CLASS);
    let pages = [0; 3].map(|_| {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            page_class.as_ptr(),
            null(),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            window,
            null_mut(),
            instance,
            null(),
        )
    });
    // The pages are drawn over the tab, not under it.
    SetWindowPos(tab, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    let mut placed = Vec::new();
    let mut hints = Vec::new();
    let mut make = |class: &str, text: &str, style: u32, id: u16, page: usize, rect: (i32, i32, i32, i32)| -> HWND {
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | style,
            0,
            0,
            0,
            0,
            pages[page],
            id as usize as HMENU,
            instance,
            null(),
        );
        placed.push(Placed { hwnd, rect });
        hwnd
    };
    let check = (BS_AUTOCHECKBOX as u32) | WS_TABSTOP;
    let button = (BS_PUSHBUTTON as u32) | WS_TABSTOP;
    let group = (BS_GROUPBOX as u32) | WS_CLIPSIBLINGS;
    let text = SS_LEFT | SS_NOPREFIX;
    let combo_style = (CBS_DROPDOWNLIST as u32) | WS_VSCROLL | WS_TABSTOP;
    let w = CLIENT_W - 2 * 10 - 2 * 14; // page width: window margin, tab border and padding
    let mut hint = |hwnd: HWND| {
        hints.push(hwnd);
        hwnd
    };

    // Page 0: typing.
    let g_method = make("BUTTON", "Kiểu gõ", group, 0, 0, (0, 0, w, 76));
    let telex = make("BUTTON", "Telex", (BS_AUTORADIOBUTTON as u32) | WS_GROUP | WS_TABSTOP, ID_TELEX, 0, (14, 22, 100, 20));
    let vni = make("BUTTON", "VNI", BS_AUTORADIOBUTTON as u32, ID_VNI, 0, (124, 22, 100, 20));
    let method_hint = hint(make("STATIC", HINT_TELEX, text, 0, 0, (14, 48, w - 28, 18)));
    let g_marks = make("BUTTON", "Dấu và chính tả", group, 0, 0, (0, 88, w, 192));
    let modern = make("BUTTON", "Bỏ dấu kiểu mới", check | WS_GROUP, ID_MODERN, 0, (14, 110, w - 28, 20));
    hint(make("STATIC", "hoà, thuý thay vì hòa, thúy.", text, 0, 0, (32, 130, w - 46, 18)));
    let spell = make("BUTTON", "Kiểm tra chính tả", check, ID_SPELL, 0, (14, 158, w - 28, 20));
    hint(make("STATIC", "Chỉ thêm dấu khi chữ có thể là tiếng Việt.", text, 0, 0, (32, 178, w - 46, 18)));
    let restore = make("BUTTON", "Tự trả về chữ gốc", check, ID_RESTORE, 0, (14, 206, w - 28, 20));
    hint(make(
        "STATIC",
        "Gõ xong một từ không phải tiếng Việt thì trả lại đúng các phím đã gõ.",
        text,
        0,
        0,
        (32, 226, w - 46, 36),
    ));

    // Page 1: programs.
    let g_apps = make("BUTTON", "Luôn gõ tiếng Anh trong", group, 0, 1, (0, 0, w, 236));
    let list = make(
        "LISTBOX",
        "",
        (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL | WS_BORDER | WS_TABSTOP | WS_GROUP,
        ID_LIST,
        1,
        (14, 22, w - 28 - 90, 100),
    );
    let remove = make("BUTTON", "Xoá", button, ID_REMOVE, 1, (w - 14 - 80, 22, 80, 26));
    let combo = make("COMBOBOX", "", combo_style, ID_COMBO, 1, (14, 132, w - 28 - 180, 300));
    let add = make("BUTTON", "Thêm", button, ID_ADD, 1, (w - 14 - 170, 131, 80, 26));
    make("BUTTON", "Làm mới", button, ID_REFRESH, 1, (w - 14 - 80, 131, 80, 26));
    hint(make(
        "STATIC",
        "Chọn trong các ứng dụng đang mở rồi bấm Thêm; trong ngoặc là tên file chạy. Thêm xong, chuyển sang \
         ứng dụng đó: icon ở khay thành E là đúng. Thường dùng cho game, Terminal, phần mềm thiết kế.",
        text,
        0,
        1,
        (14, 166, w - 28, 56),
    ));
    let remember = make("BUTTON", "Nhớ tiếng Việt / tiếng Anh theo từng ứng dụng", check | WS_GROUP, ID_REMEMBER, 1, (2, 248, w - 2, 20));
    hint(make(
        "STATIC",
        "Quay lại ứng dụng nào thì dùng lại chế độ bạn để lần trước ở đó.",
        text,
        0,
        1,
        (20, 268, w - 20, 18),
    ));

    // Page 2: system.
    let g_hotkey = make("BUTTON", "Phím tắt bật/tắt tiếng Việt", group, 0, 2, (0, 0, w, 80));
    let hotkey = make("COMBOBOX", "", combo_style | WS_GROUP, ID_HOTKEY, 2, (14, 22, 200, 300));
    hint(make("STATIC", "Nhấp vào biểu tượng V/E ở khay hệ thống cũng chuyển được.", text, 0, 2, (14, 52, w - 28, 18)));
    let autostart = make("BUTTON", "Khởi động cùng Windows", check | WS_GROUP, ID_AUTOSTART, 2, (2, 92, w - 2, 20));
    let autostart_note = hint(make(
        "STATIC",
        "Đang chạy với quyền admin, govn đã tự khởi động khi đăng nhập.",
        text,
        0,
        2,
        (20, 112, w - 20, 18),
    ));
    let admin = make("BUTTON", "Chạy với quyền admin", check, ID_ADMIN, 2, (2, 136, w - 2, 20));
    let admin_note = hint(make("STATIC", ADMIN_NOTE, text, 0, 2, (20, 156, w - 20, 36)));
    let version = format!("govn {}", update::CURRENT_VERSION);
    let g_update = make("BUTTON", &version, group, 0, 2, (0, 198, w, 92));
    let check_btn = make("BUTTON", "Kiểm tra cập nhật", button | WS_GROUP, ID_CHECK, 2, (14, 220, 150, 26));
    let download = make("BUTTON", "Mở trang tải về", button, ID_DOWNLOAD, 2, (172, 220, 150, 26));
    let status = make("STATIC", "", text, 0, 2, (14, 252, w - 28, 34));

    // Group boxes sit under the controls drawn on them. The z-order is also the Tab-key order,
    // which otherwise follows creation order.
    for g in [g_method, g_marks, g_apps, g_hotkey, g_update] {
        SetWindowPos(g, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }

    Ui {
        window,
        tab,
        pages,
        placed,
        hints,
        telex,
        vni,
        method_hint,
        modern,
        spell,
        restore,
        list,
        remove,
        combo,
        add,
        remember,
        hotkey,
        autostart,
        autostart_note,
        admin,
        admin_note,
        check: check_btn,
        download,
        status,
    }
}

/// Positions everything for the window's DPI and sets the font.
unsafe fn layout(ui: &Ui) {
    let dpi = GetDpiForWindow(ui.window).max(96);
    let font = make_font(dpi);
    let old = FONT.replace(font);
    let margin = scale(10, dpi);
    let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetClientRect(ui.window, &mut client);
    let (tw, th) = (client.right - 2 * margin, client.bottom - 2 * margin);
    SetWindowPos(ui.tab, null_mut(), margin, margin, tw, th, SWP_NOZORDER | SWP_NOACTIVATE);
    send(ui.tab, WM_SETFONT, font as usize, 0);
    let mut page = RECT { left: margin, top: margin, right: margin + tw, bottom: margin + th };
    send(ui.tab, TCM_ADJUSTRECT, 0, (&raw mut page) as isize);
    for p in ui.pages {
        SetWindowPos(p, null_mut(), page.left, page.top, page.right - page.left, page.bottom - page.top, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    let (ox, oy) = (scale(10, dpi), scale(8, dpi));
    for p in &ui.placed {
        let (x, y, w, h) = p.rect;
        SetWindowPos(
            p.hwnd,
            null_mut(),
            ox + scale(x, dpi),
            oy + scale(y, dpi),
            scale(w, dpi),
            scale(h, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        send(p.hwnd, WM_SETFONT, font as usize, 0);
    }
    if !old.is_null() {
        DeleteObject(old);
    }
    InvalidateRect(ui.window, null(), 1);
}

/// Shows the current page's controls, minus the ones that only appear in some states.
unsafe fn show_page(ui: &Ui) {
    let page = send(ui.tab, TCM_GETCURSEL, 0, 0).max(0) as usize;
    let (update_available, admin_on) = data(|d| (d.update_available, d.admin_target.is_none() && checked(ui.admin)))
        .unwrap_or((false, false));
    for p in &ui.placed {
        let hidden = (p.hwnd == ui.download && !update_available) || (p.hwnd == ui.autostart_note && !admin_on);
        ShowWindow(p.hwnd, if hidden { SW_HIDE } else { SW_SHOWNA });
    }
    for (i, &p) in ui.pages.iter().enumerate() {
        ShowWindow(p, if i == page { SW_SHOWNA } else { SW_HIDE });
    }
}

unsafe fn fill_apps(ui: &Ui) {
    let (labels, running) = data(|d| (list_labels(&d.english, &d.apps), running_labels(&d.apps))).unwrap_or_default();
    fill(ui.list, &labels, LB_ADDSTRING, LB_RESETCONTENT);
    fill(ui.combo, &running, CB_ADDSTRING, CB_RESETCONTENT);
    send(ui.combo, CB_SETCURSEL, 0, 0);
    enable(ui.add, false);
    enable(ui.remove, false);
}

/// Sets every control from the saved settings.
unsafe fn load(ui: &Ui, s: &Settings, admin_on: bool, autostart_on: bool) {
    let vni = s.method == InputMethod::Vni;
    set_checked(ui.telex, !vni);
    set_checked(ui.vni, vni);
    set_text(ui.method_hint, if vni { HINT_VNI } else { HINT_TELEX });
    set_checked(ui.modern, s.modern_style);
    set_checked(ui.spell, s.spell_check);
    set_checked(ui.restore, s.auto_restore);
    fill_apps(ui);
    set_checked(ui.remember, s.remember_apps);
    let hotkeys: Vec<String> = Hotkey::ALL.iter().map(|h| h.label().to_string()).collect();
    fill(ui.hotkey, &hotkeys, CB_ADDSTRING, CB_RESETCONTENT);
    send(ui.hotkey, CB_SETCURSEL, Hotkey::ALL.iter().position(|&h| h == s.hotkey).unwrap_or(0), 0);
    set_admin_state(ui, admin_on, autostart_on);
}

unsafe fn set_admin_state(ui: &Ui, admin_on: bool, autostart_on: bool) {
    set_checked(ui.admin, admin_on);
    enable(ui.admin, true);
    set_checked(ui.autostart, admin_on || autostart_on);
    enable(ui.autostart, !admin_on);
    set_text(ui.admin_note, ADMIN_NOTE);
}

unsafe fn typing_changed(ui: &Ui) {
    let vni = checked(ui.vni);
    set_text(ui.method_hint, if vni { HINT_VNI } else { HINT_TELEX });
    let (modern, spell, restore) = (checked(ui.modern), checked(ui.spell), checked(ui.restore));
    let Some(path) = data(|d| d.path.clone()) else { return };
    edit(&path, |s| {
        s.method = if vni { InputMethod::Vni } else { crate::config::TELEX };
        s.modern_style = modern;
        s.spell_check = spell;
        s.auto_restore = restore;
    });
}

/// Adds or removes a program from the English list.
unsafe fn change_apps(ui: &Ui, change: impl FnOnce(&mut Settings)) {
    let Some(path) = data(|d| d.path.clone()) else { return };
    let mut list = Vec::new();
    if edit(&path, |s| {
        change(s);
        list = s.english_apps.clone();
    }) {
        data(|d| d.english = list);
        let labels = data(|d| list_labels(&d.english, &d.apps)).unwrap_or_default();
        fill(ui.list, &labels, LB_ADDSTRING, LB_RESETCONTENT);
        enable(ui.remove, false);
    }
}

unsafe fn command(ui: &Ui, id: u16, code: u32) {
    match (id, code) {
        (ID_TELEX | ID_VNI | ID_MODERN | ID_SPELL | ID_RESTORE, BN_CLICKED) => typing_changed(ui),
        (ID_LIST, LBN_SELCHANGE) => enable(ui.remove, send(ui.list, LB_GETCURSEL, 0, 0) >= 0),
        (ID_COMBO, CBN_SELCHANGE) => enable(ui.add, send(ui.combo, CB_GETCURSEL, 0, 0) > 0),
        (ID_ADD, BN_CLICKED) => {
            // Item 0 of the combo box is the placeholder.
            let i = send(ui.combo, CB_GETCURSEL, 0, 0);
            let exe = (i as usize).checked_sub(1).and_then(|i| data(|d| d.apps.get(i).map(|a| a.exe.clone())).flatten());
            if let Some(exe) = exe {
                change_apps(ui, |s| {
                    s.add_english_app(&exe);
                });
            }
        }
        (ID_REMOVE, BN_CLICKED) => {
            let i = send(ui.list, LB_GETCURSEL, 0, 0);
            if let Some(exe) = data(|d| d.english.get(i as usize).cloned()).flatten() {
                change_apps(ui, |s| s.english_apps.retain(|a| *a != exe));
            }
        }
        (ID_REFRESH, BN_CLICKED) => {
            let apps = running_apps();
            data(|d| d.apps = apps);
            fill_apps(ui);
        }
        (ID_REMEMBER, BN_CLICKED) => {
            let on = checked(ui.remember);
            if let Some(path) = data(|d| d.path.clone()) {
                edit(&path, |s| s.remember_apps = on);
            }
        }
        (ID_HOTKEY, CBN_SELCHANGE) => {
            let i = send(ui.hotkey, CB_GETCURSEL, 0, 0);
            if let (Some(&h), Some(path)) = (Hotkey::ALL.get(i as usize), data(|d| d.path.clone())) {
                edit(&path, |s| s.hotkey = h);
            }
        }
        (ID_AUTOSTART, BN_CLICKED) => {
            let on = checked(ui.autostart);
            let Some(key) = data(|d| d.autostart_key.clone()) else { return };
            if !tray::autostart_set(&key, on) {
                set_checked(ui.autostart, !on);
                tray::message_box("Không đổi được cài đặt khởi động cùng Windows.");
            }
        }
        (ID_ADMIN, BN_CLICKED) => admin_clicked(ui),
        (ID_CHECK, BN_CLICKED) => check_update(ui),
        (ID_DOWNLOAD, BN_CLICKED) => {
            if !update::open_releases_page() {
                tray::message_box("Không mở được trang tải về.");
            }
        }
        (2, _) => {
            // Esc, through IsDialogMessageW.
            DestroyWindow(ui.window);
        }
        _ => {}
    }
}

/// The keyboard process asks for confirmation and UAC, then may restart; watch the task until it changes.
unsafe fn admin_clicked(ui: &Ui) {
    let on = checked(ui.admin);
    let window = keyboard_window();
    let sent = !window.is_null() && {
        AllowSetForegroundWindow(ASFW_ANY);
        PostMessageW(window, WM_COMMAND, CMD_ADMIN, 0) != 0
    };
    if !sent {
        set_checked(ui.admin, !on);
        set_text(
            ui.admin_note,
            "Không gửi được lệnh tới govn. Hãy mở cửa sổ này từ biểu tượng ở khay hệ thống rồi thử lại.",
        );
        return;
    }
    enable(ui.admin, false);
    data(|d| d.admin_target = Some((on, Instant::now())));
    SetTimer(ui.window, TIMER_ADMIN, 1000, None);
}

unsafe fn admin_tick(ui: &Ui) {
    let Some((Some((on, since)), task, key)) = data(|d| (d.admin_target, d.task.clone(), d.autostart_key.clone())) else {
        return;
    };
    let now = admin::task_exists(&task);
    // Cancelled at the prompt: the task never changes, so give up after a while.
    if now != on && since.elapsed() < Duration::from_secs(120) {
        return;
    }
    KillTimer(ui.window, TIMER_ADMIN);
    data(|d| d.admin_target = None);
    set_admin_state(ui, now, tray::autostart_get(&key).is_some());
    show_page(ui);
}

unsafe fn check_update(ui: &Ui) {
    enable(ui.check, false);
    set_text(ui.check, "Đang kiểm tra...");
    set_text(ui.status, "");
    data(|d| d.update_available = false);
    show_page(ui);
    let window = ui.window as isize;
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(update::check).unwrap_or(Check::Failed);
        let (kind, version) = match result {
            Check::UpToDate => (0, null_mut()),
            Check::Available(v) => (1, Box::into_raw(Box::new(v))),
            Check::Failed => (2, null_mut()),
        };
        unsafe {
            if PostMessageW(window as HWND, WM_UPDATE_DONE, kind, version as isize) == 0 && !version.is_null() {
                drop(Box::from_raw(version));
            }
        }
    });
}

unsafe fn update_done(ui: &Ui, kind: usize, version: *mut String) {
    let version = (!version.is_null()).then(|| *Box::from_raw(version));
    enable(ui.check, true);
    set_text(ui.check, "Kiểm tra cập nhật");
    let current = update::CURRENT_VERSION;
    let text = match (kind, version) {
        (0, _) => format!("Bạn đang dùng bản mới nhất ({current})."),
        (1, Some(v)) => format!("Có bản mới {v}. Thoát govn rồi chép đè file govn.exe."),
        _ => "Không kiểm tra được. Hãy kiểm tra kết nối mạng rồi thử lại.".to_string(),
    };
    set_text(ui.status, &text);
    data(|d| d.update_available = kind == 1);
    show_page(ui);
}

unsafe extern "system" fn wndproc(window: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            if ui(|ui| command(ui, (wp & 0xffff) as u16, (wp >> 16) as u32 & 0xffff)).is_some() {
                return 0;
            }
        }
        WM_NOTIFY => {
            let header = &*(lp as *const NMHDR);
            if header.code == TCN_SELCHANGE {
                ui(|ui| show_page(ui));
                return 0;
            }
        }
        WM_CTLCOLORSTATIC => {
            // Checkboxes, group boxes and labels sit on the white page.
            let dc = wp as HDC;
            let grey = ui(|ui| ui.hints.contains(&(lp as HWND))).unwrap_or(false);
            SetTextColor(dc, GetSysColor(if grey { COLOR_GRAYTEXT } else { COLOR_WINDOWTEXT }));
            SetBkColor(dc, GetSysColor(COLOR_WINDOW));
            return GetSysColorBrush(COLOR_WINDOW) as LRESULT;
        }
        WM_TIMER if wp == TIMER_ADMIN => {
            ui(|ui| admin_tick(ui));
            return 0;
        }
        WM_UPDATE_DONE => {
            if ui(|ui| update_done(ui, wp, lp as *mut String)).is_none() && lp != 0 {
                drop(Box::from_raw(lp as *mut String));
            }
            return 0;
        }
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(window, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            ui(|ui| layout(ui));
            return 0;
        }
        WM_DESTROY if GetWindowLongPtrW(window, GWL_STYLE) as u32 & WS_CHILD == 0 => {
            PostQuitMessage(0);
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(window, msg, wp, lp)
}

/// The monitor under the mouse, which is where the tray icon was clicked: its work area and DPI.
fn monitor_under_mouse() -> (RECT, u32) {
    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..std::mem::zeroed() };
        GetMonitorInfoW(monitor, &mut info);
        let (mut dx, mut dy) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (info.rcWork, dx.max(96))
    }
}

pub fn run(path: PathBuf, task: String, autostart_key: String) -> i32 {
    unsafe {
        CreateMutexW(null(), 0, wide("Local\\govn-settings").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let w = FindWindowW(wide(CLASS).as_ptr(), null());
            if !w.is_null() {
                ShowWindow(w, SW_RESTORE);
                SetForegroundWindow(w);
            }
            return 0;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&icc);

        let instance = GetModuleHandleW(null());
        let (work, dpi) = monitor_under_mouse();
        // The app icon is resource 1, see govn.rc.
        #[allow(clippy::manual_dangling_ptr)] // MAKEINTRESOURCE(1)
        let icon = |size: i32| LoadImageW(instance, 1usize as *const u16, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
        let class_name = wide(CLASS);
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hIcon: icon(scale(32, dpi)),
            hIconSm: icon(scale(16, dpi)),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszClassName: class_name.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&class);
        // Pages share the window procedure: their controls report to it.
        let page_class = wide(PAGE_CLASS);
        RegisterClassExW(&WNDCLASSEXW { lpszClassName: page_class.as_ptr(), hIcon: null_mut(), hIconSm: null_mut(), ..class });

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let ex_style = WS_EX_CONTROLPARENT;
        let mut r = RECT { left: 0, top: 0, right: scale(CLIENT_W, dpi), bottom: scale(CLIENT_H, dpi) };
        AdjustWindowRectExForDpi(&mut r, style, 0, ex_style, dpi);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let x = (work.left + (work.right - work.left - w) / 2).max(work.left);
        let y = (work.top + (work.bottom - work.top - h) / 2).max(work.top);
        let window =
            CreateWindowExW(ex_style, class_name.as_ptr(), wide(TITLE).as_ptr(), style, x, y, w, h, null_mut(), null_mut(), instance, null());
        if window.is_null() {
            tray::message_box("Không mở được cửa sổ cài đặt.");
            return 1;
        }

        let s = Settings::load(&path);
        let admin_on = admin::task_exists(&task);
        let autostart_on = tray::autostart_get(&autostart_key).is_some();
        DATA.with(|d| {
            *d.borrow_mut() = Some(Data {
                path,
                task,
                autostart_key,
                apps: running_apps(),
                english: s.english_apps.clone(),
                admin_target: None,
                update_available: false,
            })
        });
        let controls = create_controls(window);
        load(&controls, &s, admin_on, autostart_on);
        layout(&controls);
        UI.with(|u| {
            let _ = u.set(controls);
        });
        ui(|ui| show_page(ui));
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // Ctrl+Tab and Ctrl+Shift+Tab switch pages, as in other Windows settings dialogs.
            if msg.message == WM_KEYDOWN && msg.wParam == VK_TAB as usize && GetKeyState(VK_CONTROL as i32) < 0 {
                ui(|ui| {
                    let back = GetKeyState(VK_SHIFT as i32) < 0;
                    let page = send(ui.tab, TCM_GETCURSEL, 0, 0);
                    send(ui.tab, TCM_SETCURSEL, ((page + if back { 2 } else { 1 }) % 3) as usize, 0);
                    show_page(ui);
                });
                continue;
            }
            if IsDialogMessageW(window, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let font = FONT.replace(null_mut());
        if !font.is_null() {
            DeleteObject(font);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_apps_lists_each_program_once() {
        let apps = running_apps();
        let mut exes: Vec<&str> = apps.iter().map(|a| a.exe.as_str()).collect();
        assert!(exes.iter().all(|e| *e == e.to_lowercase()));
        exes.sort();
        exes.dedup();
        assert_eq!(exes.len(), apps.len());
        assert!(apps.iter().all(|a| a.label.ends_with(&a.exe) || a.label.ends_with(&format!("({})", a.exe))));
        assert_eq!(running_labels(&apps).len(), apps.len() + 1);
    }

    #[test]
    fn labels() {
        assert_eq!(app_label("code.exe", Some("Visual Studio Code")), "Visual Studio Code (code.exe)");
        assert_eq!(app_label("zalo.exe", Some("Zalo")), "zalo.exe");
        assert_eq!(app_label("x.exe", None), "x.exe");
        assert_eq!(app_label("x.exe", Some("X.EXE")), "x.exe");
        let running = [App { exe: "code.exe".into(), label: "Visual Studio Code (code.exe)".into() }];
        assert_eq!(
            list_labels(&["code.exe".into(), "game.exe".into()], &running),
            ["Visual Studio Code (code.exe)", "game.exe"]
        );
    }

    #[test]
    fn reads_description_of_a_system_program() {
        let windir = std::env::var("WINDIR").unwrap();
        let d = file_description(&format!("{windir}\\System32\\notepad.exe")).or_else(|| file_description(&format!("{windir}\\notepad.exe")));
        assert!(d.is_some_and(|d| !d.is_empty()));
        assert_eq!(file_description("C:\\no\\such\\file.exe"), None);
    }
}
