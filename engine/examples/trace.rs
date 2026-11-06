//! Differential-test driver: same script format and output as `tools/oracle/oracle.cpp`.
use govn_engine::{Engine, InputMethod, Options, Output};
use std::io::{BufRead, Write};

fn emit(w: &mut impl Write, o: &Output) {
    let hex: String = o.text.iter().map(|u| format!("{u:04x}")).collect();
    writeln!(w, "{} {} {}", o.backs, u8::from(o.key_output), hex).unwrap();
}

fn main() {
    let mut e = Engine::new();
    let stdin = std::io::stdin();
    let mut w = std::io::BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let mut toks = line.split_whitespace().peekable();
        if toks.peek() == Some(&"CFG") {
            let v: Vec<i32> = toks.skip(1).map(|t| t.parse().unwrap()).collect();
            // numbering follows UkInputMethod in keycons.h
            e.set_input_method(match v[0] {
                0 => InputMethod::Telex,
                1 => InputMethod::Vni,
                5 => InputMethod::SimpleTelex,
                6 => InputMethod::SimpleTelex2,
                n => panic!("unsupported method {n}"),
            });
            e.options = Options {
                free_marking: v[1] != 0,
                modern_style: v[2] != 0,
                spell_check: v[3] != 0,
                auto_non_vn_restore: v[4] != 0,
            };
            writeln!(w, "CFG").unwrap();
            continue;
        }
        for t in toks {
            match t {
                "RST" => {
                    e.reset();
                    writeln!(w, "R").unwrap();
                }
                "CAPS0" => {
                    e.caps_lock = false;
                    writeln!(w, "C").unwrap();
                }
                "CAPS1" => {
                    e.caps_lock = true;
                    writeln!(w, "C").unwrap();
                }
                "BS" => emit(&mut w, &e.process_backspace()),
                _ => emit(&mut w, &e.process(u32::from_str_radix(t, 16).unwrap())),
            }
        }
    }
}
