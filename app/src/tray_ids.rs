//! Window class, messages and menu commands of the tray window. Shared with the end-to-end test.
#![allow(dead_code)]

pub const WINDOW_CLASS: &str = "GovnTrayWindow";

const WM_APP: u32 = 0x8000;
/// Sent by the notification area; `lparam` holds the mouse message.
pub const WM_TRAY: u32 = WM_APP + 1;
/// Posted by the keyboard hook after Ctrl+Shift, to refresh the icon outside the hook.
pub const WM_STATE_CHANGED: u32 = WM_APP + 2;

pub const CMD_TOGGLE: usize = 1001;
pub const CMD_TELEX: usize = 1010;
pub const CMD_VNI: usize = 1011;
pub const CMD_SIMPLE: usize = 1012;
pub const CMD_SIMPLE2: usize = 1013;
pub const CMD_MODERN: usize = 1020;
pub const CMD_SPELL: usize = 1021;
pub const CMD_RESTORE: usize = 1022;
pub const CMD_FIX_AUTO: usize = 1030;
pub const CMD_FIX_ALWAYS: usize = 1031;
pub const CMD_FIX_NEVER: usize = 1032;
pub const CMD_AUTOSTART: usize = 1040;
pub const CMD_QUIT: usize = 1099;
