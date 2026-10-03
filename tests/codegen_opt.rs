// Code-size / code-shape tests for the optimised code paths.
// Behaviour is covered by codegen_semantics.rs; these tests pin the shape of
// the emitted code so that regressions in code quality are noticed.

mod common;
use common::cpu6502::*;

/// Bytes emitted for `snippet` beyond the common `prefix` program.
fn delta(prefix: &str, snippet: &str) -> Vec<u8> {
    // The barrier keeps A-reuse (step 7) from crossing into the snippet, so
    // each test sees the snippet's code in isolation.
    let prefix = &format!("{prefix}\npoke $FB, 0");
    let base = code_of(prefix);
    let full = code_of(&format!("{prefix}\n{snippet}"));
    // both end with RTS; strip the prefix body (without its RTS) and the final RTS
    let body_start = base.len() - 1;
    full[body_start..full.len() - 1].to_vec()
}

// ── 1. Conditions compile to CMP + branch ───────────────────────────────────

#[test]
fn if_eq_const_is_cmp_and_branch() {
    // LDA x; CMP #10; BEQ +3; JMP end; LDA #1; STA z
    let code = delta("var x = 10\nvar z = 0", "if x == 10 then\n z = 1\nend");
    assert_eq!(code.len(), 13, "{code:02X?}");
    assert_eq!(&code[..7], &[0xA5, 0x02, 0xC9, 0x0A, 0xF0, 0x03, 0x4C]);
    assert_eq!(&code[9..], &[0xA9, 0x01, 0x85, 0x04]);
}

#[test]
fn if_var_lt_var_uses_cmp_zp() {
    // LDA x; CMP y; BCC +3; JMP end
    let code = delta("var x = 1\nvar y = 2\nvar z = 0", "if x < y then z = 2 end");
    assert_eq!(&code[..6], &[0xA5, 0x02, 0xC5, 0x04, 0x90, 0x03]);
    assert_eq!(code[6], 0x4C);
}

#[test]
fn if_gt_const_becomes_ge_plus_one() {
    // x > 9 ⇔ x >= 10 → one BCS instead of a BEQ/BCS pair
    let code = delta("var x = 1\nvar z = 0", "if x > 9 then z = 1 end");
    assert_eq!(&code[..6], &[0xA5, 0x02, 0xC9, 0x0A, 0xB0, 0x03]);
}

#[test]
fn if_var_nonzero_has_no_cmp() {
    // `if x` → LDA x; BNE +3; JMP end  (LDA already sets Z)
    let code = delta("var x = 1\nvar z = 0", "if x then z = 1 end");
    assert_eq!(&code[..4], &[0xA5, 0x02, 0xD0, 0x03]);
}

#[test]
fn if_not_inverts_branch() {
    // not (x == 3) → LDA x; CMP #3; BNE +3; JMP end
    let code = delta("var x = 1\nvar z = 0", "if not (x == 3) then z = 1 end");
    assert_eq!(&code[..6], &[0xA5, 0x02, 0xC9, 0x03, 0xD0, 0x03]);
}

#[test]
fn and_of_comparisons_short_circuits_without_materialising() {
    // No ROL / LDA #1 / AND — just two compare+jump pairs.
    let code = delta(
        "var a = 1\nvar b = 2\nvar t = 0",
        "if a > 10 and b < 100 then t = 1 end",
    );
    assert!(!code.contains(&0x2A), "no ROL (bool materialisation) expected: {code:02X?}");
    assert!(!contains(&code, &[0x25]), "no AND zp expected");
    // LDA a; CMP #11; BCS +3; JMP; LDA b; CMP #100; BCC +3; JMP; body
    assert_eq!(&code[..6], &[0xA5, 0x02, 0xC9, 0x0B, 0xB0, 0x03]);
    assert_eq!(&code[9..15], &[0xA5, 0x04, 0xC9, 0x64, 0x90, 0x03]);
}

#[test]
fn or_of_comparisons_short_circuits() {
    let code = delta(
        "var a = 1\nvar b = 2\nvar t = 0",
        "if a == 0 or b == 255 then t = 1 end",
    );
    assert!(!code.contains(&0x2A), "no bool materialisation: {code:02X?}");
    // a == 0 → LDA a; BNE +3 (not the jump); JMP body …
    assert_eq!(&code[..2], &[0xA5, 0x02]);
}

#[test]
fn if_cond_is_much_smaller_than_before() {
    // Old code: 24 bytes of condition for `if x == 10`. New: 9.
    let code = delta("var x = 10\nvar z = 0", "if x == 10 then\n z = 1\nend");
    assert!(code.len() <= 13, "if + body is {} bytes", code.len());
}

#[test]
fn while_loop_branches_back_directly() {
    let code = code_of("var x = 0\nwhile x < 100\n x = x + 1\nend");
    // ends with: LDA x; CMP #100; BCC body; RTS
    let n = code.len();
    assert_eq!(&code[n - 7..n - 2], &[0xA5, 0x02, 0xC9, 0x64, 0x90]);
    assert!((code[n - 2] as i8) < 0);
}

#[test]
fn while_with_long_body_falls_back_to_jmp() {
    // body > 127 bytes → the back branch is out of range → BCS +3; JMP body
    let body = "  poke $C000, x\n".repeat(40);
    let code = code_of(&format!("var x = 0\nwhile x < 3\n{body}  x = x + 1\nend"));
    let n = code.len();
    assert_eq!(&code[n - 10..n - 4], &[0xA5, 0x02, 0xC9, 0x03, 0xB0, 0x03]);
    assert_eq!(code[n - 4], 0x4C);
}

#[test]
fn select_on_variable_compares_in_place() {
    // No copy of x into a permanent ZP slot, each case is LDA x; CMP #n; BEQ +3; JMP next
    let code = delta(
        "var x = 7\nvar s = 0",
        "select x\n case 1:\n  s = 1\n case 7:\n  s = 2\nend",
    );
    assert_eq!(&code[..6], &[0xA5, 0x02, 0xC9, 0x01, 0xF0, 0x03]);
    assert!(!contains(&code, &[0x85, 0x06]), "no STA to a new perm slot");
}

// ── 2. Direct operands for + - and or xor, poke ─────────────────────────────

#[test]
fn add_var_const_is_lda_clc_adc_imm() {
    // z = x + y → LDA x; CLC; ADC y; STA z
    let code = delta("var x = 1\nvar y = 2\nvar z = 0", "z = x + y");
    assert_eq!(code, [0xA5, 0x02, 0x18, 0x65, 0x04, 0x85, 0x06]);
}

#[test]
fn sub_var_const_is_lda_sec_sbc_imm() {
    let code = delta("var x = 1\nvar z = 0", "z = x - 7");
    assert_eq!(code, [0xA5, 0x02, 0x38, 0xE9, 0x07, 0x85, 0x04]);
}

#[test]
fn const_minus_complex_uses_one_scratch_byte() {
    // z = 10 - (x and 3) → LDA x; AND #3; STA t; LDA #10; SEC; SBC t; STA z
    let code = delta("var x = 1\nvar z = 0", "z = 10 - (x and 3)");
    assert_eq!(
        code,
        [0xA5, 0x02, 0x29, 0x03, 0x85, 0x50, 0xA9, 0x0A, 0x38, 0xE5, 0x50, 0x85, 0x04]
    );
}

#[test]
fn commutative_op_with_simple_left_side() {
    // z = 3 + (x and 7) → LDA x; AND #7; CLC; ADC #3
    let code = delta("var x = 1\nvar z = 0", "z = 3 + (x and 7)");
    assert_eq!(code, [0xA5, 0x02, 0x29, 0x07, 0x18, 0x69, 0x03, 0x85, 0x04]);
}

#[test]
fn identity_operations_vanish() {
    for e in ["x + 0", "x - 0", "x or 0", "x xor 0", "x and 255"] {
        let code = delta("var x = 1\nvar z = 0", &format!("z = {e}"));
        assert_eq!(code, [0xA5, 0x02, 0x85, 0x04], "{e}");
    }
}

#[test]
fn poke_const_addr_stores_directly() {
    let code = delta("var x = 1", "poke $D020, x");
    assert_eq!(code, [0xA5, 0x02, 0x8D, 0x20, 0xD0]);
    let code = delta("var x = 1", "poke $FB, x");
    assert_eq!(code, [0xA5, 0x02, 0x85, 0xFB]);
}

#[test]
fn poke_word_var_addr_has_no_scratch_copy() {
    // LDA #5; LDY #0; STA (w),Y
    let code = delta("var w: word = $C000", "poke w, 5");
    assert_eq!(code, [0xA9, 0x05, 0xA0, 0x00, 0x91, 0x02]);
}

// ── 3. Multiply / divide / modulo ───────────────────────────────────────────

/// Cycles spent running `snippet` (after the `prefix` program).
fn cycles(prefix: &str, snippet: &str) -> u64 {
    let base = run_src(prefix).cpu.cycles;
    run_src(&format!("{prefix}\n{snippet}")).cpu.cycles - base
}

#[test]
fn mul_by_power_of_two_is_shifts() {
    let code = delta("var x = 3\nvar z = 0", "z = x * 8");
    assert_eq!(code, [0xA5, 0x02, 0x0A, 0x0A, 0x0A, 0x85, 0x04]);
    // constant on the left works the same
    let code = delta("var x = 3\nvar z = 0", "z = 8 * x");
    assert_eq!(code, [0xA5, 0x02, 0x0A, 0x0A, 0x0A, 0x85, 0x04]);
}

#[test]
fn mul_by_small_constant_is_shift_add() {
    // x * 10 = ((x*2)*2 + x)*2 → LDA x; STA t; ASL; ASL; CLC; ADC t; ASL
    let code = delta("var x = 3\nvar z = 0", "z = x * 10");
    assert_eq!(
        code,
        [0xA5, 0x02, 0x85, 0x50, 0x0A, 0x0A, 0x18, 0x65, 0x50, 0x0A, 0x85, 0x04]
    );
}

#[test]
fn mul_by_zero_and_one() {
    assert_eq!(delta("var x = 3\nvar z = 0", "z = x * 1"), [0xA5, 0x02, 0x85, 0x04]);
    assert_eq!(delta("var x = 3\nvar z = 0", "z = x * 0"), [0xA9, 0x00, 0x85, 0x04]);
}

#[test]
fn mul_var_var_runs_at_most_8_rounds() {
    // The old repeated-addition loop needed ~16 cycles per unit of the
    // multiplier (x * 200 ≈ 3200 cycles). The shift-add loop is bounded.
    let c = cycles("var x = 3\nvar y = 200\nvar z = 0", "z = x * y");
    assert!(c < 250, "x * 200 took {c} cycles");
    let c = cycles("var x = 3\nvar y = 255\nvar z = 0", "z = x * y");
    assert!(c < 300, "x * 255 took {c} cycles");
}

#[test]
fn div_and_mod_by_power_of_two() {
    assert_eq!(delta("var x = 3\nvar z = 0", "z = x / 4"), [0xA5, 0x02, 0x4A, 0x4A, 0x85, 0x04]);
    assert_eq!(delta("var x = 3\nvar z = 0", "z = x mod 16"), [0xA5, 0x02, 0x29, 0x0F, 0x85, 0x04]);
}

#[test]
fn div_is_bounded_regardless_of_quotient() {
    // Old code looped once per unit of the quotient: 255 / 1 → 255 rounds.
    let c = cycles("var x = 255\nvar y = 3\nvar z = 0", "z = x / y");
    assert!(c < 400, "255 / 3 took {c} cycles");
    let c = cycles("var x = 255\nvar y = 3\nvar z = 0", "z = x mod y");
    assert!(c < 400, "255 mod 3 took {c} cycles");
}

#[test]
fn div_by_zero_terminates() {
    // Used to hang forever. Now: quotient 255, remainder = dividend.
    let r = run_src("var x = 77\nvar y = 0\nvar q = x / y\nvar m = x mod y");
    assert_eq!(r.byte("q"), 255);
    assert_eq!(r.byte("m"), 77);
}

// ── 4. Increments ───────────────────────────────────────────────────────────

#[test]
fn byte_plus_minus_one_is_inc_dec() {
    for s in ["x = x + 1", "x = 1 + x", "x += 1"] {
        assert_eq!(delta("var x = 3", s), [0xE6, 0x02], "{s}");
    }
    for s in ["x = x - 1", "x -= 1"] {
        assert_eq!(delta("var x = 3", s), [0xC6, 0x02], "{s}");
    }
    assert_eq!(delta("var x = 3", "x = x + 2"), [0xE6, 0x02, 0xE6, 0x02]);
    assert_eq!(delta("var x = 3", "x -= 2"), [0xC6, 0x02, 0xC6, 0x02]);
    // larger steps stay LDA/CLC/ADC/STA
    assert_eq!(delta("var x = 3", "x += 3"), [0xA5, 0x02, 0x18, 0x69, 0x03, 0x85, 0x02]);
}

#[test]
fn word_plus_minus_one_is_16bit_inc_dec() {
    // INC lo; BNE +2; INC hi
    assert_eq!(delta("var w: word = 0", "w = w + 1"), [0xE6, 0x02, 0xD0, 0x02, 0xE6, 0x03]);
    // LDA lo; BNE +2; DEC hi; DEC lo
    assert_eq!(
        delta("var w: word = 0", "w -= 1"),
        [0xA5, 0x02, 0xD0, 0x02, 0xC6, 0x03, 0xC6, 0x02]
    );
}

#[test]
fn float_plus_one_is_not_an_increment() {
    // f + 1 goes through the float/word path, never INC lo.
    // (The value it computes — +1/256 instead of +1.0 — is a separate,
    // pre-existing issue of the float path.)
    let code = delta("var f: float = 1.5", "f = f + 1");
    assert_ne!(&code[..2], &[0xE6, 0x02]);
    assert!(code.len() > 8);
}

// ── 5. Arrays: absolute,Y instead of a zero-page pointer ────────────────────

#[test]
fn byte_array_store_with_var_index() {
    // a[i] = i → LDY i; LDA i; STA $C000,Y
    let code = delta("var a = array(16)\nvar i = 3", "a[i] = i");
    assert_eq!(code, [0xA4, 0x02, 0xA5, 0x02, 0x99, 0x00, 0xC0]);
}

#[test]
fn byte_array_store_with_const_index() {
    // a[5] = x → LDA x; STA $C005
    let code = delta("var a = array(16)\nvar x = 3", "a[5] = x");
    assert_eq!(code, [0xA5, 0x02, 0x8D, 0x05, 0xC0]);
}

#[test]
fn byte_array_store_complex_value_simple_index() {
    // a[i] = x * 2 → LDA x; ASL; LDY i; STA $C000,Y
    let code = delta("var a = array(16)\nvar i = 3\nvar x = 1", "a[i] = x * 2");
    assert_eq!(code, [0xA5, 0x04, 0x0A, 0xA4, 0x02, 0x99, 0x00, 0xC0]);
}

#[test]
fn byte_array_load_with_var_index() {
    // v = a[i] → LDY i; LDA $C000,Y; STA v
    let code = delta("var a = array(16)\nvar i = 3\nvar v = 0", "v = a[i]");
    assert_eq!(code, [0xA4, 0x02, 0xB9, 0x00, 0xC0, 0x85, 0x04]);
}

#[test]
fn array_increment_in_place() {
    // a[i] = a[i] + 1 → LDY i; LDA $C000,Y; CLC; ADC #1; LDY i; STA $C000,Y
    let code = delta("var a = array(16)\nvar i = 3", "a[i] = a[i] + 1");
    assert!(code.len() <= 14, "{} bytes: {code:02X?}", code.len());
}

#[test]
fn array_index_side_effects_keep_value_first_order() {
    // The value is evaluated before the index; an impure index must not be
    // hoisted above a value that reads what the index changes.
    let r = run_src(
        "var a = array(8)\nvar g = 1\n\
         fn bump()\n g = g + 1\n return g\nend\n\
         a[bump()] = g\n",
    );
    // value g read first (1), then bump() → index 2
    let base = r.res.map.arrays.iter().find(|x| x.name == "a").unwrap().base_addr;
    assert_eq!(r.mem(base + 2), 1);
}

// ── 6. for loops ────────────────────────────────────────────────────────────

#[test]
fn for_const_bounds_step_one() {
    // for i = 0 to 15 / a[i] = i / next:
    //   LDA #0; STA i
    // body: LDY i; LDA i; STA $C000,Y
    //   INC i; BEQ exit; LDA i; CMP #16; BCC body
    // No JMP at all (first iteration is certain), no ZP for limit/step.
    let code = delta("var a = array(16)\nvar i = 0", "for i = 0 to 15\n a[i] = i\nnext");
    assert_eq!(
        code,
        [
            0xA9, 0x00, 0x85, 0x02, // i = 0
            0xA4, 0x02, 0xA5, 0x02, 0x99, 0x00, 0xC0, // body
            0xE6, 0x02, 0xF0, 0x06, // INC i; BEQ exit
            0xA5, 0x02, 0xC9, 0x10, 0x90, 0xEF, // LDA i; CMP #16; BCC body
        ]
    );
}

#[test]
fn for_loop_uses_no_permanent_zp_for_constant_limit() {
    let r = compile_src("var i = 0\nvar n = 0\nfor i = 1 to 10\n n = n + 1\nnext\nvar after = 0");
    let after = r.map.variables.iter().find(|v| v.name == "after").unwrap();
    assert_eq!(after.zp_addr, 0x06, "no ZP bytes reserved for the loop");
}

#[test]
fn for_loop_var_limit_is_snapshotted() {
    // for i = 0 to lim → the limit is copied once (STA zp_to) and compared with CMP zp
    let code = delta("var i = 0\nvar lim = 5\nvar n = 0", "for i = 0 to lim\n n = n + 1\nnext");
    assert!(contains(&code, &[0xA5, 0x04, 0x85]), "limit snapshot: {code:02X?}");
    assert_eq!(code[8], 0x4C, "zero-trip check: JMP test");
}

#[test]
fn for_loop_is_fast() {
    // 16 iterations of an empty-ish loop: ≈ 14 cycles per round of overhead
    let c = cycles("var i = 0\nvar n = 0", "for i = 0 to 15\n n = n + 1\nnext");
    assert!(c < 16 * 25, "{c} cycles");
}

#[test]
fn for_count_down_step_minus_one() {
    // LDA i; BEQ exit; DEC i; LDA i; CMP #1; BCS body
    let code = delta("var i = 0\nvar n = 0", "for i = 10 to 1 step -1\n n = n + 1\nnext");
    assert!(
        contains(&code, &[0xA5, 0x02, 0xF0, 0x08, 0xC6, 0x02, 0xA5, 0x02, 0xC9, 0x01, 0xB0]),
        "{code:02X?}"
    );
}

// ── 7. A-register reuse across statements ───────────────────────────────────

#[test]
fn load_after_store_is_skipped() {
    // z = x + 1 / if z == 5 → …; STA z; CMP #5 (no LDA z)
    let code = delta("var x = 1\nvar z = 0", "z = x + 1\nif z == 5 then x = 0 end");
    assert!(
        contains(&code, &[0x85, 0x04, 0xC9, 0x05]),
        "STA z directly followed by CMP #5: {code:02X?}"
    );
    // y = z + 1 → STA z; CLC; ADC #1
    let code = delta("var x = 1\nvar z = 0\nvar y = 0", "z = x xor 3\ny = z + 1");
    assert!(contains(&code, &[0x85, 0x04, 0x18, 0x69, 0x01]), "{code:02X?}");
}

#[test]
fn load_after_store_kept_when_flags_are_stale() {
    // multiply loop does not end with a flag-setting load of A
    let code = delta("var x = 1\nvar y = 1\nvar z = 0\nvar t = 0", "z = x * y\nif z then t = 1 end");
    assert!(contains(&code, &[0x85, 0x06, 0xA5, 0x06]), "{code:02X?}");
}

#[test]
fn load_after_store_kept_at_loop_top() {
    let code = delta("var x = 1", "x = 2\nloop\n if x == 3 then break end\n inc x\nend");
    assert!(contains(&code, &[0x85, 0x02, 0xA5, 0x02]), "{code:02X?}");
}
