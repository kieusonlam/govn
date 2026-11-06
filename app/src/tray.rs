//! Notification-area icon, its menu, and "start with Windows".
//!
//! `STATE` is never kept borrowed across a Win32 call that can pump messages, because the keyboard
//! hook runs on this same thread and borrows it too.
use crate::admin;
use crate::config::{Settings, SuggestionFix};
use crate::tray_ids::*;
use crate::{current_settings, wide, STATE};
use govn_engine::{InputMethod, Options};
use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW,
    GdiFlush, GetStockObject, RoundRect, SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FW_BOLD, NULL_PEN, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, ChangeWindowMessageFilterEx, CheckMenuRadioItem, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, EnableMenuItem, GetCursorPos,
    GetSystemMetrics, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetForegroundWindow, TrackPopupMenu, HICON, HMENU, ICONINFO, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO, MF_BYCOMMAND, MF_CHECKED, MF_GRAYED, MSGFLT_ALLOW, IDYES, MF_POPUP, MF_SEPARATOR,
    MF_STRING, SM_CXSMICON, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_COMMAND, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    WS_POPUP,
};

const ICON_ID: u32 = 1;
const AUTOSTART_VALUE: &str = "govn";
/// Colours are COLORREF, 0x00BBGGRR.
const COLOR_VIET: u32 = 0x002F_2FD3;
const COLOR_ENGLISH: u32 = 0x0064_5A45;

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static ICON_V: Cell<HICON> = const { Cell::new(null_mut()) };
    static ICON_E: Cell<HICON> = const { Cell::new(null_mut()) };
    static SHOW_ICON: Cell<bool> = const { Cell::new(false) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static ADMIN_ENABLED: Cell<bool> = const { Cell::new(false) };
    static RESTART_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

pub fn message_box(text: &str) {
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide("govn").as_ptr(), MB_OK | MB_ICONERROR) };
}

/// A square icon with one letter. GDI leaves alpha at zero, so alpha is filled in afterwards.
unsafe fn make_icon(letter: &str, background: u32) -> HICON {
    let size = GetSystemMetrics(SM_CXSMICON).max(16);
    let dc = CreateCompatibleDC(null_mut());
    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = size;
    info.bmiHeader.biHeight = -size; // top-down
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut c_void = null_mut();
    let color = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old_bitmap = SelectObject(dc, color);

    let brush = CreateSolidBrush(background);
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    RoundRect(dc, 0, 0, size + 1, size + 1, size / 3, size / 3);

    let font = CreateFontW(
        -(size * 85 / 100),
        0,
        0,
        0,
        FW_BOLD as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        ANTIALIASED_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide("Segoe UI").as_ptr(),
    );
    let old_font = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, 0x00FF_FFFF);
    let mut rect = RECT { left: 0, top: 0, right: size, bottom: size };
    let mut text: Vec<u16> = letter.encode_utf16().collect();
    DrawTextW(dc, text.as_mut_ptr(), text.len() as i32, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    GdiFlush();

    let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize);
    for p in pixels {
        if *p & 0x00FF_FFFF != 0 {
            *p |= 0xFF00_0000;
        }
    }

    // The mask is ignored for a 32-bit icon, but it must exist and rows are padded to 16 bits.
    let mask_bits = vec![0u8; ((size as usize + 15) / 16) * 2 * size as usize];
    let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
    let icon_info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&icon_info);

    SelectObject(dc, old_font);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_bitmap);
    DeleteObject(font);
    DeleteObject(brush);
    DeleteObject(mask);
    DeleteObject(color);
    DeleteDC(dc);
    icon
}

fn snapshot() -> Option<Settings> {
    STATE.with_borrow(|s| s.as_ref().map(current_settings))
}

fn copy_utf16(dst: &mut [u16], text: &str) {
    let max = dst.len() - 1;
    let mut n = 0;
    for unit in text.encode_utf16().take(max) {
        dst[n] = unit;
        n += 1;
    }
    dst[n] = 0;
}

fn method_name(m: InputMethod) -> &'static str {
    match m {
        InputMethod::Telex => "Telex",
        InputMethod::Vni => "VNI",
        InputMethod::SimpleTelex => "Simple Telex",
        InputMethod::SimpleTelex2 => "Simple Telex 2",
    }
}

unsafe fn icon_data() -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = std::mem::zeroed();
    d.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = WINDOW.get();
    d.uID = ICON_ID;
    d
}

unsafe fn fill_icon(d: &mut NOTIFYICONDATAW) {
    let Some(s) = snapshot() else { return };
    d.uFlags |= NIF_ICON | NIF_TIP | NIF_MESSAGE;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = if s.viet { ICON_V.get() } else { ICON_E.get() };
    let tip = if s.viet {
        format!("govn: Tiếng Việt ({})\nNhấp để tắt, Ctrl+Shift", method_name(s.method))
    } else {
        "govn: English\nNhấp để bật, Ctrl+Shift".to_string()
    };
    copy_utf16(&mut d.szTip, &tip);
}

/// Can fail while the shell is still starting; `TaskbarCreated` then adds the icon again.
fn add_icon() {
    if !SHOW_ICON.get() {
        return;
    }
    unsafe {
        let mut d = icon_data();
        fill_icon(&mut d);
        Shell_NotifyIconW(NIM_ADD, &d);
    }
}

fn refresh_icon() {
    if !SHOW_ICON.get() {
        return;
    }
    unsafe {
        let mut d = icon_data();
        fill_icon(&mut d);
        Shell_NotifyIconW(NIM_MODIFY, &d);
    }
}

pub fn remove_icon() {
    if SHOW_ICON.get() {
        unsafe {
            let d = icon_data();
            Shell_NotifyIconW(NIM_DELETE, &d);
        }
    }
}

pub fn show_warning(text: &str) {
    if !SHOW_ICON.get() {
        return;
    }
    unsafe {
        let mut d = icon_data();
        d.uFlags = NIF_INFO;
        d.dwInfoFlags = NIIF_WARNING;
        copy_utf16(&mut d.szInfoTitle, "govn");
        copy_utf16(&mut d.szInfo, text);
        Shell_NotifyIconW(NIM_MODIFY, &d);
    }
}

fn exe_command() -> Option<Vec<u16>> {
    let exe = std::env::current_exe().ok()?;
    Some(wide(&format!("\"{}\"", exe.display())))
}

fn autostart_get(key: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(AUTOSTART_VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    (r == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)]))
}

fn autostart_set(key: &str, on: bool) -> bool {
    unsafe {
        if on {
            let Some(cmd) = exe_command() else { return false };
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                wide(AUTOSTART_VALUE).as_ptr(),
                REG_SZ,
                cmd.as_ptr().cast(),
                (cmd.len() * 2) as u32,
            ) == ERROR_SUCCESS
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(AUTOSTART_VALUE).as_ptr()) == ERROR_SUCCESS
        }
    }
}

/// If the program was moved, an enabled autostart entry would point at nothing; keep it current.
pub fn autostart_sync(key: &str) {
    if autostart_get(key).is_some() {
        autostart_set(key, true);
    }
}

fn autostart_key() -> String {
    STATE.with_borrow(|s| s.as_ref().map(|st| st.autostart_key.clone()).unwrap_or_default())
}

pub fn set_admin_enabled(on: bool) {
    ADMIN_ENABLED.set(on);
}

/// Admin mode was just turned on here: end this process and start the elevated task.
pub fn restart_requested() -> bool {
    RESTART_REQUESTED.get()
}

const ADMIN_WARNING: &str = "Bật để gõ được trong các ứng dụng chạy quyền admin.\n\n\
govn sẽ tự chạy với quyền admin mỗi lần đăng nhập mà không hỏi lại. Vì vậy hãy để govn.exe trong thư mục \
chỉ admin mới sửa được (ví dụ Program Files); nếu không, phần mềm khác có thể thay file này để chiếm quyền admin.\n\n\
Bật không?";

fn toggle_admin() {
    let on = !ADMIN_ENABLED.get();
    let Some((name, args, elevated, testing)) = STATE.with_borrow(|s| {
        s.as_ref().map(|st| (st.task_name.clone(), st.task_args.clone(), st.elevated, st.only_pid.is_some()))
    }) else {
        return;
    };
    if on && !testing {
        let answer = unsafe {
            MessageBoxW(WINDOW.get(), wide(ADMIN_WARNING).as_ptr(), wide("govn").as_ptr(), MB_YESNO | MB_ICONWARNING)
        };
        if answer != IDYES {
            return;
        }
    }
    // On a worker: the keyboard hook runs on this thread and must not wait for the UAC prompt.
    let window = WINDOW.get() as usize;
    std::thread::spawn(move || {
        let result = if elevated {
            if admin::set_task(&name, on, &args) { admin::RESULT_OK } else { admin::RESULT_FAILED }
        } else {
            admin::set_task_elevated(&name, on, &args)
        };
        unsafe { PostMessageW(window as HWND, WM_ADMIN_DONE, result as WPARAM, LPARAM::from(on)) };
    });
}

fn admin_changed(result: u32, on: bool) {
    match result {
        admin::RESULT_OK => {}
        admin::RESULT_CANCELLED => return,
        _ => {
            message_box("Không đổi được chế độ quyền admin.");
            return;
        }
    }
    ADMIN_ENABLED.set(on);
    // The task replaces the Run entry; turning it off must not lose "start with Windows".
    autostart_set(&autostart_key(), !on);
    let elevated = STATE.with_borrow(|s| s.as_ref().is_some_and(|st| st.elevated));
    match (on, elevated) {
        (true, false) => {
            RESTART_REQUESTED.set(true);
            unsafe { PostQuitMessage(0) };
        }
        (false, true) => show_warning("Đã tắt. Từ lần mở sau, govn chạy với quyền thường."),
        _ => {}
    }
}

fn save_settings() -> bool {
    let (settings, path) = STATE.with_borrow(|s| match s.as_ref() {
        Some(st) => (Some(current_settings(st)), st.config_path.clone()),
        None => (None, None),
    });
    match (settings, path) {
        (Some(s), Some(p)) => s.save(&p).is_ok(),
        _ => true,
    }
}

fn run_command(cmd: usize) {
    match cmd {
        CMD_QUIT => {
            unsafe { PostQuitMessage(0) };
            return;
        }
        CMD_ADMIN => {
            toggle_admin();
            return;
        }
        CMD_AUTOSTART => {
            if ADMIN_ENABLED.get() {
                return; // the task already starts govn; the menu item is greyed out
            }
            let key = autostart_key();
            let on = autostart_get(&key).is_none();
            if !autostart_set(&key, on) {
                message_box("Không đổi được cài đặt khởi động cùng Windows.");
            }
            return;
        }
        _ => {}
    }

    STATE.with_borrow_mut(|s| {
        let Some(st) = s.as_mut() else { return };
        let mut options: Options = st.engine.options;
        match cmd {
            CMD_TOGGLE => {
                st.engine.reset();
                st.engine.viet_key = !st.engine.viet_key;
            }
            CMD_TELEX => st.engine.set_input_method(InputMethod::Telex),
            CMD_VNI => st.engine.set_input_method(InputMethod::Vni),
            CMD_SIMPLE => st.engine.set_input_method(InputMethod::SimpleTelex),
            CMD_SIMPLE2 => st.engine.set_input_method(InputMethod::SimpleTelex2),
            CMD_MODERN => options.modern_style = !options.modern_style,
            CMD_SPELL => options.spell_check = !options.spell_check,
            CMD_RESTORE => options.auto_non_vn_restore = !options.auto_non_vn_restore,
            CMD_FIX_AUTO => st.suggestion_fix = SuggestionFix::Auto,
            CMD_FIX_ALWAYS => st.suggestion_fix = SuggestionFix::Always,
            CMD_FIX_NEVER => st.suggestion_fix = SuggestionFix::Never,
            _ => return,
        }
        st.engine.options = options;
        st.engine.reset();
        // The placeholder decision is made when the foreground window changes: force it again.
        st.last_foreground = null_mut();
    });
    refresh_icon();
    if !save_settings() {
        message_box("Không lưu được cài đặt.");
    }
}

unsafe fn add_item(menu: HMENU, id: usize, text: &str, checked: bool) {
    let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
    AppendMenuW(menu, flags, id, wide(text).as_ptr());
}

unsafe fn show_menu() {
    let Some(s) = snapshot() else { return };
    let admin_on = ADMIN_ENABLED.get();
    let autostart = admin_on || autostart_get(&autostart_key()).is_some();

    let methods = CreatePopupMenu();
    for (id, m) in [
        (CMD_TELEX, InputMethod::Telex),
        (CMD_VNI, InputMethod::Vni),
        (CMD_SIMPLE, InputMethod::SimpleTelex),
        (CMD_SIMPLE2, InputMethod::SimpleTelex2),
    ] {
        add_item(methods, id, method_name(m), false);
        if s.method == m {
            CheckMenuRadioItem(methods, CMD_TELEX as u32, CMD_SIMPLE2 as u32, id as u32, MF_BYCOMMAND);
        }
    }

    let fixes = CreatePopupMenu();
    add_item(fixes, CMD_FIX_AUTO, "Tự động (trình duyệt, Excel)", false);
    add_item(fixes, CMD_FIX_ALWAYS, "Luôn bật", false);
    add_item(fixes, CMD_FIX_NEVER, "Tắt", false);
    let fix_id = match s.suggestion_fix {
        SuggestionFix::Auto => CMD_FIX_AUTO,
        SuggestionFix::Always => CMD_FIX_ALWAYS,
        SuggestionFix::Never => CMD_FIX_NEVER,
    };
    CheckMenuRadioItem(fixes, CMD_FIX_AUTO as u32, CMD_FIX_NEVER as u32, fix_id as u32, MF_BYCOMMAND);

    let menu = CreatePopupMenu();
    add_item(menu, CMD_TOGGLE, "Tiếng Việt\tCtrl+Shift", s.viet);
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_POPUP, methods as usize, wide("Kiểu gõ").as_ptr());
    add_item(menu, CMD_MODERN, "Bỏ dấu kiểu mới (oà, uý)", s.modern_style);
    add_item(menu, CMD_SPELL, "Kiểm tra chính tả", s.spell_check);
    add_item(menu, CMD_RESTORE, "Tự khôi phục từ không phải tiếng Việt", s.auto_restore);
    AppendMenuW(menu, MF_POPUP, fixes as usize, wide("Sửa lỗi gợi ý tự động").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    add_item(menu, CMD_AUTOSTART, "Khởi động cùng Windows", autostart);
    if admin_on {
        EnableMenuItem(menu, CMD_AUTOSTART as u32, MF_BYCOMMAND | MF_GRAYED);
    }
    add_item(menu, CMD_ADMIN, "Chạy với quyền admin", admin_on);
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    add_item(menu, CMD_QUIT, "Thoát\tCtrl+Alt+Shift+Q", false);

    let window = WINDOW.get();
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    // Without this the menu does not close when the user clicks elsewhere.
    SetForegroundWindow(window);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, window, null());
    PostMessageW(window, WM_NULL, 0, 0);
    DestroyMenu(menu); // also destroys the submenus
    if cmd != 0 {
        run_command(cmd as usize);
    }
}

fn dispatch(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_TRAY => {
            match lparam as u32 & 0xFFFF {
                WM_LBUTTONUP => run_command(CMD_TOGGLE),
                WM_RBUTTONUP => unsafe { show_menu() },
                _ => {}
            }
            Some(0)
        }
        WM_COMMAND => {
            run_command(wparam & 0xFFFF);
            Some(0)
        }
        WM_ADMIN_DONE => {
            admin_changed(wparam as u32, lparam != 0);
            Some(0)
        }
        WM_STATE_CHANGED => {
            refresh_icon();
            save_settings();
            Some(0)
        }
        m if m != 0 && m == TASKBAR_CREATED.get() => {
            add_icon();
            Some(0)
        }
        _ => None,
    }
}

unsafe extern "system" fn window_proc(window: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A panic must not cross into the OS; ignoring one menu action is the better failure.
    match catch_unwind(AssertUnwindSafe(|| dispatch(msg, wparam, lparam))) {
        Ok(Some(r)) => r,
        _ => DefWindowProcW(window, msg, wparam, lparam),
    }
}

/// Creates the hidden window that owns the icon. Returns it, even when no icon is shown.
pub fn create(show_icon: bool) -> HWND {
    unsafe {
        let module = GetModuleHandleW(null());
        let class = wide(WINDOW_CLASS);
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(window_proc);
        wc.hInstance = module;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let window = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), module, null());
        WINDOW.set(window);
        TASKBAR_CREATED.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
        // When elevated, Windows would drop these messages because the shell runs with normal rights.
        ChangeWindowMessageFilterEx(window, TASKBAR_CREATED.get(), MSGFLT_ALLOW, null_mut());
        ChangeWindowMessageFilterEx(window, WM_TRAY, MSGFLT_ALLOW, null_mut());
        SHOW_ICON.set(show_icon);
        if show_icon {
            ICON_V.set(make_icon("V", COLOR_VIET));
            ICON_E.set(make_icon("E", COLOR_ENGLISH));
            add_icon();
        }
        window
    }
}
