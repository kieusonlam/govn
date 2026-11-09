//! User settings, stored as a small `key=value` file, and the per-program Vietnamese/English memory,
//! stored next to it in `apps.ini`.
//!
//! The settings window and the keyboard are separate processes that both write `settings.ini`, so
//! each change is read-modify-write. The memory has its own file because only the keyboard writes it.
use govn_engine::{InputMethod, Options};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SuggestionFix {
    /// Only in programs known to show inline suggestions.
    Auto,
    Always,
    Never,
}

/// What switches between Vietnamese and English. Two modifiers act when they are released together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hotkey {
    CtrlShift,
    CtrlAlt,
    AltZ,
    None,
}

impl Hotkey {
    pub const ALL: [Hotkey; 4] = [Hotkey::CtrlShift, Hotkey::CtrlAlt, Hotkey::AltZ, Hotkey::None];

    pub fn label(self) -> &'static str {
        match self {
            Hotkey::CtrlShift => "Ctrl + Shift",
            Hotkey::CtrlAlt => "Ctrl + Alt",
            Hotkey::AltZ => "Alt + Z",
            Hotkey::None => "Không dùng",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Hotkey::CtrlShift => "ctrl_shift",
            Hotkey::CtrlAlt => "ctrl_alt",
            Hotkey::AltZ => "alt_z",
            Hotkey::None => "none",
        }
    }
}

/// Programs are named by their file name in lower case, such as `code.exe`.
pub const MAX_APPS: usize = 300;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Settings {
    pub viet: bool,
    pub method: InputMethod,
    pub modern_style: bool,
    pub spell_check: bool,
    pub auto_restore: bool,
    pub suggestion_fix: SuggestionFix,
    pub hotkey: Hotkey,
    /// Always English in these programs.
    pub english_apps: Vec<String>,
    /// Each program keeps the Vietnamese/English state it was last used with.
    pub remember_apps: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let o = Options::default();
        Settings {
            viet: true,
            method: TELEX,
            modern_style: o.modern_style,
            spell_check: o.spell_check,
            auto_restore: o.auto_non_vn_restore,
            suggestion_fix: SuggestionFix::Auto,
            hotkey: Hotkey::CtrlShift,
            english_apps: Vec::new(),
            remember_apps: false,
        }
    }
}

/// `Code.EXE ` becomes `code.exe`; anything that cannot be a file name is rejected.
pub fn normalize_app(name: &str) -> Option<String> {
    let n = name.trim().to_lowercase();
    let bad = n.is_empty() || n.len() > 260 || n.chars().any(|c| c.is_control() || "\\/:*?\"<>|,=".contains(c));
    (!bad).then_some(n)
}

impl Settings {
    /// Unknown keys and bad values are ignored, so an old or hand-edited file never stops the keyboard.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            let flag = match value {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            };
            match key {
                "viet" => s.viet = flag.unwrap_or(s.viet),
                "modern" => s.modern_style = flag.unwrap_or(s.modern_style),
                "spell" => s.spell_check = flag.unwrap_or(s.spell_check),
                "restore" => s.auto_restore = flag.unwrap_or(s.auto_restore),
                "method" => s.method = parse_method(value).unwrap_or(s.method),
                "suggestion_fix" => s.suggestion_fix = parse_fix(value).unwrap_or(s.suggestion_fix),
                "hotkey" => s.hotkey = Hotkey::ALL.into_iter().find(|h| h.key() == value).unwrap_or(s.hotkey),
                "remember_apps" => s.remember_apps = flag.unwrap_or(s.remember_apps),
                "english_apps" => {
                    s.english_apps.clear();
                    for app in value.split(',').filter_map(normalize_app) {
                        s.add_english_app(&app);
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn add_english_app(&mut self, name: &str) -> bool {
        match normalize_app(name) {
            Some(n) if !self.english_apps.contains(&n) && self.english_apps.len() < MAX_APPS => {
                self.english_apps.push(n);
                true
            }
            _ => false,
        }
    }

    pub fn serialize(&self) -> String {
        let method = if self.method == InputMethod::Vni { "vni" } else { "telex" };
        let fix = match self.suggestion_fix {
            SuggestionFix::Auto => "auto",
            SuggestionFix::Always => "always",
            SuggestionFix::Never => "never",
        };
        format!(
            "viet={}\nmethod={method}\nmodern={}\nspell={}\nrestore={}\nsuggestion_fix={fix}\nhotkey={}\n\
             remember_apps={}\nenglish_apps={}\n",
            u8::from(self.viet),
            u8::from(self.modern_style),
            u8::from(self.spell_check),
            u8::from(self.auto_restore),
            self.hotkey.key(),
            u8::from(self.remember_apps),
            self.english_apps.join(","),
        )
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, &self.serialize())
    }
}

/// Writes through a temporary file, so a crash cannot leave a half-written file.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// The Vietnamese (`true`) or English state each program was last used with.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct AppMemory(BTreeMap<String, bool>);

impl AppMemory {
    pub fn path_for(settings: &Path) -> PathBuf {
        settings.with_file_name("apps.ini")
    }

    pub fn parse(text: &str) -> AppMemory {
        let mut m = AppMemory::default();
        for line in text.lines() {
            let Some((app, value)) = line.rsplit_once('=') else { continue };
            let on = match value.trim() {
                "1" => true,
                "0" => false,
                _ => continue,
            };
            if let Some(app) = normalize_app(app) {
                m.set(&app, on);
            }
        }
        m
    }

    pub fn get(&self, app: &str) -> Option<bool> {
        self.0.get(app).copied()
    }

    /// Returns whether anything changed. Past the limit, new programs are not remembered.
    pub fn set(&mut self, app: &str, on: bool) -> bool {
        if self.0.len() >= MAX_APPS && !self.0.contains_key(app) {
            return false;
        }
        self.0.insert(app.to_string(), on) != Some(on)
    }

    pub fn serialize(&self) -> String {
        self.0.iter().map(|(app, on)| format!("{app}={}\n", u8::from(*on))).collect()
    }

    pub fn load(path: &Path) -> AppMemory {
        std::fs::read_to_string(path).map(|t| AppMemory::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, &self.serialize())
    }
}

/// govn's Telex: every mark is typed in full (`uw` for ư, `ow` for ơ), as on phone keyboards.
/// `w` alone and `[ ]` stay as typed; the engine's shortcut variants are not offered.
pub const TELEX: InputMethod = InputMethod::SimpleTelex;

/// Older versions wrote "simple" and "simple2" too; all Telex variants now mean [`TELEX`].
pub fn parse_method(v: &str) -> Option<InputMethod> {
    match v {
        "telex" | "simple" | "simple2" => Some(TELEX),
        "vni" => Some(InputMethod::Vni),
        _ => None,
    }
}

pub fn parse_fix(v: &str) -> Option<SuggestionFix> {
    match v {
        "auto" => Some(SuggestionFix::Auto),
        "always" => Some(SuggestionFix::Always),
        "never" => Some(SuggestionFix::Never),
        _ => None,
    }
}

pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("govn").join("settings.ini"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            viet: false,
            method: InputMethod::Vni,
            modern_style: true,
            spell_check: false,
            auto_restore: true,
            suggestion_fix: SuggestionFix::Never,
            hotkey: Hotkey::AltZ,
            english_apps: vec!["code.exe".into(), "windowsterminal.exe".into()],
            remember_apps: true,
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
        for h in Hotkey::ALL {
            let s = Settings { hotkey: h, ..Settings::default() };
            assert_eq!(Settings::parse(&s.serialize()).hotkey, h);
        }
    }

    #[test]
    fn every_telex_variant_reads_as_telex() {
        for old in ["telex", "simple", "simple2"] {
            let s = Settings::parse(&format!("method={old}\n"));
            assert_eq!(s.method, TELEX, "{old}");
            assert!(s.serialize().contains("method=telex\n"));
        }
        assert_eq!(Settings::default().method, TELEX);
        assert_eq!(Settings::parse("method=vni\n").method, InputMethod::Vni);
    }

    #[test]
    fn bad_input_falls_back_to_defaults() {
        let d = Settings::default();
        assert_eq!(Settings::parse(""), d);
        assert_eq!(Settings::parse("rác\n=\nmethod=klingon\nviet=maybe\nunknown=1\nhotkey=f13\n"), d);
        // one bad line does not discard the good ones
        assert_eq!(Settings::parse("method=klingon\nmodern=1\n").modern_style, true);
    }

    #[test]
    fn app_names_are_cleaned() {
        let s = Settings::parse("english_apps= Code.EXE ,,a\\b.exe,code.exe,game.exe\n");
        assert_eq!(s.english_apps, ["code.exe", "game.exe"]);
        let mut s = Settings::default();
        assert!(s.add_english_app("Zalo.exe"));
        assert!(!s.add_english_app("zalo.exe"), "no duplicates");
        assert!(!s.add_english_app("  "));
        for i in 0..MAX_APPS + 5 {
            s.add_english_app(&format!("app{i}.exe"));
        }
        assert_eq!(s.english_apps.len(), MAX_APPS);
    }

    #[test]
    fn app_memory() {
        let mut m = AppMemory::parse("code.exe=0\nzalo.exe=1\nbad\nx.exe=maybe\n");
        assert_eq!(m.get("code.exe"), Some(false));
        assert_eq!(m.get("zalo.exe"), Some(true));
        assert_eq!(m.get("x.exe"), None);
        assert!(!m.set("code.exe", false), "unchanged");
        assert!(m.set("code.exe", true));
        assert_eq!(AppMemory::parse(&m.serialize()), m);
        for i in 0..MAX_APPS + 5 {
            m.set(&format!("app{i}.exe"), true);
        }
        assert_eq!(m.0.len(), MAX_APPS);
        assert!(m.set("code.exe", false), "known programs still update past the limit");
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("govn-config-test-{}", std::process::id()));
        let path = dir.join("nested").join("settings.ini");
        let s = Settings { method: InputMethod::Vni, english_apps: vec!["a.exe".into()], ..Settings::default() };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        let mem_path = AppMemory::path_for(&path);
        let mut m = AppMemory::default();
        m.set("a.exe", false);
        m.save(&mem_path).unwrap();
        assert_eq!(AppMemory::load(&mem_path), m);
        std::fs::remove_dir_all(&dir).unwrap();
        // missing file: defaults
        assert_eq!(Settings::load(&path), Settings::default());
        assert_eq!(AppMemory::load(&mem_path), AppMemory::default());
    }
}
