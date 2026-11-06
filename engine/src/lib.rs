//! Vietnamese input engine: a Rust port of UniKey's `ukengine` (Copyright (C) 2000-2005 Pham Kim Long).
//!
//! Function names follow `ukengine.cpp` so the two can be read side by side. Behaviour is kept
//! identical on purpose, including quirks, and is checked against the C++ original (see `tools/`).
//!
//! Differences from the C++ code:
//! - Output is always Unicode (UTF-16 units); legacy charsets are not handled here.
//! - Macros, VIQR and MS-Vi input are not ported yet.
//! - Out-of-range reads that are undefined behaviour in C++ resolve to a zeroed entry, so this
//!   code cannot panic or corrupt memory on any key sequence.
#![allow(non_upper_case_globals)]

mod tables;
use tables::*;

const MAX_UK_ENGINE: i32 = 128;
const VN_STD_CHAR_OFFSET: u32 = 0x10000;
const VNL_LAST_CHAR: i32 = vnl_lastChar;

// UkKeyEvName
const vneRoofAll: i32 = 0;
const vneRoof_a: i32 = 1;
const vneRoof_e: i32 = 2;
const vneRoof_o: i32 = 3;
const vneHookAll: i32 = 4;
const vneHook_uo: i32 = 5;
const vneHook_u: i32 = 6;
const vneHook_o: i32 = 7;
const vneBowl: i32 = 8;
const vneDd: i32 = 9;
const vneTone0: i32 = 10;
const vneTone1: i32 = 11;
const vneTone2: i32 = 12;
const vneTone3: i32 = 13;
const vneTone4: i32 = 14;
const vneTone5: i32 = 15;
const vne_telex_w: i32 = 16;
const vneMapChar: i32 = 17;
const vneNormal: i32 = 19;
const vneCount: i32 = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum CharType {
    Vn,
    WordBreak,
    #[default]
    NonVn,
    Reset,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Form {
    #[default]
    NonVn,
    Empty,
    C,
    V,
    Cv,
    Vc,
    Cvc,
}

/// Typing method.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputMethod {
    Telex,
    Vni,
    SimpleTelex,
    SimpleTelex2,
}

/// Engine options; defaults match `CreateDefaultUnikeyOptions`.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Allow tone and marks to be typed anywhere after the vowel.
    pub free_marking: bool,
    /// Place tones the modern way (`oà`, `oé`, `uý`).
    pub modern_style: bool,
    pub spell_check: bool,
    /// Turn a word back into the raw keys when it ends up not being Vietnamese.
    pub auto_non_vn_restore: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { free_marking: true, modern_style: false, spell_check: true, auto_non_vn_restore: false }
    }
}

/// What the caller has to do for one key press.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// `false`: let the original key through and do nothing else.
    pub handled: bool,
    /// Number of backspaces to send first.
    pub backs: usize,
    /// UTF-16 text to send after the backspaces.
    pub text: Vec<u16>,
    /// Text holds raw key strokes (auto-restore) rather than converted characters.
    pub key_output: bool,
}

#[derive(Clone, Copy, Default)]
struct KeyEvent {
    ev_type: i32,
    ch_type: CharType,
    vn_sym: i32,
    key_code: u32,
    tone: i32,
}

#[derive(Clone, Copy, Default)]
struct KeyBufEntry {
    ev: KeyEvent,
    converted: bool,
}

#[derive(Clone, Copy, Default)]
struct WordInfo {
    form: Form,
    c1_offset: i32,
    v_offset: i32,
    c2_offset: i32,
    /// `vseq` / `cseq`: a single union in the C++ code, and the code relies on that.
    seq: i32,
    caps: bool,
    tone: i32,
    vn_sym: i32,
    key_code: u32,
}

static NIL_VSEQ: VowelSeqInfo = VowelSeqInfo {
    len: 1,
    complete: false,
    con_suffix: false,
    v: [vnl_nonVnChar; 3],
    sub: [vs_nil; 3],
    roof_pos: -1,
    with_roof: vs_nil,
    hook_pos: -1,
    with_hook: vs_nil,
};
static NIL_CSEQ: ConSeqInfo = ConSeqInfo { len: 1, c: [vnl_nonVnChar; 3], suffix: false };

fn vi(vs: i32) -> &'static VowelSeqInfo {
    usize::try_from(vs).ok().and_then(|i| VSEQ_LIST.get(i)).unwrap_or(&NIL_VSEQ)
}

fn ci(cs: i32) -> &'static ConSeqInfo {
    usize::try_from(cs).ok().and_then(|i| CSEQ_LIST.get(i)).unwrap_or(&NIL_CSEQ)
}

fn at3(a: &[i32; 3], i: i32) -> i32 {
    usize::try_from(i).ok().and_then(|i| a.get(i).copied()).unwrap_or(vnl_nonVnChar)
}

fn no_tone(sym: i32) -> i32 {
    usize::try_from(sym).ok().and_then(|i| STD_VN_NO_TONE.get(i).copied()).unwrap_or(sym)
}

fn lookup_vseq(v1: i32, v2: i32, v3: i32) -> i32 {
    VSEQ_LIST.iter().position(|e| e.v == [v1, v2, v3]).map_or(vs_nil, |i| i as i32)
}

fn lookup_vseq1(v1: i32) -> i32 {
    lookup_vseq(v1, vnl_nonVnChar, vnl_nonVnChar)
}

fn lookup_vseq2(v1: i32, v2: i32) -> i32 {
    lookup_vseq(v1, v2, vnl_nonVnChar)
}

fn lookup_cseq(c1: i32, c2: i32, c3: i32) -> i32 {
    CSEQ_LIST.iter().position(|e| e.c == [c1, c2, c3]).map_or(cs_nil, |i| i as i32)
}

fn is_valid_cv(c: i32, v: i32) -> bool {
    if c == cs_nil || v == vs_nil {
        return true;
    }
    let info = vi(v);
    if (c == cs_gi && info.v[0] == vnl_i) || (c == cs_qu && info.v[0] == vnl_u) {
        return false; // gi doesn't go with i, qu doesn't go with u
    }
    if c == cs_k {
        const K_VSEQ: [i32; 12] =
            [vs_e, vs_i, vs_y, vs_er, vs_eo, vs_eu, vs_eru, vs_ia, vs_ie, vs_ier, vs_ieu, vs_ieru];
        return K_VSEQ.contains(&v);
    }
    true
}

fn is_valid_vc(v: i32, c: i32) -> bool {
    if v == vs_nil || c == cs_nil {
        return true;
    }
    if !vi(v).con_suffix || !ci(c).suffix {
        return false;
    }
    VC_PAIR_LIST.contains(&(v, c))
}

fn is_valid_cvc(c1: i32, v: i32, c2: i32) -> bool {
    if v == vs_nil {
        return c1 == cs_nil || c2 != cs_nil;
    }
    if c1 == cs_nil {
        return is_valid_vc(v, c2);
    }
    if c2 == cs_nil {
        return is_valid_cv(c1, v);
    }
    let ok_cv = is_valid_cv(c1, v);
    let ok_vc = is_valid_vc(v, c2);
    if ok_cv && ok_vc {
        return true;
    }
    if !ok_vc {
        // quyn, quynh
        if c1 == cs_qu && v == vs_y && (c2 == cs_n || c2 == cs_nh) {
            return true;
        }
        // gieng, gie^ng
        if c1 == cs_gi && (v == vs_e || v == vs_er) && (c2 == cs_n || c2 == cs_ng) {
            return true;
        }
    }
    false
}

fn change_case(x: i32) -> i32 {
    if x == vnl_nonVnChar {
        x
    } else if x & 1 == 0 {
        x + 1
    } else {
        x - 1
    }
}

fn vn_to_lower(x: i32) -> i32 {
    if x != vnl_nonVnChar && x & 1 == 0 {
        x + 1
    } else {
        x
    }
}

fn is_vn_vowel(sym: i32) -> bool {
    (0..VNL_LAST_CHAR).contains(&sym) && !AZ_NON_VOWEL.contains(&vn_to_lower(sym))
}

/// Lower-case lexi names of letters that are not vowels.
const AZ_NON_VOWEL: [i32; 21] = [
    vnl_b, vnl_c, vnl_d, vnl_dd, vnl_f, vnl_g, vnl_h, vnl_j, vnl_k, vnl_l, vnl_m, vnl_n, vnl_p, vnl_q, vnl_r, vnl_s,
    vnl_t, vnl_v, vnl_w, vnl_x, vnl_z,
];

const AZ_LEXI_UPPER: [i32; 26] = [
    vnl_A, vnl_B, vnl_C, vnl_D, vnl_E, vnl_F, vnl_G, vnl_H, vnl_I, vnl_J, vnl_K, vnl_L, vnl_M, vnl_N, vnl_O, vnl_P,
    vnl_Q, vnl_R, vnl_S, vnl_T, vnl_U, vnl_V, vnl_W, vnl_X, vnl_Y, vnl_Z,
];
const AZ_LEXI_LOWER: [i32; 26] = [
    vnl_a, vnl_b, vnl_c, vnl_d, vnl_e, vnl_f, vnl_g, vnl_h, vnl_i, vnl_j, vnl_k, vnl_l, vnl_m, vnl_n, vnl_o, vnl_p,
    vnl_q, vnl_r, vnl_s, vnl_t, vnl_u, vnl_v, vnl_w, vnl_x, vnl_y, vnl_z,
];

const WORD_BREAK_SYMS: &[u8] = b",;:.\"'!? <>=+-*/\\_@#$%&(){}[]|";

/// Western characters outside A-Z that are also Vietnamese characters. `0xC2` appears twice in the
/// original (the later entry wins), which is kept.
const ASC_VN_LEXI_LIST: [(u8, i32); 32] = [
    (0xC0, vnl_A2), (0xC1, vnl_A1), (0xC2, vnl_Ar), (0xC2, vnl_A4), (0xC8, vnl_E2), (0xC9, vnl_E1), (0xCA, vnl_Er),
    (0xCC, vnl_I2), (0xCD, vnl_I1), (0xD2, vnl_O2), (0xD3, vnl_O1), (0xD4, vnl_Or), (0xD5, vnl_O4), (0xD9, vnl_U2),
    (0xDA, vnl_U1), (0xDD, vnl_Y1), (0xE0, vnl_a2), (0xE1, vnl_a1), (0xE2, vnl_ar), (0xE3, vnl_a4), (0xE8, vnl_e2),
    (0xE9, vnl_e1), (0xEA, vnl_er), (0xEC, vnl_i2), (0xED, vnl_i1), (0xF2, vnl_o2), (0xF3, vnl_o1), (0xF4, vnl_or),
    (0xF5, vnl_o4), (0xF9, vnl_u2), (0xFA, vnl_u1), (0xFD, vnl_y1),
];

const SPECIAL_WESTERN_CHARS: [u8; 27] = [
    0x80, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8E, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9E, 0x9F,
];

const TELEX: &[(u8, i32)] = &[
    (b'Z', vneTone0), (b'S', vneTone1), (b'F', vneTone2), (b'R', vneTone3), (b'X', vneTone4), (b'J', vneTone5),
    (b'W', vne_telex_w), (b'A', vneRoof_a), (b'E', vneRoof_e), (b'O', vneRoof_o), (b'D', vneDd),
    (b'[', vneCount + vnl_oh), (b']', vneCount + vnl_uh), (b'{', vneCount + vnl_Oh), (b'}', vneCount + vnl_Uh),
];
const SIMPLE_TELEX: &[(u8, i32)] = &[
    (b'Z', vneTone0), (b'S', vneTone1), (b'F', vneTone2), (b'R', vneTone3), (b'X', vneTone4), (b'J', vneTone5),
    (b'W', vneHookAll), (b'A', vneRoof_a), (b'E', vneRoof_e), (b'O', vneRoof_o), (b'D', vneDd),
];
const SIMPLE_TELEX2: &[(u8, i32)] = &[
    (b'Z', vneTone0), (b'S', vneTone1), (b'F', vneTone2), (b'R', vneTone3), (b'X', vneTone4), (b'J', vneTone5),
    (b'W', vne_telex_w), (b'A', vneRoof_a), (b'E', vneRoof_e), (b'O', vneRoof_o), (b'D', vneDd),
];
const VNI: &[(u8, i32)] = &[
    (b'0', vneTone0), (b'1', vneTone1), (b'2', vneTone2), (b'3', vneTone3), (b'4', vneTone4), (b'5', vneTone5),
    (b'6', vneRoofAll), (b'7', vneHook_uo), (b'8', vneBowl), (b'9', vneDd),
];

fn is_alpha(k: u32) -> bool {
    k < 128 && (k as u8).is_ascii_alphabetic()
}

fn is_upper(k: u32) -> bool {
    k < 128 && (k as u8).is_ascii_uppercase()
}

/// The input engine. One instance per typing context.
pub struct Engine {
    /// Vietnamese mode on or off.
    pub viet_key: bool,
    pub options: Options,
    /// Caps Lock state, read for Telex `w` and map characters. Set before calling [`Engine::process`].
    pub caps_lock: bool,

    method: InputMethod,
    key_map: [i32; 256],
    ukc_map: [CharType; 256],
    iso_vn_lexi: [i32; 256],
    iso_std: [u32; 256],

    change_pos: i32,
    backs: i32,
    current: i32,
    single_mode: bool,
    key_current: i32,
    /// A function-level `static` in the C++ code, so it also survives `reset()`.
    used_as_map_char: bool,
    key_strokes: Box<[KeyBufEntry; MAX_UK_ENGINE as usize]>,
    buffer: Box<[WordInfo; MAX_UK_ENGINE as usize]>,
    nil_entry: WordInfo,

    out: Vec<u16>,
    output_written: bool,
    reverted: bool,
    key_restored: bool,
    key_restoring: bool,
    key_output: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        let mut ukc_map = [CharType::NonVn; 256];
        for c in ukc_map.iter_mut().take(33) {
            *c = CharType::Reset;
        }
        for c in b'a'..=b'z' {
            ukc_map[c as usize] = CharType::Vn;
        }
        for c in b'A'..=b'Z' {
            ukc_map[c as usize] = CharType::Vn;
        }
        for (asc, _) in ASC_VN_LEXI_LIST {
            ukc_map[asc as usize] = CharType::Vn;
        }
        for c in b"jJfFwW" {
            ukc_map[*c as usize] = CharType::NonVn;
        }
        for c in WORD_BREAK_SYMS {
            ukc_map[*c as usize] = CharType::WordBreak;
        }

        let mut iso_vn_lexi = [vnl_nonVnChar; 256];
        for (asc, lexi) in ASC_VN_LEXI_LIST {
            iso_vn_lexi[asc as usize] = lexi;
        }
        for c in 0..26 {
            iso_vn_lexi[b'a' as usize + c] = AZ_LEXI_LOWER[c];
            iso_vn_lexi[b'A' as usize + c] = AZ_LEXI_UPPER[c];
        }

        let mut iso_std = [0u32; 256];
        for (i, s) in iso_std.iter_mut().enumerate() {
            *s = i as u32;
        }
        for (i, c) in SPECIAL_WESTERN_CHARS.iter().enumerate() {
            iso_std[*c as usize] = (VNL_LAST_CHAR as u32 + i as u32) + VN_STD_CHAR_OFFSET;
        }
        for i in 0..256 {
            if iso_vn_lexi[i] != vnl_nonVnChar {
                iso_std[i] = iso_vn_lexi[i] as u32 + VN_STD_CHAR_OFFSET;
            }
        }

        let mut e = Engine {
            viet_key: true,
            options: Options::default(),
            caps_lock: false,
            method: InputMethod::Telex,
            key_map: [vneNormal; 256],
            ukc_map,
            iso_vn_lexi,
            iso_std,
            change_pos: 0,
            backs: 0,
            current: -1,
            single_mode: false,
            key_current: -1,
            used_as_map_char: false,
            key_strokes: Box::new([KeyBufEntry::default(); MAX_UK_ENGINE as usize]),
            buffer: Box::new([WordInfo::default(); MAX_UK_ENGINE as usize]),
            nil_entry: WordInfo::default(),
            out: Vec::new(),
            output_written: false,
            reverted: false,
            key_restored: false,
            key_restoring: false,
            key_output: false,
        };
        e.set_input_method(InputMethod::Telex);
        e
    }

    pub fn input_method(&self) -> InputMethod {
        self.method
    }

    /// Switch typing method. Also resets the current word.
    pub fn set_input_method(&mut self, im: InputMethod) {
        self.method = im;
        let map = match im {
            InputMethod::Telex => TELEX,
            InputMethod::Vni => VNI,
            InputMethod::SimpleTelex => SIMPLE_TELEX,
            InputMethod::SimpleTelex2 => SIMPLE_TELEX2,
        };
        self.key_map = [vneNormal; 256];
        for &(key, action) in map {
            self.key_map[key as usize] = action;
            if action < vneCount {
                if key.is_ascii_lowercase() {
                    self.key_map[key.to_ascii_uppercase() as usize] = action;
                } else if key.is_ascii_uppercase() {
                    self.key_map[key.to_ascii_lowercase() as usize] = action;
                }
            }
        }
        self.reset();
    }

    /// Forget the current word. Call on focus change, mouse click, arrow keys, and so on.
    pub fn reset(&mut self) {
        self.current = -1;
        self.key_current = -1;
        self.single_mode = false;
    }

    pub fn at_word_beginning(&self) -> bool {
        self.current < 0 || self.b(self.current).form == Form::Empty
    }

    // ---- buffer access: never panics, unlike raw indexing

    fn b(&self, i: i32) -> &WordInfo {
        usize::try_from(i).ok().and_then(|i| self.buffer.get(i)).unwrap_or(&self.nil_entry)
    }

    fn bm(&mut self, i: i32) -> &mut WordInfo {
        match usize::try_from(i) {
            Ok(i) if i < self.buffer.len() => &mut self.buffer[i],
            _ => {
                self.nil_entry = WordInfo::default();
                &mut self.nil_entry
            }
        }
    }

    fn iso_to_lexi(&self, key: u32) -> i32 {
        if key >= 256 {
            vnl_nonVnChar
        } else {
            self.iso_vn_lexi[key as usize]
        }
    }

    fn char_type(&self, key: u32) -> CharType {
        if key > 255 {
            if self.iso_to_lexi(key) == vnl_nonVnChar {
                CharType::NonVn
            } else {
                CharType::Vn
            }
        } else {
            self.ukc_map[key as usize]
        }
    }

    fn key_code_to_event(&self, key: u32) -> KeyEvent {
        let mut ev = KeyEvent { key_code: key, ..Default::default() };
        if key > 255 {
            ev.ev_type = vneNormal;
            ev.vn_sym = self.iso_to_lexi(key);
            ev.ch_type = if ev.vn_sym == vnl_nonVnChar { CharType::NonVn } else { CharType::Vn };
        } else {
            ev.ch_type = self.ukc_map[key as usize];
            ev.ev_type = self.key_map[key as usize];
            if (vneTone0..=vneTone5).contains(&ev.ev_type) {
                ev.tone = ev.ev_type - vneTone0;
            }
            if ev.ev_type >= vneCount {
                ev.ch_type = CharType::Vn;
                ev.vn_sym = ev.ev_type - vneCount;
                ev.ev_type = vneMapChar;
            } else {
                ev.vn_sym = self.iso_to_lexi(key);
            }
        }
        ev
    }

    fn key_code_to_symbol(&self, key: u32) -> KeyEvent {
        KeyEvent {
            key_code: key,
            ev_type: vneNormal,
            vn_sym: self.iso_to_lexi(key),
            ch_type: self.char_type(key),
            tone: 0,
        }
    }

    fn get_tone_position(&self, vs: i32, terminated: bool) -> i32 {
        let info = vi(vs);
        if info.len == 1 {
            return 0;
        }
        if info.roof_pos != -1 {
            return info.roof_pos;
        }
        if info.hook_pos != -1 {
            if vs == vs_uhoh || vs == vs_uhohi || vs == vs_uhohu {
                return 1;
            }
            return info.hook_pos;
        }
        if info.len == 3 {
            return 1;
        }
        if self.options.modern_style && (vs == vs_oa || vs == vs_oe || vs == vs_uy) {
            return 1;
        }
        if terminated {
            0
        } else {
            1
        }
    }

    fn mark_change(&mut self, pos: i32) {
        if pos < self.change_pos {
            // Unicode output: one backspace per buffered symbol.
            self.backs += (self.change_pos - 1) - pos + 1;
            self.change_pos = pos;
        }
    }

    /// `(c1, c2)` consonant sequences around the vowel of the word ending at `current`.
    fn current_c1_c2(&self) -> (i32, i32) {
        let e = *self.b(self.current);
        let c1 = if e.c1_offset != -1 { self.b(self.current - e.c1_offset).seq } else { cs_nil };
        let c2 = if e.c2_offset != -1 { self.b(self.current - e.c2_offset).seq } else { cs_nil };
        (c1, c2)
    }

    fn lookup_vseq_at(&self, v_start: i32, len: i32) -> i32 {
        match len {
            3 => lookup_vseq(self.b(v_start).vn_sym, self.b(v_start + 1).vn_sym, self.b(v_start + 2).vn_sym),
            2 => lookup_vseq2(self.b(v_start).vn_sym, self.b(v_start + 1).vn_sym),
            _ => lookup_vseq1(self.b(v_start).vn_sym),
        }
    }

    fn move_tone(&mut self, new_pos: i32, cur_pos: i32, tone: i32) {
        self.mark_change(new_pos);
        self.bm(new_pos).tone = tone;
        self.mark_change(cur_pos);
        self.bm(cur_pos).tone = 0;
    }

    fn process_roof(&mut self, ev: &mut KeyEvent) -> i32 {
        if !self.viet_key || self.current < 0 || self.b(self.current).v_offset < 0 {
            return self.process_append(ev);
        }
        let target = match ev.ev_type {
            vneRoof_a => vnl_ar,
            vneRoof_e => vnl_er,
            vneRoof_o => vnl_or,
            _ => vnl_nonVnChar,
        };

        let mut roof_removed = false;
        let v_end = self.current - self.b(self.current).v_offset;
        let vs = self.b(v_end).seq;
        let v_start = v_end - (vi(vs).len - 1);
        let cur_tone_pos = v_start + self.get_tone_position(vs, v_end == self.current);
        let tone = self.b(cur_tone_pos).tone;

        let mut double_change_uo = false;
        let mut new_vs;
        if vs == vs_uho || vs == vs_uhoh || vs == vs_uhoi || vs == vs_uhohi {
            // special cases: u+o+ -> uo^, u+o -> uo^, u+o+i -> uo^i, u+oi -> uo^i
            new_vs = lookup_vseq(vnl_u, vnl_or, vi(vs).v[2]);
            double_change_uo = true;
        } else {
            new_vs = vi(vs).with_roof;
        }

        let info;
        if new_vs == vs_nil {
            if vi(vs).roof_pos == -1 {
                return self.process_append(ev); // roof is not applicable
            }
            // a roof already exists -> undo roof
            let cur_ch = self.b(v_start + vi(vs).roof_pos).vn_sym;
            if target != vnl_nonVnChar && cur_ch != target {
                return self.process_append(ev); // specific roof and the roof character don't match
            }
            let new_ch = if cur_ch == vnl_ar {
                vnl_a
            } else if cur_ch == vnl_er {
                vnl_e
            } else {
                vnl_o
            };
            let change_pos = v_start + vi(vs).roof_pos;
            if !self.options.free_marking && change_pos != self.current {
                return self.process_append(ev);
            }
            self.mark_change(change_pos);
            self.bm(change_pos).vn_sym = new_ch;
            new_vs = self.lookup_vseq_at(v_start, vi(vs).len);
            info = vi(new_vs);
            roof_removed = true;
        } else {
            info = vi(new_vs);
            if target != vnl_nonVnChar && at3(&info.v, info.roof_pos) != target {
                return self.process_append(ev);
            }
            // check validity of new VC and CV
            let (c1, c2) = self.current_c1_c2();
            if !is_valid_cvc(c1, new_vs, c2) {
                return self.process_append(ev);
            }
            let change_pos = if double_change_uo { v_start } else { v_start + info.roof_pos };
            if !self.options.free_marking && change_pos != self.current {
                return self.process_append(ev);
            }
            self.mark_change(change_pos);
            if double_change_uo {
                self.bm(v_start).vn_sym = vnl_u;
                self.bm(v_start + 1).vn_sym = vnl_or;
            } else {
                self.bm(change_pos).vn_sym = at3(&info.v, info.roof_pos);
            }
        }

        for i in 0..info.len {
            self.bm(v_start + i).seq = at3(&info.sub, i);
        }

        // check if tone re-position is needed
        let new_tone_pos = v_start + self.get_tone_position(new_vs, v_end == self.current);
        if cur_tone_pos != new_tone_pos && tone != 0 {
            self.move_tone(new_tone_pos, cur_tone_pos, tone);
        }

        if roof_removed {
            self.single_mode = false;
            self.process_append(ev);
            self.reverted = true;
        }
        1
    }

    /// Can only be called from `process_hook`.
    fn process_hook_with_uo(&mut self, ev: &mut KeyEvent) -> i32 {
        let mut hook_removed = false;
        if !self.options.free_marking && self.b(self.current).v_offset != 0 {
            return self.process_append(ev);
        }

        let v_end = self.current - self.b(self.current).v_offset;
        let vs = self.b(v_end).seq;
        let v_start = v_end - (vi(vs).len - 1);
        let v = vi(vs).v;
        let cur_tone_pos = v_start + self.get_tone_position(vs, v_end == self.current);
        let tone = self.b(cur_tone_pos).tone;
        let new_vs;

        match ev.ev_type {
            vneHook_u => {
                if v[0] == vnl_u {
                    new_vs = vi(vs).with_hook;
                    self.mark_change(v_start);
                    self.bm(v_start).vn_sym = vnl_uh;
                } else {
                    // v[0] = vnl_uh, -> uo
                    new_vs = lookup_vseq(vnl_u, vnl_o, v[2]);
                    self.mark_change(v_start);
                    self.bm(v_start).vn_sym = vnl_u;
                    self.bm(v_start + 1).vn_sym = vnl_o;
                    hook_removed = true;
                }
            }
            vneHook_o => {
                if v[1] == vnl_o || v[1] == vnl_or {
                    if v_end == self.current
                        && vi(vs).len == 2
                        && self.b(self.current).form == Form::Cv
                        && self.b(self.current - 2).seq == cs_th
                    {
                        // o|o^ -> o+
                        new_vs = vi(vs).with_hook;
                        self.mark_change(v_start + 1);
                        self.bm(v_start + 1).vn_sym = vnl_oh;
                    } else {
                        new_vs = lookup_vseq(vnl_uh, vnl_oh, v[2]);
                        if v[0] == vnl_u {
                            self.mark_change(v_start);
                            self.bm(v_start).vn_sym = vnl_uh;
                            self.bm(v_start + 1).vn_sym = vnl_oh;
                        } else {
                            self.mark_change(v_start + 1);
                            self.bm(v_start + 1).vn_sym = vnl_oh;
                        }
                    }
                } else {
                    // v[1] = vnl_oh, -> uo
                    new_vs = lookup_vseq(vnl_u, vnl_o, v[2]);
                    if v[0] == vnl_uh {
                        self.mark_change(v_start);
                        self.bm(v_start).vn_sym = vnl_u;
                        self.bm(v_start + 1).vn_sym = vnl_o;
                    } else {
                        self.mark_change(v_start + 1);
                        self.bm(v_start + 1).vn_sym = vnl_o;
                    }
                    hook_removed = true;
                }
            }
            _ => {
                // vneHookAll, vneHookUO
                if v[0] == vnl_u {
                    if v[1] == vnl_o || v[1] == vnl_or {
                        // uo -> uo+ if prefixed by "th"
                        if (vs == vs_uo || vs == vs_uor)
                            && v_end == self.current
                            && self.b(self.current).form == Form::Cv
                            && self.b(self.current - 2).seq == cs_th
                        {
                            new_vs = vs_uoh;
                            self.mark_change(v_start + 1);
                            self.bm(v_start + 1).vn_sym = vnl_oh;
                        } else {
                            // uo -> u+o+
                            let hooked = vi(vs).with_hook;
                            self.mark_change(v_start);
                            self.bm(v_start).vn_sym = vnl_uh;
                            new_vs = vi(hooked).with_hook;
                            self.bm(v_start + 1).vn_sym = vnl_oh;
                        }
                    } else {
                        // uo+ -> u+o+
                        new_vs = vi(vs).with_hook;
                        self.mark_change(v_start);
                        self.bm(v_start).vn_sym = vnl_uh;
                    }
                } else if v[1] == vnl_o {
                    // u+o -> u+o+
                    new_vs = vi(vs).with_hook;
                    self.mark_change(v_start + 1);
                    self.bm(v_start + 1).vn_sym = vnl_oh;
                } else {
                    // v[1] == vnl_oh, u+o+ -> uo
                    new_vs = lookup_vseq(vnl_u, vnl_o, v[2]);
                    self.mark_change(v_start);
                    self.bm(v_start).vn_sym = vnl_u;
                    self.bm(v_start + 1).vn_sym = vnl_o;
                    hook_removed = true;
                }
            }
        }

        let p = vi(new_vs);
        for i in 0..p.len {
            self.bm(v_start + i).seq = at3(&p.sub, i);
        }

        // check if tone re-position is needed
        let new_tone_pos = v_start + self.get_tone_position(new_vs, v_end == self.current);
        if cur_tone_pos != new_tone_pos && tone != 0 {
            self.move_tone(new_tone_pos, cur_tone_pos, tone);
        }

        if hook_removed {
            self.single_mode = false;
            self.process_append(ev);
            self.reverted = true;
        }
        1
    }

    fn process_hook(&mut self, ev: &mut KeyEvent) -> i32 {
        if !self.viet_key || self.current < 0 || self.b(self.current).v_offset < 0 {
            return self.process_append(ev);
        }

        let v_end = self.current - self.b(self.current).v_offset;
        let vs = self.b(v_end).seq;
        let v = vi(vs).v;

        if vi(vs).len > 1
            && ev.ev_type != vneBowl
            && (v[0] == vnl_u || v[0] == vnl_uh)
            && (v[1] == vnl_o || v[1] == vnl_oh || v[1] == vnl_or)
        {
            return self.process_hook_with_uo(ev);
        }

        let v_start = v_end - (vi(vs).len - 1);
        let cur_tone_pos = v_start + self.get_tone_position(vs, v_end == self.current);
        let tone = self.b(cur_tone_pos).tone;
        let mut hook_removed = false;

        let mut new_vs = vi(vs).with_hook;
        let info;
        if new_vs == vs_nil {
            if vi(vs).hook_pos == -1 {
                return self.process_append(ev); // hook is not applicable
            }
            // a hook already exists -> undo hook
            let cur_ch = self.b(v_start + vi(vs).hook_pos).vn_sym;
            let new_ch = if cur_ch == vnl_ab {
                vnl_a
            } else if cur_ch == vnl_uh {
                vnl_u
            } else {
                vnl_o
            };
            let change_pos = v_start + vi(vs).hook_pos;
            if !self.options.free_marking && change_pos != self.current {
                return self.process_append(ev);
            }
            match ev.ev_type {
                vneHook_u if cur_ch != vnl_uh => return self.process_append(ev),
                vneHook_o if cur_ch != vnl_oh => return self.process_append(ev),
                vneBowl if cur_ch != vnl_ab => return self.process_append(ev),
                vneHook_uo if cur_ch == vnl_ab => return self.process_append(ev),
                _ => {}
            }
            self.mark_change(change_pos);
            self.bm(change_pos).vn_sym = new_ch;
            new_vs = self.lookup_vseq_at(v_start, vi(vs).len);
            info = vi(new_vs);
            hook_removed = true;
        } else {
            info = vi(new_vs);
            let hooked = at3(&info.v, info.hook_pos);
            match ev.ev_type {
                vneHook_u if hooked != vnl_uh => return self.process_append(ev),
                vneHook_o if hooked != vnl_oh => return self.process_append(ev),
                vneBowl if hooked != vnl_ab => return self.process_append(ev),
                vneHook_uo if hooked == vnl_ab => return self.process_append(ev),
                _ => {}
            }
            // check validity of new VC and CV
            let (c1, c2) = self.current_c1_c2();
            if !is_valid_cvc(c1, new_vs, c2) {
                return self.process_append(ev);
            }
            let change_pos = v_start + info.hook_pos;
            if !self.options.free_marking && change_pos != self.current {
                return self.process_append(ev);
            }
            self.mark_change(change_pos);
            self.bm(change_pos).vn_sym = hooked;
        }

        for i in 0..info.len {
            self.bm(v_start + i).seq = at3(&info.sub, i);
        }

        // check if tone re-position is needed
        let new_tone_pos = v_start + self.get_tone_position(new_vs, v_end == self.current);
        if cur_tone_pos != new_tone_pos && tone != 0 {
            self.move_tone(new_tone_pos, cur_tone_pos, tone);
        }

        if hook_removed {
            self.single_mode = false;
            self.process_append(ev);
            self.reverted = true;
        }
        1
    }

    fn process_tone(&mut self, ev: &mut KeyEvent) -> i32 {
        if self.current < 0 || !self.viet_key {
            return self.process_append(ev);
        }

        let cur = *self.b(self.current);
        if cur.form == Form::C && (cur.seq == cs_gi || cur.seq == cs_gin) {
            let p = if cur.seq == cs_gi { self.current } else { self.current - 1 };
            if self.b(p).tone == 0 && ev.tone == 0 {
                return self.process_append(ev);
            }
            self.mark_change(p);
            if self.b(p).tone == ev.tone {
                self.bm(p).tone = 0;
                self.single_mode = false;
                self.process_append(ev);
                self.reverted = true;
                return 1;
            }
            self.bm(p).tone = ev.tone;
            return 1;
        }

        if cur.v_offset < 0 {
            return self.process_append(ev);
        }

        let v_end = self.current - cur.v_offset;
        let vs = self.b(v_end).seq;
        let info = vi(vs);
        if self.options.spell_check && !self.options.free_marking && !info.complete {
            return self.process_append(ev);
        }

        if cur.form == Form::Vc || cur.form == Form::Cvc {
            let cs = cur.seq;
            if (cs == cs_c || cs == cs_ch || cs == cs_p || cs == cs_t)
                && (ev.tone == 2 || ev.tone == 3 || ev.tone == 4)
            {
                return self.process_append(ev); // c, ch, p, t suffixes don't allow ` ? ~
            }
        }

        let tone_offset = self.get_tone_position(vs, v_end == self.current);
        let tone_pos = v_end - (info.len - 1) + tone_offset;
        if self.b(tone_pos).tone == 0 && ev.tone == 0 {
            return self.process_append(ev);
        }

        if self.b(tone_pos).tone == ev.tone {
            self.mark_change(tone_pos);
            self.bm(tone_pos).tone = 0;
            self.single_mode = false;
            self.process_append(ev);
            self.reverted = true;
            return 1;
        }

        self.mark_change(tone_pos);
        self.bm(tone_pos).tone = ev.tone;
        1
    }

    fn process_dd(&mut self, ev: &mut KeyEvent) -> i32 {
        if !self.viet_key || self.current < 0 {
            return self.process_append(ev);
        }

        // Allow dd even in a non-vn sequence, because dd is used a lot in abbreviations,
        // but only if the preceding character is not a vowel.
        let cur = *self.b(self.current);
        let prev_sym = self.b(self.current - 1).vn_sym;
        if cur.form == Form::NonVn && cur.vn_sym == vnl_d && (prev_sym == vnl_nonVnChar || !is_vn_vowel(prev_sym)) {
            self.single_mode = true;
            let pos = self.current;
            self.mark_change(pos);
            let e = self.bm(pos);
            e.seq = cs_dd;
            e.vn_sym = vnl_dd;
            e.form = Form::C;
            e.c1_offset = 0;
            e.c2_offset = -1;
            e.v_offset = -1;
            return 1;
        }

        if cur.c1_offset < 0 {
            return self.process_append(ev);
        }

        let pos = self.current - cur.c1_offset;
        if !self.options.free_marking && pos != self.current {
            return self.process_append(ev);
        }

        if self.b(pos).seq == cs_d {
            self.mark_change(pos);
            let e = self.bm(pos);
            e.seq = cs_dd;
            e.vn_sym = vnl_dd;
            // never spellcheck a word which starts with dd, because it's used a lot in abbreviations
            self.single_mode = true;
            return 1;
        }

        if self.b(pos).seq == cs_dd {
            // undo dd
            self.mark_change(pos);
            let e = self.bm(pos);
            e.seq = cs_d;
            e.vn_sym = vnl_d;
            self.single_mode = false;
            self.process_append(ev);
            self.reverted = true;
            return 1;
        }

        self.process_append(ev)
    }

    fn process_map_char(&mut self, ev: &mut KeyEvent) -> i32 {
        if self.caps_lock {
            ev.vn_sym = change_case(ev.vn_sym);
        }

        let ret = self.process_append(ev);
        if !self.viet_key {
            return ret;
        }

        if self.current >= 0 {
            let f = self.b(self.current).form;
            if f != Form::Empty && f != Form::NonVn {
                return 1;
            }
        }

        if self.current < 0 {
            return 0;
        }

        // mapChar doesn't apply
        self.current -= 1;
        let entry = *self.b(self.current);

        let mut undo = false;
        // test if undo is needed
        if entry.form != Form::Empty && entry.form != Form::NonVn {
            let mut prev_sym = entry.vn_sym;
            if entry.caps {
                prev_sym -= 1;
            }
            if prev_sym == ev.vn_sym {
                if entry.form != Form::C {
                    let v_end = self.current - entry.v_offset;
                    let vs = self.b(v_end).seq;
                    let v_start = v_end - vi(vs).len + 1;
                    let cur_tone_pos = v_start + self.get_tone_position(vs, v_end == self.current);
                    let tone = self.b(cur_tone_pos).tone;
                    self.mark_change(self.current);
                    self.current -= 1;

                    // check if tone re-position is needed
                    if tone != 0 && self.current >= 0 {
                        let f = self.b(self.current).form;
                        if f == Form::V || f == Form::Cv {
                            let new_vs = self.b(self.current).seq;
                            let new_tone_pos = v_start + self.get_tone_position(new_vs, true);
                            if new_tone_pos != cur_tone_pos {
                                self.move_tone(new_tone_pos, cur_tone_pos, tone);
                            }
                        }
                    }
                } else {
                    self.mark_change(self.current);
                    self.current -= 1;
                }
                undo = true;
            }
        }

        ev.ev_type = vneNormal;
        ev.ch_type = self.char_type(ev.key_code);
        ev.vn_sym = self.iso_to_lexi(ev.key_code);
        let ret = self.process_append(ev);
        if undo {
            self.single_mode = false;
            self.reverted = true;
            return 1;
        }
        ret
    }

    fn set_w_as_map_char(&self, ev: &mut KeyEvent) {
        ev.ev_type = vneMapChar;
        ev.vn_sym = if is_upper(ev.key_code) { vnl_Uh } else { vnl_uh };
        if self.caps_lock {
            ev.vn_sym = change_case(ev.vn_sym);
        }
        ev.ch_type = CharType::Vn;
    }

    fn process_telex_w(&mut self, ev: &mut KeyEvent) -> i32 {
        if !self.viet_key {
            return self.process_append(ev);
        }

        if self.used_as_map_char {
            self.set_w_as_map_char(ev);
            let ret = self.process_map_char(ev);
            if ret == 0 {
                if self.current >= 0 {
                    self.current -= 1;
                }
                self.used_as_map_char = false;
                ev.ev_type = vneHookAll;
                return self.process_hook(ev);
            }
            return ret;
        }

        ev.ev_type = vneHookAll;
        self.used_as_map_char = false;
        let ret = self.process_hook(ev);
        if ret == 0 {
            if self.current >= 0 {
                self.current -= 1;
            }
            self.set_w_as_map_char(ev);
            self.used_as_map_char = true;
            return self.process_map_char(ev);
        }
        ret
    }

    fn process_append(&mut self, ev: &mut KeyEvent) -> i32 {
        match ev.ch_type {
            CharType::Reset => {
                self.reset();
                0
            }
            CharType::WordBreak => {
                self.single_mode = false;
                self.process_word_end(ev)
            }
            CharType::NonVn => {
                self.current += 1;
                let sym = vn_to_lower(ev.vn_sym);
                let e = self.bm(self.current);
                e.form = Form::NonVn;
                e.c1_offset = -1;
                e.c2_offset = -1;
                e.v_offset = -1;
                e.key_code = ev.key_code;
                e.vn_sym = sym;
                e.tone = 0;
                e.caps = sym != ev.vn_sym;
                0
            }
            CharType::Vn => {
                if is_vn_vowel(ev.vn_sym) {
                    let v = no_tone(vn_to_lower(ev.vn_sym));
                    if self.current >= 0 {
                        let cur = *self.b(self.current);
                        if cur.form == Form::C
                            && ((cur.seq == cs_q && v == vnl_u) || (cur.seq == cs_g && v == vnl_i))
                        {
                            return self.append_consonnant(ev); // process u after q, i after g as consonants
                        }
                    }
                    return self.append_vowel(ev);
                }
                self.append_consonnant(ev)
            }
        }
    }

    fn set_non_vn(&mut self, i: i32) {
        let e = self.bm(i);
        e.form = Form::NonVn;
        e.c1_offset = -1;
        e.c2_offset = -1;
        e.v_offset = -1;
    }

    fn append_vowel(&mut self, ev: &mut KeyEvent) -> i32 {
        self.current += 1;
        let cur = self.current;
        let lower_sym = vn_to_lower(ev.vn_sym);
        let can_sym = no_tone(lower_sym);

        {
            let e = self.bm(cur);
            e.vn_sym = can_sym;
            e.caps = lower_sym != ev.vn_sym;
            e.tone = (lower_sym - can_sym) / 2;
            e.key_code = ev.key_code;
        }

        if cur == 0 || !self.viet_key {
            let vseq = lookup_vseq1(can_sym);
            let e = self.bm(cur);
            e.form = Form::V;
            e.c1_offset = -1;
            e.c2_offset = -1;
            e.v_offset = 0;
            e.seq = vseq;
            if !self.viet_key || is_alpha(ev.key_code) {
                return 0;
            }
            self.mark_change(cur);
            return 1;
        }

        let prev = *self.b(cur - 1);
        match prev.form {
            Form::Empty => {
                let vseq = lookup_vseq1(can_sym);
                let e = self.bm(cur);
                e.form = Form::V;
                e.c1_offset = -1;
                e.c2_offset = -1;
                e.v_offset = 0;
                e.seq = vseq;
            }
            Form::NonVn | Form::Cvc | Form::Vc => self.set_non_vn(cur),
            Form::V | Form::Cv => {
                let vs = prev.seq;
                let info = vi(vs);
                let prev_tone_pos = (cur - 1) - (info.len - 1) + self.get_tone_position(vs, true);
                let mut tone = self.b(prev_tone_pos).tone;

                let mut new_vs = if lower_sym != can_sym && tone != 0 {
                    vs_nil // new sym has a tone, but there's already a preceding tone
                } else if info.len == 3 {
                    vs_nil
                } else if info.len == 2 {
                    lookup_vseq(info.v[0], info.v[1], can_sym)
                } else {
                    lookup_vseq2(info.v[0], can_sym)
                };

                if new_vs != vs_nil && prev.form == Form::Cv {
                    let cs = self.b(cur - 1 - prev.c1_offset).seq;
                    if !is_valid_cv(cs, new_vs) {
                        new_vs = vs_nil;
                    }
                }

                if new_vs == vs_nil {
                    self.set_non_vn(cur);
                } else {
                    {
                        let e = self.bm(cur);
                        e.form = prev.form;
                        e.c1_offset = if prev.form == Form::Cv { prev.c1_offset + 1 } else { -1 };
                        e.c2_offset = -1;
                        e.v_offset = 0;
                        e.seq = new_vs;
                        e.tone = 0;
                    }

                    let new_tone = (lower_sym - can_sym) / 2;
                    let base = (cur - 1) - info.len + 1;
                    if tone == 0 {
                        if new_tone != 0 {
                            tone = new_tone;
                            let tone_pos = self.get_tone_position(new_vs, true) + base;
                            self.mark_change(tone_pos);
                            self.bm(tone_pos).tone = tone;
                            return 1;
                        }
                    } else {
                        let new_tone_pos = self.get_tone_position(new_vs, true) + base;
                        if new_tone_pos != prev_tone_pos {
                            self.mark_change(prev_tone_pos);
                            self.bm(prev_tone_pos).tone = 0;
                            self.mark_change(new_tone_pos);
                            if new_tone != 0 {
                                tone = new_tone;
                            }
                            self.bm(new_tone_pos).tone = tone;
                            return 1;
                        }
                        if new_tone != 0 && new_tone != tone {
                            self.mark_change(prev_tone_pos);
                            self.bm(prev_tone_pos).tone = new_tone;
                            return 1;
                        }
                    }
                }
            }
            Form::C => {
                let new_vs = lookup_vseq1(can_sym);
                let cs = prev.seq;
                if !is_valid_cv(cs, new_vs) {
                    self.set_non_vn(cur);
                } else {
                    {
                        let e = self.bm(cur);
                        e.form = Form::Cv;
                        e.c1_offset = 1;
                        e.c2_offset = -1;
                        e.v_offset = 0;
                        e.seq = new_vs;
                    }
                    if cs == cs_gi && prev.tone != 0 {
                        if self.b(cur).tone == 0 {
                            self.bm(cur).tone = prev.tone;
                        }
                        self.mark_change(cur - 1);
                        self.bm(cur - 1).tone = 0;
                        return 1;
                    }
                }
            }
        }

        if is_alpha(ev.key_code) {
            return 0;
        }
        self.mark_change(cur);
        1
    }

    fn append_consonnant(&mut self, ev: &mut KeyEvent) -> i32 {
        self.current += 1;
        let cur = self.current;
        let lower_sym = vn_to_lower(ev.vn_sym);

        {
            let e = self.bm(cur);
            e.vn_sym = lower_sym;
            e.caps = lower_sym != ev.vn_sym;
            e.key_code = ev.key_code;
            e.tone = 0;
        }

        if cur == 0 || !self.viet_key {
            let cs = lookup_cseq(lower_sym, vnl_nonVnChar, vnl_nonVnChar);
            let e = self.bm(cur);
            e.form = Form::C;
            e.c1_offset = 0;
            e.c2_offset = -1;
            e.v_offset = -1;
            e.seq = cs;
            return 0;
        }

        let prev = *self.b(cur - 1);
        match prev.form {
            Form::NonVn => {
                self.set_non_vn(cur);
                0
            }
            Form::Empty => {
                let cs = lookup_cseq(lower_sym, vnl_nonVnChar, vnl_nonVnChar);
                let e = self.bm(cur);
                e.form = Form::C;
                e.c1_offset = 0;
                e.c2_offset = -1;
                e.v_offset = -1;
                e.seq = cs;
                0
            }
            Form::V | Form::Cv => {
                let vs = prev.seq;
                let new_vs = if vs == vs_uoh || vs == vs_uho { vs_uhoh } else { vs };
                let c1 = if prev.c1_offset != -1 { self.b(cur - 1 - prev.c1_offset).seq } else { cs_nil };
                let new_cs = lookup_cseq(lower_sym, vnl_nonVnChar, vnl_nonVnChar);

                let mut complex_event = false;
                if is_valid_cvc(c1, new_vs, new_cs) {
                    // check u+o -> u+o+
                    if vs == vs_uho {
                        self.mark_change(cur - 1);
                        let e = self.bm(cur - 1);
                        e.vn_sym = vnl_oh;
                        e.seq = vs_uhoh;
                        complex_event = true;
                    } else if vs == vs_uoh {
                        self.mark_change(cur - 2);
                        let e = self.bm(cur - 2);
                        e.vn_sym = vnl_uh;
                        e.seq = vs_uh;
                        self.bm(cur - 1).seq = vs_uhoh;
                        complex_event = true;
                    }

                    {
                        let e = self.bm(cur);
                        if prev.form == Form::V {
                            e.form = Form::Vc;
                            e.c1_offset = -1;
                        } else {
                            e.form = Form::Cvc;
                            e.c1_offset = prev.c1_offset + 1;
                        }
                        e.c2_offset = 0;
                        e.v_offset = 1;
                        e.seq = new_cs;
                    }

                    // reposition tone if needed
                    let old_idx = (cur - 1) - (vi(vs).len - 1) + self.get_tone_position(vs, true);
                    if self.b(old_idx).tone != 0 {
                        let new_idx = (cur - 1) - (vi(new_vs).len - 1) + self.get_tone_position(new_vs, false);
                        if new_idx != old_idx {
                            let tone = self.b(old_idx).tone;
                            self.move_tone(new_idx, old_idx, tone);
                            return 1;
                        }
                    }
                } else {
                    self.set_non_vn(cur);
                }
                i32::from(complex_event)
            }
            Form::C | Form::Vc | Form::Cvc => {
                let cs = prev.seq;
                let info = ci(cs);
                let mut new_cs = if info.len == 3 {
                    cs_nil
                } else if info.len == 2 {
                    lookup_cseq(info.c[0], info.c[1], lower_sym)
                } else {
                    lookup_cseq(info.c[0], lower_sym, vnl_nonVnChar)
                };

                if new_cs != cs_nil && (prev.form == Form::Vc || prev.form == Form::Cvc) {
                    // Check CVC combination
                    let c1 = if prev.c1_offset != -1 { self.b(cur - 1 - prev.c1_offset).seq } else { cs_nil };
                    let vs = self.b((cur - 1) - prev.v_offset).seq;
                    if !is_valid_cvc(c1, vs, new_cs) {
                        new_cs = cs_nil;
                    }
                }

                if new_cs == cs_nil {
                    self.set_non_vn(cur);
                } else {
                    let e = self.bm(cur);
                    match prev.form {
                        Form::C => {
                            e.form = Form::C;
                            e.c1_offset = 0;
                            e.c2_offset = -1;
                            e.v_offset = -1;
                        }
                        Form::Vc => {
                            e.form = Form::Vc;
                            e.c1_offset = -1;
                            e.c2_offset = 0;
                            e.v_offset = prev.v_offset + 1;
                        }
                        _ => {
                            e.form = Form::Cvc;
                            e.c1_offset = prev.c1_offset + 1;
                            e.c2_offset = 0;
                            e.v_offset = prev.v_offset + 1;
                        }
                    }
                    e.seq = new_cs;
                }
                0
            }
        }
    }

    fn process_no_spell_check(&mut self, ev: &KeyEvent) -> i32 {
        let cur = self.current;
        let sym = self.b(cur).vn_sym;
        if is_vn_vowel(sym) {
            let vseq = lookup_vseq1(sym);
            let e = self.bm(cur);
            e.form = Form::V;
            e.v_offset = 0;
            e.seq = vseq;
            e.c1_offset = -1;
            e.c2_offset = -1;
        } else {
            let cs = lookup_cseq(sym, vnl_nonVnChar, vnl_nonVnChar);
            let e = self.bm(cur);
            e.form = Form::C;
            e.c1_offset = 0;
            e.c2_offset = -1;
            e.v_offset = -1;
            e.seq = cs;
        }

        if ev.ev_type == vneNormal && is_alpha(self.b(cur).key_code) {
            return 0;
        }
        self.mark_change(cur);
        1
    }

    fn prepare_buffer(&mut self) {
        // symbol buffer: keep at least 10 entries available
        if self.current >= 0 && self.current + 10 >= MAX_UK_ENGINE {
            // Get rid of at least half of the current entries,
            // but don't get rid from the middle of a word.
            let mut rid = self.current / 2;
            while self.b(rid).form != Form::Empty && rid < self.current {
                rid += 1;
            }
            if rid == self.current {
                self.current = -1;
            } else {
                rid += 1;
                self.buffer.copy_within(rid as usize..(self.current + 1) as usize, 0);
                self.current -= rid;
            }
        }

        // key stroke buffer
        if self.key_current > 0 && self.key_current + 1 >= MAX_UK_ENGINE {
            let rid = self.key_current / 2;
            self.key_strokes.copy_within(rid as usize..(self.key_current + 1) as usize, 0);
            self.key_current -= rid;
        }
    }

    /// Feed one typed character (Latin-1 range for ASCII keys).
    pub fn process(&mut self, key_code: u32) -> Output {
        self.prepare_buffer();
        self.backs = 0;
        self.change_pos = self.current + 1;
        self.out.clear();
        self.output_written = false;
        self.reverted = false;
        self.key_restored = false;
        self.key_restoring = false;
        self.key_output = false;

        let mut ev = self.key_code_to_event(key_code);
        let mut ret = match ev.ev_type {
            vneRoofAll | vneRoof_a | vneRoof_e | vneRoof_o => self.process_roof(&mut ev),
            vneHookAll | vneHook_uo | vneHook_u | vneHook_o | vneBowl => self.process_hook(&mut ev),
            vneDd => self.process_dd(&mut ev),
            vneTone0..=vneTone5 => self.process_tone(&mut ev),
            vne_telex_w => self.process_telex_w(&mut ev),
            vneMapChar => self.process_map_char(&mut ev),
            _ => self.process_append(&mut ev),
        };

        if self.viet_key
            && self.current >= 0
            && self.b(self.current).form == Form::NonVn
            && ev.ch_type == CharType::Vn
            && (!self.options.spell_check || self.single_mode)
        {
            // Spell check has failed, but because we are in non-spellcheck mode,
            // the new character is considered the beginning of a new word.
            ret = self.process_no_spell_check(&ev);
        }

        // add the key to the key buffer only if it has not caused a reset
        if self.current >= 0 {
            ev.ch_type = self.char_type(ev.key_code);
            self.key_current += 1;
            let converted = ret != 0 && !self.key_restored;
            if let Some(k) = usize::try_from(self.key_current).ok().and_then(|i| self.key_strokes.get_mut(i)) {
                *k = KeyBufEntry { ev, converted };
            }
        }

        if ret == 0 {
            return Output { handled: false, backs: 0, text: Vec::new(), key_output: self.key_output };
        }

        if !self.output_written {
            self.write_output();
        }
        Output {
            handled: true,
            backs: self.backs.max(0) as usize,
            text: std::mem::take(&mut self.out),
            key_output: self.key_output,
        }
    }

    fn write_output(&mut self) {
        self.out.clear();
        for i in self.change_pos..=self.current {
            let e = *self.b(i);
            let std_char = if e.vn_sym != vnl_nonVnChar {
                let mut c = (e.vn_sym as u32).wrapping_add(VN_STD_CHAR_OFFSET);
                if e.caps {
                    c = c.wrapping_sub(1);
                }
                if e.tone != 0 {
                    c = c.wrapping_add((e.tone * 2) as u32);
                }
                c
            } else if e.key_code < 256 {
                self.iso_std[e.key_code as usize]
            } else {
                e.key_code
            };
            let unit = if std_char >= VN_STD_CHAR_OFFSET {
                UNICODE_TABLE.get((std_char - VN_STD_CHAR_OFFSET) as usize).copied().unwrap_or(0)
            } else {
                std_char as u16
            };
            self.out.push(unit);
        }
    }

    fn synch_key_stroke_buffer(&mut self) {
        if self.key_current >= 0 {
            self.key_current -= 1;
        }
        if self.current >= 0 && self.b(self.current).form == Form::Empty {
            // reached a word break: move the key stroke pointer back to the matching word break
            while self.key_current >= 0
                && self.key_strokes[self.key_current as usize].ev.ch_type != CharType::WordBreak
            {
                self.key_current -= 1;
            }
        }
    }

    /// Feed a backspace. If the result is not `handled`, let the real backspace through.
    pub fn process_backspace(&mut self) -> Output {
        self.key_output = false;
        if !self.viet_key || self.current < 0 {
            return Output::default();
        }

        self.backs = 0;
        self.change_pos = self.current + 1;
        self.mark_change(self.current);

        let cur = self.b(self.current).form;
        let prev = self.b(self.current - 1).form;
        if self.current == 0
            || matches!(cur, Form::Empty | Form::NonVn | Form::C)
            || matches!(prev, Form::C | Form::Cvc | Form::Vc)
        {
            self.current -= 1;
            self.synch_key_stroke_buffer();
            return Output { handled: self.backs > 1, backs: self.backs as usize, ..Default::default() };
        }

        let v_end = self.current - self.b(self.current).v_offset;
        let vs = self.b(v_end).seq;
        let v_start = v_end - vi(vs).len + 1;
        let new_vs = self.b(self.current - 1).seq;
        let cur_tone_pos = v_start + self.get_tone_position(vs, v_end == self.current);
        let new_tone_pos = v_start + self.get_tone_position(new_vs, true);
        let tone = self.b(cur_tone_pos).tone;

        if tone == 0
            || cur_tone_pos == new_tone_pos
            || (cur_tone_pos == self.current && self.b(self.current).tone != 0)
        {
            self.current -= 1;
            self.synch_key_stroke_buffer();
            return Output { handled: self.backs > 1, backs: self.backs as usize, ..Default::default() };
        }

        self.move_tone(new_tone_pos, cur_tone_pos, tone);
        self.current -= 1;
        self.synch_key_stroke_buffer();
        self.write_output();
        Output { handled: true, backs: self.backs as usize, text: std::mem::take(&mut self.out), key_output: false }
    }

    fn restore_key_strokes(&mut self) -> bool {
        self.key_output = true;
        if !self.last_word_has_vn_mark() {
            self.backs = 0;
            return false;
        }

        self.backs = 0;
        self.change_pos = self.current + 1;

        let mut key_start = self.key_current;
        let mut converted = false;
        while key_start >= 0 && self.key_strokes[key_start as usize].ev.ch_type != CharType::WordBreak {
            if self.key_strokes[key_start as usize].converted {
                converted = true;
            }
            key_start -= 1;
        }
        key_start += 1;

        if !converted {
            self.backs = 0;
            return false;
        }

        while self.current >= 0 && self.b(self.current).form != Form::Empty {
            self.current -= 1;
        }
        self.mark_change(self.current + 1);

        self.key_restoring = true;
        self.out.clear();
        for i in key_start..=self.key_current {
            let key = self.key_strokes[i as usize].ev.key_code;
            self.out.push(u16::from(key as u8));
            let mut ev = self.key_code_to_symbol(key);
            self.key_strokes[i as usize].converted = false;
            self.process_append(&mut ev);
        }
        self.key_restoring = false;
        true
    }

    fn process_word_end(&mut self, ev: &mut KeyEvent) -> i32 {
        let early = !self.options.spell_check || self.single_mode || self.current < 0 || self.key_restoring;
        if !early && self.options.auto_non_vn_restore && self.last_word_is_non_vn() && self.restore_key_strokes() {
            self.key_restored = true;
            self.output_written = true;
        }

        self.current += 1;
        let sym = vn_to_lower(ev.vn_sym);
        let e = self.bm(self.current);
        e.form = Form::Empty;
        e.c1_offset = -1;
        e.c2_offset = -1;
        e.v_offset = -1;
        e.key_code = ev.key_code;
        e.vn_sym = sym;
        e.caps = sym != ev.vn_sym;

        if !early && self.key_restored {
            self.out.push(u16::from(ev.key_code as u8));
            return 1;
        }
        0
    }

    fn last_word_is_non_vn(&self) -> bool {
        if self.current < 0 {
            return false;
        }
        let cur = *self.b(self.current);
        match cur.form {
            Form::NonVn => true,
            Form::Empty | Form::C => false,
            Form::V | Form::Cv => !vi(cur.seq).complete,
            Form::Vc | Form::Cvc => {
                let v_index = self.current - cur.v_offset;
                let vs = self.b(v_index).seq;
                if !vi(vs).complete {
                    return true;
                }
                let cs = cur.seq;
                let c1 = if cur.c1_offset != -1 { self.b(self.current - cur.c1_offset).seq } else { cs_nil };
                if !is_valid_cvc(c1, vs, cs) {
                    return true;
                }
                let tone_pos = (v_index - vi(vs).len + 1) + self.get_tone_position(vs, false);
                let tone = self.b(tone_pos).tone;
                (cs == cs_c || cs == cs_ch || cs == cs_p || cs == cs_t) && (tone == 2 || tone == 3 || tone == 4)
            }
        }
    }

    fn last_word_has_vn_mark(&self) -> bool {
        let mut i = self.current;
        while i >= 0 && self.b(i).form != Form::Empty {
            let e = self.b(i);
            let sym = e.vn_sym;
            if sym != vnl_nonVnChar {
                if is_vn_vowel(sym) && e.tone != 0 {
                    return true;
                }
                if usize::try_from(sym).ok().and_then(|s| STD_VN_ROOT_CHAR.get(s)) != Some(&sym) {
                    return true;
                }
            }
            i -= 1;
        }
        false
    }
}
