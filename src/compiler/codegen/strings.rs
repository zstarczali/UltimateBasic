//! Runtime strings (1.6.3).
//!
//! String values are pointers to null-terminated PETSCII. Literals live inline
//! in the code; everything computed at runtime (`a + b`, `left$`, `right$`,
//! `mid$`, numbers turned into text, `s = t` copies) is built into an 80-char
//! buffer:
//!
//! * every string variable that receives a computed value owns one buffer
//!   (`str_var_bufs`), so assignment has value semantics (`t = s` copies);
//! * nested sub-expressions (`left$(a + b, 3)`) use one scratch buffer per
//!   nesting depth (`str_tmp_bufs`);
//! * a computed argument for a `string` parameter goes into a buffer owned by
//!   that parameter (`str_param_bufs`).
//!
//! Buffers and the small runtime routines are emitted inline on first use,
//! jumped over (`JMP past`), so no post-code patching is needed. Five ZP
//! pointers/counters are allocated on first use (`str_zp`):
//!
//! | offset | name  | use |
//! |---|---|---|
//! | +0/+1 | SRC   | source string pointer |
//! | +2/+3 | DST   | write position in the buffer being built |
//! | +4    | ROOM  | characters still free in that buffer |
//! | +5    | LIMIT | max characters to take from SRC (`$FF` = all) |
//! | +6/+7 | NUM   | 16-bit number for number↔text conversion |
//! | +8    | FLAG  | scratch flag (leading-zero / minus sign) |
//! | +9    | DIG   | scratch digit |

use super::Codegen;
use crate::compiler::ast::{BinOp, Expr, VarType};

/// Characters a runtime-built string may hold (C64 screen line × 2).
pub(super) const STR_CAP: u8 = 80;

const SRC: u8 = 0;
const DST: u8 = 2;
const ROOM: u8 = 4;
const LIMIT: u8 = 5;
pub(super) const NUM: u8 = 6;
const FLAG: u8 = 8;
const DIG: u8 = 9;

/// Built-in string functions, parsed as `FnCall("left$", …)` etc.
pub(super) fn is_builtin_str_fn(name: &str) -> bool {
    matches!(name, "left$" | "right$" | "mid$")
}

#[derive(Clone, Copy)]
pub(super) enum Helper {
    Append,
    Skip,
    Len,
    Utoa,
    Itoa,
    Atoi,
    Cmp,
}

impl Codegen {
    /// ZP block of the string engine (allocated on first use).
    pub(super) fn str_zp(&mut self) -> u8 {
        if let Some(z) = self.str_zp {
            return z;
        }
        let z = self.perm_zp;
        self.perm_zp += 10;
        self.str_zp = Some(z);
        z
    }

    /// Emit `JMP past; <n zero bytes>; past:` and return the data address.
    fn emit_inline_zeros(&mut self, n: usize) -> u16 {
        self.emit(0x4C);
        let p = self.code.len();
        self.emit16(0);
        let addr = self.current_addr();
        for _ in 0..n {
            self.emit(0);
        }
        let past = self.current_addr();
        self.patch_abs(p, past);
        addr
    }

    pub(super) fn str_var_buf(&mut self, name: &str) -> u16 {
        if let Some(&a) = self.str_var_bufs.get(name) {
            return a;
        }
        let a = self.emit_inline_zeros(STR_CAP as usize + 1);
        self.str_var_bufs.insert(name.to_string(), a);
        a
    }

    fn str_tmp_buf(&mut self, depth: usize) -> u16 {
        while self.str_tmp_bufs.len() <= depth {
            let a = self.emit_inline_zeros(STR_CAP as usize + 1);
            self.str_tmp_bufs.push(a);
        }
        self.str_tmp_bufs[depth]
    }

    fn num_buf(&mut self) -> u16 {
        if let Some(a) = self.str_num_buf {
            return a;
        }
        let a = self.emit_inline_zeros(8); // "-65535\0" + spare
        self.str_num_buf = Some(a);
        a
    }

    fn chr_buf(&mut self) -> u16 {
        if let Some(a) = self.str_chr_buf {
            return a;
        }
        let a = self.emit_inline_zeros(2);
        self.str_chr_buf = Some(a);
        a
    }

    // ── small emit helpers ───────────────────────────────────────────────────

    fn lda_imm(&mut self, v: u8) {
        self.emit(0xA9);
        self.emit(v);
    }
    fn sta_zp(&mut self, z: u8) {
        self.emit(0x85);
        self.emit(z);
    }
    fn lda_zp(&mut self, z: u8) {
        self.emit(0xA5);
        self.emit(z);
    }
    /// `zp pair ← addr`
    fn set_ptr(&mut self, z: u8, addr: u16) {
        self.lda_imm(addr as u8);
        self.sta_zp(z);
        self.lda_imm((addr >> 8) as u8);
        self.sta_zp(z + 1);
    }
    fn copy_ptr(&mut self, from: u8, to: u8) {
        if from == to {
            return;
        }
        self.lda_zp(from);
        self.sta_zp(to);
        self.lda_zp(from + 1);
        self.sta_zp(to + 1);
    }
    fn jsr(&mut self, addr: u16) {
        self.emit(0x20);
        self.emit16(addr);
    }
    /// Branch with a forward target patched later; returns the offset position.
    fn bxx_fwd(&mut self, op: u8) -> usize {
        self.emit(op);
        let p = self.code.len();
        self.emit(0);
        p
    }
    fn bxx_back(&mut self, op: u8, target_addr: u16) {
        self.emit(op);
        let p = self.code.len();
        self.emit(0);
        self.patch_bxx(p, target_addr);
    }

    // ── runtime routines ─────────────────────────────────────────────────────

    /// Address of a runtime routine, emitting it (jumped over) on first use.
    pub(super) fn str_helper(&mut self, h: Helper) -> u16 {
        let key = h as usize;
        if let Some(a) = self.str_helpers[key] {
            return a;
        }
        let z = self.str_zp();
        self.emit(0x4C);
        let jmp = self.code.len();
        self.emit16(0);
        let entry = self.current_addr();
        match h {
            Helper::Append => {
                // copy (SRC) → (DST) while LIMIT and ROOM last and no $00;
                // terminate, advance DST past the copied characters.
                self.emit(0xA0);
                self.emit(0x00); // LDY #0
                let l = self.current_addr();
                self.emit(0xC4);
                self.emit(z + LIMIT); // CPY LIMIT
                let d1 = self.bxx_fwd(0xF0); // BEQ done
                self.lda_zp(z + ROOM);
                let d2 = self.bxx_fwd(0xF0); // BEQ done
                self.emit(0xB1);
                self.emit(z + SRC); // LDA (SRC),Y
                let d3 = self.bxx_fwd(0xF0); // BEQ done
                self.emit(0x91);
                self.emit(z + DST); // STA (DST),Y
                self.emit(0xC6);
                self.emit(z + ROOM); // DEC ROOM
                self.emit(0xC8); // INY
                self.bxx_back(0xD0, l); // BNE loop
                let done = self.current_addr();
                for p in [d1, d2, d3] {
                    self.patch_bxx(p, done);
                }
                self.lda_imm(0);
                self.emit(0x91);
                self.emit(z + DST); // STA (DST),Y — terminator
                self.emit(0x98); // TYA
                self.emit(0x18); // CLC
                self.emit(0x65);
                self.emit(z + DST); // ADC DST
                self.sta_zp(z + DST);
                let nc = self.bxx_fwd(0x90); // BCC +2
                self.emit(0xE6);
                self.emit(z + DST + 1); // INC DST+1
                let here = self.current_addr();
                self.patch_bxx(nc, here);
                self.emit(0x60); // RTS
            }
            Helper::Skip => {
                // advance SRC by A characters, never past the terminator
                self.emit(0xAA); // TAX
                let r1 = self.bxx_fwd(0xF0); // BEQ rts
                let l = self.current_addr();
                self.emit(0xA0);
                self.emit(0x00); // LDY #0
                self.emit(0xB1);
                self.emit(z + SRC); // LDA (SRC),Y
                let r2 = self.bxx_fwd(0xF0); // BEQ rts
                self.emit(0xE6);
                self.emit(z + SRC); // INC SRC
                let nc = self.bxx_fwd(0xD0); // BNE +2
                self.emit(0xE6);
                self.emit(z + SRC + 1); // INC SRC+1
                let here = self.current_addr();
                self.patch_bxx(nc, here);
                self.emit(0xCA); // DEX
                self.bxx_back(0xD0, l); // BNE loop
                let r = self.current_addr();
                self.patch_bxx(r1, r);
                self.patch_bxx(r2, r);
                self.emit(0x60);
            }
            Helper::Len => {
                // A = length of (SRC)
                self.emit(0xA0);
                self.emit(0xFF); // LDY #$FF
                let l = self.current_addr();
                self.emit(0xC8); // INY
                self.emit(0xB1);
                self.emit(z + SRC); // LDA (SRC),Y
                self.bxx_back(0xD0, l); // BNE loop
                self.emit(0x98); // TYA
                self.emit(0x60);
            }
            Helper::Utoa | Helper::Itoa => {
                // NUM → decimal text in num_buf (no leading zeros); Itoa treats
                // NUM as signed and writes a leading '-'.
                let buf = self.num_buf();
                // (num_buf is emitted inline *here*, inside the routine body,
                //  behind its own JMP — harmless)
                self.emit(0xA0);
                self.emit(0x00); // LDY #0
                let mut core_jmp = None;
                if matches!(h, Helper::Itoa) {
                    self.lda_zp(z + NUM + 1);
                    let pos = self.bxx_fwd(0x10); // BPL core
                    core_jmp = Some(pos);
                    self.lda_imm(b'-');
                    self.emit(0x8D);
                    self.emit16(buf); // STA buf
                    self.emit(0xC8); // INY
                    // NUM = -NUM
                    self.emit(0x38); // SEC
                    self.lda_imm(0);
                    self.emit(0xE5);
                    self.emit(z + NUM); // SBC NUM
                    self.sta_zp(z + NUM);
                    self.lda_imm(0);
                    self.emit(0xE5);
                    self.emit(z + NUM + 1); // SBC NUM+1
                    self.sta_zp(z + NUM + 1);
                }
                let core = self.current_addr();
                if let Some(p) = core_jmp {
                    self.patch_bxx(p, core);
                }
                self.emit(0xA2);
                self.emit(0x00); // LDX #0
                self.emit(0x86);
                self.emit(z + FLAG); // STX FLAG
                let dl = self.current_addr();
                self.lda_imm(0x30);
                self.sta_zp(z + DIG); // DIG = '0'
                let sub = self.current_addr();
                self.emit(0x38); // SEC
                self.lda_zp(z + NUM);
                self.emit(0xFD);
                let tlo = self.code.len();
                self.emit16(0); // SBC tbl_lo,X (patched)
                self.emit(0x48); // PHA
                self.lda_zp(z + NUM + 1);
                self.emit(0xFD);
                let thi = self.code.len();
                self.emit16(0); // SBC tbl_hi,X (patched)
                let nosub = self.bxx_fwd(0x90); // BCC nosub
                self.sta_zp(z + NUM + 1);
                self.emit(0x68); // PLA
                self.sta_zp(z + NUM);
                self.emit(0xE6);
                self.emit(z + DIG); // INC DIG
                self.bxx_back(0xD0, sub); // BNE sub (DIG never wraps to 0)
                let ns = self.current_addr();
                self.patch_bxx(nosub, ns);
                self.emit(0x68); // PLA (discard)
                self.lda_zp(z + DIG);
                self.emit(0xC9);
                self.emit(0x30); // CMP #'0'
                let wr = self.bxx_fwd(0xD0); // BNE write
                self.lda_zp(z + FLAG);
                let next = self.bxx_fwd(0xF0); // BEQ next (leading zero)
                self.lda_zp(z + DIG);
                let w = self.current_addr();
                self.patch_bxx(wr, w);
                self.emit(0x99);
                self.emit16(buf); // STA buf,Y
                self.emit(0xC8); // INY
                self.emit(0xE6);
                self.emit(z + FLAG); // INC FLAG (started)
                let nx = self.current_addr();
                self.patch_bxx(next, nx);
                self.emit(0xE8); // INX
                self.emit(0xE0);
                self.emit(0x04); // CPX #4
                self.bxx_back(0xD0, dl); // BNE digit loop
                self.lda_zp(z + NUM);
                self.emit(0x18); // CLC
                self.emit(0x69);
                self.emit(0x30); // ADC #'0' (last digit, always written)
                self.emit(0x99);
                self.emit16(buf); // STA buf,Y
                self.emit(0xC8); // INY
                self.lda_imm(0);
                self.emit(0x99);
                self.emit16(buf); // STA buf,Y — terminator
                self.emit(0x60); // RTS
                let tbl = self.current_addr();
                for v in [10000u16, 1000, 100, 10] {
                    self.emit(v as u8);
                }
                for v in [10000u16, 1000, 100, 10] {
                    self.emit((v >> 8) as u8);
                }
                self.patch_abs(tlo, tbl);
                self.patch_abs(thi, tbl + 4);
            }
            Helper::Atoi => {
                // (SRC) → NUM: skip spaces, optional '-', digits until a non-digit
                self.lda_imm(0);
                self.sta_zp(z + NUM);
                self.sta_zp(z + NUM + 1);
                self.sta_zp(z + FLAG);
                self.emit(0xA0);
                self.emit(0x00); // LDY #0
                let sp = self.current_addr();
                self.emit(0xB1);
                self.emit(z + SRC); // LDA (SRC),Y
                self.emit(0xC9);
                self.emit(0x20); // CMP #' '
                let ns = self.bxx_fwd(0xD0); // BNE not_space
                self.emit(0xC8); // INY
                self.bxx_back(0xD0, sp); // BNE spaces
                let nsa = self.current_addr();
                self.patch_bxx(ns, nsa);
                self.emit(0xC9);
                self.emit(b'-'); // CMP #'-'
                let dl_fwd = self.bxx_fwd(0xD0); // BNE digits
                self.emit(0xE6);
                self.emit(z + FLAG); // INC FLAG (negative)
                self.emit(0xC8); // INY
                let dl = self.current_addr();
                self.patch_bxx(dl_fwd, dl);
                self.emit(0xB1);
                self.emit(z + SRC); // LDA (SRC),Y
                self.emit(0x38); // SEC
                self.emit(0xE9);
                self.emit(0x30); // SBC #'0'
                self.emit(0xC9);
                self.emit(10); // CMP #10
                let end = self.bxx_fwd(0xB0); // BCS end (not a digit)
                self.sta_zp(z + DIG);
                // NUM = NUM * 10 + DIG  (NUM*2 saved on the stack, + NUM*8)
                self.emit(0x06);
                self.emit(z + NUM); // ASL NUM
                self.emit(0x26);
                self.emit(z + NUM + 1); // ROL NUM+1
                self.lda_zp(z + NUM + 1);
                self.emit(0x48); // PHA
                self.lda_zp(z + NUM);
                self.emit(0x48); // PHA
                for _ in 0..2 {
                    self.emit(0x06);
                    self.emit(z + NUM);
                    self.emit(0x26);
                    self.emit(z + NUM + 1);
                }
                self.emit(0x68); // PLA (lo of NUM*2)
                self.emit(0x18); // CLC
                self.emit(0x65);
                self.emit(z + NUM);
                self.sta_zp(z + NUM);
                self.emit(0x68); // PLA (hi of NUM*2)
                self.emit(0x65);
                self.emit(z + NUM + 1);
                self.sta_zp(z + NUM + 1);
                self.lda_zp(z + DIG);
                self.emit(0x18); // CLC
                self.emit(0x65);
                self.emit(z + NUM);
                self.sta_zp(z + NUM);
                let nc = self.bxx_fwd(0x90); // BCC +2
                self.emit(0xE6);
                self.emit(z + NUM + 1);
                let h2 = self.current_addr();
                self.patch_bxx(nc, h2);
                self.emit(0xC8); // INY
                self.bxx_back(0xD0, dl); // BNE digits
                let e = self.current_addr();
                self.patch_bxx(end, e);
                self.lda_zp(z + FLAG);
                let rts = self.bxx_fwd(0xF0); // BEQ rts
                self.emit(0x38); // SEC — NUM = -NUM
                self.lda_imm(0);
                self.emit(0xE5);
                self.emit(z + NUM);
                self.sta_zp(z + NUM);
                self.lda_imm(0);
                self.emit(0xE5);
                self.emit(z + NUM + 1);
                self.sta_zp(z + NUM + 1);
                let r = self.current_addr();
                self.patch_bxx(rts, r);
                self.emit(0x60);
            }
            Helper::Cmp => {
                // A = 1 if (SRC) equals (DST), else 0
                self.emit(0xA0);
                self.emit(0x00); // LDY #0
                let l = self.current_addr();
                self.emit(0xB1);
                self.emit(z + SRC); // LDA (SRC),Y
                self.emit(0xD1);
                self.emit(z + DST); // CMP (DST),Y
                let ne = self.bxx_fwd(0xD0); // BNE not_equal
                self.emit(0xC9);
                self.emit(0x00); // CMP #0
                let eq = self.bxx_fwd(0xF0); // BEQ equal
                self.emit(0xC8); // INY
                self.bxx_back(0xD0, l); // BNE loop
                let e = self.current_addr();
                self.patch_bxx(eq, e);
                self.lda_imm(1);
                self.emit(0x60);
                let n = self.current_addr();
                self.patch_bxx(ne, n);
                self.lda_imm(0);
                self.emit(0x60);
            }
        }
        let past = self.current_addr();
        self.patch_abs(jmp, past);
        self.str_helpers[key] = Some(entry);
        entry
    }

    // ── classification ───────────────────────────────────────────────────────

    /// Numbers that `str$` / concatenation convert with the 16-bit routine.
    fn is_wide_number(&self, e: &Expr) -> bool {
        self.can_be_word_result(e) || self.is_signed_expr(e)
    }

    /// Conservative: could evaluating `e` read string variable `name`?
    fn str_expr_reads(&self, e: &Expr, name: &str) -> bool {
        match e {
            Expr::StringLit(_) | Expr::Number(_) => false,
            Expr::Var(n) => n == name,
            Expr::BinOp(l, _, r) => self.str_expr_reads(l, name) || self.str_expr_reads(r, name),
            Expr::FnCall(f, args) if is_builtin_str_fn(f) => {
                args.iter().any(|a| self.str_expr_reads(a, name))
            }
            Expr::ArrayGet(_, i) => self.str_expr_reads(i, name),
            Expr::StrN(i) | Expr::ChrStr(i) => self.str_expr_reads(i, name),
            _ => true,
        }
    }

    // ── building ─────────────────────────────────────────────────────────────

    /// SRC ← pointer to the value of string expression `e` (building it into
    /// the scratch buffer of `depth` if needed). Numbers become decimal text.
    fn emit_str_src(&mut self, e: &Expr, depth: usize) {
        let z = self.str_zp();
        match e {
            Expr::StringLit(s) => {
                let a = self.emit_inline_string(&s.clone());
                self.set_ptr(z + SRC, a);
            }
            Expr::Var(n) if matches!(self.var_types.get(n), Some(VarType::Str)) => {
                self.mark_used(n);
                if let Some(p) = self.var_addr(n) {
                    self.copy_ptr(p, z + SRC);
                }
            }
            Expr::ArrayGet(a, _) if self.str_arrays.contains(a.as_str()) => {
                let e = e.clone();
                self.gen_word_assign(z + SRC, &e);
            }
            Expr::StrN(inner) if !self.is_wide_number(inner) => {
                let e2 = e.clone();
                self.eval_expr(&e2); // strn helper → strn_zp
                if let Some(p) = self.strn_zp {
                    self.copy_ptr(p, z + SRC);
                }
            }
            Expr::StrN(inner) => {
                let inner = (**inner).clone();
                self.emit_number_text(&inner);
            }
            _ if self.is_string_expr(e) => {
                // nested computed string: build into this depth's scratch
                // buffer, keeping the outer build's DST / ROOM
                let buf = self.str_tmp_buf(depth);
                self.lda_zp(z + DST);
                self.emit(0x48);
                self.lda_zp(z + DST + 1);
                self.emit(0x48);
                self.lda_zp(z + ROOM);
                self.emit(0x48);
                self.emit_str_build(e, buf, depth + 1);
                self.emit(0x68);
                self.sta_zp(z + ROOM);
                self.emit(0x68);
                self.sta_zp(z + DST + 1);
                self.emit(0x68);
                self.sta_zp(z + DST);
                self.set_ptr(z + SRC, buf);
            }
            _ => {
                let e = e.clone();
                self.emit_number_text(&e);
            }
        }
    }

    /// Numeric expression → decimal text in num_buf; SRC ← num_buf.
    fn emit_number_text(&mut self, e: &Expr) {
        let z = self.str_zp();
        if self.is_float_like(e) {
            // Q8.8: integer part, '.', two decimals — use the integer part only
            // for now and append ".DD" from the fraction byte
            let tmp = self.tmp_zp;
            self.tmp_zp += 2;
            let e2 = e.clone();
            if !self.gen_word_assign(tmp, &e2) {
                self.eval_expr_word(&e2, tmp, tmp + 1);
            }
            self.lda_zp(tmp + 1);
            self.sta_zp(z + NUM);
            self.lda_imm(0);
            self.sta_zp(z + NUM + 1);
            let utoa = self.str_helper(Helper::Utoa);
            self.jsr(utoa);
            // append ".DD": frac*100 >> 8 → two digits; reuse num_buf end
            let buf = self.num_buf();
            // find end (Y = length) via Len on num_buf
            self.set_ptr(z + SRC, buf);
            let len = self.str_helper(Helper::Len);
            self.jsr(len);
            self.emit(0xA8); // TAY
            self.lda_imm(b'.');
            self.emit(0x99);
            self.emit16(buf);
            self.emit(0xC8);
            // DIG*: frac*100>>8 = (frac*25)>>6; compute with a loop: count
            // how many times 41/16 fits… simpler: hundredths = (lo * 100) >> 8
            // via 8x8 shift-add into NUM (16-bit)
            self.lda_imm(0);
            self.sta_zp(z + NUM);
            self.sta_zp(z + NUM + 1);
            // NUM = lo * 100 (100 = 64 + 32 + 4)
            for shift in [6u8, 5, 2] {
                self.lda_zp(tmp);
                self.sta_zp(z + DIG);
                self.lda_imm(0);
                self.sta_zp(z + FLAG);
                for _ in 0..shift {
                    self.emit(0x06);
                    self.emit(z + DIG);
                    self.emit(0x26);
                    self.emit(z + FLAG);
                }
                self.emit(0x18);
                self.lda_zp(z + NUM);
                self.emit(0x65);
                self.emit(z + DIG);
                self.sta_zp(z + NUM);
                self.lda_zp(z + NUM + 1);
                self.emit(0x65);
                self.emit(z + FLAG);
                self.sta_zp(z + NUM + 1);
            }
            // hundredths in NUM+1 (0..99): tens = /10 by subtraction
            self.lda_zp(z + NUM + 1);
            self.emit(0xA2);
            self.emit(0x30); // LDX #'0'
            let l = self.current_addr();
            self.emit(0xC9);
            self.emit(10); // CMP #10
            let done = self.bxx_fwd(0x90); // BCC done
            self.emit(0xE9);
            self.emit(10); // SBC #10 (C=1)
            self.emit(0xE8); // INX
            self.bxx_back(0xD0, l); // BNE loop (X never 0)
            let d = self.current_addr();
            self.patch_bxx(done, d);
            self.emit(0x48); // PHA (units)
            self.emit(0x8A); // TXA
            self.emit(0x99);
            self.emit16(buf);
            self.emit(0xC8);
            self.emit(0x68); // PLA
            self.emit(0x18);
            self.emit(0x69);
            self.emit(0x30); // ADC #'0'
            self.emit(0x99);
            self.emit16(buf);
            self.emit(0xC8);
            self.lda_imm(0);
            self.emit(0x99);
            self.emit16(buf);
            self.set_ptr(z + SRC, buf);
            return;
        }
        let e2 = e.clone();
        if !self.gen_word_assign(z + NUM, &e2) {
            self.eval_expr_word(&e2, z + NUM, z + NUM + 1);
        }
        let h = if self.is_signed_expr(e) { Helper::Itoa } else { Helper::Utoa };
        let a = self.str_helper(h);
        self.jsr(a);
        let buf = self.num_buf();
        self.set_ptr(z + SRC, buf);
    }

    /// Build string expression `e` into the buffer at `buf`
    /// (DST = buf, ROOM = capacity, then append each part).
    pub(super) fn emit_str_build(&mut self, e: &Expr, buf: u16, depth: usize) {
        let z = self.str_zp();
        self.set_ptr(z + DST, buf);
        self.lda_imm(STR_CAP);
        self.sta_zp(z + ROOM);
        self.lda_imm(0);
        self.emit(0x8D);
        self.emit16(buf); // empty result even if nothing is appended
        self.emit_str_append(e, depth);
    }

    fn emit_str_append(&mut self, e: &Expr, depth: usize) {
        let z = self.str_zp();
        match e {
            Expr::BinOp(l, BinOp::Add, r) if self.is_string_expr(e) => {
                let (l, r) = ((**l).clone(), (**r).clone());
                self.emit_str_append(&l, depth);
                self.emit_str_append(&r, depth);
            }
            Expr::FnCall(f, args) if is_builtin_str_fn(f) => {
                let f = f.clone();
                let args = args.clone();
                let s = args.first().cloned().unwrap_or(Expr::StringLit(String::new()));
                let n1 = args.get(1).cloned().unwrap_or(Expr::Number(1));
                let n2 = args.get(2).cloned();
                // numeric arguments first (into scratch), then the source
                let t1 = self.eval_to_tmp(&n1);
                let t2 = n2.as_ref().map(|e| self.eval_to_tmp(e));
                self.emit_str_src(&s, depth);
                match f.as_str() {
                    "left$" => {
                        self.lda_zp(t1);
                        self.sta_zp(z + LIMIT);
                    }
                    "right$" => {
                        let len = self.str_helper(Helper::Len);
                        self.jsr(len);
                        self.emit(0x38); // SEC
                        self.emit(0xE5);
                        self.emit(t1); // SBC n
                        let ok = self.bxx_fwd(0xB0); // BCS ok
                        self.lda_imm(0); // n >= len: take everything
                        let here = self.current_addr();
                        self.patch_bxx(ok, here);
                        let skip = self.str_helper(Helper::Skip);
                        self.jsr(skip);
                        self.lda_imm(0xFF);
                        self.sta_zp(z + LIMIT);
                    }
                    _ => {
                        // mid$(s, start [, len]) — start is 1-based
                        self.lda_zp(t1);
                        let zero = self.bxx_fwd(0xF0); // BEQ (start 0 = 1)
                        self.emit(0x38);
                        self.emit(0xE9);
                        self.emit(1); // SBC #1
                        let here = self.current_addr();
                        self.patch_bxx(zero, here);
                        let skip = self.str_helper(Helper::Skip);
                        self.jsr(skip);
                        match t2 {
                            Some(t) => self.lda_zp(t),
                            None => self.lda_imm(0xFF),
                        }
                        self.sta_zp(z + LIMIT);
                    }
                }
                let app = self.str_helper(Helper::Append);
                self.jsr(app);
            }
            Expr::ChrStr(inner) => {
                let inner = (**inner).clone();
                let cb = self.chr_buf();
                self.eval_expr(&inner);
                self.emit(0x8D);
                self.emit16(cb); // STA chr_buf (chr_buf+1 stays 0)
                self.set_ptr(z + SRC, cb);
                self.lda_imm(0xFF);
                self.sta_zp(z + LIMIT);
                let app = self.str_helper(Helper::Append);
                self.jsr(app);
            }
            _ => {
                self.emit_str_src(e, depth);
                self.lda_imm(0xFF);
                self.sta_zp(z + LIMIT);
                let app = self.str_helper(Helper::Append);
                self.jsr(app);
            }
        }
    }

    // ── statement-level entry points ────────────────────────────────────────

    /// `s = <string expression>` for a string variable at `zp`, value
    /// semantics: the result is copied into `s`'s own buffer.
    pub(super) fn emit_str_assign(&mut self, name: &str, zp: u8, e: &Expr) {
        let own = self.str_var_buf(name);
        if self.str_expr_reads(e, name) {
            let tmp = self.str_tmp_buf(0);
            self.emit_str_build(e, tmp, 1);
            let z = self.str_zp();
            self.set_ptr(z + DST, own);
            self.lda_imm(STR_CAP);
            self.sta_zp(z + ROOM);
            self.set_ptr(z + SRC, tmp);
            self.lda_imm(0xFF);
            self.sta_zp(z + LIMIT);
            let app = self.str_helper(Helper::Append);
            self.jsr(app);
        } else {
            self.emit_str_build(e, own, 0);
        }
        self.set_ptr(zp, own);
        self.a_cache = None;
    }

    /// Computed string → a parameter's own buffer; `zp` ← its address.
    pub(super) fn emit_str_param(&mut self, key: &str, zp: u8, e: &Expr) {
        let buf = match self.str_param_bufs.get(key) {
            Some(&b) => b,
            None => {
                let b = self.emit_inline_zeros(STR_CAP as usize + 1);
                self.str_param_bufs.insert(key.to_string(), b);
                b
            }
        };
        self.emit_str_build(e, buf, 0);
        self.set_ptr(zp, buf);
    }

    /// `print <computed string>`: build into scratch buffer 0 and print it.
    pub(super) fn emit_str_print(&mut self, e: &Expr) {
        let tmp = self.str_tmp_buf(0);
        self.emit_str_build(e, tmp, 1);
        let z = self.str_zp();
        self.set_ptr(z + SRC, tmp);
        self.print_str_via_ptr(z + SRC);
    }

    /// A ← 1 if the two string expressions are equal (0 otherwise);
    /// `negate` gives `<>`.
    pub(super) fn emit_str_compare(&mut self, l: &Expr, r: &Expr, negate: bool) {
        let z = self.str_zp();
        self.emit_str_src(l, 1);
        // keep l's pointer on the stack while r is evaluated
        self.lda_zp(z + SRC);
        self.emit(0x48);
        self.lda_zp(z + SRC + 1);
        self.emit(0x48);
        self.emit_str_src(r, 2);
        self.emit(0x68);
        self.sta_zp(z + DST + 1);
        self.emit(0x68);
        self.sta_zp(z + DST);
        let cmp = self.str_helper(Helper::Cmp);
        self.jsr(cmp);
        if negate {
            self.emit(0x49);
            self.emit(0x01); // EOR #1
        }
    }

    /// A ← length of any string expression.
    pub(super) fn emit_str_len(&mut self, e: &Expr) {
        self.emit_str_src(e, 1);
        let len = self.str_helper(Helper::Len);
        self.jsr(len);
    }

    /// `names(i) = <string expression>`: copy the value into element `i`'s own
    /// 32-byte slot (31 characters) and point the element at it, so elements
    /// do not share a buffer (`input names(i)` in a loop, `names(i) = s`).
    pub(super) fn emit_str_array_store(&mut self, arr: &str, idx: &Expr, val: &Expr) {
        const SLOT: u16 = 32;
        let z = self.str_zp();
        let n_elems = self.array_sizes.get(arr).copied().unwrap_or(0) / 2;
        let slots = match self.str_array_slots.get(arr) {
            Some(&a) => a,
            None => {
                let a = self.emit_inline_zeros((n_elems * SLOT) as usize);
                self.str_array_slots.insert(arr.to_string(), a);
                a
            }
        };
        // index once into a hidden word var (used for the slot and the element)
        let ivar = "strslot__idx";
        let izp = self.alloc_var(ivar);
        self.var_types.insert(ivar.to_string(), VarType::Word);
        if !self.gen_word_assign(izp, idx) {
            self.eval_expr(idx);
            self.sta_zp(izp);
            self.lda_imm(0);
            self.sta_zp(izp + 1);
        }
        // value → scratch buffer 0
        let tmp = self.str_tmp_buf(0);
        self.emit_str_build(val, tmp, 1);
        // P = slots + idx * 32
        let pvar = "strslot__ptr";
        let pzp = self.alloc_var(pvar);
        self.var_types.insert(pvar.to_string(), VarType::Word);
        self.copy_ptr(izp, pzp);
        for _ in 0..5 {
            self.emit(0x06);
            self.emit(pzp); // ASL lo
            self.emit(0x26);
            self.emit(pzp + 1); // ROL hi
        }
        self.emit(0x18);
        self.lda_zp(pzp);
        self.emit(0x69);
        self.emit(slots as u8);
        self.sta_zp(pzp);
        self.lda_zp(pzp + 1);
        self.emit(0x69);
        self.emit((slots >> 8) as u8);
        self.sta_zp(pzp + 1);
        // copy scratch → slot (31 chars max)
        self.copy_ptr(pzp, z + DST);
        self.lda_imm((SLOT - 1) as u8);
        self.sta_zp(z + ROOM);
        self.set_ptr(z + SRC, tmp);
        self.lda_imm(0xFF);
        self.sta_zp(z + LIMIT);
        let app = self.str_helper(Helper::Append);
        self.jsr(app);
        // element ← slot address (ordinary word-array store)
        let st = crate::compiler::ast::Stmt::ArraySet(
            arr.to_string(),
            Expr::Var(ivar.to_string()),
            Expr::Var(pvar.to_string()),
        );
        self.gen_stmt_inner(&st);
        self.a_cache = None;
    }

    /// Convert the typed-in text at `buf` into the 16-bit / Q8.8 variable at
    /// `var_zp` (`input` into an integer or double).
    pub(super) fn emit_input_number(&mut self, buf: u16, var_zp: u8, float: bool) {
        let z = self.str_zp();
        self.set_ptr(z + SRC, buf);
        let atoi = self.str_helper(Helper::Atoi);
        self.jsr(atoi); // integer part (stops at '.')
        if !float {
            self.copy_ptr(z + NUM, var_zp);
            return;
        }
        // Q8.8: hi = integer part, lo = round-up(hundredths * 256 / 100)
        // (table first, jumped over, so every branch below stays short)
        let table: Vec<u8> = (0u32..100).map(|f| ((f * 256 + 99) / 100) as u8).collect();
        let t = self.emit_inline_zeros(table.len());
        for (k, b) in table.into_iter().enumerate() {
            let pos = (t - self.load_addr) as usize + k;
            self.code[pos] = b;
        }
        self.lda_zp(z + NUM);
        self.sta_zp(var_zp + 1);
        self.lda_imm(0);
        self.sta_zp(var_zp);
        // find '.'
        self.emit(0xA0);
        self.emit(0x00); // LDY #0
        let find = self.current_addr();
        self.emit(0xB1);
        self.emit(z + SRC); // LDA (SRC),Y
        let done1 = self.bxx_fwd(0xF0); // BEQ done (no '.')
        self.emit(0xC9);
        self.emit(b'.');
        let dot = self.bxx_fwd(0xF0); // BEQ dot
        self.emit(0xC8); // INY
        self.bxx_back(0xD0, find);
        let d = self.current_addr();
        self.patch_bxx(dot, d);
        // first decimal → FLAG = d1 * 10
        self.emit(0xC8); // INY
        self.emit(0xB1);
        self.emit(z + SRC);
        self.emit(0x38);
        self.emit(0xE9);
        self.emit(0x30); // SBC #'0'
        self.emit(0xC9);
        self.emit(10);
        let done2 = self.bxx_fwd(0xB0); // BCS done
        self.sta_zp(z + DIG);
        self.emit(0x0A); // ASL (×2)
        self.emit(0x0A); // ASL (×4)
        self.emit(0x18);
        self.emit(0x65);
        self.emit(z + DIG); // ADC d1 (×5)
        self.emit(0x0A); // ASL (×10)
        self.sta_zp(z + FLAG);
        // optional second decimal
        self.emit(0xC8); // INY
        self.emit(0xB1);
        self.emit(z + SRC);
        self.emit(0x38);
        self.emit(0xE9);
        self.emit(0x30);
        self.emit(0xC9);
        self.emit(10);
        let one = self.bxx_fwd(0xB0); // BCS one digit only
        self.emit(0x18);
        self.emit(0x65);
        self.emit(z + FLAG);
        self.sta_zp(z + FLAG);
        let o = self.current_addr();
        self.patch_bxx(one, o);
        // lo = table[FLAG]
        self.emit(0xA6);
        self.emit(z + FLAG); // LDX FLAG
        self.emit(0xBD);
        self.emit16(t); // LDA table,X
        self.sta_zp(var_zp);
        let done = self.current_addr();
        self.patch_bxx(done1, done);
        self.patch_bxx(done2, done);
    }

    /// NUM ← value of the decimal text of string expression `e`
    /// (16-bit, leading spaces and '-' accepted).
    pub(super) fn emit_str_val(&mut self, e: &Expr) -> u8 {
        self.emit_str_src(e, 1);
        let atoi = self.str_helper(Helper::Atoi);
        self.jsr(atoi);
        self.str_zp() + NUM
    }
}
