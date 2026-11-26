// 1.6.8: mistakes that used to crash the compiler or assemble silently to a wrong address are
// compile errors now: a sprite number that is not a constant 0-7, an inline-asm branch out of
// range, an undefined inline-asm label. `sprite multicolor` is the long form of `sprite multi`.

mod common;
use common::cpu6502::Cpu;
use ultimate_basic::compiler::{compile, CompileOptions};

const OPTS: CompileOptions = CompileOptions { basic_stub: false, explicit: false, basic_print: false };

fn errors(src: &str) -> Vec<String> {
    compile(src, &OPTS).errors
}

#[test]
fn sprite_multicolor_is_the_long_form_of_multi() {
    for src in ["sprite multicolor 3, on\n", "sprite multi 3, on\n"] {
        let res = compile(src, &OPTS);
        assert!(res.errors.is_empty(), "{src}: {:?}", res.errors);
        let mut cpu = Cpu::new(&res.prg);
        cpu.run_until_main_rts(10_000);
        assert_eq!(cpu.mem[0xD01C] & 0x08, 0x08, "{src}");
    }
}

#[test]
fn a_sprite_number_in_a_variable_is_an_error_not_a_crash() {
    let errs = errors("var i = 2\nsprite on i\nsprite multicolor i, on\nsprite 1 + i, 10, 20\n");
    assert!(errs.iter().any(|e| e.contains("sprite on: the sprite number must be a constant")), "{errs:?}");
    assert!(errs.iter().any(|e| e.contains("sprite multicolor:")), "{errs:?}");
    assert!(errs.iter().any(|e| e.starts_with("sprite:")), "{errs:?}");
}

#[test]
fn a_sprite_number_over_7_is_an_error() {
    let errs = errors("sprite color 8, 1\n");
    assert!(errs.iter().any(|e| e.contains("sprite number 8 is out of range")), "{errs:?}");
}

#[test]
fn an_inline_asm_branch_out_of_range_is_an_error() {
    let body = "  NOP\n".repeat(200);
    let errs = errors(&format!("asm {{\n  BEQ far\n{body}far:\n  RTS\n}}\n"));
    assert!(errs.iter().any(|e| e.contains("branch target is 200 bytes away")), "{errs:?}");
    // 127 bytes forward still fits
    let body = "  NOP\n".repeat(127);
    let errs = errors(&format!("asm {{\n  BEQ near\n{body}near:\n  RTS\n}}\n"));
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn an_undefined_inline_asm_label_is_an_error() {
    let errs = errors("asm {\n  JMP nowhere\n}\n");
    assert!(errs.iter().any(|e| e.contains("undefined label")), "{errs:?}");
}
