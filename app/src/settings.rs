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
use crate::{wide, window_exe_name};
use govn_engine::InputMethod;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::path::{Path, PathBuf};
use std::ptr::null;
use std::rc::Rc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{GetLastError, BOOL, ERROR_ALREADY_EXISTS, HWND, LPARAM, POINT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
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

fn method_from(ui: &SettingsWindow, previous: InputMethod) -> InputMethod {
    match (ui.get_method(), ui.get_w_shortcuts()) {
        (1, _) => InputMethod::Vni,
        // Simple Telex 2 (w but no brackets) is kept for those who chose it in an older version.
        (_, true) if previous == InputMethod::SimpleTelex2 => InputMethod::SimpleTelex2,
        (_, true) => InputMethod::Telex,
        (_, false) => InputMethod::SimpleTelex,
    }
}

/// Programs with a visible top-level window, by file name, after a placeholder item.
fn running_apps() -> Vec<SharedString> {
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
    let mut names: Vec<String> = windows
        .into_iter()
        .filter(|&w| {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(w, &mut pid) };
            pid != own
        })
        .filter_map(window_exe_name)
        .collect();
    names.sort();
    names.dedup();
    std::iter::once("Chọn ứng dụng…".to_string()).chain(names).map(SharedString::from).collect()
}

/// Centres the window on the screen under the mouse, which is where the tray icon was clicked.
/// Right after `show` the window still reports its minimum size, so the size is set here
/// (the preferred size in settings.slint) before it is used.
fn center(ui: &SettingsWindow) {
    ui.window().set_size(slint::LogicalSize::new(560.0, 480.0));
    let size = slint::LogicalSize::new(560.0, 480.0).to_physical(ui.window().scale_factor());
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

fn set_apps(model: &VecModel<SharedString>, apps: &[String]) {
    model.set_vec(apps.iter().map(SharedString::from).collect::<Vec<_>>());
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
    ui.set_w_shortcuts(s.method != InputMethod::SimpleTelex);
    ui.set_modern_style(s.modern_style);
    ui.set_spell_check(s.spell_check);
    ui.set_auto_restore(s.auto_restore);

    let english = Rc::new(VecModel::<SharedString>::default());
    set_apps(&english, &s.english_apps);
    ui.set_english_apps(ModelRc::from(english.clone()));
    let running = Rc::new(VecModel::from(running_apps()));
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
                s.method = method_from(&ui, s.method);
                s.modern_style = ui.get_modern_style();
                s.spell_check = ui.get_spell_check();
                s.auto_restore = ui.get_auto_restore();
            });
        }
    });

    ui.on_add_app({
        let (english, path) = (english.clone(), path.clone());
        move |name| {
            let mut apps = Vec::new();
            if edit(&path, |s| {
                s.add_english_app(&name);
                apps = s.english_apps.clone();
            }) {
                set_apps(&english, &apps);
            }
        }
    });
    ui.on_remove_app({
        let (english, path) = (english.clone(), path.clone());
        move |i| {
            let Some(name) = english.row_data(i as usize) else { return };
            let mut apps = Vec::new();
            if edit(&path, |s| {
                s.english_apps.retain(|a| a.as_str() != name.as_str());
                apps = s.english_apps.clone();
            }) {
                set_apps(&english, &apps);
            }
        }
    });
    ui.on_refresh_running({
        let (ui, running) = (ui.as_weak(), running.clone());
        move || {
            running.set_vec(running_apps());
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
    fn running_apps_lists_names_once() {
        let apps = running_apps();
        let apps = apps[1..].to_vec();
        let mut sorted = apps.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(apps, sorted);
        assert!(apps.iter().all(|a| a.as_str() == a.to_lowercase()));
    }
}
