//! User settings, stored as a small `key=value` file.
use govn_engine::{InputMethod, Options};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SuggestionFix {
    /// Only in programs known to show inline suggestions.
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    pub viet: bool,
    pub method: InputMethod,
    pub modern_style: bool,
    pub spell_check: bool,
    pub auto_restore: bool,
    pub suggestion_fix: SuggestionFix,
}

impl Default for Settings {
    fn default() -> Self {
        let o = Options::default();
        Settings {
            viet: true,
            method: InputMethod::Telex,
            modern_style: o.modern_style,
            spell_check: o.spell_check,
            auto_restore: o.auto_non_vn_restore,
            suggestion_fix: SuggestionFix::Auto,
        }
    }
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
                _ => {}
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        let method = match self.method {
            InputMethod::Telex => "telex",
            InputMethod::Vni => "vni",
            InputMethod::SimpleTelex => "simple",
            InputMethod::SimpleTelex2 => "simple2",
        };
        let fix = match self.suggestion_fix {
            SuggestionFix::Auto => "auto",
            SuggestionFix::Always => "always",
            SuggestionFix::Never => "never",
        };
        format!(
            "viet={}\nmethod={method}\nmodern={}\nspell={}\nrestore={}\nsuggestion_fix={fix}\n",
            u8::from(self.viet),
            u8::from(self.modern_style),
            u8::from(self.spell_check),
            u8::from(self.auto_restore),
        )
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    /// Writes through a temporary file, so a crash cannot leave a half-written settings file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(&tmp, path)
    }
}

pub fn parse_method(v: &str) -> Option<InputMethod> {
    match v {
        "telex" => Some(InputMethod::Telex),
        "vni" => Some(InputMethod::Vni),
        "simple" => Some(InputMethod::SimpleTelex),
        "simple2" => Some(InputMethod::SimpleTelex2),
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
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn bad_input_falls_back_to_defaults() {
        let d = Settings::default();
        assert_eq!(Settings::parse(""), d);
        assert_eq!(Settings::parse("rác\n=\nmethod=klingon\nviet=maybe\nunknown=1\n"), d);
        // one bad line does not discard the good ones
        assert_eq!(Settings::parse("method=klingon\nmodern=1\n").modern_style, true);
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("govn-config-test-{}", std::process::id()));
        let path = dir.join("nested").join("settings.ini");
        let s = Settings { method: InputMethod::SimpleTelex2, ..Settings::default() };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::remove_dir_all(&dir).unwrap();
        // missing file: defaults
        assert_eq!(Settings::load(&path), Settings::default());
    }
}
