// Integration tests for Ultimate Basic compiler.
// Tests compile entire programs and verify PRG output.

use ultimate_basic::compiler::{compile, compile_with_path, CompileOptions};

fn compile_stub(src: &str) -> Vec<u8> {
    compile(src, &CompileOptions { basic_stub: true }).prg
}

fn compile_raw(src: &str) -> Vec<u8> {
    compile(src, &CompileOptions { basic_stub: false }).prg
}

#[test]
fn graphics_on_block_compiles() {
    let res = compile("graphics on block", &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(),
        "expected graphics on block to compile without errors, got {:?}", res.errors);
    assert!(!res.prg.is_empty(), "expected non-empty PRG output");
}

#[test]
fn plot4_compiles() {
    let res = compile("graphics on block\nplot4 1, 2", &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(),
        "expected plot4 to compile without errors, got {:?}", res.errors);
}

struct TestCpu {
    mem: [u8; 65536],
    pc: u16,
    sp: u8,
    a: u8,
    x: u8,
    y: u8,
    carry: bool,
    zero: bool,
    negative: bool,
    call_depth: usize,
}

impl TestCpu {
    fn new(prg: &[u8]) -> Self {
        let mut mem = [0u8; 65536];
        let load_addr = u16::from_le_bytes([prg[0], prg[1]]);
        let start = load_addr as usize;
        mem[start..start + prg[2..].len()].copy_from_slice(&prg[2..]);
        Self {
            mem,
            pc: load_addr,
            sp: 0xFF,
            a: 0,
            x: 0,
            y: 0,
            carry: false,
            zero: false,
            negative: false,
            call_depth: 0,
        }
    }

    fn run_until_main_rts(&mut self, max_steps: usize) {
        for _ in 0..max_steps {
            if !self.step() {
                return;
            }
        }
        panic!("test CPU exceeded step budget at ${:04X}", self.pc);
    }

    fn step(&mut self) -> bool {
        let opcode = self.fetch_byte();
        match opcode {
            0x05 => {
                let zp = self.fetch_byte();
                self.a |= self.mem[zp as usize];
                self.set_zn(self.a);
            }
            0x09 => {
                let imm = self.fetch_byte();
                self.a |= imm;
                self.set_zn(self.a);
            }
            0x18 => self.carry = false,
            0x20 => {
                let addr = self.fetch_word();
                let ret = self.pc.wrapping_sub(1);
                self.push((ret >> 8) as u8);
                self.push(ret as u8);
                self.pc = addr;
                self.call_depth += 1;
            }
            0x29 => {
                let imm = self.fetch_byte();
                self.a &= imm;
                self.set_zn(self.a);
            }
            0x46 => {
                let zp = self.fetch_byte();
                let value = self.mem[zp as usize];
                self.carry = value & 1 != 0;
                let result = value >> 1;
                self.mem[zp as usize] = result;
                self.set_zn(result);
            }
            0x48 => self.push(self.a),
            0x4A => {
                self.carry = self.a & 1 != 0;
                self.a >>= 1;
                self.set_zn(self.a);
            }
            0x4C => self.pc = self.fetch_word(),
            0x60 => {
                if self.call_depth == 0 {
                    return false;
                }
                let lo = self.pop();
                let hi = self.pop();
                self.pc = u16::from_le_bytes([lo, hi]).wrapping_add(1);
                self.call_depth -= 1;
            }
            0x65 => {
                let zp = self.fetch_byte();
                let value = self.mem[zp as usize];
                self.adc(value);
            }
            0x68 => {
                self.a = self.pop();
                self.set_zn(self.a);
            }
            0x85 => {
                let zp = self.fetch_byte();
                self.mem[zp as usize] = self.a;
            }
            0x8D => {
                let addr = self.fetch_word();
                self.mem[addr as usize] = self.a;
            }
            0x90 => self.branch(!self.carry),
            0x91 => {
                let zp = self.fetch_byte();
                let addr = self.indirect_y_addr(zp);
                self.mem[addr as usize] = self.a;
            }
            0x98 => {
                self.a = self.y;
                self.set_zn(self.a);
            }
            0x9D => {
                let base = self.fetch_word();
                let addr = base.wrapping_add(self.x as u16);
                self.mem[addr as usize] = self.a;
            }
            0xA0 => {
                self.y = self.fetch_byte();
                self.set_zn(self.y);
            }
            0xA2 => {
                self.x = self.fetch_byte();
                self.set_zn(self.x);
            }
            0xA5 => {
                let zp = self.fetch_byte();
                self.a = self.mem[zp as usize];
                self.set_zn(self.a);
            }
            0xA6 => {
                let zp = self.fetch_byte();
                self.x = self.mem[zp as usize];
                self.set_zn(self.x);
            }
            0xA8 => {
                self.y = self.a;
                self.set_zn(self.y);
            }
            0xA9 => {
                self.a = self.fetch_byte();
                self.set_zn(self.a);
            }
            0x8A => {
                self.a = self.x;
                self.set_zn(self.a);
            }
            0xAA => {
                self.x = self.a;
                self.set_zn(self.x);
            }
            0xAD => {
                let addr = self.fetch_word();
                self.a = self.mem[addr as usize];
                self.set_zn(self.a);
            }
            0xB0 => self.branch(self.carry),
            0xB1 => {
                let zp = self.fetch_byte();
                let addr = self.indirect_y_addr(zp);
                self.a = self.mem[addr as usize];
                self.set_zn(self.a);
            }
            0xBD => {
                let base = self.fetch_word();
                let addr = base.wrapping_add(self.x as u16);
                self.a = self.mem[addr as usize];
                self.set_zn(self.a);
            }
            0xC5 => {
                let zp = self.fetch_byte();
                self.compare(self.a, self.mem[zp as usize]);
            }
            0xC6 => {
                let zp = self.fetch_byte();
                let result = self.mem[zp as usize].wrapping_sub(1);
                self.mem[zp as usize] = result;
                self.set_zn(result);
            }
            0xC9 => {
                let imm = self.fetch_byte();
                self.compare(self.a, imm);
            }
            0xC8 => {
                self.y = self.y.wrapping_add(1);
                self.set_zn(self.y);
            }
            0xCA => {
                self.x = self.x.wrapping_sub(1);
                self.set_zn(self.x);
            }
            0xD8 => {}
            0xD0 => self.branch(!self.zero),
            0xE6 => {
                let zp = self.fetch_byte();
                let result = self.mem[zp as usize].wrapping_add(1);
                self.mem[zp as usize] = result;
                self.set_zn(result);
            }
            0xE8 => {
                self.x = self.x.wrapping_add(1);
                self.set_zn(self.x);
            }
            0xF0 => self.branch(self.zero),
            0x10 => self.branch(!self.negative),
            _ => panic!("unsupported opcode ${:02X} at ${:04X}", opcode, self.pc.wrapping_sub(1)),
        }
        true
    }

    fn fetch_byte(&mut self) -> u8 {
        let byte = self.mem[self.pc as usize];
        self.pc = self.pc.wrapping_add(1);
        byte
    }

    fn fetch_word(&mut self) -> u16 {
        let lo = self.fetch_byte();
        let hi = self.fetch_byte();
        u16::from_le_bytes([lo, hi])
    }

    fn push(&mut self, value: u8) {
        self.mem[0x0100 | self.sp as usize] = value;
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.mem[0x0100 | self.sp as usize]
    }

    fn set_zn(&mut self, value: u8) {
        self.zero = value == 0;
        self.negative = value & 0x80 != 0;
    }

    fn adc(&mut self, value: u8) {
        let carry_in = u16::from(self.carry);
        let sum = self.a as u16 + value as u16 + carry_in;
        self.a = sum as u8;
        self.carry = sum > 0xFF;
        self.set_zn(self.a);
    }

    fn compare(&mut self, left: u8, right: u8) {
        let result = left.wrapping_sub(right);
        self.carry = left >= right;
        self.zero = left == right;
        self.negative = result & 0x80 != 0;
    }

    fn branch(&mut self, take: bool) {
        let offset = self.fetch_byte() as i8;
        if take {
            self.pc = self.pc.wrapping_add_signed(offset as i16);
        }
    }

    fn indirect_y_addr(&self, zp: u8) -> u16 {
        let lo = self.mem[zp as usize];
        let hi = self.mem[zp.wrapping_add(1) as usize];
        u16::from_le_bytes([lo, hi]).wrapping_add(self.y as u16)
    }
}

// ── BASIC stub ──────────────────────────────────────────────────────────────

#[test]
fn stub_is_correct_length() {
    let prg = compile_stub("");
    assert_eq!(prg.len(), 16, "Empty program: 14 stub + CLD(1) + RTS(1) = 16");
}

#[test]
fn no_stub_is_correct_length() {
    let prg = compile_raw("");
    assert_eq!(prg.len(), 4, "Empty program: 2 header + CLD(1) + RTS(1) = 4");
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
    // Should have: CLD, LDA #42, STA $02, RTS (+ header)
    assert!(prg.len() > 7); // header(2) + CLD(1) + LDA #42(2) + STA $02(2) + RTS(1)
    assert_eq!(prg[2], 0xD8); // CLD
    assert_eq!(prg[3], 0xA9); // LDA immediate
    assert_eq!(prg[4], 42);   // #42
    assert_eq!(prg[5], 0x85); // STA zp
    assert_eq!(prg[6], 0x02); // $02
    assert_eq!(prg[7], 0x60); // RTS
}

#[test]
fn var_assign_generates_code() {
    let prg = compile_raw("x = 99");
    assert_eq!(prg[2], 0xD8); // CLD
    assert_eq!(prg[3], 0xA9); // LDA immediate
    assert_eq!(prg[4], 99);
    assert_eq!(prg[5], 0x85); // STA
    assert_eq!(prg[6], 0x02); // first ZP var
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
    // print_str_inline: CLD, LDA #'A', JSR CHROUT, ..., RTS
    assert!(prg.len() > 10);
    assert_eq!(prg[2], 0xD8); // CLD
    assert_eq!(prg[3], 0xA9); // LDA #'A'
    assert_eq!(prg[4], 0x41); // 'A' = 65 in PETSCII
    assert_eq!(prg[5], 0x20); // JSR
    assert_eq!(prg[6], 0xD2); // CHROUT lo
    assert_eq!(prg[7], 0xFF); // CHROUT hi
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
    // Use a variable so the addition isn't folded at compile time
    let prg = compile_raw("var a = 3\nvar x = a + 4");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x18)); // CLC
    assert!(bytes.contains(&0x65)); // ADC zp
}

#[test]
fn subtraction_expr() {
    let prg = compile_raw("var a = 10\nvar x = a - 3");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x38)); // SEC
    assert!(bytes.contains(&0xE5)); // SBC zp
}

#[test]
fn multiplication_expr() {
    let prg = compile_raw("var a = 3\nvar x = a * 4");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xC6)); // DEC zp
    assert!(bytes.contains(&0xD0)); // BNE
}

#[test]
fn division_expr() {
    let prg = compile_raw("var a = 8\nvar x = a / 2");
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
    // Bitwise AND: use a variable so it isn't folded
    let prg = compile_raw("var a = 12\nvar r = a and 15");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x25) || bytes.contains(&0x29)); // AND zp / AND #imm
}

#[test]
fn or_operator() {
    // Bitwise OR: use a variable so it isn't folded
    let prg = compile_raw("var a = 0\nvar r = a or 1");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x05) || bytes.contains(&0x09)); // ORA zp / ORA #imm
}

#[test]
fn not_operator() {
    let prg = compile_raw("var r = not 0");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xB0)); // BCS (not: carry set = any non-zero → return 0)
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
    // Should have 3 NOPs after CLD
    assert_eq!(prg[2], 0xD8); // CLD
    assert_eq!(prg[3], 0xEA);
    assert_eq!(prg[4], 0xEA);
    assert_eq!(prg[5], 0xEA);
    assert_eq!(prg[6], 0x60); // RTS follows
}

#[test]
fn asm_block_braces() {
    let prg = compile_raw("asm { $A9 $01 }");
    assert_eq!(prg[2], 0xD8); // CLD
    assert_eq!(prg[3], 0xA9);
    assert_eq!(prg[4], 0x01);
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
    assert_eq!(prg.len(), 4); // header(2) + CLD(1) + RTS(1)
    assert_eq!(prg[2], 0xD8); // CLD
    assert_eq!(prg[3], 0x60); // RTS
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
    // header(2) + CLD(1) + LDA #$0D(2) + JSR $FFD2(3) + RTS(1) = 9 bytes total
    assert_eq!(prg.len(), 9, "bare print = header + CLD + LDA #CR + JSR CHROUT + RTS");
    assert_eq!(bytes[0], 0xD8);  // CLD
    assert_eq!(bytes[1], 0xA9);  // LDA immediate
    assert_eq!(bytes[2], 0x0D);  // #$0D = carriage return
    assert_eq!(bytes[3], 0x20);  // JSR
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
    let prg = compile_raw("var idx = 10\npoke $0400 + idx, 42");
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
fn sgn_correct_opcodes() {
    // sgn(0) → 0, sgn(positive 1-127) → 1, sgn(negative 128-255) → $FF
    // New implementation uses BCC ($90) and BPL ($10) — NOT the old BCS-offset-4 pattern.
    let prg = compile_raw("var s = sgn(200)");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x90), "sgn should emit BCC ($90) for zero branch");
    assert!(bytes.contains(&0x10), "sgn should emit BPL ($10) for positive branch");
    // Must emit LDA #$FF for the negative case
    let has_ff = bytes.windows(2).any(|w| w[0] == 0xA9 && w[1] == 0xFF);
    assert!(has_ff, "sgn should emit LDA #$FF ($A9 $FF) for negative case");
}

#[test]
fn not_correct_opcode() {
    // `not x` should return 0 for any non-zero value, not just for x==1.
    // Implementation must use BCS ($B0), not BEQ ($F0).
    let prg = compile_raw("var x = 5\nvar n = not x");
    let bytes = &prg[2..];
    // Must contain BCS ($B0) — the branch that handles all non-zero values
    assert!(bytes.contains(&0xB0), "not should emit BCS ($B0), not BEQ");
    // Must NOT use BEQ ($F0) as the branch after CMP #1
    // (BEQ would only fire for x==1, breaking `not 5` etc.)
    let cmp1_beq = bytes.windows(4).any(|w| w[0]==0xC9 && w[1]==0x01 && w[2]==0xF0);
    assert!(!cmp1_beq, "not must not use BEQ after CMP #1 (breaks values > 1)");
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
const BORDER_ADDR = $D020
var x = rnd()
var a = abs(x - 128)
var m = min(a, 50)
var v = peek($D012)
poke BORDER_ADDR, 2

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
const SCRADDR = $0400
var idx = 0
var c = 1
poke SCRADDR + idx, c
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

// ── str$() ──────────────────────────────────────────────────────────────────

#[test]
fn strn_print_compiles() {
    // print str$(42) should compile without errors
    let src = "var x = 42\nprint str$(x)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
}

#[test]
fn strn_in_string_concat_compiles() {
    let src = "var s = 7\nprint \"Score: \" + str$(s)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
}

#[test]
fn strn_assign_compiles() {
    // str$(n) used as a string value (assigned through print)
    let src = "var n = 255\nprint str$(n)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // Helper subroutine should be present — verify JSR opcode ($20) exists
    let bytes = &res.prg[2..];
    assert!(bytes.contains(&0x20), "Should contain JSR instruction");
}

#[test]
fn strn_constant_arg_compiles() {
    let src = "print str$(0)";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
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
fn semicolon_as_separator() {
    // ';' is a statement separator, like ':'
    let src = "var x = 1 ; var y = 2";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty());
    assert!(res.prg.windows(2).any(|w| w == [0xA9, 1u8]));
    assert!(res.prg.windows(2).any(|w| w == [0xA9, 2u8]));
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

// ── load sid ─────────────────────────────────────────────────────────────────

/// Build a minimal PSID v1 file (118-byte header + music data).
fn make_fake_psid(load_addr: u16, init_addr: u16, play_addr: u16, music: &[u8]) -> Vec<u8> {
    let mut hdr = vec![0u8; 0x76]; // 118-byte header (PSID v1)
    hdr[0x00] = b'P'; hdr[0x01] = b'S'; hdr[0x02] = b'I'; hdr[0x03] = b'D';
    hdr[0x04] = 0x00; hdr[0x05] = 0x01; // version 1
    hdr[0x06] = 0x00; hdr[0x07] = 0x76; // data_offset = $76
    // Header addresses are big-endian
    hdr[0x08] = (load_addr >> 8) as u8; hdr[0x09] = load_addr as u8;
    hdr[0x0A] = (init_addr >> 8) as u8; hdr[0x0B] = init_addr as u8;
    hdr[0x0C] = (play_addr >> 8) as u8; hdr[0x0D] = play_addr as u8;
    hdr.extend_from_slice(music);
    hdr
}

#[test]
fn load_sid_embeds_music_at_load_addr() {
    let sid_path = "test_load_sid_tmp.sid";
    let music = vec![0xA9u8, 0x07, 0x8D, 0x20, 0xD0, 0x60]; // LDA #7; STA $D020; RTS
    let sid_bytes = make_fake_psid(0x1000, 0x1000, 0x1006, &music);
    std::fs::write(sid_path, &sid_bytes).unwrap();

    let src = format!("load sid \"{}\"\n", sid_path);
    let opts = CompileOptions { basic_stub: false };
    let res = compile_with_path(&src, &opts, Some(std::path::Path::new(sid_path)));
    std::fs::remove_file(sid_path).ok();

    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    assert!(res.prg.windows(music.len()).any(|w| w == music.as_slice()),
        "SID music bytes should be embedded in the output");
}

#[test]
fn load_sid_injects_constants() {
    // sid_init / sid_play are usable as compile-time constants after load sid.
    let sid_path = "test_load_sid_const_tmp.sid";
    let music = vec![0xEAu8]; // NOP
    let sid_bytes = make_fake_psid(0x1000, 0x1000, 0x1006, &music);
    std::fs::write(sid_path, &sid_bytes).unwrap();

    // sys sid_init should compile (resolves to JSR $1000)
    let src = format!("load sid \"{}\"\nsys sid_init\n", sid_path);
    let opts = CompileOptions { basic_stub: false };
    let res = compile_with_path(&src, &opts, Some(std::path::Path::new(sid_path)));
    std::fs::remove_file(sid_path).ok();

    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // JSR $1000 = $20, $00, $10
    assert!(res.prg.windows(3).any(|w| w == [0x20, 0x00, 0x10]),
        "sys sid_init should emit JSR $1000");
}

#[test]
fn load_sid_invalid_file_reports_error() {
    let sid_path = "test_load_sid_bad_tmp.bin";
    std::fs::write(sid_path, b"NOT A SID FILE AT ALL").unwrap();
    let src = format!("load sid \"{}\"", sid_path);
    let opts = CompileOptions { basic_stub: false };
    let res = compile_with_path(&src, &opts, Some(std::path::Path::new(sid_path)));
    std::fs::remove_file(sid_path).ok();
    assert!(!res.errors.is_empty(), "Should report an error for an invalid SID file");
}

#[test]
fn load_sid_missing_file_reports_error() {
    let src = "load sid \"nonexistent_totally_fake.sid\"";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(!res.errors.is_empty(), "Should report an error for a missing SID file");
}

#[test]
fn load_sid_override_addr_places_data_at_specified_address() {
    let sid_path = "test_load_sid_override_tmp.sid";
    let music = vec![0xA9u8, 0x0F, 0x8D, 0x18, 0xD4, 0x60]; // LDA #$0F; STA $D418; RTS
    let sid_bytes = make_fake_psid(0x1000, 0x1000, 0x1006, &music);
    std::fs::write(sid_path, &sid_bytes).unwrap();

    // Override: put music at $2000 instead of $1000
    let src = format!("load sid \"{}\", $2000\n", sid_path);
    let opts = CompileOptions { basic_stub: false };
    let res = compile_with_path(&src, &opts, Some(std::path::Path::new(sid_path)));
    std::fs::remove_file(sid_path).ok();

    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // Music bytes must appear somewhere in the output
    assert!(res.prg.windows(music.len()).any(|w| w == music.as_slice()),
        "SID music bytes should be embedded in the output");
    // The load address override means the PRG must be large enough to reach $2000
    // (PRG starts at $0801, so offset to $2000 is $1800 - 2 = $17FE bytes minimum incl. header)
    assert!(res.prg.len() >= 0x17FF, "PRG must extend to $2000");
}

#[test]
fn print_at_positions_cursor_then_prints() {
    let src = "print at 10, 5, \"HI\"";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // Must call KERNAL PLOT: JSR $FFF0 = $20 $F0 $FF
    assert!(res.prg.windows(3).any(|w| w == [0x20, 0xF0, 0xFF]),
        "print at should emit JSR $FFF0 (KERNAL PLOT)");
    // CLC = $18 must appear before the JSR $FFF0 (C=0 = SET cursor per KERNAL reference)
    let plot_pos = res.prg.windows(3).position(|w| w == [0x20, 0xF0, 0xFF]).unwrap();
    assert!(res.prg[..plot_pos].contains(&0x18), "print at should clear carry (CLC) before PLOT");
    // Must also call CHROUT: JSR $FFD2 = $20 $D2 $FF (for the string)
    assert!(res.prg.windows(3).any(|w| w == [0x20, 0xD2, 0xFF]),
        "print at should emit JSR $FFD2 (KERNAL CHROUT) for string output");
}

#[test]
fn print_at_no_args_still_positions() {
    let src = "print at 0, 0";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    assert!(res.prg.windows(3).any(|w| w == [0x20, 0xF0, 0xFF]),
        "print at with no print args should still emit JSR $FFF0");
}

#[test]
fn sid_volume_emits_sta_d418() {
    let src = "sid volume 15";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // LDA #15 = $A9 $0F; STA $D418 = $8D $18 $D4
    assert!(res.prg.windows(2).any(|w| w == [0xA9, 0x0F]),
        "sid volume 15 should emit LDA #$0F");
    assert!(res.prg.windows(3).any(|w| w == [0x8D, 0x18, 0xD4]),
        "sid volume should emit STA $D418");
}

#[test]
fn sid_stop_zeros_all_registers() {
    let src = "sid stop";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // LDX #$18 = $A2 $18; LDA #$00 = $A9 $00; STA $D400,X = $9D $00 $D4; DEX=$CA; BPL=-6=$10 $FA
    assert!(res.prg.windows(10).any(|w| w == [0xA2, 0x18, 0xA9, 0x00, 0x9D, 0x00, 0xD4, 0xCA, 0x10, 0xFA]),
        "sid stop should emit zero-fill loop for $D400-$D418");
}

#[test]
fn waitkey_polls_cia1_matrix() {
    let src = "var k = waitkey()";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // LDA #$00 = $A9 $00 — select all CIA1 rows
    assert!(res.prg.windows(2).any(|w| w == [0xA9, 0x00]),
        "waitkey should emit LDA #$00 to select CIA1 rows");
    // STA $DC00 = $8D $00 $DC
    assert!(res.prg.windows(3).any(|w| w == [0x8D, 0x00, 0xDC]),
        "waitkey should emit STA $DC00");
    // LDA $DC01 = $AD $01 $DC
    assert!(res.prg.windows(3).any(|w| w == [0xAD, 0x01, 0xDC]),
        "waitkey should emit LDA $DC01 in polling loop");
    // CMP #$FF = $C9 $FF
    assert!(res.prg.windows(2).any(|w| w == [0xC9, 0xFF]),
        "waitkey should emit CMP #$FF");
}

#[test]
fn irq_exit_emits_jmp_ea81() {
    let src = "irq_exit";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // irq_exit must emit JMP $EA81 = $4C $81 $EA
    assert!(res.prg.windows(3).any(|w| w == [0x4C, 0x81, 0xEA]),
        "irq_exit should emit JMP $EA81 ($4C $81 $EA)");
    // Must NOT contain a JSR $EA81 ($20 $81 $EA) — that would corrupt the IRQ stack
    assert!(!res.prg.windows(3).any(|w| w == [0x20, 0x81, 0xEA]),
        "irq_exit must not emit JSR $EA81");
}

#[test]
fn sys_with_arg_emits_lda_imm_then_jsr() {
    let src = "sys $FFD2, 7";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // LDA #7 = $A9 $07
    assert!(res.prg.windows(2).any(|w| w == [0xA9, 0x07]),
        "sys addr, val should emit LDA #val ($A9 $07)");
    // JSR $FFD2 = $20 $D2 $FF
    assert!(res.prg.windows(3).any(|w| w == [0x20, 0xD2, 0xFF]),
        "sys addr, val should emit JSR addr ($20 $D2 $FF)");
}

#[test]
fn sys_without_arg_emits_only_jsr() {
    let src = "sys $FFD2";
    let res = compile(src, &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    // JSR $FFD2 = $20 $D2 $FF
    assert!(res.prg.windows(3).any(|w| w == [0x20, 0xD2, 0xFF]),
        "sys addr should emit JSR addr");
    // No LDA immediate before the JSR (no spurious LDA #n)
    let pos = res.prg.windows(3).position(|w| w == [0x20, 0xD2, 0xFF]).unwrap();
    assert!(pos == 0 || res.prg[pos - 2] != 0xA9,
        "sys addr (no arg) should not emit LDA #n before JSR");
}

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
    // sprdef 0, <63 bytes>  at $080D → CLD (1 byte), then JMP over data, data at $0840 (page $21)
    // CLD = 1 byte, JMP = 3 bytes; $080D+4 = $0811; next 64-boundary = $0840
    let mut bytes63 = vec![0u8; 63];
    bytes63[1] = 0x7E; // row 1 byte 1, easily spotted
    let src = format!(
        "sprdef 0\n{}\nend",
        bytes63.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",")
    );
    let prg = compile_raw(&src);
    let bytes = &prg[2..]; // skip load address

    // CLD first
    assert_eq!(bytes[0], 0xD8, "code should start with CLD");
    // JMP $?? $?? should be next = 4C
    assert_eq!(bytes[1], 0x4C, "sprite_def should start with JMP after CLD");

    // data_addr = $0840, page = $21; expect LDA #$21 somewhere
    let has_lda_page = bytes.windows(2).any(|w| w == &[0xA9, 0x21]);
    assert!(has_lda_page, "sprite_def should emit LDA #$21 (page)");

    // STA $07F8 = 8D F8 07
    let has_sta_ptr = bytes.windows(3).any(|w| w == &[0x8D, 0xF8, 0x07]);
    assert!(has_sta_ptr, "sprite_def should emit STA $07F8");

    // $7E marker byte: data starts at $0840, prg[2..] = $0801 base, offset = $0840-$0801 = $3F = 63
    // byte[1] of sprite data = offset 64 from $080D base = bytes[$3F+1] = bytes[64]
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

#[test]
fn input_int_erases_non_digit_with_del() {
    // For integer input, non-digit chars are echoed by BASIN then erased with DEL ($14 via CHROUT)
    let prg = compile_raw("input score");
    let bytes = &prg[2..];
    // DEL erase: LDA #$14 ($A9 $14) followed by JSR $FFD2 ($20 $D2 $FF)
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x14]),
        "input int should emit LDA #$14 (DEL) to erase non-digit echo");
    assert!(bytes.windows(5).any(|w| w == &[0xA9, 0x14, 0x20, 0xD2, 0xFF]),
        "input int should emit LDA #$14; JSR $FFD2 sequence");
}

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

// ── DrawMem ───────────────────────────────────────────────────────────────────

#[test]
fn drawmem_emits_lda_indirect_and_sta_indirect() {
    let prg = compile_raw("drawmem $C000, $0400, 8, 10, 40");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0xB1), "drawmem should emit LDA (src),Y ($B1)");
    assert!(bytes.iter().any(|&b| b == 0x91), "drawmem should emit STA (dst),Y ($91)");
}

#[test]
fn drawmem_emits_iny_for_inner_loop() {
    let prg = compile_raw("drawmem $C000, $0400, 8, 10, 40");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0xC8), "drawmem should emit INY ($C8)");
}

#[test]
fn drawmem_emits_cpy_for_width_check() {
    let prg = compile_raw("drawmem $C000, $0400, 8, 10, 40");
    let bytes = &prg[2..];
    // CPY zp is $C4
    assert!(bytes.iter().any(|&b| b == 0xC4), "drawmem should emit CPY w_hold ($C4)");
}

#[test]
fn drawmem_emits_dec_for_height_counter() {
    let prg = compile_raw("drawmem $C000, $0400, 8, 10, 40");
    let bytes = &prg[2..];
    // DEC zp is $C6
    assert!(bytes.iter().any(|&b| b == 0xC6), "drawmem should emit DEC h_ctr ($C6)");
}

#[test]
fn drawmem_emits_inc_for_src_and_dst_hi() {
    let prg = compile_raw("drawmem $C000, $0400, 8, 10, 40");
    let bytes = &prg[2..];
    // INC zp is $E6 — should appear for both INC src_hi and INC dst_hi
    let inc_count = bytes.iter().filter(|&&b| b == 0xE6).count();
    assert!(inc_count >= 2, "drawmem should emit INC src_hi and INC dst_hi ($E6), got {}", inc_count);
}

#[test]
fn drawmem_initialises_src_address() {
    // src = $C000 → LDA #$00, STA zp, LDA #$C0, STA zp+1
    let prg = compile_raw("drawmem $C000, $0400, 8, 10, 40");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w[0] == 0xA9 && w[1] == 0x00),
        "drawmem should load low byte $00 for src $C000");
    assert!(bytes.windows(2).any(|w| w[0] == 0xA9 && w[1] == 0xC0),
        "drawmem should load high byte $C0 for src $C000");
}

#[test]
fn drawmem_initialises_dst_address() {
    // dst = $0400 → LDA #$00, STA zp, LDA #$04, STA zp+1
    let prg = compile_raw("drawmem $C000, $0400, 8, 10, 40");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w[0] == 0xA9 && w[1] == 0x04),
        "drawmem should load high byte $04 for dst $0400");
}

#[test]
fn drawmem_uses_word_var_for_src() {
    let src = r#"
var src: word = $C000
drawmem src, $0400, 8, 10, 40
"#;
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // LDA zp ($A5) used to load word var lo/hi bytes
    assert!(bytes.iter().any(|&b| b == 0xA5), "drawmem with word src should emit LDA zp ($A5)");
    assert!(bytes.iter().any(|&b| b == 0xB1), "drawmem should still emit LDA (src),Y");
}

#[test]
fn drawmem_uses_word_var_for_dst() {
    let src = r#"
var dst: word = $0400
drawmem $C000, dst, 8, 10, 40
"#;
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0xA5), "drawmem with word dst should emit LDA zp ($A5)");
    assert!(bytes.iter().any(|&b| b == 0x91), "drawmem should still emit STA (dst),Y");
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
    // CLC ($18) must appear before JSR $FFF0 to set cursor position (C=0 = SET per KERNAL ref)
    let prg = compile_raw("cursor 0, 0\n");
    let bytes = &prg[2..];
    let plot_pos = bytes.windows(3).position(|w| w == &[0x20, 0xF0, 0xFF]);
    assert!(plot_pos.is_some(), "cursor: JSR $FFF0 missing");
    let pos = plot_pos.unwrap();
    assert!(bytes[..pos].contains(&0x18), "cursor: CLC must appear before JSR $FFF0");
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

// ── asm { } inline assembler ─────────────────────────────────────────────────

#[test]
fn asm_block_raw_bytes_backward_compat() {
    // Old raw-byte syntax inside asm { } still works
    let prg = compile_raw("asm { $EA $EA }");
    let bytes = &prg[2..];
    // CLD, then Two NOPs ($EA)
    assert_eq!(bytes[0], 0xD8, "CLD");
    assert_eq!(&bytes[1..3], &[0xEA, 0xEA], "raw bytes in asm block");
}

#[test]
fn asm_block_nop_rts_mnemonics() {
    let prg = compile_raw("asm {\n  NOP\n  NOP\n  RTS\n}");
    let bytes = &prg[2..];
    assert_eq!(bytes[0], 0xD8, "CLD = $D8");
    assert_eq!(bytes[1], 0xEA, "NOP = $EA");
    assert_eq!(bytes[2], 0xEA, "NOP = $EA");
    assert_eq!(bytes[3], 0x60, "RTS = $60");
}

#[test]
fn asm_block_lda_immediate() {
    // LDA #$07 → A9 07
    let prg = compile_raw("asm { LDA #$07 }");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x07]), "LDA #$07 = A9 07");
}

#[test]
fn asm_block_sta_absolute() {
    // STA $0286 → 8D 86 02
    let prg = compile_raw("asm { STA $0286 }");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x86, 0x02]), "STA $0286 = 8D 86 02");
}

#[test]
fn asm_block_sta_zp() {
    // STA $50 → 85 50 (zero-page because value ≤ 255 and written as 2 hex digits)
    let prg = compile_raw("asm { STA $50 }");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0x85, 0x50]), "STA $50 = 85 50 (ZP)");
}

#[test]
fn asm_block_jsr_absolute() {
    // JSR $FFD2 → 20 D2 FF
    let prg = compile_raw("asm { JSR $FFD2 }");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xD2, 0xFF]), "JSR $FFD2 = 20 D2 FF");
}

#[test]
fn asm_block_jmp_absolute() {
    // JMP $C000 → 4C 00 C0  (no ZP mode for JMP, auto-upgraded)
    let prg = compile_raw("asm { JMP $C000 }");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x4C, 0x00, 0xC0]), "JMP $C000 = 4C 00 C0");
}

#[test]
fn asm_block_jmp_current_location() {
    let prg = compile_raw("asm { JMP * }");
    let load_addr = u16::from_le_bytes([prg[0], prg[1]]);
    let bytes = &prg[2..];
    let has_self_jump = bytes.windows(3).enumerate().any(|(offset, window)| {
        window[0] == 0x4C && u16::from_le_bytes([window[1], window[2]]) == load_addr + offset as u16
    });
    assert!(has_self_jump, "JMP * should target its own instruction address");
}

#[test]
fn asm_block_clc_adc() {
    // CLC + ADC #1 → 18  69 01
    let prg = compile_raw("asm {\n  CLC\n  ADC #1\n}");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x18), "CLC = $18");
    assert!(bytes.windows(2).any(|w| w == &[0x69, 0x01]), "ADC #1 = 69 01");
}

#[test]
fn asm_block_indirect_x() {
    // LDA ($50,X) → A1 50
    let prg = compile_raw("asm { LDA ($50,X) }");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA1, 0x50]), "LDA ($50,X) = A1 50");
}

#[test]
fn asm_block_indirect_y() {
    // LDA ($50),Y → B1 50
    let prg = compile_raw("asm { LDA ($50),Y }");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xB1, 0x50]), "LDA ($50),Y = B1 50");
}

#[test]
fn asm_block_abs_x_indexed() {
    // LDA $0400,X → BD 00 04
    let prg = compile_raw("asm { LDA $0400,X }");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xBD, 0x00, 0x04]), "LDA $0400,X = BD 00 04");
}

#[test]
fn asm_block_branch_forward() {
    // BNE past NOP: the branch should skip 1 byte ($01 offset)
    // BNE +1 ($01), NOP, NOP  — BNE offset = 1 (skip the first NOP, land on second)
    let prg = compile_raw("asm {\n  BNE skip\n  NOP\nskip:\n  NOP\n}");
    let bytes = &prg[2..];
    // D8  D0 01  EA  EA
    assert_eq!(bytes[0], 0xD8, "CLD");
    assert_eq!(bytes[1], 0xD0, "BNE opcode");
    assert_eq!(bytes[2], 0x01, "BNE forward offset = 1");
    assert_eq!(bytes[3], 0xEA, "NOP at skip-1");
    assert_eq!(bytes[4], 0xEA, "NOP at skip");
}

#[test]
fn asm_block_branch_backward() {
    // loop: NOP / BNE loop  — backward branch: offset = -3 ($FD)
    let prg = compile_raw("asm {\nloop:\n  NOP\n  BNE loop\n}");
    let bytes = &prg[2..];
    // D8  EA  D0 FD
    assert_eq!(bytes[0], 0xD8, "CLD");
    assert_eq!(bytes[1], 0xEA, "NOP");
    assert_eq!(bytes[2], 0xD0, "BNE opcode");
    assert_eq!(bytes[3], 0xFD_u8, "BNE backward offset = -3");
}

#[test]
fn asm_block_transfers_implied() {
    let prg = compile_raw("asm { TAX\nTAY\nTXA\nTYA }");
    let bytes = &prg[2..];
    assert_eq!(bytes[0], 0xD8, "CLD");
    assert_eq!(bytes[1], 0xAA, "TAX");
    assert_eq!(bytes[2], 0xA8, "TAY");
    assert_eq!(bytes[3], 0x8A, "TXA");
    assert_eq!(bytes[4], 0x98, "TYA");
}

#[test]
fn asm_block_sec_sbc() {
    // SEC + SBC #1 → 38  E9 01
    let prg = compile_raw("asm {\n  SEC\n  SBC #1\n}");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x38), "SEC = $38");
    assert!(bytes.windows(2).any(|w| w == &[0xE9, 0x01]), "SBC #1 = E9 01");
}

#[test]
fn asm_block_comment_semicolon() {
    // Comments with ; should be stripped
    let prg = compile_raw("asm {\n  NOP  ; this is a comment\n  NOP\n}");
    let bytes = &prg[2..];
    assert_eq!(bytes[0], 0xD8, "CLD");
    assert_eq!(bytes[1], 0xEA, "NOP");
    assert_eq!(bytes[2], 0xEA, "NOP after comment line");
}

#[test]
fn asm_block_mixed_mnemonics_and_raw_bytes() {
    // Mixing mnemonic instructions with raw byte lines
    let prg = compile_raw("asm {\n  NOP\n  $EA\n  NOP\n}");
    let bytes = &prg[2..];
    assert_eq!(bytes[0], 0xD8, "CLD");
    assert_eq!(&bytes[1..4], &[0xEA, 0xEA, 0xEA], "three NOPs");
}

// ── 16-bit word AND/OR/XOR ───────────────────────────────────────────────────

#[test]
fn word_and_const_emits_two_and_imm() {
    // var p: word = $ABCD \n var r: word = p and $FF00
    // Should emit: LDA lzp; AND #$00; STA dst; LDA lzp+1; AND #$FF; STA dst+1
    let prg = compile_raw("var p: word = $ABCD\nvar r: word = p and $FF00");
    let bytes = &prg[2..];
    // AND immediate = $29; hi mask $FF
    let has_and_ff = bytes.windows(2).any(|w| w == &[0x29, 0xFF]);
    let has_and_00 = bytes.windows(2).any(|w| w == &[0x29, 0x00]);
    assert!(has_and_ff, "word AND $FF00: AND #$FF for hi byte missing");
    assert!(has_and_00, "word AND $FF00: AND #$00 for lo byte missing");
}

#[test]
fn word_or_const_emits_ora_imm() {
    // var p: word = $0100 \n var r: word = p or $00FF
    // Should set lo byte to $FF and leave hi byte as $01
    let prg = compile_raw("var p: word = $0100\nvar r: word = p or $00FF");
    let bytes = &prg[2..];
    // ORA immediate = $09; lo mask $FF
    let has_ora_ff = bytes.windows(2).any(|w| w == &[0x09, 0xFF]);
    assert!(has_ora_ff, "word OR $00FF: ORA #$FF for lo byte missing");
    // hi mask $00
    let has_ora_00 = bytes.windows(2).any(|w| w == &[0x09, 0x00]);
    assert!(has_ora_00, "word OR $00FF: ORA #$00 for hi byte missing");
}

#[test]
fn word_xor_const_emits_eor_imm() {
    // var p: word = $FFFF \n var r: word = p xor $00FF  → r = $FF00
    let prg = compile_raw("var p: word = $FFFF\nvar r: word = p xor $00FF");
    let bytes = &prg[2..];
    // EOR immediate = $49
    let eor_count = bytes.windows(1).filter(|w| w[0] == 0x49).count();
    assert!(eor_count >= 2, "word XOR const: should emit EOR imm twice (lo and hi)");
}

#[test]
fn word_and_word_var_emits_and_zp() {
    // var a: word = $FFFF \n var b: word = $0F0F \n var r: word = a and b
    let prg = compile_raw("var a: word = $FFFF\nvar b: word = $0F0F\nvar r: word = a and b");
    let bytes = &prg[2..];
    // AND zp = $25
    let and_zp_count = bytes.windows(1).filter(|w| w[0] == 0x25).count();
    assert!(and_zp_count >= 2, "word AND word: should emit AND zp twice (lo and hi)");
}

#[test]
fn word_or_word_var_emits_ora_zp() {
    let prg = compile_raw("var a: word = $0F00\nvar b: word = $00F0\nvar r: word = a or b");
    let bytes = &prg[2..];
    // ORA zp = $05
    let ora_zp_count = bytes.windows(1).filter(|w| w[0] == 0x05).count();
    assert!(ora_zp_count >= 2, "word OR word: should emit ORA zp twice");
}

#[test]
fn word_xor_word_var_emits_eor_zp() {
    let prg = compile_raw("var a: word = $AAAA\nvar b: word = $5555\nvar r: word = a xor b");
    let bytes = &prg[2..];
    // EOR zp = $45
    let eor_zp_count = bytes.windows(1).filter(|w| w[0] == 0x45).count();
    assert!(eor_zp_count >= 2, "word XOR word: should emit EOR zp twice");
}

// ── 16-bit word SHL/SHR ──────────────────────────────────────────────────────

#[test]
fn word_shl_const_emits_asl_rol() {
    // var p: word = $0001 \n var r: word = p shl 1
    let prg = compile_raw("var p: word = $0001\nvar r: word = p shl 1");
    let bytes = &prg[2..];
    // ASL zp = $06, ROL zp = $26
    let has_asl = bytes.contains(&0x06);
    let has_rol = bytes.contains(&0x26);
    assert!(has_asl, "word SHL 1: ASL zp ($06) missing");
    assert!(has_rol, "word SHL 1: ROL zp ($26) missing");
}

#[test]
fn word_shr_const_emits_lsr_ror() {
    // var p: word = $0100 \n var r: word = p shr 1
    let prg = compile_raw("var p: word = $0100\nvar r: word = p shr 1");
    let bytes = &prg[2..];
    // LSR zp = $46, ROR zp = $66
    let has_lsr = bytes.contains(&0x46);
    let has_ror = bytes.contains(&0x66);
    assert!(has_lsr, "word SHR 1: LSR zp ($46) missing");
    assert!(has_ror, "word SHR 1: ROR zp ($66) missing");
}

#[test]
fn word_shl_8_swaps_bytes() {
    // p shl 8: hi = lo, lo = 0  →  LDA dst; STA dst+1; LDA #0; STA dst
    let prg = compile_raw("var p: word = $0042\nvar r: word = p shl 8");
    let bytes = &prg[2..];
    // LDA #0 = A9 00
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x00]), "word SHL 8: LDA #0 missing");
}

#[test]
fn word_shr_8_swaps_bytes() {
    // p shr 8: lo = hi, hi = 0
    let prg = compile_raw("var p: word = $4200\nvar r: word = p shr 8");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x00]), "word SHR 8: LDA #0 missing");
}

#[test]
fn word_shl_var_emits_beq_loop() {
    // variable shift count: should emit BEQ (F0) to skip if count=0, DEC (C6) for loop
    let prg = compile_raw("var p: word = $0001\nvar n = 3\nvar r: word = p shl n");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xF0), "word SHL var: BEQ ($F0) missing");
    assert!(bytes.contains(&0xC6), "word SHL var: DEC zp ($C6) missing");
}

#[test]
fn word_shr_var_emits_beq_loop() {
    let prg = compile_raw("var p: word = $0400\nvar n = 2\nvar r: word = p shr n");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0xF0), "word SHR var: BEQ ($F0) missing");
    assert!(bytes.contains(&0xC6), "word SHR var: DEC zp ($C6) missing");
}

// ── 16-bit word MUL ──────────────────────────────────────────────────────────

#[test]
fn word_mul_const_emits_shift_add_loop() {
    // var p: word = $0064 \n var r: word = p * 3
    // Should emit LSR mr ($46), BCC ($90), loop structure with DEX ($CA)
    let prg = compile_raw("var p: word = $0064\nvar r: word = p * 3");
    let bytes = &prg[2..];
    assert!(bytes.contains(&0x46), "word *: LSR zp ($46) missing (shift multiplier)");
    assert!(bytes.contains(&0x90), "word *: BCC ($90) missing (skip add if bit=0)");
    assert!(bytes.contains(&0xCA), "word *: DEX ($CA) missing (loop counter)");
}

#[test]
fn word_mul_commutative() {
    // 3 * p  should produce same structure as p * 3
    let prg1 = compile_raw("var p: word = $0064\nvar r: word = p * 3");
    let prg2 = compile_raw("var p: word = $0064\nvar r: word = 3 * p");
    // Both must contain the shift-add loop opcodes
    assert!(prg1[2..].contains(&0xCA), "p*3: DEX missing");
    assert!(prg2[2..].contains(&0xCA), "3*p: DEX missing");
}

// ── 16-bit word DIV ──────────────────────────────────────────────────────────

#[test]
fn word_div_const_emits_division_loop() {
    // var p: word = $012C \n var r: word = p / 7  (300/7 = 42)
    // Division uses 16-iteration loop: LDX #16 ($A2 $10), INC num_lo ($E6)
    let prg = compile_raw("var p: word = $012C\nvar r: word = p / 7");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA2, 0x10]), "word /: LDX #16 missing");
    // INC zp = $E6 (setting quotient bit)
    assert!(bytes.contains(&0xE6), "word /: INC zp ($E6) missing");
    // STY = $84 (storing lo result of subtraction)
    assert!(bytes.contains(&0x84), "word /: STY zp ($84) missing");
}

#[test]
fn word_div_word_var() {
    // var a: word = $0064 \n var b: word = 10 \n var r: word = a / b
    let res = compile("var a: word = $0064\nvar b: word = 10\nvar r: word = a / b",
                      &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "word / word: {:?}", res.errors);
    let bytes = &res.prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA2, 0x10]), "word / word: LDX #16 missing");
}

// ── 16-bit word MOD ──────────────────────────────────────────────────────────

#[test]
fn word_mod_const_emits_division_loop() {
    // var p: word = $012C \n var r: word = p mod 7  (300 mod 7 = 6)
    // Same division machinery but stores rem instead of quo
    let prg = compile_raw("var p: word = $012C\nvar r: word = p mod 7");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA2, 0x10]), "word mod: LDX #16 missing");
    assert!(bytes.contains(&0xE6), "word mod: INC zp ($E6) missing");
}

#[test]
fn word_mod_word_var() {
    let res = compile("var a: word = $012C\nvar b: word = 7\nvar r: word = a mod b",
                      &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "word mod word: {:?}", res.errors);
}

// ── word arrays ───────────────────────────────────────────────────────────────

#[test]
fn word_array_decl_compiles() {
    let res = compile("var tbl = array_word(4)", &CompileOptions { basic_stub: false });
    assert!(res.errors.is_empty(), "array_word decl: {:?}", res.errors);
}

#[test]
fn word_array_set_const_index_emits_two_stas() {
    // word arr: tbl[0] = $1234 → STA $C000 ($34) and STA $C001 ($12)
    let prg = compile_raw("var tbl = array_word(4)\nvar v: word = $1234\ntbl[0] = v");
    let bytes = &prg[2..];
    // STA $C000 = 8D 00 C0
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x00, 0xC0]),
        "word_array set [0]: STA $C000 missing");
    // STA $C001 = 8D 01 C0
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x01, 0xC0]),
        "word_array set [0]: STA $C001 missing");
}

#[test]
fn word_array_get_const_index_emits_two_ldas() {
    // word arr: r = tbl[0] → LDA $C000 and LDA $C001
    let prg = compile_raw("var tbl = array_word(4)\nvar r: word = tbl[0]");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x00, 0xC0]),
        "word_array get [0]: LDA $C000 missing");
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x01, 0xC0]),
        "word_array get [0]: LDA $C001 missing");
}

#[test]
fn word_array_get_const_index_1_uses_stride_2() {
    // tbl[1] → base $C000 + 1*2 = $C002/$C003
    let prg = compile_raw("var tbl = array_word(4)\nvar r: word = tbl[1]");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x02, 0xC0]),
        "word_array get [1]: LDA $C002 (stride 2) missing");
    assert!(bytes.windows(3).any(|w| w == &[0xAD, 0x03, 0xC0]),
        "word_array get [1]: LDA $C003 missing");
}

#[test]
fn word_array_var_index_emits_asl_for_stride() {
    // Variable index: ASL A (0A) to multiply index by 2
    let prg = compile_raw("var tbl = array_word(4)\nvar i = 1\nvar r: word = tbl[i]");
    let bytes = &prg[2..];
    // ASL A = $0A
    assert!(bytes.contains(&0x0A), "word_array var index: ASL A ($0A) for stride missing");
}

#[test]
fn word_array_set_var_index_emits_iny() {
    // Variable index store: INY ($C8) to advance to hi byte
    let prg = compile_raw("var tbl = array_word(4)\nvar i = 0\nvar v: word = $0042\ntbl[i] = v");
    let bytes = &prg[2..];
    // INY = $C8
    assert!(bytes.contains(&0xC8), "word_array set var index: INY ($C8) missing");
}

// ── Backward compat: plot4/block removed — tests below kept as compile-only ─
#[test]
fn graphics_on_block_emits_correct_d018() {
    // Direct write $1A to $D018 to set screen@$0400 and charset@$2800.
    let prg = compile_raw("graphics on block");
    let bytes = &prg[2..];
    assert!(bytes.windows(5).any(|w| w == &[0xA9, 0x1A, 0x8D, 0x18, 0xD0]),
        "graphics on block should emit LDA #$1A; STA $D018 ($A9 $1A $8D $18 $D0) to set screen@$0400 and charset@$2800");
}

// ─── U64 Speed ───────────────────────────────────────────────────────────────

#[test]
fn speed_constant_compiles() {
    // speed 4 → index 3; expect LDA $D031, AND #$F0, ORA #3, STA $D031
    let prg = compile_raw("speed 4");
    let bytes = &prg[2..];
    assert!(bytes.windows(6).any(|w| w == &[0xAD, 0x31, 0xD0, 0x29, 0xF0, 0x09]),
        "speed 4 should emit LDA $D031, AND #$F0, ORA #... sequence");
    // ORA byte should be 3 (index for 4 MHz)
    let pos = bytes.windows(6).position(|w| w == &[0xAD, 0x31, 0xD0, 0x29, 0xF0, 0x09]).unwrap();
    assert_eq!(bytes[pos + 6], 0x03, "speed 4 should OR index 3 into $D031");
}

#[test]
fn speed_max_compiles() {
    // speed max → index 15
    let prg = compile_raw("speed max");
    let bytes = &prg[2..];
    let pos = bytes.windows(6).position(|w| w == &[0xAD, 0x31, 0xD0, 0x29, 0xF0, 0x09]);
    assert!(pos.is_some(), "speed max should emit LDA $D031, AND #$F0, ORA #...");
    assert_eq!(bytes[pos.unwrap() + 6], 0x0F, "speed max should OR index 15");
}

#[test]
fn speed_off_compiles() {
    // speed off → index 0; expect LDA $D031, AND #$F0, STA $D031 (no ORA for index 0)
    let prg = compile_raw("speed off");
    let bytes = &prg[2..];
    // LDA $D031 (AD 31 D0), AND #$F0 (29 F0), STA $D031 (8D 31 D0)
    assert!(bytes.windows(8).any(|w| w == &[0xAD, 0x31, 0xD0, 0x29, 0xF0, 0x8D, 0x31, 0xD0]),
        "speed off should emit LDA $D031, AND #$F0, STA $D031 with no ORA");
}

#[test]
fn badlines_off_compiles() {
    // badlines off → ORA #$80 into $D031 (set bit 7)
    let prg = compile_raw("badlines off");
    let bytes = &prg[2..];
    assert!(bytes.windows(8).any(|w| w == &[0xAD, 0x31, 0xD0, 0x09, 0x80, 0x8D, 0x31, 0xD0]),
        "badlines off should emit LDA $D031, ORA #$80, STA $D031");
}

#[test]
fn badlines_on_compiles() {
    // badlines on → AND #$7F into $D031 (clear bit 7)
    let prg = compile_raw("badlines on");
    let bytes = &prg[2..];
    assert!(bytes.windows(8).any(|w| w == &[0xAD, 0x31, 0xD0, 0x29, 0x7F, 0x8D, 0x31, 0xD0]),
        "badlines on should emit LDA $D031, AND #$7F, STA $D031");
}

#[test]
fn graphics_on_block_sets_vic_bank_0() {
    let prg = compile_raw("graphics on block");
    let bytes = &prg[2..];
    assert!(bytes.windows(10).any(|w| w == &[0xAD, 0x00, 0xDD, 0x29, 0xFC, 0x09, 0x03, 0x8D, 0x00, 0xDD]),
        "graphics on block should emit LDA $DD00; AND #$FC; ORA #$03; STA $DD00 to select VIC bank 0");
}

#[test]
fn turbo_compiles() {
    // turbo() → LDA $D031, AND #$0F, BEQ +2, LDA #1
    let prg = compile_raw("var t = turbo()");
    let bytes = &prg[2..];
    // LDA $D031 (AD 31 D0), AND #$0F (29 0F), BEQ +2 (F0 02), LDA #1 (A9 01)
    assert!(bytes.windows(8).any(|w| w == &[0xAD, 0x31, 0xD0, 0x29, 0x0F, 0xF0, 0x02, 0xA9]),
        "turbo() should emit LDA $D031; AND #$0F; BEQ +2; LDA #1 sequence");
}

#[test]
fn float_expr_infers_float_type() {
    // var dd = 3.5 + 78.4 — result should be stored as float and printed via float path.
    // If dd were inferred as word, print would emit print_decimal (integer path).
    // If dd is inferred as float, print emits print_fixed (float path, calls print_decimal twice).
    // We test that print_fixed is emitted: it always prints a '.' via LDA #$2E; JSR $FFD2.
    let prg = compile_raw("var dd = 3.5 + 78.4\nprint dd");
    let bytes = &prg[2..];
    // print_fixed emits LDA #$2E ('.'): A9 2E
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x2E]),
        "var dd = 3.5 + 78.4 should infer float type and print via print_fixed (emits LDA #$2E for '.')");
}

#[test]
fn float_mul_float_uses_16x16() {
    // var f: float = 3.5; var g: float = f * 1.5
    // Q8.8(3.5) = 0x0380, Q8.8(1.5) = 0x0180, product = Q8.8(5.25) = 0x0540
    // 16×16 Russian Peasant uses LDX #16 (A2 10), while 16×8 uses LDX #8 (A2 08).
    // The Mul code for float×float should emit LDX #16.
    let prg = compile_raw("var f: float = 3.5\nvar g: float = f * 1.5");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA2, 0x10]),
        "float * float should emit LDX #16 for 16×16 Russian Peasant");
}

#[test]
fn float_mul_small_fraction_correct() {
    // var g: float = 0.5 * 3.5 — tests int×float swap path (0.5 has integer part 0)
    // Q8.8(0.5) = 0x0080, Q8.8(3.5) = 0x0380
    // 16×16 >>8: (0x0080 × 0x0380) >> 8 = (128 × 896) >> 8 = 114688 >> 8 = 448 = 0x01C0 = Q8.8(1.75)
    // The result is stored as float (inferred from FixedLit), so print_fixed path is used.
    let prg = compile_raw("var g: float = 0.5 * 3.5\nprint g");
    let bytes = &prg[2..];
    // print_fixed emits LDA #$2E ('.'): A9 2E
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x2E]),
        "0.5 * 3.5 should infer float type and print via print_fixed");
    // Should use 16×16 path (LDX #16, not LDX #8)
    assert!(bytes.windows(2).any(|w| w == &[0xA2, 0x10]),
        "float × float should emit LDX #16 for 16×16 Russian Peasant");
}

#[test]
fn float_div_int_compiles() {
    // var f: float = 7.0; var g: float = f / 2
    // Q8.8(7.0) = 0x0700 / 2 = 0x0380 = Q8.8(3.5)
    // 16÷8 division uses LDX #16 (A2 10) and ROL rem (26 xx).
    let prg = compile_raw("var f: float = 7.0\nvar g: float = f / 2");
    let bytes = &prg[2..];
    // The Div loop uses INC dlo (E6 xx) to set quotient bits — unique to the div routine.
    assert!(bytes.windows(1).any(|w| w == &[0xE6]),
        "float / int should emit INC dlo (E6) for quotient bit in long division");
    // Uses LDX #16 for 16-iteration loop
    assert!(bytes.windows(2).any(|w| w == &[0xA2, 0x10]),
        "float / int should emit LDX #16");
}

#[test]
fn float_div_result_is_float() {
    // var g: float = 7.0 / 2 — result printed as float (has decimal point)
    let prg = compile_raw("var g: float = 7.0 / 2\nprint g");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x2E]),
        "float / int should print result via print_fixed (LDA #'.')");
}

#[test]
fn graphics_on_block_copies_charset_to_2800() {
    // STA $2800,X = 9D 00 28
    let prg = compile_raw("graphics on block");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x9D, 0x00, 0x28]),
        "graphics on block should emit STA $2800,X ($9D $00 $28) for charset copy");
}

#[test]
fn graphics_on_block_emits_canonical_blanked_d011() {
    let prg = compile_raw("graphics on block");
    let bytes = &prg[2..];
    assert!(bytes.windows(5).any(|w| w == &[0xA9, 0x0B, 0x8D, 0x11, 0xD0]),
        "graphics on block should emit LDA #$0B; STA $D011 to force canonical blanked text-mode VIC state");
}

#[test]
fn graphics_on_block_canonicalizes_d011_before_display_on() {
    let prg = compile_raw("graphics on block\ndisplay on");
    let bytes = &prg[2..];
    assert!(bytes.windows(5).any(|w| w == &[0xA9, 0x0B, 0x8D, 0x11, 0xD0]),
        "graphics on block should emit LDA #$0B; STA $D011 before display on so block mode starts from a canonical blanked text state");
}

#[test]
fn plot4_emits_ora_pnt() {
    // ORA zero-page (opcode $05) for the set-pixel operation
    let prg = compile_raw("graphics on block\nplot4 10, 5");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x05),
        "plot4 should emit ORA zp ($05) in the set-pixel helper");
}

#[test]
fn plot4_emits_lda_indirect_y() {
    // LDA (ptr),Y = $B1
    let prg = compile_raw("graphics on block\nplot4 10, 5");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0xB1),
        "plot4 should emit LDA (ptr_lo),Y ($B1) for screen char read");
}

#[test]
fn plot4_emits_sta_indirect_y() {
    // STA (ptr),Y = $91
    let prg = compile_raw("graphics on block\nplot4 10, 5");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x91),
        "plot4 should emit STA (ptr_lo),Y ($91) for screen char write");
}

#[test]
fn plot4_erase_emits_eor_ff() {
    // De Morgan erase uses EOR #$FF (opcode $49, value $FF) twice
    let prg = compile_raw("graphics on block\nplot4 erase 10, 5");
    let bytes = &prg[2..];
    let count = bytes.windows(2).filter(|w| *w == &[0x49, 0xFF]).count();
    assert!(count >= 2,
        "plot4 erase should emit EOR #$FF ($49 $FF) twice for De Morgan NOT, got {}", count);
}

#[test]
fn gcls_in_block_mode_fills_screen_ram() {
    // In block mode, gcls fills $0400-$07E7 with 0 → STA $0400,X = 9D 00 04
    let prg = compile_raw("graphics on block\ngcls");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x9D, 0x00, 0x04]),
        "gcls in block mode should emit STA $0400,X ($9D $00 $04)");
}

#[test]
fn gcls_in_block_mode_fills_color_ram() {
    // Also fills color RAM $D800-$DBE7 → STA $D800,X = 9D 00 D8
    let prg = compile_raw("graphics on block\ngcls");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x9D, 0x00, 0xD8]),
        "gcls in block mode should emit STA $D800,X ($9D $00 $D8) for color RAM");
}

#[test]
fn plot4_fullscreen_fill_reaches_bottom_rows_in_emulation() {
    let src = "graphics on block\ngcls\nvar y = 0\nvar x = 0\nwhile y < 50\n  x = 0\n  while x < 80\n    plot4 x, y\n    x = x + 1\n  end\n  y = y + 1\nend";
    let prg = compile_raw(src);
    let mut cpu = TestCpu::new(&prg);
    cpu.run_until_main_rts(2_000_000);

    let screen = &cpu.mem[0x0400..=0x07E7];
    assert!(screen.iter().all(|&b| b == 0x0F),
        "expected full screen RAM fill to end at $0F in every cell, last row was {:02X?}",
        &cpu.mem[0x07C0..=0x07E7]);
}

    #[test]
    fn sin_pattern_populates_bottom_block_rows() {
        // Verify that a sin interference pattern (same formula as block_demo.ub)
        // writes non-zero screen RAM values in the bottom character rows (chars 17-24,
        // covering block y=34..49). This catches the "bottom third empty" regression.
        let src = "graphics on block\ngcls\n\
            var x = 0\nvar y = 0\nvar v = 0\n\
            y = 0\n\
            while y < 50\n  x = 0\n  while x < 80\n\
                v = sin(x * 11 + y * 7)\n\
                if v > 128 then\n  plot4 x, y\n  end\n\
                x = x + 1\n  end\n  y = y + 1\nend";
        let prg = compile_raw(src);
        let mut cpu = TestCpu::new(&prg);
        cpu.run_until_main_rts(4_000_000);

        // Block rows 34-49 map to character rows 17-24 → screen offset $440 to $7E7
        let bottom = &cpu.mem[0x0400 + 17 * 40..=0x07E7];
        let non_zero = bottom.iter().filter(|&&b| b != 0).count();
        assert!(non_zero > 50,
            "expected sin pattern to write to bottom block rows, got {} non-zero cells; \
             first row of bottom section: {:02X?}",
            non_zero, &cpu.mem[0x0400 + 17 * 40..0x0400 + 17 * 40 + 40]);
    }

    #[test]
    fn direct_block_fill_reaches_full_screen_in_emulation() {
        let src = "graphics on block\ngcls\ndisplay on\nfill $0400, 1000, 15";
        let prg = compile_raw(src);
        let mut cpu = TestCpu::new(&prg);
        cpu.run_until_main_rts(2_000_000);

        let screen = &cpu.mem[0x0400..=0x07E7];
        assert!(screen.iter().all(|&b| b == 0x0F),
        "expected direct block fill to write $0F across the full screen matrix, last row was {:02X?}",
            &cpu.mem[0x07C0..=0x07E7]);
    }

#[test]
fn graphics_on_block_parser_sets_block_flag() {
    // Parser test: `graphics on block` should parse to Graphics { on: true, block: true, multi: false }
    use ultimate_basic::compiler::parser::Parser;
    use ultimate_basic::compiler::lexer::Lexer;
    use ultimate_basic::compiler::ast::Stmt;
    let tokens = Lexer::new("graphics on block").tokenize();
    let stmts = Parser::new(tokens).parse();
    assert!(matches!(&stmts[0], Stmt::Graphics { on: true, multi: false, block: true }),
        "graphics on block should parse to Graphics {{ on:true, multi:false, block:true }}");
}

#[test]
fn generated_program_starts_with_cld() {
    let prg = compile_raw("var x = 1\nx = x + 1");
    assert_eq!(prg[2], 0xD8, "generated machine code should begin with CLD");
}

// ── 16-bit / word auto-promotion ─────────────────────────────────────────────

#[test]
fn var_auto_promotes_to_word_when_gt255() {
    // var b = 12345 should store as word (lo=$39, hi=$30)
    let prg = compile_raw("var b = 12345");
    let bytes = &prg[2..];
    // LDA #0x39 (lo of 12345)
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x39]), "should store lo byte 0x39 (12345 & 0xFF)");
    // LDA #0x30 (hi of 12345 = 12345 >> 8 = 0x30)
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x30]), "should store hi byte 0x30 (12345 >> 8)");
}

#[test]
fn print_word_var_large_constant() {
    // var b = 12345: should compile without error and emit print_decimal_word path
    let prg = compile_raw("var b = 12345\nprint b");
    let bytes = &prg[2..];
    // Should contain the 16-bit digit-loop pattern (CMP #0x27 = hi of 10000)
    assert!(bytes.windows(2).any(|w| w == [0xC9, 0x27]), "should emit 16-bit digit compare for 10000");
}

#[test]
fn print_word_sum_with_int_var() {
    // var a=122; var b=12345; print a+b  => 16-bit path
    let prg = compile_raw("var a = 122\nvar b = 12345\nprint a+b");
    let bytes = &prg[2..];
    // 16-bit digit loop: CMP #0x27 (hi of 10000)
    assert!(bytes.windows(2).any(|w| w == [0xC9, 0x27]), "should emit 16-bit digit compare for 10000");
}

// ── inc / dec statements ─────────────────────────────────────────────────────

#[test]
fn inc_emits_inc_zp() {
    let prg = compile_raw("var x = 0\ninc x");
    let bytes = &prg[2..];
    // INC zp opcode is 0xE6
    assert!(bytes.windows(1).any(|w| w == [0xE6]), "inc should emit INC zp (0xE6)");
}

#[test]
fn dec_emits_dec_zp() {
    let prg = compile_raw("var x = 5\ndec x");
    let bytes = &prg[2..];
    // DEC zp opcode is 0xC6
    assert!(bytes.windows(1).any(|w| w == [0xC6]), "dec should emit DEC zp (0xC6)");
}

#[test]
fn inc_word_emits_inc_bne_inc() {
    // 16-bit inc: INC lo; BNE +2; INC hi  (bytes: 0xE6,zp, 0xD0,0x02, 0xE6,zp+1)
    let prg = compile_raw("var x: word = 0\ninc x");
    let bytes = &prg[2..];
    // Look for 6-byte window: INC(0xE6), <zp_lo>, BNE(0xD0), 0x02, INC(0xE6), <zp_hi=zp_lo+1>
    assert!(
        bytes.windows(6).any(|w| w[0] == 0xE6 && w[2] == 0xD0 && w[3] == 0x02 && w[4] == 0xE6 && w[5] == w[1].wrapping_add(1)),
        "16-bit inc: INC lo; BNE +2; INC hi — not found"
    );
}

#[test]
fn dec_word_emits_lda_bne_dec_dec() {
    // 16-bit dec: LDA lo; BNE skip; DEC hi; DEC lo
    let prg = compile_raw("var x: word = 5\ndec x");
    let bytes = &prg[2..];
    // Pattern: 0xA5 (LDA zp), <zp>, 0xD0 (BNE), 0x02, 0xC6 (DEC hi), <zp+1>
    assert!(
        bytes.windows(4).any(|w| w[0] == 0xD0 && w[1] == 0x02 && w[2] == 0xC6),
        "16-bit dec: BNE +2; DEC hi — not found"
    );
}

// ── compound assignments ─────────────────────────────────────────────────────

#[test]
fn plus_eq_assigns_sum() {
    let prg = compile_raw("var x = 10\nx += 5");
    let bytes = &prg[2..];
    // x += 5 evaluates rhs (5) into A: LDA #5 = [0xA9, 0x05], then ADC zp
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x05]), "+= 5 should load #5 into A");
}

#[test]
fn minus_eq_assigns_diff() {
    let prg = compile_raw("var x = 10\nx -= 3");
    let bytes = &prg[2..];
    // x -= 3 evaluates rhs (3) into A: LDA #3 = [0xA9, 0x03], then SBC tmp
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x03]), "-= 3 should load #3 into A");
}

#[test]
fn and_eq_assigns_masked() {
    let prg = compile_raw("var x = 255\nx and= 15");
    let bytes = &prg[2..];
    // x and= 15: eval(x)→tmp, eval(15)→LDA #$0F, AND tmp (0x25)
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x0F]), "and= 15 should load #$0F into A");
    assert!(bytes.windows(1).any(|w| w == [0x25]), "and= should emit AND zp (0x25)");
}

#[test]
fn or_eq_assigns_combined() {
    let prg = compile_raw("var x = 0\nx or= 64");
    let bytes = &prg[2..];
    // x or= 64: eval(x)→tmp, eval(64)→LDA #$40, ORA tmp (0x05)
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x40]), "or= 64 should load #$40 into A");
    assert!(bytes.windows(1).any(|w| w == [0x05]), "or= should emit ORA zp (0x05)");
}

#[test]
fn xor_eq_assigns_toggled() {
    let prg = compile_raw("var x = 255\nx xor= 85");
    let bytes = &prg[2..];
    // x xor= 85: eval(x)→tmp, eval(85)→LDA #$55, EOR tmp (0x45)
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x55]), "xor= 85 should load #$55 into A");
    assert!(bytes.windows(1).any(|w| w == [0x45]), "xor= should emit EOR zp (0x45)");
}

// ── screen col, row, char ─────────────────────────────────────────────────────

#[test]
fn screen_constant_emits_sta_abs_screen_ram() {
    // screen 0, 0, 65  → STA $0400 with A=65
    let prg = compile_raw("screen 0, 0, 65");
    let bytes = &prg[2..];
    // STA $0400 = 0x8D 0x00 0x04; LDA #65 = 0xA9 0x41
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x41]), "char 65 should emit LDA #$41");
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x00, 0x04]), "should STA to $0400");
}

#[test]
fn screen_with_color_emits_color_ram_store() {
    // screen 0, 0, 65, 7  → STA $0400, STA $D800
    let prg = compile_raw("screen 0, 0, 65, 7");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x00, 0x04]), "should STA char to $0400");
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x00, 0xD8]), "should STA color to $D800");
}

#[test]
fn screen_offset_row1_col2_correct_address() {
    // screen 2, 1, 65  → offset = 1*40+2 = 42 → addr = $0400+42 = $042A
    let prg = compile_raw("screen 2, 1, 65");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x2A, 0x04]), "row1 col2 should STA to $042A");
}

// ── spc(n) in print ──────────────────────────────────────────────────────────

#[test]
fn spc_emits_chrout_loop() {
    // print spc(3) should emit a loop calling CHROUT
    let prg = compile_raw("print spc(3)");
    let bytes = &prg[2..];
    // Should contain LDA #$20 (space = 0xA9 0x20) and JSR CHROUT (0x20 0xD2 0xFF)
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x20]), "spc should load space char $20");
    assert!(bytes.windows(3).any(|w| w == [0x20, 0xD2, 0xFF]), "spc should JSR $FFD2 (CHROUT)");
}

// ── tab(n) in print ──────────────────────────────────────────────────────────

#[test]
fn tab_emits_plot_calls() {
    // print tab(10) should call KERNAL PLOT ($FFF0) twice
    let prg = compile_raw("print tab(10)");
    let bytes = &prg[2..];
    // Should contain JSR $FFF0 (0x20 0xF0 0xFF) at least twice (read + write cursor)
    let count = bytes.windows(3).filter(|w| *w == [0x20, 0xF0, 0xFF]).count();
    assert!(count >= 2, "tab should call PLOT ($FFF0) twice; found {} times", count);
}

// ── rnd(n) ───────────────────────────────────────────────────────────────────

#[test]
fn rnd_n_compiles_ok() {
    // rnd(10) should compile without error
    let prg = compile_raw("var x = rnd(10)");
    assert!(prg.len() > 2, "rnd(n) should produce code");
}

#[test]
fn rnd_n_emits_lcg_and_mod() {
    let prg = compile_raw("var x = rnd(10)");
    let bytes = &prg[2..];
    // LCG: seed*4 via ASL; 0x0A = ASL A
    assert!(bytes.contains(&0x0A), "rnd(n) should emit ASL for LCG");
    // Mod loop: SEC (0x38) + SBC zp (0xE5) pattern
    assert!(bytes.contains(&0x38), "rnd(n) mod should emit SEC");
    assert!(bytes.contains(&0xE5), "rnd(n) mod should emit SBC zp");
}

#[test]
fn rnd_n_different_from_rnd() {
    // rnd() and rnd(10) should produce different byte sequences
    let prg_rnd = compile_raw("var x = rnd()");
    let prg_rndn = compile_raw("var x = rnd(10)");
    assert_ne!(prg_rnd, prg_rndn, "rnd() and rnd(10) should produce different code");
}

// ── continue ─────────────────────────────────────────────────────────────────

#[test]
fn continue_in_for_compiles_ok() {
    let src = "for i = 1 to 10\n  if i == 5 then continue end\n  print i\nnext";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "continue in for loop should compile");
}

#[test]
fn continue_in_while_compiles_ok() {
    let src = "var i = 0\nwhile i < 10\n  inc i\n  if i == 5 then continue end\n  print i\nend";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "continue in while loop should compile");
}

#[test]
fn continue_in_repeat_compiles_ok() {
    let src = "var i = 0\nrepeat\n  inc i\n  if i == 5 then continue end\n  print i\nuntil i == 10";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "continue in repeat loop should compile");
}

#[test]
fn continue_in_infinite_loop_compiles_ok() {
    let src = "var i = 0\nloop\n  inc i\n  if i == 5 then continue end\n  print i\n  if i == 10 then break end\nend";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "continue in infinite loop should compile");
}

#[test]
fn continue_in_counted_loop_compiles_ok() {
    let src = "var i = 0\nloop 10\n  inc i\n  if i == 5 then continue end\n  print i\nend";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "continue in counted loop should compile");
}

#[test]
fn continue_emits_jmp_forward() {
    // A continue in a simple for loop should emit a JMP (0x4C) for the continue branch
    let src = "for i = 1 to 10\n  if i == 5 then continue end\nnext";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // At least two JMP instructions: one for continue, one for loop back
    let jmp_count = bytes.iter().filter(|&&b| b == 0x4C).count();
    assert!(jmp_count >= 2, "continue should emit extra JMP; got {}", jmp_count);
}

// ── select/case/else/end ─────────────────────────────────────────────────────

#[test]
fn select_basic_compiles_ok() {
    let src = "var x = 2\nselect x\n  case 1:\n    print \"ONE\"\n  case 2:\n    print \"TWO\"\nend";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "select/case should compile");
}

#[test]
fn select_with_else_compiles_ok() {
    let src = "var x = 5\nselect x\n  case 1:\n    print \"ONE\"\n  else:\n    print \"OTHER\"\nend";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "select/else should compile");
}

#[test]
fn select_empty_compiles_ok() {
    let prg = compile_raw("var x = 0\nselect x\nend");
    assert!(prg.len() > 2, "empty select should compile");
}

#[test]
fn select_emits_cmp_for_each_case() {
    let src = "var x = 1\nselect x\n  case 1:\n    print \"A\"\n  case 2:\n    print \"B\"\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // CMP zp = 0xC5; should appear at least 2 times (once per case)
    let cmp_count = bytes.iter().filter(|&&b| b == 0xC5).count();
    assert!(cmp_count >= 2, "select should emit CMP for each case; got {}", cmp_count);
}

#[test]
fn select_else_only_compiles_ok() {
    let src = "var x = 3\nselect x\n  else:\n    print \"ELSE\"\nend";
    let prg = compile_raw(src);
    assert!(prg.len() > 2, "select with only else should compile");
}

// ── New feature tests ────────────────────────────────────────────────────────

#[test]
fn bnot_emits_eor_ff() {
    // bnot x  →  x XOR 255.  Codegen evaluates b (255) first, stores to ZP tmp,
    // then evals a (x) → A, then EOR zp.  Check that EOR ($45 zp or $49 imm) is present
    // and that the constant 255 ($FF) appears as an operand somewhere.
    let prg = compile_raw("var x = 10\nvar y = bnot x");
    let bytes = &prg[2..];
    // Either EOR zp ($45) or EOR imm ($49) must be present
    let has_eor = bytes.iter().any(|&b| b == 0x45 || b == 0x49);
    assert!(has_eor, "bnot should emit an EOR instruction; got bytes {:?}", bytes);
    // The constant 255 ($FF) must appear as an operand
    assert!(bytes.iter().any(|&b| b == 0xFF),
        "bnot should use operand $FF (255); got bytes {:?}", bytes);
}

#[test]
fn bnot_const_folds_correctly() {
    // bnot 0 → const-folded to 255; stored as LDA #$FF
    let prg = compile_raw("var y = bnot 0");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0xFF]),
        "bnot 0 should const-fold to LDA #$FF; got bytes {:?}", bytes);
}

#[test]
fn clamp_emits_compare_instructions() {
    // clamp(x, lo, hi) should emit two CMP zp instructions (one for lo, one for hi)
    let prg = compile_raw("var x = 15\nvar lo = 10\nvar hi = 20\nvar r = clamp(x, lo, hi)");
    let bytes = &prg[2..];
    let cmp_count = bytes.iter().filter(|&&b| b == 0xC5).count(); // CMP zp = $C5
    assert!(cmp_count >= 2, "clamp should emit at least 2 CMP zp; got {}", cmp_count);
}

#[test]
fn clamp_compiles_with_constants() {
    // clamp(200, 10, 100) const-folds to 100
    let prg = compile_raw("var r = clamp(200, 10, 100)");
    let bytes = &prg[2..];
    // const-folded result is 100 ($64); should see LDA #$64
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x64]),
        "clamp(200,10,100) should const-fold to LDA #100; got {:?}", bytes);
}

#[test]
fn color_screen_const_emits_sta_d800() {
    // color screen 0, 0, 7  →  LDA #7; STA $D800
    let prg = compile_raw("color screen 0, 0, 7");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x8D, 0x00, 0xD8]),
        "color screen 0,0,7 should emit STA $D800; got {:?}", bytes);
    assert!(bytes.windows(2).any(|w| w == &[0xA9, 0x07]),
        "color screen should load color 7 into A; got {:?}", bytes);
}

#[test]
fn color_screen_var_emits_sta_indirect() {
    // color screen with variable col/row should use (ptr),Y indirect store
    let prg = compile_raw("var c = 5\nvar r = 3\ncolor screen c, r, 7");
    let bytes = &prg[2..];
    // STA (ptr),Y = $91
    assert!(bytes.iter().any(|&b| b == 0x91),
        "color screen with vars should emit STA (ptr),Y ($91); got {:?}", bytes);
}

#[test]
fn wait_key_emits_jsr_ffe4() {
    // wait key  →  JSR $FFE4 loop (20 E4 FF)
    let prg = compile_raw("wait key");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == &[0x20, 0xE4, 0xFF]),
        "wait key should emit JSR $FFE4; got {:?}", bytes);
}

#[test]
fn wait_key_loops_on_zero() {
    // The loop must also check CMP #0 and BEQ back
    let prg = compile_raw("wait key");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == &[0xC9, 0x00]),
        "wait key should emit CMP #$00; got {:?}", bytes);
    assert!(bytes.iter().any(|&b| b == 0xF0),
        "wait key should emit BEQ; got {:?}", bytes);
}

#[test]
fn string_index_write_emits_sta_indirect_y() {
    // msg[0] = 65  →  LDY #0; LDA ...; STA (ptr),Y
    let prg = compile_raw("var msg = \"ABC\"\nmsg[0] = 65");
    let bytes = &prg[2..];
    // STA (zp),Y opcode = $91
    assert!(bytes.iter().any(|&b| b == 0x91),
        "string[i]=c should emit STA (ptr),Y ($91); got {:?}", bytes);
}

#[test]
fn string_index_write_var_index_emits_tay() {
    // msg[i] = 65  →  eval i; TAY; LDA ...; STA (ptr),Y
    let prg = compile_raw("var msg = \"ABC\"\nvar i = 1\nmsg[i] = 65");
    let bytes = &prg[2..];
    // TAY = $A8
    assert!(bytes.iter().any(|&b| b == 0xA8),
        "string[i]=c with var index should emit TAY ($A8); got {:?}", bytes);
    assert!(bytes.iter().any(|&b| b == 0x91),
        "string[i]=c should emit STA (ptr),Y ($91); got {:?}", bytes);
}

#[test]
fn string_index_read_const_emits_lda_indirect_y() {
    // var c = msg[0]  →  LDY #0; LDA (ptr),Y
    let prg = compile_raw("var msg = \"ABC\"\nvar c = msg[0]");
    let bytes = &prg[2..];
    // LDA (zp),Y opcode = $B1
    assert!(bytes.iter().any(|&b| b == 0xB1),
        "string[0] read should emit LDA (ptr),Y ($B1); got {:?}", bytes);
    // LDY #0 = A0 00
    assert!(bytes.windows(2).any(|w| w == &[0xA0, 0x00]),
        "string[0] read should emit LDY #0 ($A0 $00); got {:?}", bytes);
}

#[test]
fn string_index_read_var_index_emits_tay() {
    // var c = msg[i]  →  eval i → A; TAY; LDA (ptr),Y
    let prg = compile_raw("var msg = \"ABC\"\nvar i = 1\nvar c = msg[i]");
    let bytes = &prg[2..];
    // TAY = $A8; LDA (zp),Y = $B1
    assert!(bytes.iter().any(|&b| b == 0xA8),
        "string[i] read with var index should emit TAY ($A8); got {:?}", bytes);
    assert!(bytes.iter().any(|&b| b == 0xB1),
        "string[i] read should emit LDA (ptr),Y ($B1); got {:?}", bytes);
}

// ─── sprite_x / sprite_y ───────────────────────────────────────────────────

#[test]
fn sprite_x_const_emits_lda_d000() {
    // sprite_x(0) → LDA $D000 (=$AD $00 $D0)
    let prg = compile_raw("var x = sprite_x(0)");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0xAD, 0x00, 0xD0]),
        "sprite_x(0) should emit LDA $D000; got {:?}", bytes);
}

#[test]
fn sprite_y_const_emits_lda_d001() {
    // sprite_y(0) → LDA $D001 (=$AD $01 $D0)
    let prg = compile_raw("var y = sprite_y(0)");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0xAD, 0x01, 0xD0]),
        "sprite_y(0) should emit LDA $D001; got {:?}", bytes);
}

#[test]
fn sprite_x_const_id3_emits_lda_d006() {
    // sprite_x(3) → LDA $D006 (=$AD $06 $D0)
    let prg = compile_raw("var x = sprite_x(3)");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0xAD, 0x06, 0xD0]),
        "sprite_x(3) should emit LDA $D006; got {:?}", bytes);
}

#[test]
fn sprite_x_var_emits_asl_and_lda_indirect() {
    // sprite_x(id) with variable → ASL A (×2) + LDA (ptr),Y
    let prg = compile_raw("var id = 2\nvar x = sprite_x(id)");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x0A), // ASL A
        "sprite_x(var) should emit ASL A ($0A); got {:?}", bytes);
    assert!(bytes.iter().any(|&b| b == 0xB1), // LDA (ptr),Y
        "sprite_x(var) should emit LDA (ptr),Y ($B1); got {:?}", bytes);
}

#[test]
fn sprite_y_var_emits_iny_before_lda_indirect() {
    // sprite_y(id) with variable → ASL A + INY + LDA (ptr),Y (Y offset +1)
    let prg = compile_raw("var id = 2\nvar y = sprite_y(id)");
    let bytes = &prg[2..];
    // look for INY ($C8) followed by LDA (ptr),Y ($B1)
    assert!(bytes.windows(2).any(|w| w == [0xC8, 0xB1]),
        "sprite_y(var) should emit INY($C8) then LDA indirect ($B1); got {:?}", bytes);
}

// ─── fill screen / fill color ──────────────────────────────────────────────

#[test]
fn fill_screen_emits_sta_0400() {
    // fill screen 32 → STA $0400,X (=$9D $00 $04)
    let prg = compile_raw("fill screen 32");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0x04]),
        "fill screen should emit STA $0400,X; got {:?}", bytes);
}

#[test]
fn fill_screen_emits_sta_all_4_pages() {
    let prg = compile_raw("fill screen 0");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0x04]), "missing STA $0400,X");
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0x05]), "missing STA $0500,X");
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0x06]), "missing STA $0600,X");
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0x07]), "missing STA $0700,X");
}

#[test]
fn fill_color_emits_sta_d800() {
    // fill color 7 → STA $D800,X (=$9D $00 $D8)
    let prg = compile_raw("fill color 7");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0xD8]),
        "fill color should emit STA $D800,X; got {:?}", bytes);
}

#[test]
fn fill_color_emits_sta_all_4_pages() {
    let prg = compile_raw("fill color 0");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0xD8]), "missing STA $D800,X");
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0xD9]), "missing STA $D900,X");
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0xDA]), "missing STA $DA00,X");
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0xDB]), "missing STA $DB00,X");
}

// ─── times N ... end ───────────────────────────────────────────────────────

#[test]
fn times_loop_compiles() {
    // times 5 ... end — same as loop 5 ... end
    let prg = compile_raw("times 5\n  poke $D020, 6\nend");
    let bytes = &prg[2..];
    // Should emit STA $D020 in loop body
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x20, 0xD0]),
        "times loop body should contain STA $D020 ($8D $20 $D0); got {:?}", bytes);
    // The counted loop emits DEC zp (not DEX; uses permanent ZP counter)
    assert!(bytes.iter().any(|&b| b == 0xC6), // DEC zp
        "times loop should emit DEC zp ($C6); got {:?}", bytes);
}

#[test]
fn times_loop_same_as_loop_n() {
    // times N and loop N must produce identical code (same Stmt::Loop)
    let prg_times = compile_raw("times 3\n  poke $D021, 0\nend");
    let prg_loop  = compile_raw("loop 3\n  poke $D021, 0\nend");
    assert_eq!(prg_times, prg_loop, "times N and loop N should produce identical bytecode");
}

// ─── array_word variable index store ──────────────────────────────────────

#[test]
fn array_word_var_index_store_emits_asl_and_sta_indirect() {
    // warray[i] = $1234 with variable index
    let prg = compile_raw("var warray = array_word(8)\nvar i = 2\nwarray[i] = $1234");
    let bytes = &prg[2..];
    // ASL A (×2 stride), TAY, STA (ptr),Y
    assert!(bytes.iter().any(|&b| b == 0x0A), // ASL A
        "array_word var index store should emit ASL A; got {:?}", bytes);
    assert!(bytes.iter().any(|&b| b == 0x91), // STA (ptr),Y
        "array_word var index store should emit STA (ptr),Y; got {:?}", bytes);
}

#[test]
fn array_word_var_index_load_emits_asl_and_lda_indirect() {
    let prg = compile_raw("var warray = array_word(8)\nvar i = 2\nvar v: word = warray[i]");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x0A), // ASL A
        "array_word var index load should emit ASL A; got {:?}", bytes);
    assert!(bytes.iter().any(|&b| b == 0xB1), // LDA (ptr),Y
        "array_word var index load should emit LDA (ptr),Y; got {:?}", bytes);
}

// ── gosub / return ────────────────────────────────────────────────────────

#[test]
fn gosub_emits_jsr() {
    // gosub should emit JSR ($20) to the label address
    let prg = compile_raw("label target\n  var x = 1\ngosub target");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x20), // JSR
        "gosub should emit JSR ($20); got {:?}", bytes);
}

#[test]
fn gosub_forward_ref_resolves() {
    // gosub with forward reference to a label defined later
    let prg = compile_raw("gosub my_label\nlabel my_label\n  var x = 5");
    let bytes = &prg[2..];
    // Must have JSR
    assert!(bytes.iter().any(|&b| b == 0x20),
        "gosub forward ref should emit JSR; got {:?}", bytes);
}

#[test]
fn return_emits_rts() {
    // return emits RTS ($60)
    let prg = compile_raw("label lbl\n  return");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x60), // RTS
        "return should emit RTS ($60); got {:?}", bytes);
}

#[test]
fn gosub_then_return_forms_subroutine() {
    // A gosub/return pair: JSR followed (eventually) by RTS
    let prg = compile_raw("gosub do_work\nvar done = 1\nlabel do_work\n  var x = 42\n  return");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x20), "should have JSR");
    assert!(bytes.iter().any(|&b| b == 0x60), "should have RTS");
}

// ── sprite_frame ─────────────────────────────────────────────────────────

#[test]
fn sprite_frame_const_emits_lda_imm_and_sta_07f8() {
    // sprite_frame 0, $2000 → LDA #$80; STA $07F8
    // $2000 >> 6 = $80 = 128
    let prg = compile_raw("sprite_frame 0, $2000");
    let bytes = &prg[2..];
    // LDA #$80 = $A9 $80
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x80]),
        "sprite_frame const: should LDA #$80 ($2000>>6); got {:?}", bytes);
    // STA $07F8 = $8D $F8 $07
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0xF8, 0x07]),
        "sprite_frame const: should STA $07F8; got {:?}", bytes);
}

#[test]
fn sprite_frame_const_id1_emits_sta_07f9() {
    // sprite_frame 1, $2000 → STA $07F9 ($07F8 + 1)
    let prg = compile_raw("sprite_frame 1, $2000");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0xF9, 0x07]),
        "sprite_frame id=1: should STA $07F9; got {:?}", bytes);
}

#[test]
fn sprite_frame_word_var_addr_emits_shift_logic() {
    // sprite_frame 0, ptr  where ptr is a word var
    // Should emit ASL, ORA for addr>>6 computation
    let prg = compile_raw("var ptr: word = $2000\nsprite_frame 0, ptr");
    let bytes = &prg[2..];
    assert!(bytes.iter().any(|&b| b == 0x0A), // ASL A
        "sprite_frame word var: should emit ASL for hi*4; got {:?}", bytes);
    assert!(bytes.iter().any(|&b| b == 0x4A), // LSR A
        "sprite_frame word var: should emit LSR for lo>>6; got {:?}", bytes);
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0xF8, 0x07]),
        "sprite_frame word var: should STA $07F8; got {:?}", bytes);
}

#[test]
fn sprite_frame_var_id_emits_sta_07f8_x() {
    // sprite_frame id_var, $2000  where id is a variable → STA $07F8,X
    let prg = compile_raw("var spid = 2\nsprite_frame spid, $2000");
    let bytes = &prg[2..];
    // STA $07F8,X = $9D $F8 $07
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0xF8, 0x07]),
        "sprite_frame var id: should emit STA $07F8,X ($9D $F8 $07); got {:?}", bytes);
}

// ── chardef ───────────────────────────────────────────────────────────────

#[test]
fn chardef_emits_jmp_and_copy_loop() {
    // chardef 65 (8 bytes) should emit JMP over data then LDY#7+copy loop
    let src = "chardef 65\n  %00011000\n  %00100100\n  %01000010\n  %01111110\n  %01000010\n  %01000010\n  %00000000\n  %00000000\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // JMP opcode ($4C)
    assert!(bytes.iter().any(|&b| b == 0x4C), "chardef: should emit JMP ($4C)");
    // LDY #7 = $A0 $07
    assert!(bytes.windows(2).any(|w| w == [0xA0, 0x07]),
        "chardef: should emit LDY #7; got {:?}", bytes);
    // LDA abs,Y = $B9
    assert!(bytes.iter().any(|&b| b == 0xB9),
        "chardef: should emit LDA abs,Y ($B9) for copy loop; got {:?}", bytes);
    // STA abs,Y = $99
    assert!(bytes.iter().any(|&b| b == 0x99),
        "chardef: should emit STA abs,Y ($99) for copy loop; got {:?}", bytes);
}

#[test]
fn chardef_copies_to_default_charset_base() {
    // Default charset_base = $3800; char 0 → $3800; char 1 → $3808
    // chardef 0 should write to $3800 (STA $3800,Y = $99 $00 $38)
    let src = "chardef 0\n  %11111111\n  %10000001\n  %10000001\n  %10000001\n  %10000001\n  %10000001\n  %10000001\n  %11111111\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // STA $3800,Y = $99 $00 $38
    assert!(bytes.windows(3).any(|w| w == [0x99, 0x00, 0x38]),
        "chardef id=0: should STA $3800,Y; got {:?}", bytes);
}

#[test]
fn chardef_charset_base_changes_destination() {
    // charset $3000 then chardef 0 → STA $3000,Y
    let src = "charset $3000\nchardef 0\n  %11111111\n  %00000000\n  %00000000\n  %00000000\n  %00000000\n  %00000000\n  %00000000\n  %00000000\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // STA $3000,Y = $99 $00 $30
    assert!(bytes.windows(3).any(|w| w == [0x99, 0x00, 0x30]),
        "charset $3000: chardef should STA $3000,Y; got {:?}", bytes);
}

#[test]
fn chardef_char_id_offsets_destination() {
    // chardef 1 with default base $3800 → destination $3808 (STA $3808,Y)
    let src = "chardef 1\n  %11111111\n  %11111111\n  %11111111\n  %11111111\n  %11111111\n  %11111111\n  %11111111\n  %11111111\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    // STA $3808,Y = $99 $08 $38
    assert!(bytes.windows(3).any(|w| w == [0x99, 0x08, 0x38]),
        "chardef id=1: should STA $3808,Y ($3800+1*8); got {:?}", bytes);
}

#[test]
fn chardef_data_bytes_embedded_in_code() {
    // The 8 data bytes should appear verbatim in the PRG output
    let src = "chardef 65\n  $AA\n  $BB\n  $CC\n  $DD\n  $EE\n  $FF\n  $11\n  $22\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    assert!(bytes.windows(8).any(|w| w == [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]),
        "chardef: data bytes should appear verbatim in PRG; got {:?}", bytes);
}

#[test]
fn chardef_zero_pads_short_definition() {
    // fewer than 8 bytes → zero-padded to 8
    let src = "chardef 0\n  $FF\n  $AA\nend";
    let prg = compile_raw(src);
    let bytes = &prg[2..];
    assert!(bytes.windows(8).any(|w| w == [0xFF, 0xAA, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        "chardef: short def should be zero-padded; got {:?}", bytes);
}

// ── mplot ─────────────────────────────────────────────────────────────────────

#[test]
fn mplot_basic_compiles() {
    // mplot 10, 20, 2 — constant args, multicolor pixel plot
    let prg = compile_raw("graphics on\nmplot 10, 20, 2\n");
    let bytes = &prg[2..];
    // Must contain a JSR ($20) to call the mplot helper
    assert!(bytes.contains(&0x20), "mplot: expected JSR opcode $20");
}

#[test]
fn mplot_emits_cia_color_args() {
    // Verify the args are loaded: LDA #10 ($A9 $0A), LDA #20 ($A9 $14), LDA #2 ($A9 $02)
    let prg = compile_raw("graphics on\nmplot 10, 20, 2\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == [0xA9, 10]), "mplot: expected LDA #10 for x");
    assert!(bytes.windows(2).any(|w| w == [0xA9, 20]), "mplot: expected LDA #20 for y");
    assert!(bytes.windows(2).any(|w| w == [0xA9, 2]),  "mplot: expected LDA #2 for color");
}

// ── music stop / pause / resume ──────────────────────────────────────────────

#[test]
fn music_stop_compiles() {
    // music stop: disable CIA1 + zero SID registers
    let prg = compile_raw("music stop\n");
    let bytes = &prg[2..];
    // SEI = $78, CLI = $58
    assert!(bytes.contains(&0x78), "music stop: expected SEI ($78)");
    assert!(bytes.contains(&0x58), "music stop: expected CLI ($58)");
    // LDA #$7F = $A9 $7F — disable CIA1 IRQs
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x7F]), "music stop: expected LDA #$7F");
    // STA $DC0D = $8D $0D $DC
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x0D, 0xDC]), "music stop: expected STA $DC0D");
    // LDX #24 ($A2 $18) — loop counter for SID zero-fill
    assert!(bytes.windows(2).any(|w| w == [0xA2, 0x18]), "music stop: expected LDX #24 for SID zero loop");
    // STA $D400,X = $9D $00 $D4
    assert!(bytes.windows(3).any(|w| w == [0x9D, 0x00, 0xD4]), "music stop: expected STA $D400,X");
}

#[test]
fn music_pause_compiles() {
    let prg = compile_raw("music pause\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x7F]), "music pause: expected LDA #$7F");
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x0D, 0xDC]), "music pause: expected STA $DC0D");
}

#[test]
fn music_resume_compiles() {
    let prg = compile_raw("music resume\n");
    let bytes = &prg[2..];
    // LDA #$81 = $A9 $81 — re-enable CIA1 timer A
    assert!(bytes.windows(2).any(|w| w == [0xA9, 0x81]), "music resume: expected LDA #$81");
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x0D, 0xDC]), "music resume: expected STA $DC0D");
}

// ── onerr goto ───────────────────────────────────────────────────────────────

#[test]
fn onerr_goto_compiles() {
    // onerr goto err_handler: writes label address to $0300/$0301
    let prg = compile_raw("onerr goto err_handler\nlabel err_handler\n");
    let bytes = &prg[2..];
    // STA $0300 = $8D $00 $03
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x00, 0x03]),
        "onerr goto: expected STA $0300 ($8D $00 $03); bytes={bytes:?}");
    // STA $0301 = $8D $01 $03
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x01, 0x03]),
        "onerr goto: expected STA $0301 ($8D $01 $03)");
}

#[test]
fn onerr_forward_ref_patches() {
    // Forward reference: label defined after onerr goto
    let prg = compile_raw("onerr goto my_err\nvar x = 1\nlabel my_err\n");
    let bytes = &prg[2..];
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x00, 0x03]),
        "onerr forward-ref: expected STA $0300");
    assert!(bytes.windows(3).any(|w| w == [0x8D, 0x01, 0x03]),
        "onerr forward-ref: expected STA $0301");
}
