//! The settings window, a separate process started with `--settings`.
//!
//! The keyboard never loads this code, so its memory stays small. Each change is written to the
//! settings file (read-modify-write, since the keyboard writes it too) and the keyboard is told to
//! read it again. Admin mode is still switched by the keyboard process, which owns the UAC flow.
use crate::admin;
use crate::config::{Hotkey, Settings};
use crate::tray;
use crate::tray_ids::{CMD_ADMIN, WINDOW_CLASS, WM_RELOAD};
use crate::update::{self, Check};
use crate::{wide, window_exe_path};
use govn_engine::InputMethod;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{GetLastError, BOOL, ERROR_ALREADY_EXISTS, HWND, LPARAM, POINT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, EnumWindows, FindWindowW, GetCursorPos, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsWindowVisible, PostMessageW, SetForegroundWindow, ShowWindow, ASFW_ANY, GW_OWNER, SW_RESTORE, WM_COMMAND,
};

slint::include_modules!();

const TITLE: &str = "govn - Cài đặt";
const ADMIN_NOTE: &str = "Để gõ được trong ứng dụng chạy quyền admin. Chỉ bật khi govn.exe nằm ở thư mục chỉ admin sửa được, như Program Files.";

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

fn method_from(ui: &SettingsWindow) -> InputMethod {
    if ui.get_method() == 1 { InputMethod::Vni } else { crate::config::TELEX }
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

/// The combo box items: a placeholder first, since the combo box always shows a selection.
fn running_labels(apps: &[App]) -> Vec<SharedString> {
    std::iter::once(SharedString::from("Chọn ứng dụng…")).chain(apps.iter().map(|a| SharedString::from(&a.label))).collect()
}

/// Fits the tallest tab, "Hệ thống"; keep in step with `preferred-*` in settings.slint.
const SIZE: slint::LogicalSize = slint::LogicalSize { width: 560.0, height: 408.0 };

/// Centres the window on the screen under the mouse, which is where the tray icon was clicked.
/// Right after `show` the window still reports its minimum size, so the size is set here first.
fn center(ui: &SettingsWindow) {
    ui.window().set_size(SIZE);
    let size = SIZE.to_physical(ui.window().scale_factor());
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..unsafe { std::mem::zeroed() } };
    let found = unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut info) != 0
    };
    if !found || size.width == 0 {
        return;
    }
    let work = info.rcWork;
    let x = work.left + (work.right - work.left - size.width as i32) / 2;
    let y = work.top + (work.bottom - work.top - size.height as i32) / 2;
    ui.window().set_position(slint::PhysicalPosition::new(x.max(work.left), y.max(work.top)));
}

/// The list names running programs like the combo box does; the others by file name.
fn set_apps(model: &VecModel<SharedString>, apps: &[String], running: &[App]) {
    let label = |exe: &String| running.iter().find(|a| &a.exe == exe).map_or(exe.clone(), |a| a.label.clone());
    model.set_vec(apps.iter().map(|e| SharedString::from(label(e))).collect::<Vec<_>>());
}

pub fn run(path: PathBuf, task: String, autostart_key: String) -> i32 {
    unsafe {
        CreateMutexW(null(), 0, wide("Local\\govn-settings").as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let w = FindWindowW(null(), wide(TITLE).as_ptr());
            if !w.is_null() {
                ShowWindow(w, SW_RESTORE);
                SetForegroundWindow(w);
            }
            return 0;
        }
    }
    let Ok(ui) = SettingsWindow::new() else {
        tray::message_box("Không mở được cửa sổ cài đặt.");
        return 1;
    };
    let s = Settings::load(&path);
    let path = Rc::new(path);

    ui.set_method(i32::from(s.method == InputMethod::Vni));
    ui.set_modern_style(s.modern_style);
    ui.set_spell_check(s.spell_check);
    ui.set_auto_restore(s.auto_restore);

    let apps = Rc::new(RefCell::new(running_apps()));
    let english = Rc::new(VecModel::<SharedString>::default());
    set_apps(&english, &s.english_apps, &apps.borrow());
    ui.set_english_apps(ModelRc::from(english.clone()));
    let running = Rc::new(VecModel::from(running_labels(&apps.borrow())));
    ui.set_running_apps(ModelRc::from(running.clone()));
    ui.set_remember_apps(s.remember_apps);

    let labels: Vec<SharedString> = Hotkey::ALL.iter().map(|h| SharedString::from(h.label())).collect();
    ui.set_hotkeys(ModelRc::from(Rc::new(VecModel::from(labels))));
    ui.set_hotkey(Hotkey::ALL.iter().position(|&h| h == s.hotkey).unwrap_or(0) as i32);

    let admin_on = admin::task_exists(&task);
    ui.set_admin(admin_on);
    ui.set_admin_note(ADMIN_NOTE.into());
    ui.set_autostart(admin_on || tray::autostart_get(&autostart_key).is_some());
    ui.set_autostart_enabled(!admin_on);
    ui.set_version(update::CURRENT_VERSION.into());

    ui.on_typing_changed({
        let (ui, path) = (ui.as_weak(), path.clone());
        move || {
            let Some(ui) = ui.upgrade() else { return };
            edit(&path, |s| {
                s.method = method_from(&ui);
                s.modern_style = ui.get_modern_style();
                s.spell_check = ui.get_spell_check();
                s.auto_restore = ui.get_auto_restore();
            });
        }
    });

    // The file names behind the rows of the list, which show labels.
    let shown = Rc::new(RefCell::new(s.english_apps.clone()));
    // Item 0 of the combo box is the placeholder.
    ui.on_add_app({
        let (english, path, apps, shown) = (english.clone(), path.clone(), apps.clone(), shown.clone());
        move |i| {
            let Some(exe) = (i as usize).checked_sub(1).and_then(|i| apps.borrow().get(i).map(|a| a.exe.clone())) else {
                return;
            };
            let mut list = Vec::new();
            if edit(&path, |s| {
                s.add_english_app(&exe);
                list = s.english_apps.clone();
            }) {
                set_apps(&english, &list, &apps.borrow());
                *shown.borrow_mut() = list;
            }
        }
    });
    ui.on_remove_app({
        let (english, path, apps, shown) = (english.clone(), path.clone(), apps.clone(), shown.clone());
        move |i| {
            let Some(exe) = shown.borrow().get(i as usize).cloned() else { return };
            let mut list = Vec::new();
            if edit(&path, |s| {
                s.english_apps.retain(|a| *a != exe);
                list = s.english_apps.clone();
            }) {
                set_apps(&english, &list, &apps.borrow());
                *shown.borrow_mut() = list;
            }
        }
    });
    ui.on_refresh_running({
        let (ui, running, apps) = (ui.as_weak(), running.clone(), apps.clone());
        move || {
            *apps.borrow_mut() = running_apps();
            running.set_vec(running_labels(&apps.borrow()));
            if let Some(ui) = ui.upgrade() {
                ui.set_running_index(0);
            }
        }
    });
    ui.on_remember_changed({
        let path = path.clone();
        move |on| {
            edit(&path, |s| s.remember_apps = on);
        }
    });

    ui.on_hotkey_changed({
        let path = path.clone();
        move |i| {
            if let Some(&h) = Hotkey::ALL.get(i as usize) {
                edit(&path, |s| s.hotkey = h);
            }
        }
    });
    ui.on_autostart_changed({
        let ui = ui.as_weak();
        move |on| {
            if !tray::autostart_set(&autostart_key, on) {
                tray::message_box("Không đổi được cài đặt khởi động cùng Windows.");
                if let Some(ui) = ui.upgrade() {
                    ui.set_autostart(!on);
                }
            }
        }
    });

    // The keyboard process asks for confirmation and UAC, then may restart; watch the task until it changes.
    let admin_timer = Rc::new(Timer::default());
    ui.on_admin_changed({
        let (ui, timer, task) = (ui.as_weak(), admin_timer.clone(), task.clone());
        move |on| {
            let Some(handle) = ui.upgrade() else { return };
            let window = keyboard_window();
            let sent = !window.is_null() && unsafe {
                AllowSetForegroundWindow(ASFW_ANY);
                PostMessageW(window, WM_COMMAND, CMD_ADMIN, 0)
            } != 0;
            if !sent {
                handle.set_admin(!on);
                handle.set_admin_note(
                    "Không gửi được lệnh tới govn. Hãy mở cửa sổ này từ biểu tượng ở khay hệ thống rồi thử lại.".into(),
                );
                return;
            }
            handle.set_admin_busy(true);
            let started = std::time::Instant::now();
            let (ui, task, me) = (ui.clone(), task.clone(), Rc::downgrade(&timer));
            timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
                let Some(handle) = ui.upgrade() else { return };
                let now = admin::task_exists(&task);
                // Cancelled at the prompt: the task never changes, so give up after a while.
                if now == on || started.elapsed() > Duration::from_secs(120) {
                    handle.set_admin(now);
                    handle.set_admin_busy(false);
                    handle.set_autostart_enabled(!now);
                    if now {
                        handle.set_autostart(true);
                    }
                    handle.set_admin_note(ADMIN_NOTE.into());
                    if let Some(t) = me.upgrade() {
                        t.stop();
                    }
                }
            });
        }
    });

    ui.on_check_update({
        let ui = ui.as_weak();
        move || {
            let Some(handle) = ui.upgrade() else { return };
            handle.set_checking(true);
            handle.set_update_status("".into());
            handle.set_update_available(false);
            let ui = ui.clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(update::check).unwrap_or(Check::Failed);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(handle) = ui.upgrade() else { return };
                    handle.set_checking(false);
                    let current = update::CURRENT_VERSION;
                    let (text, available) = match result {
                        Check::UpToDate => (format!("Bạn đang dùng bản mới nhất ({current})."), false),
                        Check::Available(v) => (format!("Có bản mới {v}. Thoát govn rồi chép đè file govn.exe."), true),
                        Check::Failed => ("Không kiểm tra được. Hãy kiểm tra kết nối mạng rồi thử lại.".to_string(), false),
                    };
                    handle.set_update_status(text.into());
                    handle.set_update_available(available);
                });
            });
        }
    });
    ui.on_open_download(|| {
        if !update::open_releases_page() {
            tray::message_box("Không mở được trang tải về.");
        }
    });

    if ui.show().is_err() {
        return 1;
    }
    center(&ui);
    i32::from(slint::run_event_loop().is_err())
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
    }

    #[test]
    fn reads_description_of_a_system_program() {
        let windir = std::env::var("WINDIR").unwrap();
        let d = file_description(&format!("{windir}\\System32\\notepad.exe")).or_else(|| file_description(&format!("{windir}\\notepad.exe")));
        assert!(d.is_some_and(|d| !d.is_empty()));
        assert_eq!(file_description("C:\\no\\such\\file.exe"), None);
    }
}
