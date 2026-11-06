//! Real Vietnamese words, expected output written by hand from the language rather than from the C++ engine.
use govn_engine::{Engine, InputMethod, Options};

/// Types `keys` into a model text field and returns what the field ends up holding.
fn type_all(e: &mut Engine, keys: &str) -> String {
    let mut field: Vec<u16> = Vec::new();
    for ch in keys.chars() {
        let o = if ch == '\u{8}' { e.process_backspace() } else { e.process(ch as u32) };
        if o.handled {
            field.truncate(field.len().saturating_sub(o.backs));
            field.extend(&o.text);
        } else if ch == '\u{8}' {
            field.pop();
        } else {
            field.push(ch as u16);
        }
    }
    String::from_utf16(&field).unwrap()
}

fn telex(keys: &str) -> String {
    type_all(&mut Engine::new(), keys)
}

fn vni(keys: &str) -> String {
    let mut e = Engine::new();
    e.set_input_method(InputMethod::Vni);
    type_all(&mut e, keys)
}

#[test]
fn telex_words() {
    for (keys, want) in [
        ("tieengs vieetj", "tiếng việt"),
        ("cooj", "cộ"),
        ("nguoiwf", "người"),
        ("dduwowngf", "đường"),
        ("Vieetj Nam", "Việt Nam"),
        ("ddatj", "đạt"),
        ("ddaatj", "đật"),
        ("hoaf", "hòa"),
        ("quyeenf", "quyền"),
        ("giaor", "giảo"),
        ("nghieeng", "nghiêng"),
        ("thuowr", "thuở"),
        ("khoer", "khỏe"),
        ("uw", "ư"),
        ("w", "ư"),
        ("as", "á"),
        ("aa", "â"),
        ("aw", "ă"),
    ] {
        assert_eq!(telex(keys), want, "keys: {keys}");
    }
}

#[test]
fn telex_tone_can_be_typed_before_the_final_consonant() {
    assert_eq!(telex("tiesng"), "tiếng".replace('ế', "é"));
    assert_eq!(telex("vieejt"), "việt");
    assert_eq!(telex("toans"), "toán");
}

#[test]
fn telex_undo_by_repeating_the_key() {
    assert_eq!(telex("ass"), "as");
    assert_eq!(telex("aaa"), "aa");
    assert_eq!(telex("ddd"), "dd");
}

#[test]
fn vni_words() {
    for (keys, want) in [
        ("tie61ng vie65t", "tiếng việt"),
        ("d9uo72ng", "đường"),
        ("nguo72i", "người"),
        ("a1", "á"),
        ("a6", "â"),
        ("a8", "ă"),
    ] {
        assert_eq!(vni(keys), want, "keys: {keys}");
    }
}

#[test]
fn modern_tone_style() {
    let mut e = Engine::new();
    e.options = Options { modern_style: true, ..Options::default() };
    assert_eq!(type_all(&mut e, "hoaf"), "hoà");
    assert_eq!(telex("hoaf"), "hòa");
}

#[test]
fn non_vietnamese_word_is_left_alone() {
    assert_eq!(telex("https"), "https");
    assert_eq!(telex("file"), "file");
    // Telex treats a leading w as the letter ư, same as UniKey
    assert_eq!(telex("window"), "ưindow");
}

#[test]
fn auto_restore_gives_back_raw_keys() {
    let mut e = Engine::new();
    e.options = Options { auto_non_vn_restore: true, ..Options::default() };
    // "ss" undoes the tone, so nothing is left to restore
    assert_eq!(type_all(&mut e, "tesst "), "test ");
    e.reset();
    assert_eq!(type_all(&mut e, "cofnputer "), "cofnputer ");
}

#[test]
fn backspace_moves_the_tone_back() {
    // "hoà" style: removing the last vowel must keep the tone on the remaining one
    assert_eq!(telex("toaf\u{8}"), "tò");
    assert_eq!(telex("tieengs\u{8}\u{8}"), "tiế");
}

#[test]
fn reset_starts_a_new_word() {
    let mut e = Engine::new();
    assert_eq!(type_all(&mut e, "a"), "a");
    e.reset();
    // after a reset the next "s" has no vowel to attach to
    assert_eq!(type_all(&mut e, "s"), "s");
}

#[test]
fn off_mode_passes_everything_through() {
    let mut e = Engine::new();
    e.viet_key = false;
    assert_eq!(type_all(&mut e, "tieengs vieetj"), "tieengs vieetj");
}

#[test]
fn no_key_sequence_panics() {
    // Small deterministic generator: every byte value, plus backspaces and resets, long enough to
    // wrap the 128-entry buffers. Run in all four methods.
    for im in [InputMethod::Telex, InputMethod::Vni, InputMethod::SimpleTelex, InputMethod::SimpleTelex2] {
        let mut e = Engine::new();
        e.set_input_method(im);
        let mut s: u32 = 0x1234_5678;
        for i in 0..400_000u32 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            match s % 53 {
                0 => {
                    e.process_backspace();
                }
                1 => e.reset(),
                2 => e.caps_lock = !e.caps_lock,
                3 => {
                    e.process(s >> 8 & 0xFFFF);
                }
                4..=12 => {
                    e.process(u32::from(b"aeiouyAEO"[(i % 9) as usize]));
                }
                _ => {
                    e.process(s >> 8 & 0xFF);
                }
            }
        }
    }
}
