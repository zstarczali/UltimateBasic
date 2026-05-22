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
    assert!(bytes.contains(&0xD0)); // BNE
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
    let prg = compile_raw("var c = getch");
    let bytes = &prg[2..];
    // JSR $FFE4, CMP #0, BEQ loop
    assert!(bytes.contains(&0x20)); // JSR
    let has_ffe4 = bytes.windows(3).any(|w| w == &[0x20, 0xE4, 0xFF]);
    assert!(has_ffe4, "Should have JSR $FFE4");
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
    let large = compile_raw("cls manual").len();
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
    let src = "var score = 42\nint_to_str score, $0340";
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
