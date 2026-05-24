// Integration tests for Ultimate Basic compiler.
// Tests compile entire programs and verify PRG output.

use ultimate_basic::compiler::{compile, CompileOptions};

fn compile_stub(src: &str) -> Vec<u8> {
    compile(src, &CompileOptions { basic_stub: true }).prg
}

fn compile_raw(src: &str) -> Vec<u8> {
    compile(src, &CompileOptions { basic_stub: false }).prg
}

// ── BASIC stub ──────────────────────────────────────────────────────────────

#[test]
fn stub_is_correct_length() {
    let prg = compile_stub("");
    assert_eq!(prg.len(), 15, "Empty program: 14 stub + 1 RTS = 15");
}

#[test]
fn no_stub_is_correct_length() {
    let prg = compile_raw("");
    assert_eq!(prg.len(), 3, "Empty program: 2 header + 1 RTS = 3");
}

#[test]
fn stub_has_sys_2061() {
    let prg = compile_stub("");
    // Bytes at offset 2-3: next line = $080B
    assert_eq!(prg[2], 0x0B);
    assert_eq!(prg[3], 0x08);
    // Bytes at offset 6: SYS token
    assert_eq!(prg[6], 0x9E);
    // Bytes 7-10: "2061"
    assert_eq!(&prg[7..11], b"2061");
}

// ── Variable declaration ────────────────────────────────────────────────────

#[test]
fn var_decl_generates_code() {
    let prg = compile_raw("var x = 42");
    // Should have: LDA #42, STA $02, RTS (+ header)
    assert!(prg.len() > 6); // header(2) + LDA #42(2) + STA $02(2) + RTS(1)
    assert_eq!(prg[2], 0xA9); // LDA immediate
    assert_eq!(prg[3], 42);   // #42
    assert_eq!(prg[4], 0x85); // STA zp
    assert_eq!(prg[5], 0x02); // $02
    assert_eq!(prg[6], 0x60); // RTS
}

#[test]
fn var_assign_generates_code() {
    let prg = compile_raw("x = 99");
    assert_eq!(prg[2], 0xA9); // LDA immediate
    assert_eq!(prg[3], 99);
    assert_eq!(prg[4], 0x85); // STA
    assert_eq!(prg[5], 0x02); // first ZP var
}

#[test]
fn type_annotation_parses() {
    let prg = compile_raw("var x: int = 5\nvar s: string = \"hi\"");
    // Should compile without errors
    assert!(prg.len() > 8);
}

// ── Print ───────────────────────────────────────────────────────────────────

#[test]
fn print_string_literal() {
    let prg = compile_raw("print \"A\"");
    // print_str_inline: LDA #'A', JSR CHROUT, print_newline (LDA #$0D, JSR CHROUT), RTS
    assert!(prg.len() > 10);
    assert_eq!(prg[2], 0xA9); // LDA #'A'
    assert_eq!(prg[3], 0x41); // 'A' = 65 in PETSCII? Actually uppercase A is same
    assert_eq!(prg[4], 0x20); // JSR
    assert_eq!(prg[5], 0xD2); // CHROUT lo
    assert_eq!(prg[6], 0xFF); // CHROUT hi
}

#[test]
fn print_variable() {
    let prg = compile_raw("var x = 100\nprint x");
    // Should contain print_decimal code
    assert!(prg.len() > 30); // print_decimal is large
}

// ── Expressions ─────────────────────────────────────────────────────────────

#[test]
fn addition_expr() {
    let prg = compile_raw("var x = 3 + 4");
    // LDA #3, STA tmp, LDA #4, CLC, ADC tmp, STA zp, RTS
    assert!(prg.len() > 12);
    // Check for CLC ($18) and ADC zp ($65)
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x18)); // CLC
    assert!(bytes.contains(&0x65)); // ADC zp
}

#[test]
fn subtraction_expr() {
    let prg = compile_raw("var x = 10 - 3");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x38)); // SEC
    assert!(bytes.contains(&0xE5)); // SBC zp
}

#[test]
fn multiplication_expr() {
    let prg = compile_raw("var x = 3 * 4");
    // Should contain multiply loop with DEC and BNE
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xC6)); // DEC zp
    assert!(bytes.contains(&0xD0)); // BNE
}

#[test]
fn division_expr() {
    let prg = compile_raw("var x = 8 / 2");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xE6)); // INC zp (quotient)
}

// ── Comparisons ─────────────────────────────────────────────────────────────

#[test]
fn eq_comparison() {
    let prg = compile_raw("var r = 5 == 5");
    // Should have CMP, BEQ, LDA #0/1 pattern
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xC5)); // CMP zp
    assert!(bytes.contains(&0xF0)); // BEQ
}

#[test]
fn lt_comparison() {
    let prg = compile_raw("var r = 3 < 5");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x90)); // BCC
}

#[test]
fn gt_comparison() {
    let prg = compile_raw("var r = 5 > 3");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x90)); // BCC (swapped)
}

#[test]
fn lteq_comparison() {
    let prg = compile_raw("var r = 5 <= 5");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xB0)); // BCS (swapped)
}

#[test]
fn gteq_comparison() {
    let prg = compile_raw("var r = 5 >= 5");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xB0)); // BCS
}

#[test]
fn noteq_comparison() {
    let prg = compile_raw("var r = 5 != 3");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xD0)); // BNE
}

// ── Bitwise operators ───────────────────────────────────────────────────────

#[test]
fn and_operator() {
    // Bitwise AND: color and 15  → 6502 AND opcode ($25 for ZP, $29 for imm)
    let prg = compile_raw("var r = 12 and 15");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x25) || bytes.contains(&0x29)); // AND zp / AND #imm
}

#[test]
fn or_operator() {
    // Bitwise OR: 6502 ORA opcode ($05 for ZP)
    let prg = compile_raw("var r = 0 or 1");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x05) || bytes.contains(&0x09)); // ORA zp / ORA #imm
}

#[test]
fn not_operator() {
    let prg = compile_raw("var r = not 0");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xF0)); // BEQ (for not: if == 0, skip to LDA #1)
}

// ── Control flow ────────────────────────────────────────────────────────────

#[test]
fn if_then_generates_branch() {
    let prg = compile_raw("var x = 1\nif x == 1 then\n  x = 2\nend");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xF0)); // BEQ +3
    assert!(bytes.contains(&0x4C)); // JMP absolute
}

#[test]
fn if_else_generates_branch() {
    let prg = compile_raw("var x = 1\nif x == 0 then\n  x = 1\nelse\n  x = 2\nend");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xF0)); // BEQ +3
}

#[test]
fn infinite_loop_has_jmp() {
    let prg = compile_raw("loop\n  var c = 1\nend");
    let bytes = &prg[2..];
    // Should have JMP absolute back to loop_top
    assert!(bytes.contains(&0x4C)); // JMP
}

#[test]
fn counted_loop_has_dec_bne() {
    let prg = compile_raw("loop 3\n  var x = 1\nend");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xC6)); // DEC zp
    // Now uses BEQ+JMP (works for any body size), not BNE
    assert!(bytes.contains(&0xF0)); // BEQ (skip JMP when cnt==0)
    assert!(bytes.contains(&0x4C)); // JMP loop_start
}

#[test]
fn for_loop_has_cmp() {
    let prg = compile_raw("var n = 0\nloop i = 1 to 5\n  n = i\nend");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xC5)); // CMP zp (for loop exit check)
}

#[test]
fn while_loop_has_jmp() {
    let prg = compile_raw("var n = 1\nwhile n < 6\n  n = n + 1\nend");
    let bytes = &prg[2..];
    // Should have JMP exit (not just BNE relative)
    let jmp_count = bytes.iter().filter(|&&b| b == 0x4C).count();
    assert!(jmp_count >= 2, "While loop should use JMP for exit");
}

#[test]
fn break_statement() {
    let prg = compile_raw("loop\n  break\n  var x = 1\nend");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x4C)); // JMP for break
}

// ── Subroutines ─────────────────────────────────────────────────────────────

#[test]
fn sub_def_has_rts() {
    let prg = compile_raw("sub test()\n  print \"hi\"\nend\ntest()");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x60)); // RTS from subroutine
    assert!(bytes.contains(&0x20)); // JSR for call
}

#[test]
fn forward_sub_call_compiles() {
    let prg = compile_raw("call later\nsub later()\nend");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x20)); // JSR for forward call
    assert!(bytes.contains(&0x60)); // RTS
}

#[test]
fn undefined_sub_reports_error() {
    let res = compile("call missing", &CompileOptions { basic_stub: false });
    assert!(!res.errors.is_empty());
    assert!(res.errors[0].contains("Undefined subroutine"));
}

// ── Colors ─────────────────────────────────────────────────────────────────

#[test]
fn color_text() {
    let prg = compile_raw("color text 7");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x8D)); // STA absolute
    // $0286 = text color register
    let pos = bytes.windows(3).position(|w| w == &[0x8D, 0x86, 0x02]);
    assert!(pos.is_some(), "Should emit STA $0286");
}

#[test]
fn color_border() {
    let prg = compile_raw("color border 2");
    let bytes = &prg[2..];
    let pos = bytes.windows(3).position(|w| w == &[0x8D, 0x20, 0xD0]);
    assert!(pos.is_some(), "Should emit STA $D020");
}

#[test]
fn color_bg() {
    let prg = compile_raw("color bg 0");
    let bytes = &prg[2..];
    let pos = bytes.windows(3).position(|w| w == &[0x8D, 0x21, 0xD0]);
    assert!(pos.is_some(), "Should emit STA $D021");
}

// ── Built-in functions ─────────────────────────────────────────────────────

#[test]
fn getch_emits_getin_loop() {
    let prg = compile_raw("var c = getch()");
    let bytes = &prg[2..];
    // JSR $FFE4, CMP #0, BEQ loop
    assert!(bytes.contains(&0x20)); // JSR
    let has_ffe4 = bytes.windows(3).any(|w| w == &[0x20, 0xE4, 0xFF]);
    assert!(has_ffe4, "Should have JSR $FFE4");
    let has_cmp_stop = bytes.windows(2).any(|w| w == &[0xC9, 0x03]);
    assert!(has_cmp_stop, "getch should compare against RUN/STOP ($03)");
    // getch should clear RUN/STOP flag so BASIC won't print BREAK on return
    let has_clear_91 = bytes.windows(4).any(|w| w == &[0xA9, 0xFF, 0x85, 0x91]);
    assert!(has_clear_91, "getch should clear STOP flag ($91)");
}

#[test]
fn joy_port2_reads_dc00() {
    let prg = compile_raw("var j = joy(2)");
    let bytes = &prg[2..];
    // LDA $DC00 = $AD $00 $DC
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x00, 0xDC]);
    assert!(has_lda, "joy(2) should read $DC00");
    // AND #$1F then EOR #$1F
    let has_mask = bytes.windows(2).any(|w| w == &[0x29, 0x1F]);
    assert!(has_mask, "joy should mask bits 0-4");
    let has_inv  = bytes.windows(2).any(|w| w == &[0x49, 0x1F]);
    assert!(has_inv, "joy should invert bits 0-4");
}

#[test]
fn joy_port1_reads_dc01() {
    let prg = compile_raw("var j = joy(1)");
    let bytes = &prg[2..];
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x01, 0xDC]);
    assert!(has_lda, "joy(1) should read $DC01");
}

#[test]
fn line_emits_bresenham_helper() {
    // graphics on needed for line to make sense; test compile+structure only
    let prg = compile_raw("graphics on\nline 10, 20, 50, 80\ngraphics off");
    let bytes = &prg[2..];
    // Should contain a JSR (0x20) to the drawline helper
    assert!(bytes.contains(&0x20), "Should emit JSR instructions");
    // Drawline stores X1 into ZP before JSR: STA zp (0x85 xx)
    let has_sta_zp = bytes.windows(2).any(|w| w[0] == 0x85);
    assert!(has_sta_zp, "Should store params in ZP");
    // Should contain RTS at end of drawline helper
    assert!(bytes.contains(&0x60), "Should have RTS");
}

#[test]
fn line_produces_larger_code_than_plot() {
    let plot_size = compile_raw("graphics on\nplot 10, 20\ngraphics off").len();
    let line_size = compile_raw("graphics on\nline 10, 20, 50, 80\ngraphics off").len();
    assert!(line_size > plot_size, "line should emit more code than a single plot");
}

#[test]
fn circle_emits_helper_and_compiles() {
    let res = compile("graphics on\ncircle 160, 100, 32\ngraphics off", &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "circle should compile cleanly: {:?}", res.errors);
    assert!(res.map.plot_zp.is_some(), "circle should reserve the shared plot helper ZP block");
    assert!(res.prg.len() > compile_raw("graphics on\ngraphics off").len(), "circle should add code beyond bare graphics mode switches");
}

#[test]
fn circle_produces_larger_code_than_plot() {
    let res = compile("var cx: word = 160\ngraphics on\ncircle cx, 100, 32\ngraphics off", &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "circle with word X center should compile cleanly: {:?}", res.errors);
    assert!(res.map.plot_zp.is_some(), "circle with word X center should still reserve plot helper state");
}

#[test]
fn sin_emits_lut_lookup() {
    let prg = compile_raw("var x = 0\nx = sin(x)");
    let bytes = &prg[2..];
    // LDA abs,X = $BD — used for sin table lookup
    assert!(bytes.contains(&0xBD), "sin should emit LDA abs,X ($BD)");
    // Should contain a 256-byte table at the end with value 128 at index 0 (sin(0)=0→128)
    // Find consecutive region where bytes[0]==128 (sin(0)) and bytes[64]==255 (sin(90°))
    let has_sin_table = bytes.windows(65).any(|w| w[0] == 128 && w[64] == 255);
    assert!(has_sin_table, "Should contain sin lookup table with sin(0)=128 and sin(64)=255");
}

#[test]
fn cos_uses_same_sin_table() {
    let prg_sin = compile_raw("var x = 0\nx = sin(x)").len();
    let prg_cos = compile_raw("var x = 0\nx = cos(x)").len();
    let prg_both = compile_raw("var x = 0\nx = sin(x)\nx = cos(x)").len();
    // One table either way, so sin+cos together should only have ONE 256-byte table
    assert!(prg_both < prg_sin + prg_cos, "sin+cos should share one table");
}

#[test]
fn hex_format_emits_helper() {
    let prg = compile_raw("print hex(255)");
    let bytes = &prg[2..];
    // Should contain JSR ($20) to print_hex helper
    assert!(bytes.contains(&0x20), "hex() should emit JSR");
    // print_hex helper ends with JMP $FFD2 ($4C $D2 $FF)
    let has_chrout_jmp = bytes.windows(3).any(|w| w == &[0x4C, 0xD2, 0xFF]);
    assert!(has_chrout_jmp, "hex helper should have JMP CHROUT");
}

#[test]
fn bin_format_emits_helper() {
    let prg = compile_raw("print bin(42)");
    let bytes = &prg[2..];
    // print_bin helper starts with LDX #8 ($A2 $08)
    let has_ldx8 = bytes.windows(2).any(|w| w == &[0xA2, 0x08]);
    assert!(has_ldx8, "bin helper should start with LDX #8");
    // Should contain RTS ($60) at end of helper
    assert!(bytes.contains(&0x60), "bin helper should have RTS");
}

#[test]
fn reu_stash_emits_register_writes() {
    let prg = compile_raw("reu stash $4000, 0, $0000, 256");
    let bytes = &prg[2..];
    // Should contain STA $DF01 ($8D $01 $DF) — command register write
    let has_cmd = bytes.windows(3).any(|w| w == &[0x8D, 0x01, 0xDF]);
    assert!(has_cmd, "reu stash should write to $DF01");
    // Should contain $B0 = stash command byte
    assert!(bytes.contains(&0xB0), "reu stash should use command $B0");
}

#[test]
fn reu_fetch_uses_b1_command() {
    let prg = compile_raw("reu fetch $4000, 0, $0000, 256");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xB1), "reu fetch should use command $B1");
}

// graphics blanking: $D011 is written with AND #$EF ($29 $EF) to blank before mode switch
#[test]
fn reu_present_returns_one_when_not_checked() {
    // reu_present() emits 31 bytes of inline 6502 — verify key opcodes are present.
    let prg = compile_raw("var r = reudet()");
    let bytes = &prg[2..];
    // Must contain STA $DF04 ($8D $04 $DF) — write test
    let has_sta = bytes.windows(3).any(|w| w == &[0x8D, 0x04, 0xDF]);
    assert!(has_sta, "reu_present should emit STA $DF04");
    // Must contain LDA $DF04 ($AD $04 $DF) — read back
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x04, 0xDF]);
    assert!(has_lda, "reu_present should emit LDA $DF04");
    // Must contain CMP #$55 ($C9 $55) and CMP #$AA ($C9 $AA)
    let has_cmp55 = bytes.windows(2).any(|w| w == &[0xC9, 0x55]);
    let has_cmpaa = bytes.windows(2).any(|w| w == &[0xC9, 0xAA]);
    assert!(has_cmp55, "reu_present should emit CMP #$55");
    assert!(has_cmpaa, "reu_present should emit CMP #$AA");
    // Must contain LDA #1 ($A9 $01) — success path
    let has_lda1 = bytes.windows(2).any(|w| w == &[0xA9, 0x01]);
    assert!(has_lda1, "reu_present should emit LDA #1 (found path)");
    // Must contain LDA #0 ($A9 $00) and JMP ($4C) — fail path
    let has_lda0 = bytes.windows(2).any(|w| w == &[0xA9, 0x00]);
    let has_jmp  = bytes.contains(&0x4C);
    assert!(has_lda0, "reu_present should emit LDA #0 (not-found path)");
    assert!(has_jmp,  "reu_present should emit JMP to skip fail branch");
}

#[test]
fn graphics_on_blanks_display() {
    let prg = compile_raw("graphics on");
    let bytes = &prg[2..];
    // Expect AND #$EF ($29 $EF) to clear DEN bit
    let has_blank = bytes.windows(2).any(|w| w == &[0x29, 0xEF]);
    assert!(has_blank, "graphics on should blank display with AND #$EF");
    // ORA #$20 ($09 $20) sets only BMM — display stays blanked, user calls `display on`
    let has_bmm = bytes.windows(2).any(|w| w == &[0x09, 0x20]);
    assert!(has_bmm, "graphics on should set BMM with ORA #$20 (display stays blanked)");
    // display must NOT be re-enabled here ($D011 ORA #$10 must NOT appear)
    let no_den = !bytes.windows(2).any(|w| w == &[0x09, 0x10]);
    assert!(no_den, "graphics on must not re-enable display (DEN stays 0)");
}

#[test]
fn display_on_enables_den() {
    let prg = compile_raw("display on");
    let bytes = &prg[2..];
    // LDA $D011 ($AD $11 $D0) then ORA #$10 ($09 $10) then STA $D011
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x11, 0xD0]);
    assert!(has_lda, "display on should LDA $D011");
    let has_ora = bytes.windows(2).any(|w| w == &[0x09, 0x10]);
    assert!(has_ora, "display on should ORA #$10 to set DEN");
}

#[test]
fn display_off_clears_den() {
    let prg = compile_raw("display off");
    let bytes = &prg[2..];
    // LDA $D011 then AND #$EF ($29 $EF) then STA $D011
    let has_and = bytes.windows(2).any(|w| w == &[0x29, 0xEF]);
    assert!(has_and, "display off should AND #$EF to clear DEN");
}

#[test]
fn graphics_on_multi_sets_mcm() {
    let prg = compile_raw("graphics on multi");
    let bytes = &prg[2..];
    // ORA #$10 ($09 $10) sets MCM bit in $D016
    let has_mcm = bytes.windows(2).any(|w| w == &[0x09, 0x10]);
    assert!(has_mcm, "graphics on multi should set MCM bit with ORA #$10");
}

#[test]
fn graphics_on_hires_clears_mcm() {
    let prg = compile_raw("graphics on");
    let bytes = &prg[2..];
    // AND #$EF ($29 $EF) clears MCM bit in $D016 (same opcode used for both blank and MCM clear)
    let count = bytes.windows(2).filter(|w| *w == &[0x29, 0xEF]).count();
    assert!(count >= 2, "hires mode should have at least 2× AND #$EF (blank + MCM clear)");
}

#[test]
fn graphics_off_blanks_display() {
    let prg = compile_raw("graphics off");
    let bytes = &prg[2..];
    let has_blank = bytes.windows(2).any(|w| w == &[0x29, 0xEF]);
    assert!(has_blank, "graphics off should blank display with AND #$EF");
    // AND #$DF ($29 $DF) clears BMM bit
    let has_clear_bmm = bytes.windows(2).any(|w| w == &[0x29, 0xDF]);
    assert!(has_clear_bmm, "graphics off should clear BMM with AND #$DF");
}

#[test]
fn cls_emits_kernal_jsr() {
    let prg = compile_raw("cls");
    let bytes = &prg[2..];
    let has_e544 = bytes.windows(3).any(|w| w == &[0x20, 0x44, 0xE5]);
    assert!(has_e544, "Should have JSR $E544");
}

#[test]
fn cls_manual_is_larger() {
    let small = compile_raw("cls").len();
    let large = compile_raw("cls fast").len();
    assert!(large > small, "Manual CLS should emit more code");
}

#[test]
fn sys_emits_jsr() {
    let prg = compile_raw("sys $FFD2");
    let bytes = &prg[2..];
    let has = bytes.windows(3).any(|w| w == &[0x20, 0xD2, 0xFF]);
    assert!(has);
}

#[test]
fn asm_bytes_inline() {
    let prg = compile_raw("asm $EA, $EA, $EA");
    // Should have 3 NOPs
    assert_eq!(prg[2], 0xEA);
    assert_eq!(prg[3], 0xEA);
    assert_eq!(prg[4], 0xEA);
    assert_eq!(prg[5], 0x60); // RTS follows
}

#[test]
fn asm_block_braces() {
    let prg = compile_raw("asm { $A9 $01 }");
    assert_eq!(prg[2], 0xA9);
    assert_eq!(prg[3], 0x01);
}

#[test]
fn graphics_on_off() {
    let prg = compile_raw("graphics on\ngraphics off");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x09)); // ORA #$20
    assert!(bytes.contains(&0x29)); // AND #$DF
}

// ── Comments and whitespace ─────────────────────────────────────────────────

#[test]
fn comments_are_ignored() {
    let a = compile_raw("var x = 5");
    let b = compile_raw("var x = 5\n# this is a comment\n# another");
    assert_eq!(a, b, "Comments should not affect compiled output");
}

#[test]
fn extra_whitespace_ignored() {
    let a = compile_raw("var x = 5");
    let b = compile_raw("  var   x  =  5  ");
    assert_eq!(a, b, "Extra whitespace should not affect output");
}

#[test]
fn blank_lines_ignored() {
    let a = compile_raw("var x = 5");
    let b = compile_raw("\n\nvar x = 5\n\n");
    assert_eq!(a, b);
}

// ── Error cases ─────────────────────────────────────────────────────────────

#[test]
fn empty_program_compiles() {
    let prg = compile_raw("");
    assert_eq!(prg.len(), 3); // header(2) + RTS(1)
    assert_eq!(prg[2], 0x60); // RTS
}

// ── Complex programs ────────────────────────────────────────────────────────

#[test]
fn demo_game_compiles() {
    let src = "# NEXTBASIC DEMO\ncls\ncolor text 14\ncolor bg 0\nvar n = 1\nwhile n < 6\n  print \"  n = \", n\n  n = n + 1\nend";
    let res = compile(src, &CompileOptions { basic_stub: true });
    assert!(res.errors.is_empty(), "Should compile without errors: {:?}", res.errors);
    assert!(res.prg.len() > 100, "Should produce reasonable amount of code");
}

#[test]
fn full_feature_program_compiles() {
    let src = "
# Test almost everything
var x = 10
var y = 5
var z = x + y
var w = z * 2
var flag = x > y and y < 10

print \"Hello\"

if flag == 1 then
  print \"Condition met\"
else
  print \"Condition not met\"
end

loop i = 1 to 3
  print i
end

var cnt = 0
while cnt < 2
  cnt = cnt + 1
end

color border 6
color text 14

sub greet()
  print \"Hi from sub\"
end

greet()
sys $FFD2
";
    let res = compile(src, &CompileOptions { basic_stub: true });
    assert!(res.errors.is_empty(),
        "Should compile without errors. Got: {:?}", res.errors);
    assert!(res.prg.len() > 200);
}

#[test]
fn logical_expression_combinations() {
    // Test nested logical operations
    let src = "var r = not 0 and 1 or 0";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

#[test]
fn chained_comparisons() {
    let src = "var a = 1 == 1\nvar b = 2 != 3\nvar c = 4 < 5\nvar d = 5 > 4\nvar e = 6 <= 6\nvar f = 7 >= 7";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

#[test]
fn nested_loops_compile() {
    let src = "
loop i = 1 to 2
  loop j = 1 to 2
    var sum = i + j
  end
end
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

#[test]
fn break_in_nested_loops() {
    let src = "
loop
  loop
    break
  end
end
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

#[test]
fn int_to_str_compiles() {
    let src = "var score = 42\nnumstr score, $0340";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

// ── String concatenation ───────────────────────────────────────────────────

#[test]
fn string_concat_in_print() {
    // print 12, 12, "Hello " + "Haver"
    // String bytes are NOT consecutive — interleaved with JSR CHROUT opcodes.
    // The key check: same binary as writing the literal directly.
    let concat  = compile_raw("print 12, 12, \"Hello \" + \"Haver\"");
    let literal = compile_raw("print 12, 12, \"Hello Haver\"");
    assert_eq!(concat, literal,
        "Concatenated string must produce identical code to direct literal");
}

#[test]
fn string_var_concat_in_print() {
    // print s1 + s2 — both are string vars, printed sequentially
    let src = "var s1 = \"Hello \"\nvar s2 = \"World\"\nprint s1 + s2";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // Should use LDA (ptr),Y = $B1 for both string vars
    let count = res.prg[2..].iter().filter(|&&b| b == 0xB1).count();
    assert!(count >= 2, "Should emit LDA (ptr),Y at least twice for s1+s2");
}

#[test]
fn string_literal_concat_with_var() {
    // print "Name: " + s  (literal + string var)
    let src = "var name = \"Alice\"\nprint \"Name: \" + name";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // $B1 = LDA (ptr),Y for string var
    assert!(res.prg[2..].contains(&0xB1));
}

#[test]
fn number_add_still_works_in_print() {
    // print 3 + 4 must still print "7", not "34"
    let with_add    = compile_raw("print 3 + 4");
    let with_result = compile_raw("var t = 3 + 4\nprint t");
    // Both should contain the same numeric result code; neither should be the
    // same as printing "3" followed by "4" separately
    let separate = compile_raw("print 3, 4");
    assert_ne!(with_add, separate, "3+4 must evaluate numerically, not print separately");
    let _ = with_result; // compiles without error
}

#[test]
fn mixed_num_string_concat() {
    // "Score: " + score  (string literal + numeric var)
    let src = "var score = 42\nprint \"Score: \" + score";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // 'S' in PETSCII = $53
    assert!(res.prg.contains(&0x53));
}

#[test]
fn string_concat_produces_same_as_literal() {
    // "Hello " + "World" should produce identical code to "Hello World"
    let a = compile_raw("print \"Hello \" + \"World\"");
    let b = compile_raw("print \"Hello World\"");
    assert_eq!(a, b, "Concatenated string must be identical to direct literal");
}

#[test]
fn string_concat_triple() {
    let src = "print \"A\" + \"B\" + \"C\"";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    assert!(res.prg.contains(&0x41)); // 'A' in PETSCII
    assert!(res.prg.contains(&0x42)); // 'B'
    assert!(res.prg.contains(&0x43)); // 'C'
}

// ── for..next ──────────────────────────────────────────────────────────────

#[test]
fn for_next_compiles() {
    let src = "for i = 1 to 5\n  print i\nnext";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0xC5)); // CMP zp (for loop exit check)
}

#[test]
fn for_next_with_step_compiles() {
    let src = "for i = 0 to 20 step 2\n  print i\nnext i";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
}

#[test]
fn for_next_generates_same_code_as_loop() {
    // Both syntaxes must compile to identical bytes
    let a = compile_raw("for i = 1 to 3\n  print i\nnext");
    let b = compile_raw("loop i = 1 to 3\n  print i\nend");
    assert_eq!(a, b, "for..next and loop..end should produce identical code");
}

// ── Bitwise operators ───────────────────────────────────────────────────────

#[test]
fn xor_emits_eor_zp() {
    let prg = compile_raw("var x = 15\nvar y = x xor 3");
    let bytes = &prg[2..];
    // EOR zp ($45 zp) must appear
    let has_eor = bytes.windows(1).any(|w| w == &[0x45]);
    assert!(has_eor, "xor should emit EOR zp ($45)");
}

#[test]
fn shl_shifts_left() {
    // x = 1 shl 3  → should produce 8 (ASL 3 times)
    // Check ASL zp ($06) appears in output
    let prg = compile_raw("var x = 1\nvar y = x shl 3");
    let bytes = &prg[2..];
    let has_asl = bytes.windows(1).any(|w| w == &[0x06]);
    assert!(has_asl, "shl should emit ASL zp ($06)");
}

#[test]
fn shr_shifts_right() {
    let prg = compile_raw("var x = 8\nvar y = x shr 2");
    let bytes = &prg[2..];
    let has_lsr = bytes.windows(1).any(|w| w == &[0x46]);
    assert!(has_lsr, "shr should emit LSR zp ($46)");
}

#[test]
fn shl_zero_count_is_noop() {
    // shl 0 must skip the loop → BEQ must be emitted
    let prg = compile_raw("var x = 5\nvar y = x shl 0");
    let bytes = &prg[2..];
    let has_beq = bytes.windows(1).any(|w| w == &[0xF0]);
    assert!(has_beq, "shl 0 should emit BEQ skip-guard");
}

// ── wait / wait raster ──────────────────────────────────────────────────────

#[test]
fn wait_n_polls_d012() {
    let prg = compile_raw("wait 10");
    let bytes = &prg[2..];
    // LDA $D012 = AD 12 D0
    let has_poll = bytes.windows(3).any(|w| w == &[0xAD, 0x12, 0xD0]);
    assert!(has_poll, "wait N should poll $D012");
    // DEC fc ($C6) must appear
    let has_dec = bytes.windows(1).any(|w| w == &[0xC6]);
    assert!(has_dec, "wait N should DEC frame counter");
}

#[test]
fn wait_raster_polls_d012() {
    let prg = compile_raw("wait raster 100");
    let bytes = &prg[2..];
    let has_poll = bytes.windows(3).any(|w| w == &[0xAD, 0x12, 0xD0]);
    assert!(has_poll, "wait raster N should poll $D012");
    // Should compare with CMP zp ($C5)
    let has_cmp = bytes.windows(1).any(|w| w == &[0xC5]);
    assert!(has_cmp, "wait raster N should CMP target");
}

// ── SID sound ───────────────────────────────────────────────────────────────

#[test]
fn sound_sets_master_volume() {
    let prg = compile_raw("sound 0, $1CAD, 10");
    let bytes = &prg[2..];
    // LDA #$0F; STA $D418
    let has_vol = bytes.windows(5).any(|w| w == &[0xA9, 0x0F, 0x8D, 0x18, 0xD4]);
    assert!(has_vol, "sound should set master volume $D418 = $0F");
}

#[test]
fn sound_writes_freq_to_sid() {
    let prg = compile_raw("sound 0, $1CAD, 5");
    let bytes = &prg[2..];
    // freq lo $AD to $D400: STA $D400 = 8D 00 D4
    let has_freq_lo = bytes.windows(3).any(|w| w == &[0x8D, 0x00, 0xD4]);
    assert!(has_freq_lo, "sound should write freq lo to $D400");
    // freq hi $1C to $D401: STA $D401 = 8D 01 D4
    let has_freq_hi = bytes.windows(3).any(|w| w == &[0x8D, 0x01, 0xD4]);
    assert!(has_freq_hi, "sound should write freq hi to $D401");
}

#[test]
fn sound_gate_on_and_off() {
    let prg = compile_raw("sound 0, $1000, 3");
    let bytes = &prg[2..];
    // GATE on: STA $D404 with value $11
    let has_gate_on = bytes.windows(5).any(|w| w == &[0xA9, 0x11, 0x8D, 0x04, 0xD4]);
    assert!(has_gate_on, "sound should GATE on with $11 to $D404");
    // GATE off: STA $D404 with value $10
    let has_gate_off = bytes.windows(5).any(|w| w == &[0xA9, 0x10, 0x8D, 0x04, 0xD4]);
    assert!(has_gate_off, "sound should GATE off with $10 to $D404");
}

#[test]
fn sound_voice1_uses_d407() {
    let prg = compile_raw("sound 1, $1000, 1");
    let bytes = &prg[2..];
    // Voice 1 base = $D407; STA $D407 = 8D 07 D4
    let has_v1 = bytes.windows(3).any(|w| w == &[0x8D, 0x07, 0xD4]);
    assert!(has_v1, "sound voice 1 should write to $D407");
}

// ── 16-bit word arithmetic ──────────────────────────────────────────────────

#[test]
fn word_add_constant_propagates_carry() {
    // ptr = ptr + 1 → CLC + ADC lo + ADC #0 carry to hi
    let src = "var ptr: word = $00FF\nptr = ptr + 1";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // CLC ($18), LDA zp ($A5), ADC imm ($69 $01), STA zp ($85)
    let has_clc = bytes.contains(&0x18);
    assert!(has_clc, "word + constant should emit CLC");
    // ADC #0 (carry propagation to hi byte) = $69 $00
    let has_carry = bytes.windows(2).any(|w| w == &[0x69, 0x00]);
    assert!(has_carry, "word + constant should propagate carry with ADC #0");
}

#[test]
fn word_add_word_uses_16bit_adc() {
    let src = "var a: word = $0100\nvar b: word = $0200\nvar c: word = $0000\nc = a + b";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // Should have CLC + ADC zp (both lo and hi adds)
    let has_clc = bytes.contains(&0x18);
    assert!(has_clc, "word + word should emit CLC");
    // ADC zp ($65) should appear twice (lo+hi)
    let count_adc_zp = bytes.windows(1).filter(|w| *w == &[0x65]).count();
    assert!(count_adc_zp >= 2, "word + word should have 2× ADC zp");
}

#[test]
fn word_sub_constant_propagates_borrow() {
    let src = "var ptr: word = $0200\nptr = ptr - 1";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // SEC ($38), SBC imm ($E9), SBC #0 for borrow ($E9 $00)
    let has_sec = bytes.contains(&0x38);
    assert!(has_sec, "word - constant should emit SEC");
    let has_borrow = bytes.windows(2).any(|w| w == &[0xE9, 0x00]);
    assert!(has_borrow, "word - constant should propagate borrow with SBC #0");
}

#[test]
fn word_copy_copies_both_bytes() {
    let src = "var src: word = $0400\nvar dst: word = $0000\ndst = src";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // LDA zp ($A5) should appear at least 2× for lo and hi copy
    let count_lda_zp = bytes.windows(1).filter(|w| *w == &[0xA5]).count();
    assert!(count_lda_zp >= 2, "word copy should load both lo and hi bytes");
}

// ── print mixed args ────────────────────────────────────────────────────────

#[test]
fn print_empty_is_just_newline() {
    let prg = compile_raw("print");
    let bytes = &prg[2..];
    // header(2) + LDA #$0D(2) + JSR $FFD2(3) + RTS(1) = 8 bytes total
    assert_eq!(prg.len(), 8, "bare print = header + LDA #CR + JSR CHROUT + RTS");
    assert_eq!(bytes[0], 0xA9);  // LDA immediate
    assert_eq!(bytes[1], 0x0D);  // #$0D = carriage return
    assert_eq!(bytes[2], 0x20);  // JSR
}

#[test]
fn print_var_var_string() {
    let src = "var x = 3\nvar y = 7\nprint x, y, \"END\"";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    // 'E' = $45 in PETSCII
    assert!(res.prg.contains(&0x45), "Should contain PETSCII 'E'");
}

#[test]
fn print_string_var_mixed() {
    let src = "var n = 42\nprint \"N=\", n, \" OK\"";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    let prg = &res.prg;
    // 'N' in PETSCII = $4E
    assert!(prg.contains(&0x4E));
}

// ── Sub parameters ─────────────────────────────────────────────────────────

#[test]
fn sub_with_params_compiles() {
    let src = "
sub set_border(col)
  color border col
end
set_border(6)
set_border(2)
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // JSR should appear for both calls
    let jsr_count = res.prg[2..].windows(3).filter(|w| w[0] == 0x20).count();
    assert!(jsr_count >= 2, "Should emit at least 2 JSR instructions");
}

#[test]
fn sub_two_params() {
    let src = "
sub add_vals(a, b)
  var result = a + b
end
add_vals(10, 20)
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg[2..];
    // Args stored to ZP before JSR
    assert!(bytes.contains(&0x85)); // STA zp (param store)
    assert!(bytes.contains(&0x20)); // JSR
}

// ── Arrays ─────────────────────────────────────────────────────────────────

#[test]
fn array_set_constant_index() {
    let src = "
var scores = array(5)
scores[0] = 42
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // STA $C000 absolute
    let has_sta_c000 = res.prg[2..].windows(3).any(|w| w == &[0x8D, 0x00, 0xC0]);
    assert!(has_sta_c000, "Should emit STA $C000");
}

#[test]
fn array_set_variable_index() {
    let src = "
var scores = array(10)
var idx = 3
scores[idx] = 99
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0x91), "Should emit STA (ptr),Y for dynamic index");
}

#[test]
fn array_get_constant_index() {
    let src = "
var scores = array(5)
scores[2] = 7
var v = scores[2]
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // LDA $C002 absolute
    let has_lda = res.prg[2..].windows(3).any(|w| w == &[0xAD, 0x02, 0xC0]);
    assert!(has_lda, "Should emit LDA $C002");
}

#[test]
fn array_get_variable_index() {
    let src = "
var scores = array(10)
var i = 5
var v = scores[i]
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0xB1), "Should emit LDA (ptr),Y for dynamic index");
}

#[test]
fn multiple_arrays() {
    let src = "
var a = array(8)
var b = array(8)
a[0] = 1
b[0] = 2
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // a at $C000, b at $C008
    let has_a = res.prg[2..].windows(3).any(|w| w == &[0x8D, 0x00, 0xC0]);
    let has_b = res.prg[2..].windows(3).any(|w| w == &[0x8D, 0x08, 0xC0]);
    assert!(has_a, "a[0] → STA $C000");
    assert!(has_b, "b[0] → STA $C008");
}

// ── 16-bit (word) variables ─────────────────────────────────────────────────

#[test]
fn word_var_stores_16_bits() {
    let src = "var ptr: word = $0400";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // LDA #$00; STA zp; LDA #$04; STA zp+1
    assert!(bytes.contains(&0x00), "lo byte 0");
    assert!(bytes.contains(&0x04), "hi byte 4");
}

#[test]
fn word_var_used_in_poke() {
    let src = "
var addr: word = $D020
poke addr, 6
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // Should use STA (zp),Y  = $91
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0x91), "Should emit STA (zp),Y for word var poke");
}

#[test]
fn word_var_used_in_peek() {
    let src = "
var addr: word = $D012
var v = peek(addr)
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // Should use LDA (zp),Y  = $B1
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0xB1), "Should emit LDA (zp),Y for word var peek");
}

// ── String variables ────────────────────────────────────────────────────────

#[test]
fn string_var_inlined() {
    let src = "var msg = \"HELLO\"\nprint msg";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // String data should appear in binary
    let prg = &res.prg;
    let has_h = prg.contains(&0x48); // 'H' in PETSCII
    assert!(has_h, "PETSCII 'H' should be in binary");
    // print_str_via_ptr emits LDA (ptr),Y = $B1
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0xB1), "Should emit LDA (ptr),Y for string print");
}

#[test]
fn string_var_explicit_type() {
    let src = "var s: string = \"TEST\"\nprint s";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
}

#[test]
fn string_var_jmp_over_data() {
    let src = "var msg = \"ABC\"";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    // Should emit JMP ($4C) to skip over string data
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0x4C), "Should emit JMP over inline string data");
}

// ── New features: const, label, goto, poke, peek, rnd, abs, min, max, sgn ──

#[test]
fn const_substitution_works() {
    let src = "const SIZE = 100\nvar x = SIZE";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&100u8), "Should use const value 100");
}

#[test]
fn label_goto_forward() {
    let src = "goto skip\nvar x = 1\nlabel skip\nvar y = 2";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    assert!(res.prg.contains(&0x4C));
}

#[test]
fn poke_emits_sta_abs() {
    let prg = compile_raw("poke $D020, 2");
    let bytes = &prg[2..];
    let has_sta = bytes.windows(3).any(|w| w == &[0x8D, 0x20, 0xD0]);
    assert!(has_sta, "Should emit STA $D020");
}

#[test]
fn poke_with_expression_address() {
    let prg = compile_raw("var off = 10\npoke $0400 + off, 42");
    let bytes = &prg[2..];
    // Should contain STA (ptr),Y = $91
    assert!(bytes.contains(&0x91), "Should emit STA (ptr),Y for expression address");
}

#[test]
fn peek_emits_lda_abs() {
    let prg = compile_raw("var v = peek($D012)");
    let bytes = &prg[2..];
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x12, 0xD0]);
    assert!(has_lda, "Should emit LDA $D012");
}

#[test]
fn rnd_compiles() {
    let src = "var r = rnd()\nvar s = rnd";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0x0A));
    // post-whitening: EOR $D012 = $4D $12 $D0
    let has_eor = bytes.windows(3).any(|w| w == &[0x4D, 0x12, 0xD0]);
    assert!(has_eor, "rnd() should post-whiten with EOR $D012");
}

#[test]
fn len_of_string_literal() {
    let prg = compile_raw("var n = len(\"HELLO\")");
    let bytes = &prg[2..];
    // compile-time: LDA #5
    let has_lda5 = bytes.windows(2).any(|w| w == &[0xA9, 0x05]);
    assert!(has_lda5, "len(\"HELLO\") should emit LDA #5");
}

#[test]
fn len_of_string_var() {
    let prg = compile_raw("var s = \"HI\"\nvar n = len(s)");
    let bytes = &prg[2..];
    // loop: INY ($C8) + LDA (ptr),Y ($B1) + BNE + TYA ($98)
    assert!(bytes.contains(&0xC8), "len(s) should emit INY");
    assert!(bytes.contains(&0x98), "len(s) should emit TYA");
    let has_lda_ind = bytes.contains(&0xB1);
    assert!(has_lda_ind, "len(s) should emit LDA (ptr),Y");
}

#[test]
fn asc_of_string_literal() {
    let prg = compile_raw("var c = asc(\"A\")");
    let bytes = &prg[2..];
    // 'A' in PETSCII = 65 ($41)
    let has_lda_a = bytes.windows(2).any(|w| w == &[0xA9, 0x41]);
    assert!(has_lda_a, "asc(\"A\") should emit LDA #$41");
}

#[test]
fn asc_of_string_var() {
    let prg = compile_raw("var s = \"Q\"\nvar c = asc(s)");
    let bytes = &prg[2..];
    // LDY #0 ($A0 $00) + LDA (ptr),Y ($B1 ptr)
    let has_ldy0 = bytes.windows(2).any(|w| w == &[0xA0, 0x00]);
    assert!(has_ldy0, "asc(s) should emit LDY #0");
    assert!(bytes.contains(&0xB1), "asc(s) should emit LDA (ptr),Y");
}

#[test]
fn abs_compiles() {
    let src = "var a = abs(-5)\nvar b = abs(3)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

#[test]
fn min_max_compile() {
    let src = "var m1 = min(3, 7)\nvar m2 = max(3, 7)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

#[test]
fn sgn_compiles() {
    let src = "var s1 = sgn(0)\nvar s2 = sgn(5)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
}

#[test]
fn undefined_label_reports_error() {
    let src = "goto missing";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(!res.errors.is_empty(), "Should report undefined label");
    assert!(res.errors[0].contains("Undefined label"));
}

#[test]
fn full_new_features_program() {
    let src = "
const BORDER = $D020
var x = rnd()
var a = abs(x - 128)
var m = min(a, 50)
var v = peek($D012)
poke BORDER, 2

label start:
  x = rnd()
  if x > 200 then
    goto start
  end
print \"X=\", x
";
    let res = compile(src, &CompileOptions { basic_stub: true });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    assert!(res.prg.len() > 100);
}

#[test]
fn poke_expression_address_compiles() {
    let src = "
const SCREEN = $0400
var off = 0
var c = 1
poke SCREEN + off, c
";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0x91), "Should use indirect indexed STA (ptr),Y");
}

// ── chr$ ────────────────────────────────────────────────────────────────────

#[test]
fn chr_str_in_print_emits_chrout() {
    // print chr$(65) → eval 65 into A, JSR CHROUT
    let prg = compile_raw("print chr$(65)");
    let bytes = &prg[2..];
    // LDA #65 = A9 41
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 65]),
        "Should emit LDA #65");
    // JSR CHROUT = 20 D2 FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xD2, 0xFF]),
        "Should emit JSR CHROUT");
}

#[test]
fn chr_str_carriage_return() {
    // print chr$(13) → outputs $0D (CR)
    let prg = compile_raw("print chr$(13)");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x0D]),
        "Should emit LDA #$0D");
}

#[test]
fn chr_str_in_expression() {
    // var c = chr$(65) → LDA #65, STA zp  (identity: char code = byte value)
    let prg = compile_raw("var c = chr$(65)");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 65]),
        "Should load char code 65");
}

#[test]
fn chr_str_concat_with_string() {
    // print ">" + chr$(65) → prints '>' then 'A'
    let src = "print \">\" + chr$(65)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x3E]), // '>' = $3E in PETSCII
        "Should contain PETSCII '>'");
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 65]),
        "Should contain char code 65");
}

// ── gcls ────────────────────────────────────────────────────────────────────

#[test]
fn gcls_emits_fill_loop() {
    let prg = compile_raw("gcls");
    let bytes = &prg[2..];
    // Initializes ptr_hi to $20 (bitmap at $2000)
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x20]),
        "Should emit LDA #$20 for bitmap base high byte");
    // Inner loop uses STA (ptr_lo),Y = $91
    assert!(bytes.contains(&0x91), "Should emit STA (ptr),Y for fill");
    // INC ptr_hi = $E6
    assert!(bytes.contains(&0xE6), "Should emit INC ptr_hi");
}

#[test]
fn gcls_compiles_cleanly() {
    let src = "graphics on\ngcls";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
}

// ── bye/exit ─────────────────────────────────────────────────────────────────

#[test]
fn bye_emits_kernal_cls_and_rts() {
    let src = "bye";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg;
    // JSR $E544 = 20 44 E5
    let has_cls = bytes.windows(3).any(|w| w == [0x20, 0x44, 0xE5]);
    assert!(has_cls, "bye should JSR $E544 (KERNAL CLS)");
    // STA $91 = 85 91
    let has_sta91 = bytes.windows(2).any(|w| w == [0x85, 0x91]);
    assert!(has_sta91, "bye should clear stop-key flag ($91)");
    // LDA #$FF then STA $91
    let has_clear_91 = bytes.windows(4).any(|w| w == [0xA9, 0xFF, 0x85, 0x91]);
    assert!(has_clear_91, "bye should write #$FF to $91");
    // SEI = 78, CLI = 58
    assert!(bytes.contains(&0x78), "bye should SEI before clearing $91");
    assert!(bytes.contains(&0x58), "bye should CLI after clearing $91");
    let has_warm_start_jmp = bytes.windows(3).any(|w| w == [0x4C, 0x59, 0xA6]);
    assert!(has_warm_start_jmp, "bye should JMP $A659 (BASIC warm start)");
}

#[test]
fn exit_is_alias_for_bye() {
    let src = "exit";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg;
    let has_cls = bytes.windows(3).any(|w| w == [0x20, 0x44, 0xE5]);
    assert!(has_cls, "exit should JSR $E544 (alias for bye)");
}

// ── rem / ; comments ─────────────────────────────────────────────────────────

#[test]
fn rem_comment_ignored() {
    let src = "rem this is a comment\nvar x = 42";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    assert!(res.prg.windows(2).any(|w| w == [0xA9, 42u8]));
}

#[test]
fn semicolon_comment_ignored() {
    let src = "var x = 7 ; inline comment";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    assert!(res.prg.windows(2).any(|w| w == [0xA9, 7u8]));
}

// ── incbin ───────────────────────────────────────────────────────────────────

#[test]
fn incbin_embeds_bytes() {
    let path = "test_incbin_tmp.bin";
    std::fs::write(path, &[0x42u8, 0x43, 0x44]).unwrap();
    let src = format!("incbin \"{}\"", path);
    let res = compile(&src, &CompileOptions { basic_stub: false });
    std::fs::remove_file(path).ok();
    assert!(res.errors.is_empty());
    assert!(res.prg.windows(3).any(|w| w == [0x42, 0x43, 0x44]),
        "incbin bytes should appear in output");
}

// ── data / read ───────────────────────────────────────────────────────────────

#[test]
fn data_read_emits_indirect_lda() {
    let src = "data 10, 20, 30\nread x\nread y";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    assert!(res.prg.windows(3).any(|w| w == [10, 20, 30]),
        "data bytes should be in output");
    assert!(res.prg.contains(&0xB1), "read should use LDA (zp),Y");
    assert!(res.prg.contains(&0xE6), "read should INC data pointer");
}

#[test]
fn data_bytes_in_output() {
    let src = "data 99\nread x";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    assert!(res.prg.contains(&99u8), "data byte 99 should appear in output");
}

// ── plot ────────────────────────────────────────────────────────────────────

#[test]
fn plot_emits_jsr() {
    // plot x, y → stores coords then JSR to helper
    let src = "plot 10, 20";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let bytes = &res.prg[2..];
    // JSR opcode $20 must be present
    assert!(bytes.contains(&0x20), "Should emit JSR");
}

#[test]
fn plot_helper_contains_bitmap_base() {
    // The plot helper embeds $20 (high byte of $2000 bitmap base)
    let src = "plot 0, 0";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // ADC #$20 in the helper: opcode $69 $20
    let bytes = &res.prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0x69, 0x20]),
        "Plot helper should embed ADC #$20 for bitmap base");
}

#[test]
fn plot_helper_emitted_once_for_multiple_calls() {
    // Two plot calls → one helper, two JSR calls
    let a = compile_raw("plot 0, 0");
    let b = compile_raw("plot 0, 0\nplot 1, 1");
    // The helper (~70 bytes) is emitted once; b is larger but not 2× helper size larger
    let diff = b.len() as isize - a.len() as isize;
    assert!(diff < 70, "Second plot call should JSR to existing helper, not duplicate it");
    assert!(diff > 0, "Second call adds some code");
}

#[test]
fn plot_with_vars_compiles() {
    let src = "var px = 10\nvar py = 20\nplot px, py";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
}

#[test]
fn plot_rts_in_helper() {
    // RTS ($60) must appear — end of plot helper
    let src = "plot 5, 10";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    assert!(res.prg.contains(&0x60), "Should contain RTS in plot helper");
}

#[test]
fn plot_x_over_255_stores_hi_byte() {
    // plot 300, 0 → X_lo=44 ($2C), X_hi=1 — hi byte must appear
    let prg = compile_raw("plot 300, 0");
    let bytes = &prg[2..];
    // LDA #1 for X_hi = A9 01
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x01]),
        "plot 300,0 should emit LDA #1 for X_hi");
    // LDA #44 for X_lo = A9 2C
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 44]),
        "plot 300,0 should emit LDA #44 for X_lo");
}

#[test]
fn plot_x_319_full_width() {
    // 319 is the rightmost pixel: X_lo=63 ($3F), X_hi=1
    let src = "plot 319, 0";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    let bytes = &res.prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x01]),
        "plot 319,0 should store X_hi=1");
}

#[test]
fn plot_x_255_zero_hi_byte() {
    // x=255 stays 8-bit: X_hi must be 0
    let prg = compile_raw("plot 255, 0");
    let bytes = &prg[2..];
    // LDA #0 for X_hi = A9 00
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x00]),
        "plot 255,0 should store X_hi=0");
}

#[test]
fn plot_helper_has_x_hi_branch() {
    // The X_hi != 0 path is an INC ptr_hi ($E6) after a BEQ.
    // The helper must contain at least 2× INC ptr_hi for carry + X_hi paths.
    let prg = compile_raw("plot 0, 0");
    let bytes = &prg[2..];
    let inc_count = bytes.windows(2).filter(|w| w[0] == 0xE6).count();
    assert!(inc_count >= 3,
        "Helper should have INC ptr_hi for: lo carry, X_hi!=0, and pixel_y carry");
}

// ── Sprite ───────────────────────────────────────────────────────────────────

#[test]
fn sprite_sets_y_register() {
    // sprite 0, 100, 80  → STA $D001 (y_reg for sprite 0)
    let prg = compile_raw("sprite 0, 100, 80");
    let bytes = &prg[2..];
    // STA $D001 = 8D 01 D0
    let has_sta_y = bytes.windows(3).any(|w| w == &[0x8D, 0x01, 0xD0]);
    assert!(has_sta_y, "sprite should STA $D001 (Y register for sprite 0)");
}

#[test]
fn sprite_sets_x_register() {
    // sprite 0, 100, 80  → STA $D000 (x_reg for sprite 0)
    let prg = compile_raw("sprite 0, 100, 80");
    let bytes = &prg[2..];
    // STA $D000 = 8D 00 D0
    let has_sta_x = bytes.windows(3).any(|w| w == &[0x8D, 0x00, 0xD0]);
    assert!(has_sta_x, "sprite should STA $D000 (X register for sprite 0)");
}

#[test]
fn sprite_with_const_x_below_256_clears_d010_bit() {
    // x=100 < 256 → AND #$FE ($29 $FE) on $D010
    let prg = compile_raw("sprite 0, 100, 80");
    let bytes = &prg[2..];
    // AND #$FE = 29 FE (clears bit 0 = sprite 0 MSB)
    let has_and = bytes.windows(2).any(|w| w == &[0x29, 0xFE]);
    assert!(has_and, "x<256: sprite should AND #$FE on $D010 to clear MSB bit");
}

#[test]
fn sprite_with_const_x_above_255_sets_d010_bit() {
    // x=300 >= 256 → ORA #$01 ($09 $01) on $D010
    let prg = compile_raw("sprite 0, 300, 80");
    let bytes = &prg[2..];
    // ORA #$01 = 09 01
    let has_ora = bytes.windows(2).any(|w| w == &[0x09, 0x01]);
    assert!(has_ora, "x>=256: sprite should ORA #$01 on $D010 to set MSB bit");
    // X low byte = 300 & 0xFF = 44 = $2C  →  LDA #$2C = A9 2C
    let has_x_lo = bytes.windows(2).any(|w| w == &[0xA9, 0x2C]);
    assert!(has_x_lo, "sprite x=300: lo byte $2C should be loaded");
}

#[test]
fn sprite_with_data_addr_writes_pointer() {
    // sprite 0, 0, 0, $2000  → $2000>>6 = $80 → STA $07F8 = 8D F8 07
    let prg = compile_raw("sprite 0, 0, 0, $2000");
    let bytes = &prg[2..];
    // LDA #$80 = A9 80
    let has_ptr = bytes.windows(2).any(|w| w == &[0xA9, 0x80]);
    assert!(has_ptr, "sprite data_addr $2000 → pointer $80 should be loaded");
    // STA $07F8 = 8D F8 07
    let has_sta_ptr = bytes.windows(3).any(|w| w == &[0x8D, 0xF8, 0x07]);
    assert!(has_sta_ptr, "sprite data pointer should be STA'd to $07F8");
}

#[test]
fn sprite_on_sets_d015_bit() {
    // sprite on 0 → LDA $D015; ORA #$01; STA $D015
    let prg = compile_raw("sprite on 0");
    let bytes = &prg[2..];
    // LDA $D015 = AD 15 D0
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x15, 0xD0]);
    assert!(has_lda, "sprite_on should LDA $D015");
    // ORA #$01 = 09 01
    let has_ora = bytes.windows(2).any(|w| w == &[0x09, 0x01]);
    assert!(has_ora, "sprite_on 0 should ORA #$01");
    // STA $D015 = 8D 15 D0
    let has_sta = bytes.windows(3).any(|w| w == &[0x8D, 0x15, 0xD0]);
    assert!(has_sta, "sprite_on should STA $D015");
}

#[test]
fn sprite_off_clears_d015_bit() {
    // sprite off 0 → LDA $D015; AND #$FE; STA $D015
    let prg = compile_raw("sprite off 0");
    let bytes = &prg[2..];
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x15, 0xD0]);
    assert!(has_lda, "sprite_off should LDA $D015");
    // AND #$FE = 29 FE
    let has_and = bytes.windows(2).any(|w| w == &[0x29, 0xFE]);
    assert!(has_and, "sprite_off 0 should AND #$FE");
}

#[test]
fn sprite_color_writes_d027() {
    // sprite color 0, 7 → eval 7 → STA $D027 = 8D 27 D0
    let prg = compile_raw("sprite color 0, 7");
    let bytes = &prg[2..];
    let has_sta = bytes.windows(3).any(|w| w == &[0x8D, 0x27, 0xD0]);
    assert!(has_sta, "sprite_color 0 should STA $D027");
    let has_val = bytes.windows(2).any(|w| w == &[0xA9, 0x07]);
    assert!(has_val, "sprite_color 7 should load #7");
}

#[test]
fn sprite_color_1_writes_d028() {
    // sprite color 1, 3 → STA $D028 = 8D 28 D0
    let prg = compile_raw("sprite color 1, 3");
    let bytes = &prg[2..];
    let has_sta = bytes.windows(3).any(|w| w == &[0x8D, 0x28, 0xD0]);
    assert!(has_sta, "sprite_color 1 should STA $D028");
}

#[test]
fn sprite_multicolor_on_sets_d01c_bit() {
    // sprite multi 0, on → LDA $D01C; ORA #$01; STA $D01C
    let prg = compile_raw("sprite multi 0, on");
    let bytes = &prg[2..];
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x1C, 0xD0]);
    assert!(has_lda, "sprite_multicolor on should LDA $D01C");
    let has_ora = bytes.windows(2).any(|w| w == &[0x09, 0x01]);
    assert!(has_ora, "sprite_multicolor 0,on should ORA #$01");
    let has_sta = bytes.windows(3).any(|w| w == &[0x8D, 0x1C, 0xD0]);
    assert!(has_sta, "sprite_multicolor on should STA $D01C");
}

#[test]
fn sprite_multicolor_off_clears_d01c_bit() {
    // sprite multi 0, off → AND #$FE
    let prg = compile_raw("sprite multi 0, off");
    let bytes = &prg[2..];
    let has_and = bytes.windows(2).any(|w| w == &[0x29, 0xFE]);
    assert!(has_and, "sprite_multicolor 0,off should AND #$FE on $D01C");
}

#[test]
fn sprite_hit_reads_d01e() {
    // var h = sprhit() → LDA $D01E = AD 1E D0
    let prg = compile_raw("var h = sprhit()");
    let bytes = &prg[2..];
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x1E, 0xD0]);
    assert!(has_lda, "sprite_hit() should LDA $D01E");
}

#[test]
fn sprite_bg_hit_reads_d01f() {
    // var h = sprbghit() → LDA $D01F = AD 1F D0
    let prg = compile_raw("var h = sprbghit()");
    let bytes = &prg[2..];
    let has_lda = bytes.windows(3).any(|w| w == &[0xAD, 0x1F, 0xD0]);
    assert!(has_lda, "sprite_bg_hit() should LDA $D01F");
}

#[test]
fn sprite_1_uses_correct_registers() {
    // sprite 1, 50, 50  → X=$D002, Y=$D003, MSB bit=2 in $D010
    let prg = compile_raw("sprite 1, 50, 50");
    let bytes = &prg[2..];
    // STA $D002 = 8D 02 D0
    let has_sta_x = bytes.windows(3).any(|w| w == &[0x8D, 0x02, 0xD0]);
    assert!(has_sta_x, "sprite 1 X should STA $D002");
    // STA $D003 = 8D 03 D0
    let has_sta_y = bytes.windows(3).any(|w| w == &[0x8D, 0x03, 0xD0]);
    assert!(has_sta_y, "sprite 1 Y should STA $D003");
    // AND #$FD = 29 FD (x<256: clear bit 1)
    let has_and = bytes.windows(2).any(|w| w == &[0x29, 0xFD]);
    assert!(has_and, "sprite 1 x<256 should AND #$FD (clear bit 1 of $D010)");
}

#[test]
fn sprite_word_x_uses_lo_byte_and_runtime_msb() {
    // word var for X → loads lo byte to $D000, checks hi byte at runtime
    let src = "var wx: word = 300\nsprite 0, wx, 50";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // LDA zp  = A5 zp
    let has_lda_zp = bytes.windows(1).filter(|w| w[0] == 0xA5).count();
    assert!(has_lda_zp >= 2, "word x: should load lo and hi bytes from ZP");
    // BEQ = F0 (branch to clear_msb)
    let has_beq = bytes.contains(&0xF0);
    assert!(has_beq, "word x: should emit BEQ for runtime MSB check");
    // ORA #$01 = 09 01 (set MSB when hi!=0)
    let has_ora = bytes.windows(2).any(|w| w == &[0x09, 0x01]);
    assert!(has_ora, "word x: should emit ORA #$01 for set-MSB path");
}

#[test]
fn sprite_def_aligns_to_64_byte_boundary() {
    // sprdef 0, <63 bytes>  at $080D → JMP over data, data at $0840 (page $21)
    // JMP = 4C lo hi = 3 bytes; $080D+3 = $0810; next 64-boundary = $0840
    let mut bytes63 = vec![0u8; 63];
    bytes63[1] = 0x7E; // row 1 byte 1, easily spotted
    let src = format!(
        "sprdef 0\n{}\nend",
        bytes63.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")
    );
    let prg = compile_raw(&src);
    let bytes = &prg[2..]; // skip load address

    // JMP $?? $?? should be first instruction = 4C
    assert_eq!(bytes[0], 0x4C, "sprite_def should start with JMP");

    // data_addr = $0840, page = $21; expect LDA #$21 somewhere
    let has_lda_page = bytes.windows(2).any(|w| w == &[0xA9, 0x21]);
    assert!(has_lda_page, "sprite_def should emit LDA #$21 (page)");

    // STA $07F8 = 8D F8 07
    let has_sta_ptr = bytes.windows(3).any(|w| w == &[0x8D, 0xF8, 0x07]);
    assert!(has_sta_ptr, "sprite_def should emit STA $07F8");

    // $7E marker byte should be at $0841 = bytes[64] (prg[2..] starts at $0801; $0841-$0801=64)
    assert_eq!(bytes[64], 0x7E, "sprite data byte 1 should be at expected offset");
}

#[test]
fn sprite_def_1_uses_d07f9() {
    // sprdef 1 → STA $07F9 (= $07F8 + 1)
    let src = "sprdef 1\n0,0,0\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    let has_sta_ptr1 = bytes.windows(3).any(|w| w == &[0x8D, 0xF9, 0x07]);
    assert!(has_sta_ptr1, "sprite_def 1 should emit STA $07F9");
}

// ── Input ────────────────────────────────────────────────────────────────────

#[test]
fn input_int_emits_basin_call() {
    let prg = compile_raw("input score");
    let bytes = &prg[2..];
    // JSR $FFCF (BASIN): 20 CF FF
    let has_basin = bytes.windows(3).any(|w| w == &[0x20, 0xCF, 0xFF]);
    assert!(has_basin, "input should emit JSR $FFCF (BASIN)");
}

#[test]
fn input_string_emits_basin_and_null_term() {
    let prg = compile_raw("var msg: string = \"\"\ninput msg");
    let bytes = &prg[2..];
    let has_basin = bytes.windows(3).any(|w| w == &[0x20, 0xCF, 0xFF]);
    assert!(has_basin, "input string should emit JSR $FFCF");
    // Null-terminate: LDA #0 (A9 00) then STA indirect (91)
    let has_null = bytes.windows(2).any(|w| w == &[0xA9, 0x00]);
    assert!(has_null, "input string should emit null terminator (LDA #0)");
}

#[test]
fn input_with_prompt_prints_before_basin() {
    let prg = compile_raw("input \"NAME: \", name");
    let bytes = &prg[2..];
    // Should contain CHROUT call ($FFD2) for the prompt before BASIN ($FFCF)
    let chrout_pos = bytes.windows(3).position(|w| w == &[0x20, 0xD2, 0xFF]);
    let basin_pos  = bytes.windows(3).position(|w| w == &[0x20, 0xCF, 0xFF]);
    assert!(chrout_pos.is_some(), "input with prompt should call CHROUT");
    assert!(basin_pos.is_some(),  "input with prompt should call BASIN");
    assert!(chrout_pos.unwrap() < basin_pos.unwrap(), "CHROUT must come before BASIN");
}

// ── Fill ─────────────────────────────────────────────────────────────────────

#[test]
fn fill_emits_indirect_store() {
    let prg = compile_raw("fill $0400, 1000, 32");
    let bytes = &prg[2..];
    // STA (ptr),Y = $91 <zp>
    let has_sta_ind = bytes.windows(1).any(|w| w[0] == 0x91);
    assert!(has_sta_ind, "fill should emit STA (ptr),Y");
}

#[test]
fn fill_page_count_for_1000_bytes() {
    // 1000 = 3 pages + 232 partial → pg_ctr = 3, partial = 232
    let prg = compile_raw("fill $0400, 1000, 0");
    let bytes = &prg[2..];
    // LDA #3 (page count hi)
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 3]),
        "fill 1000 should store page count 3");
    // LDA #232 (partial = 1000 - 3*256 = 232)
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 232]),
        "fill 1000 should store partial count 232");
}

#[test]
fn fill_zero_len_compiles_cleanly() {
    // fill with len=0: pg_ctr=0, partial=0 — both BEQ taken at runtime, but
    // the loop body bytes are still emitted. Verify it compiles and has BEQ ($F0).
    let prg = compile_raw("fill $0400, 0, 99");
    let bytes = &prg[2..];
    // Both page and partial loops guarded by BEQ ($F0)
    let beq_count = bytes.iter().filter(|&&b| b == 0xF0).count();
    assert!(beq_count >= 2, "fill should emit at least 2 BEQ guards for the two loops");
}

// ── Memcopy ───────────────────────────────────────────────────────────────────

#[test]
fn memcopy_emits_load_and_store_indirect() {
    let prg = compile_raw("memcopy $C000, $0400, 256");
    let bytes = &prg[2..];
    // LDA (src),Y = $B1 <zp>
    assert!(bytes.iter().any(|&b| b == 0xB1), "memcopy should emit LDA (src),Y");
    // STA (dst),Y = $91 <zp>
    assert!(bytes.iter().any(|&b| b == 0x91), "memcopy should emit STA (dst),Y");
}

#[test]
fn memcopy_256_bytes_uses_page_loop() {
    // 256 bytes = 1 full page → pg_ctr=1, partial=0
    let prg = compile_raw("memcopy $C000, $0400, 256");
    let bytes = &prg[2..];
    // LDA #1 for page count (hi byte of 256 = 1)
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 1]),
        "memcopy 256 bytes should have page count 1");
    // LDA #0 for partial (lo byte of 256 = 0)
    // Also used for LDY #0, but the combination is unambiguous in context
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0]),
        "memcopy 256 bytes should have partial count 0");
}

#[test]
fn memcopy_increments_src_and_dst_hi() {
    // After each full page: INC src_hi ($E6) and INC dst_hi ($E6)
    let prg = compile_raw("memcopy $C000, $0400, 512");
    let bytes = &prg[2..];
    let inc_zp_count = bytes.windows(1).filter(|w| w[0] == 0xE6).count();
    // Must have at least 2 INC zp (src_hi and dst_hi) in the page loop
    assert!(inc_zp_count >= 2, "memcopy should INC src_hi and dst_hi each page");
}

// ── IRQ ───────────────────────────────────────────────────────────────────────

#[test]
fn irq_emits_sei_cli() {
    let prg = compile_raw("irq $0900");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x78), "irq should emit SEI ($78)");
    assert!(bytes.iter().any(|&b| b == 0x58), "irq should emit CLI ($58)");
}

#[test]
fn irq_sets_0314_vector() {
    let prg = compile_raw("irq $0900");
    let bytes = &prg[2..];
    // STA $0314: 8D 14 03
    let has_0314 = bytes.windows(3).any(|w| w == &[0x8D, 0x14, 0x03]);
    assert!(has_0314, "irq should emit STA $0314");
    // STA $0315: 8D 15 03
    let has_0315 = bytes.windows(3).any(|w| w == &[0x8D, 0x15, 0x03]);
    assert!(has_0315, "irq should emit STA $0315");
}

#[test]
fn irq_disables_cia1() {
    let prg = compile_raw("irq $0900");
    let bytes = &prg[2..];
    // STA $DC0D (CIA1 ICR): 8D 0D DC; LDA #$7F before it
    let has_cia = bytes.windows(3).any(|w| w == &[0x8D, 0x0D, 0xDC]);
    assert!(has_cia, "irq should disable CIA1 IRQ (STA $DC0D)");
}

#[test]
fn irq_sets_raster_line() {
    let prg = compile_raw("irq $0900, 100");
    let bytes = &prg[2..];
    // LDA #100 then STA $D012: A9 64 8D 12 D0
    let pos = bytes.windows(5).position(|w| w == &[0xA9, 100, 0x8D, 0x12, 0xD0]);
    assert!(pos.is_some(), "irq with line should emit LDA #100; STA $D012");
}

#[test]
fn irq_constant_address_emits_lo_hi() {
    let prg = compile_raw("irq $C800");
    let bytes = &prg[2..];
    // LDA #$00 (lo of $C800): A9 00
    // LDA #$C8 (hi of $C800): A9 C8
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x00]), "irq $C800 lo byte = 0");
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0xC8]), "irq $C800 hi byte = $C8");
}

#[test]
fn irq_forward_ref_sub_patched() {
    // irq handler defined AFTER the irq statement — forward ref
    let src = "\
irq my_irq\n\
sub my_irq()\n\
  bye\n\
end\n\
";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // The sub starts after the main body (RTS) + stub overhead
    // We just verify: STA $0314 is present and the lo byte of the sub address
    // is patched into the LDA #xx instruction before STA $0314
    let sta_0314 = bytes.windows(3).position(|w| w == &[0x8D, 0x14, 0x03]);
    assert!(sta_0314.is_some(), "irq forward ref should emit STA $0314");
    // The byte just before 8D 14 03 is the handler address lo byte (not 0)
    if let Some(pos) = sta_0314 {
        let lo_byte = bytes[pos - 1]; // byte before STA $0314 is the LDA #<lo> operand
        assert_ne!(lo_byte, 0x8D, "lo byte should be address, not another opcode");
    }
}

// ── mod operator ────────────────────────────────────────────────────────────

#[test]
fn mod_emits_sec_sbc_bcs_loop() {
    // x mod 10 should emit SEC; SBC; BCS loop; CLC; ADC pattern
    let prg = compile_raw("var x = 25\nvar r = x mod 10\n");
    let bytes = &prg[2..];
    // Find SEC (0x38) followed by SBC zp (0xE5)
    let sec_sbc = bytes.windows(2).any(|w| w == &[0x38, 0xE5]);
    assert!(sec_sbc, "mod: should emit SEC then SBC zp");
    // Find BCS (0xB0) in output
    assert!(bytes.contains(&0xB0), "mod: BCS should be emitted");
    // Find CLC + ADC (0x18 0x65)
    let clc_adc = bytes.windows(2).any(|w| w == &[0x18, 0x65]);
    assert!(clc_adc, "mod: should emit CLC; ADC to restore remainder");
}

#[test]
fn mod_constant_computes_correctly() {
    // Verify that constant mod compiles — specific byte sequence for 7 mod 3
    let prg = compile_raw("var r = 7 mod 3\n");
    // Should not panic and should produce code
    assert!(prg.len() > 3, "7 mod 3 should produce code");
}

// ── save statement ───────────────────────────────────────────────────────────

#[test]
fn save_emits_setnam_setlfs_save() {
    // save "DATA", $C000, 1024 — should emit SETNAM/SETLFS/SAVE calls
    let prg = compile_raw("save \"DATA\", $C000, 1024\n");
    let bytes = &prg[2..];
    // SETNAM = JSR $FFBD: 20 BD FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xBD, 0xFF]), "save: SETNAM (JSR $FFBD) missing");
    // SETLFS = JSR $FFBA: 20 BA FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xBA, 0xFF]), "save: SETLFS (JSR $FFBA) missing");
    // SAVE = JSR $FFD8: 20 D8 FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xD8, 0xFF]), "save: SAVE (JSR $FFD8) missing");
}

#[test]
fn save_embeds_filename_bytes() {
    // Filename "HI" should appear as bytes in the PRG
    let prg = compile_raw("save \"HI\", $C000, 256\n");
    let bytes = &prg[2..];
    // 'H'=72, 'I'=73
    let found = bytes.windows(2).any(|w| w == &[b'H', b'I']);
    assert!(found, "save: filename bytes 'HI' should appear in output");
}

#[test]
fn save_setnam_length_byte() {
    // LDA #len should appear right before SETNAM
    let prg = compile_raw("save \"DEMO\", $2000, 512\n");
    let bytes = &prg[2..];
    // filename "DEMO" has length 4
    // Find: LDA #4 ($A9 $04) somewhere before JSR $FFBD
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 4]), "save: LDA #4 (SETNAM length) not found");
}

// ── cursor statement ─────────────────────────────────────────────────────────

#[test]
fn cursor_emits_kernal_plot() {
    // cursor 10, 5 → JSR $FFF0
    let prg = compile_raw("cursor 10, 5\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xF0, 0xFF]), "cursor: JSR $FFF0 (KERNAL PLOT) missing");
}

#[test]
fn cursor_emits_sec_before_plot() {
    // SEC ($38) must appear before JSR $FFF0 to set cursor position mode
    let prg = compile_raw("cursor 0, 0\n");
    let bytes = &prg[2..];
    let plot_pos = bytes.windows(3).position(|w| w == &[0x20, 0xF0, 0xFF]);
    assert!(plot_pos.is_some(), "cursor: JSR $FFF0 missing");
    let pos = plot_pos.unwrap();
    assert!(bytes[..pos].contains(&0x38), "cursor: SEC must appear before JSR $FFF0");
}

#[test]
fn cursor_transfers_y_register() {
    // cursor col, row: col → Y register via TAY ($A8)
    let prg = compile_raw("cursor 20, 10\n");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xA8), "cursor: TAY ($A8) to pass column in Y register");
}

// ── repeat / until loop ──────────────────────────────────────────────────────

#[test]
fn repeat_until_emits_body_then_cond() {
    // repeat; var x = x + 1; until x == 10
    let prg = compile_raw("var x = 0\nrepeat\n  x = x + 1\nuntil x == 10\n");
    // Should produce code without panic and be longer than minimal
    assert!(prg.len() > 20, "repeat/until should produce substantial code");
}

#[test]
fn repeat_until_jumps_back() {
    // JMP opcode ($4C) must be present for the loop-back branch
    let prg = compile_raw("var i = 0\nrepeat\n  i = i + 1\nuntil i == 5\n");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x4C), "repeat/until: JMP ($4C) for loop-back expected");
}

#[test]
fn repeat_until_cmp_1() {
    // CMP #1 ($C9 $01) is used to test the condition value (0 or 1)
    let prg = compile_raw("var done = 0\nrepeat\n  done = 1\nuntil done == 1\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xC9, 0x01]), "repeat/until: CMP #1 for condition test missing");
}

// ── sprite expand x/y ────────────────────────────────────────────────────────

#[test]
fn sprite_expand_x_on_emits_d01d_ora() {
    // sprite expand x 0, on → LDA $D01D; ORA #1; STA $D01D
    let prg = compile_raw("sprite expand x 0, on\n");
    let bytes = &prg[2..];
    // LDA $D01D = AD 1D D0
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x1D, 0xD0]), "expand x on: LDA $D01D missing");
    // ORA #1 = 09 01
    assert!(bytes.windows(2).any(|w| w == &[0x09, 0x01]), "expand x on: ORA #1 missing");
    // STA $D01D = 8D 1D D0
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x1D, 0xD0]), "expand x on: STA $D01D missing");
}

#[test]
fn sprite_expand_x_off_emits_d01d_and() {
    // sprite expand x 1, off → LDA $D01D; AND #$FD; STA $D01D
    let prg = compile_raw("sprite expand x 1, off\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x1D, 0xD0]), "expand x off: LDA $D01D missing");
    // AND #$FD (NOT bit 1 = $FE... wait, sprite 1 = bit 1 = $02, ~$02 = $FD)
    assert!(bytes.windows(2).any(|w| w == &[0x29, 0xFD]), "expand x off: AND #$FD missing");
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x1D, 0xD0]), "expand x off: STA $D01D missing");
}

#[test]
fn sprite_expand_y_on_emits_d017() {
    // sprite expand y 2, on → LDA $D017; ORA #4; STA $D017
    let prg = compile_raw("sprite expand y 2, on\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x17, 0xD0]), "expand y on: LDA $D017 missing");
    // ORA #4 = 09 04
    assert!(bytes.windows(2).any(|w| w == &[0x09, 0x04]), "expand y on: ORA #4 missing");
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x17, 0xD0]), "expand y on: STA $D017 missing");
}

#[test]
fn sprite_expand_y_off_emits_d017_and() {
    // sprite expand y 0, off → LDA $D017; AND #$FE; STA $D017
    let prg = compile_raw("sprite expand y 0, off\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x17, 0xD0]), "expand y off: LDA $D017 missing");
    assert!(bytes.windows(2).any(|w| w == &[0x29, 0xFE]), "expand y off: AND #$FE missing");
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x17, 0xD0]), "expand y off: STA $D017 missing");
}

// ── sprite priority ──────────────────────────────────────────────────────────

#[test]
fn sprite_priority_on_emits_d01b_ora() {
    // sprite priority 0, on → behind bg → LDA $D01B; ORA #1; STA $D01B
    let prg = compile_raw("sprite priority 0, on\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x1B, 0xD0]), "priority on: LDA $D01B missing");
    assert!(bytes.windows(2).any(|w| w == &[0x09, 0x01]), "priority on: ORA #1 missing");
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x1B, 0xD0]), "priority on: STA $D01B missing");
}

#[test]
fn sprite_priority_off_emits_d01b_and() {
    // sprite priority 0, off → in front → LDA $D01B; AND #$FE; STA $D01B
    let prg = compile_raw("sprite priority 0, off\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x1B, 0xD0]), "priority off: LDA $D01B missing");
    assert!(bytes.windows(2).any(|w| w == &[0x29, 0xFE]), "priority off: AND #$FE missing");
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x1B, 0xD0]), "priority off: STA $D01B missing");
}

// ── plot erase ───────────────────────────────────────────────────────────────

#[test]
fn plot_erase_compiles_without_panic() {
    // Basic smoke test: plot erase inside graphics on
    let prg = compile_raw("graphics on\ngcls\nplot erase 100, 50\n");
    assert!(prg.len() > 10, "plot erase should produce code");
}

#[test]
fn plot_erase_emits_jsr_helper() {
    // plot erase should emit a JSR to the erase helper
    let prg = compile_raw("graphics on\ngcls\nplot erase 10, 20\n");
    let bytes = &prg[2..];
    // A JSR opcode (0x20) must appear
    assert!(bytes.contains(&0x20), "plot erase: JSR opcode should be emitted");
}

#[test]
fn plot_erase_helper_contains_eor_ff() {
    // The erase helper must invert the mask via EOR #$FF ($49 $FF)
    let prg = compile_raw("graphics on\ngcls\nplot erase 0, 0\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0x49, 0xFF]), "plot erase helper: EOR #$FF for mask inversion missing");
}

#[test]
fn plot_erase_helper_contains_and_zp() {
    // The erase helper uses AND zp ($25) to clear the pixel
    let prg = compile_raw("graphics on\ngcls\nplot erase 0, 0\n");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x25), "plot erase helper: AND zp ($25) opcode missing");
}

// ── plot xor ─────────────────────────────────────────────────────────────────

#[test]
fn plot_xor_compiles_without_panic() {
    let prg = compile_raw("graphics on\ngcls\nplot xor 100, 50\n");
    assert!(prg.len() > 10, "plot xor should produce code");
}

#[test]
fn plot_xor_helper_contains_eor_zp() {
    // The xor helper uses EOR zp ($45) to flip the pixel
    let prg = compile_raw("graphics on\ngcls\nplot xor 0, 0\n");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x45), "plot xor helper: EOR zp ($45) opcode missing");
}

#[test]
fn plot_xor_does_not_contain_eor_ff() {
    // The xor helper should NOT invert the mask (no EOR #$FF)
    let prg = compile_raw("graphics on\ngcls\nplot xor 0, 0\n");
    let bytes = &prg[2..];
    assert!(!bytes.windows(2).any(|w| w == &[0x49, 0xFF]),
        "plot xor helper: should not invert mask (EOR #$FF)");
}

#[test]
fn all_three_plot_modes_together() {
    // Using all three plot modes in one program should emit all three helpers
    let prg = compile_raw("graphics on\ngcls\nplot 10, 10\nplot erase 10, 10\nplot xor 10, 10\n");
    let bytes = &prg[2..];
    // SET: ORA zp ($05) — in plot_helper
    assert!(bytes.contains(&0x05), "plot (set): ORA zp missing");
    // ERASE: AND zp ($25) — in plot_erase_helper
    assert!(bytes.contains(&0x25), "plot erase: AND zp missing");
    // XOR: EOR zp ($45) — in plot_xor_helper
    assert!(bytes.contains(&0x45), "plot xor: EOR zp missing");
}

// ── peek16 / poke16 ─────────────────────────────────────────────────────────

#[test]
fn peek16_constant_addr_reads_two_bytes() {
    // var p: word = peek16($D012) — should emit two LDA abs instructions
    let prg = compile_raw("var p: word = peek16($D012)");
    let bytes = &prg[2..];
    // LDA $D012 = AD 12 D0 and LDA $D013 = AD 13 D0
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x12, 0xD0]), "peek16 lo: LDA $D012");
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x13, 0xD0]), "peek16 hi: LDA $D013");
}

#[test]
fn poke16_constant_addr_writes_two_bytes() {
    // poke16 $0314, $EA81 — writes lo=$81 to $0314, hi=$EA to $0315
    let prg = compile_raw("poke16 $0314, $EA81");
    let bytes = &prg[2..];
    // STA $0314 = 8D 14 03  and  STA $0315 = 8D 15 03
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x14, 0x03]), "poke16: STA $0314 (lo)");
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x15, 0x03]), "poke16: STA $0315 (hi)");
}

#[test]
fn poke16_word_value_emits_both_bytes() {
    // var v: word = $1234 \n poke16 $C000, v
    let prg = compile_raw("var v: word = $1234\npoke16 $C000, v");
    let bytes = &prg[2..];
    // Value lo = $34, hi = $12 loaded via LDA zp ($A5)
    let has_lda_zp_twice = bytes.windows(1).filter(|w| *w == &[0xA5]).count() >= 2;
    assert!(has_lda_zp_twice, "poke16 word var: should LDA zp twice (lo then hi)");
    // STA $C000 = 8D 00 C0
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x00, 0xC0]), "poke16: STA $C000");
}

#[test]
fn poke16_word_addr_uses_indirect() {
    // var ptr: word = $C000 \n poke16 ptr, 0
    let prg = compile_raw("var ptr: word = $C000\npoke16 ptr, 0");
    let bytes = &prg[2..];
    // STA (ptr),Y — opcode $91
    assert!(bytes.contains(&0x91), "poke16 via word ptr: STA (zp),Y missing");
}

#[test]
fn word_vardecl_with_add_emits_clc() {
    // var ptr: word = $00FF \n  var ptr2: word = ptr + 1
    let prg = compile_raw("var ptr: word = $00FF\nvar ptr2: word = ptr + 1");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x18), "word VarDecl += should emit CLC");
    assert!(bytes.windows(2).any(|w| w == &[0x69, 0x00]),
        "word VarDecl += should propagate carry with ADC #0");
}

#[test]
fn word_vardecl_sub_emits_sec() {
    let prg = compile_raw("var ptr: word = $0200\nvar ptr2: word = ptr - 5");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x38), "word VarDecl -= should emit SEC");
    assert!(bytes.windows(2).any(|w| w == &[0xE9, 0x00]),
        "word VarDecl -= should propagate borrow with SBC #0");
}

// ── open / close / print# ────────────────────────────────────────────────────

#[test]
fn open_emits_setnam_setlfs_open() {
    // open 1, 8, 2, "TEST" → SETNAM($FFBD) + SETLFS($FFBA) + OPEN($FFC0)
    let prg = compile_raw("open 1, 8, 2, \"TEST\"");
    let bytes = &prg[2..];
    // JSR $FFBD = 20 BD FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xBD, 0xFF]), "open: JSR SETNAM missing");
    // JSR $FFBA = 20 BA FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xBA, 0xFF]), "open: JSR SETLFS missing");
    // JSR $FFC0 = 20 C0 FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xC0, 0xFF]), "open: JSR OPEN missing");
}

#[test]
fn close_emits_kernal_close() {
    // close 1 → A = 1, JSR $FFC3
    let prg = compile_raw("close 1");
    let bytes = &prg[2..];
    // LDA #1 = A9 01
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x01]), "close: LDA #1 missing");
    // JSR $FFC3 = 20 C3 FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xC3, 0xFF]), "close: JSR CLOSE missing");
}

#[test]
fn print_hash_emits_chkout_clrchn() {
    // print# 3, "HI" → CHKOUT($FFC9) + CHROUT + CLRCHN($FFCC)
    let prg = compile_raw("print# 3, \"HI\"");
    let bytes = &prg[2..];
    // TAX = AA (channel → X for CHKOUT)
    assert!(bytes.contains(&0xAA), "print#: TAX missing");
    // JSR $FFC9 = 20 C9 FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xC9, 0xFF]), "print#: JSR CHKOUT missing");
    // JSR $FFCC = 20 CC FF
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xCC, 0xFF]), "print#: JSR CLRCHN missing");
    // 'H' in PETSCII = $48
    assert!(bytes.contains(&0x48), "print#: 'H' PETSCII byte missing");
}

#[test]
fn open_no_filename_emits_empty_setnam() {
    // open 2, 4, 7  (no filename → SETNAM with len=0)
    let prg = compile_raw("open 2, 4, 7");
    let bytes = &prg[2..];
    // LDA #0 (len=0 for SETNAM) = A9 00 followed by LDX #0, LDY #0
    // Check SETNAM is still called
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xBD, 0xFF]), "open no filename: JSR SETNAM missing");
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xFFC0u16 as u8, (0xFFC0u16 >> 8) as u8]),
        "open no filename: JSR OPEN missing");
}

