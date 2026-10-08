//! Optimised code paths of the 6502 code generator.
//!
//! * Conditions (`if` / `while` / `until` / `select`) compile to a `CMP` followed
//!   directly by branches instead of materialising a 0/1 value and re-testing it.
//! * Comparison operands that are constants or byte variables are used directly
//!   as `CMP #n` / `CMP zp` operands instead of being staged in scratch zero page.

use super::Codegen;
use crate::compiler::ast::{BinOp, Expr, VarType};

/// Relation between A and the operand of the preceding `CMP` (unsigned).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pred {
    Eq,
    Ne,
    Lt,
    Ge,
    Gt,
    Le,
    /// Statically known outcome (no flags are tested).
    Always,
    Never,
}

impl Pred {
    fn negate(self) -> Pred {
        match self {
            Pred::Eq => Pred::Ne,
            Pred::Ne => Pred::Eq,
            Pred::Lt => Pred::Ge,
            Pred::Ge => Pred::Lt,
            Pred::Gt => Pred::Le,
            Pred::Le => Pred::Gt,
            Pred::Always => Pred::Never,
            Pred::Never => Pred::Always,
        }
    }

    /// `a OP b` ⇔ `b mirror(OP) a`
    fn mirror(self) -> Pred {
        match self {
            Pred::Lt => Pred::Gt,
            Pred::Gt => Pred::Lt,
            Pred::Le => Pred::Ge,
            Pred::Ge => Pred::Le,
            p => p,
        }
    }

    pub(super) fn from_binop(op: &BinOp) -> Option<Pred> {
        Some(match op {
            BinOp::Eq => Pred::Eq,
            BinOp::NotEq => Pred::Ne,
            BinOp::Lt => Pred::Lt,
            BinOp::Gt => Pred::Gt,
            BinOp::LtEq => Pred::Le,
            BinOp::GtEq => Pred::Ge,
            _ => return None,
        })
    }

}

/// How a condition that is not a comparison is interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Truth {
    /// Any non-zero value is true (`if`).
    NonZero,
    /// Only the value 1 is true (`while`, `until` — historical semantics).
    EqualsOne,
}

/// Where a conditional jump goes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Target {
    /// Not yet known: the jump is a patchable `JMP abs`, operand positions are returned.
    Forward,
    /// Already emitted address (loop top): a short branch is used when in range.
    Back(u16),
}

/// A directly addressable 8-bit operand.
#[derive(Clone, Copy, Debug)]
pub(super) enum Operand {
    Imm(u8),
    Zp(u8),
}

const BEQ: u8 = 0xF0;
const BNE: u8 = 0xD0;
const BCC: u8 = 0x90;
const BCS: u8 = 0xB0;
const JMP: u8 = 0x4C;

impl Codegen {
    // ── operands ────────────────────────────────────────────────────────────

    /// A constant or byte variable that can be used as an instruction operand
    /// without evaluating it into A first. Marks the variable used.
    pub(super) fn simple_operand(&mut self, e: &Expr) -> Option<Operand> {
        match e {
            Expr::Number(n) if (0..=255).contains(n) => Some(Operand::Imm(*n as u8)),
            Expr::Var(name) => {
                if matches!(
                    self.var_types.get(name),
                    Some(VarType::Word) | Some(VarType::Float)
                ) {
                    return None;
                }
                self.mark_used(name);
                Some(match self.var_addr(name) {
                    Some(zp) => Operand::Zp(zp),
                    None => Operand::Imm(0), // same as eval_expr: unknown var reads 0
                })
            }
            _ => None,
        }
    }

    /// Emit `<op> operand` where `imm_op` is the immediate-mode opcode of an
    /// ALU instruction (ORA/AND/EOR/ADC/LDA/CMP/SBC); the zero-page opcode is
    /// derived from it (`imm_op - 4`).
    pub(super) fn emit_alu(&mut self, imm_op: u8, operand: Operand) {
        match operand {
            Operand::Imm(v) => {
                self.emit(imm_op);
                self.emit(v);
            }
            Operand::Zp(zp) => {
                self.emit(imm_op - 4);
                self.emit(zp);
            }
        }
    }

    /// Expressions without side effects: they may be skipped (short-circuit
    /// evaluation, statically decided comparisons).
    pub(super) fn is_pure(e: &Expr) -> bool {
        match e {
            Expr::Number(_) | Expr::Var(_) => true,
            Expr::BinOp(l, _, r) => Self::is_pure(l) && Self::is_pure(r),
            Expr::Not(x) => Self::is_pure(x),
            Expr::ArrayGet(_, idx) => Self::is_pure(idx),
            _ => false,
        }
    }

    /// Expressions whose value is always 0 or 1 (so bitwise and/or behave like
    /// logical and/or).
    fn is_bool(e: &Expr) -> bool {
        match e {
            Expr::Not(_) => true,
            Expr::BinOp(_, op, _) if Pred::from_binop(op).is_some() => true,
            Expr::BinOp(l, BinOp::And | BinOp::Or | BinOp::Xor, r) => {
                Self::is_bool(l) && Self::is_bool(r)
            }
            _ => false,
        }
    }

    /// True when both sides of a comparison are 8-bit.
    fn is_byte_compare(&self, l: &Expr, r: &Expr) -> bool {
        !self.can_be_word_result(l)
            && !self.can_be_word_result(r)
            && !self.is_string_expr(l)
            && !self.is_string_expr(r)
    }

    // ── arrays ──────────────────────────────────────────────────────────────
    //
    // Array indices are 8-bit, so `base,Y` (absolute indexed) addresses exactly
    // the same bytes as the former `(ptr),Y` with ptr = base — without having
    // to build the pointer in zero page first.

    /// Load the (8-bit) array index into Y; `word` doubles it for 2-byte elements.
    pub(super) fn emit_index_to_y(&mut self, idx: &Expr, word: bool) {
        if !word {
            if let Some(op) = self.simple_operand(idx) {
                match op {
                    Operand::Imm(v) => {
                        self.emit(0xA0);
                        self.emit(v); // LDY #v
                    }
                    Operand::Zp(zp) => {
                        self.emit(0xA4);
                        self.emit(zp); // LDY zp
                    }
                }
                return;
            }
        }
        self.eval_expr(idx);
        if word {
            self.emit(0x0A); // ASL A (×2 stride)
        }
        self.emit(0xA8); // TAY
    }

    /// `LDA base,Y` for a byte array element with a non-constant index.
    pub(super) fn emit_byte_array_load(&mut self, base: u16, idx: &Expr) {
        self.emit_index_to_y(idx, false);
        self.emit(0xB9);
        self.emit16(base); // LDA base,Y
    }

    /// `arr[idx] = val` for a byte array. The value is evaluated before the
    /// index unless the order cannot matter.
    pub(super) fn emit_byte_array_store(&mut self, base: u16, idx: &Expr, val: &Expr) {
        if let Expr::Number(n) = idx {
            self.eval_expr(val);
            self.emit(0x8D);
            self.emit16(base.wrapping_add(*n as u16)); // STA base+n
            return;
        }
        let val_first_irrelevant = matches!(val, Expr::Number(_)) || Self::is_pure(idx);
        if val_first_irrelevant {
            if let Some(v) = self.simple_operand(val) {
                // LDY idx; LDA #v / LDA v; STA base,Y
                self.emit_index_to_y(idx, false);
                self.emit_alu(0xA9, v);
                self.emit(0x99);
                self.emit16(base);
                return;
            }
        }
        if let Some(op) = self.simple_operand(idx) {
            // val → A; LDY idx; STA base,Y
            self.eval_expr(val);
            match op {
                Operand::Imm(v) => {
                    self.emit(0xA0);
                    self.emit(v);
                }
                Operand::Zp(zp) => {
                    self.emit(0xA4);
                    self.emit(zp);
                }
            }
            self.emit(0x99);
            self.emit16(base); // STA base,Y
            return;
        }
        let t = self.eval_to_tmp(val);
        self.emit_index_to_y(idx, false);
        self.emit(0xA5);
        self.emit(t); // LDA tmp
        self.emit(0x99);
        self.emit16(base); // STA base,Y
    }

    // ── A-register reuse across statements ─────────────────────────────────
    //
    // `x = …` ends with `STA x`; if the next statement starts by loading x,
    // that `LDA x` is redundant. It is only skipped when
    //   1. the evaluation of `…` ended with an instruction that set N/Z from
    //      A (consumers such as `if x` / `abs(x)` branch on those flags), and
    //   2. the next statement is of a kind whose first instruction is never a
    //      jump target (see `may_reuse_a`), and
    //   3. the load is the very first byte emitted after the `STA`.

    /// Statements that may start by reusing A from the preceding statement.
    pub(super) fn may_reuse_a(stmt: &crate::compiler::ast::Stmt) -> bool {
        use crate::compiler::ast::Stmt;
        matches!(
            stmt,
            Stmt::Assign(..)
                | Stmt::VarDecl { .. }
                | Stmt::If(..)
                | Stmt::Poke(..)
                | Stmt::ArraySet(..)
                | Stmt::Select { .. }
        )
    }

    /// Called right after `STA zp` that stored the value of `expr`.
    pub(super) fn note_a_holds(&mut self, zp: u8, expr: &Expr) {
        self.a_cache = if Self::ends_with_a_flags(expr) {
            Some((zp, self.code.len()))
        } else {
            None
        };
    }

    /// True when the 8-bit code for `expr` ends with an instruction whose N/Z
    /// flags reflect the result in A (conservative: false when unsure).
    fn ends_with_a_flags(e: &Expr) -> bool {
        match e {
            Expr::Number(_) | Expr::Var(_) | Expr::ArrayGet(..) => true,
            Expr::BinOp(l, op, r) => match op {
                // identity operations emit only the left operand
                BinOp::Add | BinOp::Sub | BinOp::Or | BinOp::Xor => {
                    matches!(**r, Expr::Number(0)).then(|| Self::ends_with_a_flags(l)).unwrap_or(true)
                }
                BinOp::And => {
                    matches!(**r, Expr::Number(255)).then(|| Self::ends_with_a_flags(l)).unwrap_or(true)
                }
                // bool materialisation ends with LDA / ROL / EOR
                _ => Pred::from_binop(op).is_some(),
            },
            _ => false,
        }
    }

    // ── for loops ───────────────────────────────────────────────────────────

    /// `for var = from to limit [step s] … next`, 8-bit loop variable.
    ///
    /// Rotated layout (the limit test sits at the bottom):
    ///
    /// ```text
    ///         var = from; [zp_to = limit]; [zp_step = s]
    ///         JMP test          ; omitted when from/limit are constants that
    ///                           ; guarantee a first iteration
    /// body:   …
    /// cont:   var += step       ; exit on 8-bit wrap-around (past 255 / below 0)
    /// test:   LDA var; CMP limit; branch to body while var <= limit (>= when
    ///         counting down)
    /// exit:
    /// ```
    ///
    /// `limit` and a non-constant `step` are evaluated once, before the loop.
    pub(super) fn gen_for_loop(
        &mut self,
        var: &str,
        from: &Expr,
        to: &Expr,
        step: Option<&Expr>,
        body: &[crate::compiler::ast::Stmt],
    ) {
        let zp = self.alloc_var(var);
        let perm_before = self.perm_zp;
        let mut temps = 0; // permanent ZP bytes taken for limit/step snapshots

        // Step: constant (with direction) or a runtime value (counting up).
        #[derive(Clone, Copy)]
        enum Step {
            Up(u8),
            Down(u8),
            Var(u8), // zp slot, treated as positive
        }

        // from → var
        self.eval_expr(from);
        self.emit(0x85);
        self.emit(zp);

        // limit: immediate constant, or a snapshot in permanent ZP
        let limit = match to {
            Expr::Number(n) if (0..=255).contains(n) => Operand::Imm(*n as u8),
            _ => {
                let z = self.perm_zp;
                self.perm_zp += 1;
                temps += 1;
                self.eval_expr(to);
                self.emit(0x85);
                self.emit(z);
                Operand::Zp(z)
            }
        };
        let step = match step {
            None => Step::Up(1),
            Some(Expr::Number(n)) if *n < 0 => Step::Down(n.unsigned_abs() as u8),
            Some(Expr::Number(n)) => Step::Up(*n as u8),
            Some(e) => {
                let z = self.perm_zp;
                self.perm_zp += 1;
                temps += 1;
                self.eval_expr(e);
                self.emit(0x85);
                self.emit(z);
                Step::Var(z)
            }
        };
        let down = matches!(step, Step::Down(_));

        // A first iteration is certain when both ends are constants in order.
        let first_iteration_certain = match (from, limit) {
            (Expr::Number(f), Operand::Imm(t)) if (0..=255).contains(f) => {
                if down {
                    *f as u8 >= t
                } else {
                    *f as u8 <= t
                }
            }
            _ => false,
        };
        let entry_patch = if first_iteration_certain {
            None
        } else {
            self.emit(JMP); // JMP test
            let p = self.code.len();
            self.emit16(0);
            Some(p)
        };

        self.break_patches.push(vec![]);
        self.continue_patches.push(vec![]);
        let body_top = self.current_addr();
        self.gen_stmts(body);
        self.tmp_zp = super::TMP_BASE;

        // cont: var += step, leaving the loop when the variable wraps around
        let cont = self.current_addr();
        for pos in self.continue_patches.pop().unwrap_or_default() {
            self.patch_abs(pos, cont);
        }
        let exit_branch = match step {
            Step::Up(1) => {
                self.emit(0xE6);
                self.emit(zp); // INC var
                self.emit(BEQ); // BEQ exit (255 → 0)
                let p = self.code.len();
                self.emit(0);
                p
            }
            Step::Down(1) => {
                self.emit(0xA5);
                self.emit(zp); // LDA var
                self.emit(BEQ); // BEQ exit (0 - 1 would wrap)
                let p = self.code.len();
                self.emit(0);
                self.emit(0xC6);
                self.emit(zp); // DEC var
                p
            }
            Step::Up(c) | Step::Down(c) | Step::Var(c) => {
                self.emit(0xA5);
                self.emit(zp); // LDA var
                let (prefix, imm_op, exit_op) = match step {
                    Step::Down(_) => (0x38, 0xE9, BCC), // SEC; SBC; borrow → exit
                    _ => (0x18, 0x69, BCS),             // CLC; ADC; carry → exit
                };
                self.emit(prefix);
                let operand = match step {
                    Step::Var(z) => Operand::Zp(z),
                    _ => Operand::Imm(c),
                };
                self.emit_alu(imm_op, operand);
                self.emit(0x85);
                self.emit(zp); // STA var
                self.emit(exit_op);
                let p = self.code.len();
                self.emit(0);
                p
            }
        };

        // test: jump back to the body while var is within the limit
        let test = self.current_addr();
        if let Some(p) = entry_patch {
            self.patch_abs(p, test);
        }
        let pred = match (limit, down) {
            (Operand::Imm(255), false) | (Operand::Imm(0), true) => Pred::Always,
            (Operand::Imm(t), false) => {
                // var <= t ⇔ var < t+1
                self.emit(0xA5);
                self.emit(zp);
                self.emit_alu(0xC9, Operand::Imm(t + 1));
                Pred::Lt
            }
            (op, _) => {
                self.emit(0xA5);
                self.emit(zp);
                self.emit_alu(0xC9, op);
                if down { Pred::Ge } else { Pred::Le }
            }
        };
        self.emit_jump_on(pred, Target::Back(body_top));

        let loop_end = self.current_addr();
        self.patch_bxx(exit_branch, loop_end);
        for pos in self.break_patches.pop().unwrap_or_default() {
            self.patch_abs(pos, loop_end);
        }
        // The limit/step snapshots are dead after the loop: hand their
        // permanent ZP bytes back when the body declared nothing above them.
        if self.perm_zp == perm_before + temps {
            self.perm_zp = perm_before;
        }
    }

    /// `for` with a 16-bit (`word` / `integer`) counter. Same rotated shape as
    /// the 8-bit loop: body, `var += step` (exit on unsigned carry, or signed
    /// overflow for signed counters), then a 16-bit `var <= limit` (`>=` when
    /// counting down) test that jumps back to the body.
    pub(super) fn gen_for_loop_word(
        &mut self,
        var: &str,
        from: &Expr,
        to: &Expr,
        step: Option<&Expr>,
        body: &[crate::compiler::ast::Stmt],
        signed: bool,
    ) {
        let zp = self.alloc_var(var);
        let perm_before = self.perm_zp;
        let mut temps = 0u8;

        // from → var (16-bit)
        if !self.gen_word_assign(zp, from) {
            self.eval_expr(from);
            self.emit(0x85);
            self.emit(zp);
            self.emit(0xA9);
            self.emit(0x00);
            self.emit(0x85);
            self.emit(zp + 1);
        }
        // a 16-bit operand: constant or a 2-byte snapshot in permanent ZP
        #[derive(Clone, Copy)]
        enum W {
            Imm(u16),
            Zp(u8),
        }
        let mut snapshot = |this: &mut Self, e: &Expr| -> W {
            if let Expr::Number(n) = e {
                return W::Imm(*n as u16);
            }
            let z = this.perm_zp;
            this.perm_zp += 2;
            temps += 2;
            if !this.gen_word_assign(z, e) {
                this.eval_expr_word(e, z, z + 1);
            }
            W::Zp(z)
        };
        let limit = snapshot(self, to);
        let (step_op, down) = match step {
            None => (W::Imm(1), false),
            Some(Expr::Number(n)) if *n < 0 => (W::Imm(n.unsigned_abs()), true),
            Some(e) => (snapshot(self, e), false),
        };
        let lo = |w: W| match w {
            W::Imm(v) => Operand::Imm(v as u8),
            W::Zp(z) => Operand::Zp(z),
        };
        let hi = |w: W| match w {
            W::Imm(v) => Operand::Imm((v >> 8) as u8),
            W::Zp(z) => Operand::Zp(z + 1),
        };

        self.emit(JMP); // JMP test
        let entry = self.code.len();
        self.emit16(0);

        self.break_patches.push(vec![]);
        self.continue_patches.push(vec![]);
        let body_top = self.current_addr();
        self.gen_stmts(body);
        self.tmp_zp = super::TMP_BASE;

        let cont = self.current_addr();
        for pos in self.continue_patches.pop().unwrap_or_default() {
            self.patch_abs(pos, cont);
        }
        // var ± step; leave on wrap-around (carry / borrow, or V when signed)
        let (prefix, alu) = if down { (0x38u8, 0xE9u8) } else { (0x18, 0x69) };
        self.emit(prefix);
        self.emit(0xA5);
        self.emit(zp);
        self.emit_alu(alu, lo(step_op));
        self.emit(0x85);
        self.emit(zp);
        self.emit(0xA5);
        self.emit(zp + 1);
        self.emit_alu(alu, hi(step_op));
        self.emit(0x85);
        self.emit(zp + 1);
        let exit_op = if signed { 0x70 } else if down { BCC } else { BCS }; // BVS / BCC / BCS
        self.emit(exit_op);
        let exit_branch = self.code.len();
        self.emit(0);

        // test: continue while var <= limit (up) / var >= limit (down)
        let test = self.current_addr();
        self.patch_abs(entry, test);
        let t = self.alloc_tmp();
        // big - small, no borrow (C = 1) ⇒ continue
        let (big_is_var, small) = if down { (true, limit) } else { (false, limit) };
        // hi byte of the subtrahend (flipped for a signed compare) → t
        if big_is_var {
            self.emit_alu(0xA9, hi(small)); // LDA limit_hi
        } else {
            self.emit(0xA5);
            self.emit(zp + 1); // LDA var_hi
        }
        if signed {
            self.emit_alu(0x49, Operand::Imm(0x80)); // EOR #$80
        }
        self.emit(0x85);
        self.emit(t);
        self.emit(0x38); // SEC
        if big_is_var {
            self.emit(0xA5);
            self.emit(zp); // LDA var_lo
            self.emit_alu(0xE9, lo(small)); // SBC limit_lo
            self.emit(0xA5);
            self.emit(zp + 1); // LDA var_hi
        } else {
            self.emit_alu(0xA9, lo(small)); // LDA limit_lo
            self.emit(0xE5);
            self.emit(zp); // SBC var_lo
            self.emit_alu(0xA9, hi(small)); // LDA limit_hi
        }
        if signed {
            self.emit_alu(0x49, Operand::Imm(0x80)); // EOR #$80
        }
        self.emit(0xE5);
        self.emit(t); // SBC t
        // BCC +3 (skip the JMP when the test fails); JMP body_top
        self.emit(BCC);
        self.emit(3);
        self.emit(JMP);
        self.emit16(body_top);

        let loop_end = self.current_addr();
        self.patch_bxx(exit_branch, loop_end);
        for pos in self.break_patches.pop().unwrap_or_default() {
            self.patch_abs(pos, loop_end);
        }
        if self.perm_zp == perm_before + temps {
            self.perm_zp = perm_before;
        }
    }

    // ── increments ──────────────────────────────────────────────────────────

    /// Recognise `x = x + c`, `x = c + x`, `x = x - c` where the constant step
    /// is cheaper as repeated INC/DEC: ±1 or ±2 for byte variables (2 bytes
    /// per INC vs. 7 for LDA/CLC/ADC/STA), ±1 for word variables.
    /// Float variables are excluded (their `+ 1` means +1.0 = high byte).
    pub(super) fn self_increment(&self, name: &str, expr: &Expr) -> Option<i16> {
        let limit = match self.var_types.get(name) {
            None | Some(VarType::Int) => 2,
            Some(VarType::Word) => 1,
            _ => return None,
        };
        self.var_addr(name)?;
        let is_self = |e: &Expr| matches!(e, Expr::Var(v) if v == name);
        let delta = match expr {
            Expr::BinOp(l, BinOp::Add, r) => match (&**l, &**r) {
                (x, Expr::Number(n)) | (Expr::Number(n), x) if is_self(x) => *n,
                _ => return None,
            },
            Expr::BinOp(l, BinOp::Sub, r) => match (&**l, &**r) {
                (x, Expr::Number(n)) if is_self(x) => -*n,
                _ => return None,
            },
            _ => return None,
        };
        (delta != 0 && delta.abs() <= limit).then_some(delta)
    }

    // ── arithmetic / bitwise ────────────────────────────────────────────────

    /// `+ - and or xor` with a directly addressable operand:
    /// `LDA l; CLC; ADC #n` instead of staging l in scratch zero page.
    /// Returns false (nothing emitted) when the generic path must be used.
    pub(super) fn emit_binop_direct(&mut self, l: &Expr, op: &BinOp, r: &Expr) -> bool {
        let (imm_op, prefix) = match op {
            BinOp::Add => (0x69, Some(0x18)), // ADC, CLC
            BinOp::Sub => (0xE9, Some(0x38)), // SBC, SEC
            BinOp::And => (0x29, None),
            BinOp::Or => (0x09, None),
            BinOp::Xor => (0x49, None),
            BinOp::Mul => {
                self.emit_mul8(l, r);
                return true;
            }
            BinOp::Div | BinOp::Mod => {
                self.emit_divmod8(l, r, matches!(op, BinOp::Mod));
                return true;
            }
            _ => return false,
        };

        // Identities: x+0, x-0, x or 0, x xor 0, x and 255 → x
        if let Expr::Number(n) = r {
            let identity = match op {
                BinOp::And => *n == 255,
                _ => *n == 0,
            };
            if identity {
                self.eval_expr(l);
                return true;
            }
        }

        if let Some(operand) = self.simple_operand(r) {
            self.eval_expr(l);
            if let Some(p) = prefix {
                self.emit(p);
            }
            self.emit_alu(imm_op, operand);
            return true;
        }

        // Left side simple, right side complex: evaluate r first. This is only
        // done when it cannot change what l reads (l is a constant, or r has
        // no side effects).
        let may_reorder = matches!(l, Expr::Number(_)) || Self::is_pure(r);
        if !may_reorder {
            return false;
        }
        let Some(l_operand) = self.simple_operand(l) else {
            return false;
        };
        self.eval_expr(r);
        if matches!(op, BinOp::Sub) {
            // l - r: r → tmp; LDA l; SEC; SBC tmp
            let tmp = self.tmp_zp;
            self.tmp_zp += 1;
            self.emit(0x85);
            self.emit(tmp); // STA tmp
            self.emit_alu(0xA9, l_operand); // LDA l
            self.emit(0x38); // SEC
            self.emit(0xE5);
            self.emit(tmp); // SBC tmp
        } else {
            // commutative: A = r; OP l
            if let Some(p) = prefix {
                self.emit(p);
            }
            self.emit_alu(imm_op, l_operand);
        }
        true
    }

    fn alloc_tmp(&mut self) -> u8 {
        let t = self.tmp_zp;
        self.tmp_zp += 1;
        t
    }

    /// Evaluate `e` into a fresh scratch byte (`LDA …; STA tmp`).
    pub(super) fn eval_to_tmp(&mut self, e: &Expr) -> u8 {
        self.eval_expr(e);
        let t = self.alloc_tmp();
        self.emit(0x85);
        self.emit(t); // STA tmp
        t
    }

    /// 8-bit multiply, result (low byte of the product) in A.
    pub(super) fn emit_mul8(&mut self, l: &Expr, r: &Expr) {
        // Constant on the left: multiplication is commutative and a constant
        // has no side effects, so swap it to the right.
        if matches!(l, Expr::Number(_)) && !matches!(r, Expr::Number(_)) {
            return self.emit_mul8(r, l);
        }
        if let Expr::Number(n) = r {
            let c = *n as u8;
            match c {
                0 => {
                    if !Self::is_pure(l) {
                        self.eval_expr(l);
                    }
                    self.emit(0xA9);
                    self.emit(0x00); // LDA #0
                    return;
                }
                1 => return self.eval_expr(l),
                _ if c.is_power_of_two() => {
                    self.eval_expr(l);
                    for _ in 0..c.trailing_zeros() {
                        self.emit(0x0A); // ASL A
                    }
                    return;
                }
                _ => {
                    // Horner's scheme over the bits of c (MSB first):
                    // A = l; per lower bit: ASL A [; CLC; ADC l]
                    let top = 7 - c.leading_zeros();
                    let len = 2 + top + 3 * (c.count_ones() - 1);
                    if len <= 16 {
                        let t = self.eval_to_tmp(l); // A = l as well
                        for bit in (0..top).rev() {
                            self.emit(0x0A); // ASL A
                            if c & (1 << bit) != 0 {
                                self.emit(0x18); // CLC
                                self.emit(0x65);
                                self.emit(t); // ADC l
                            }
                        }
                        return;
                    }
                }
            }
        }

        // Shift-add loop (at most 8 rounds, stops as soon as the multiplier
        // runs out of set bits):
        //        LDA #0
        //        BEQ test
        // add:   CLC
        //        ADC m        ; m = l << i
        // loop:  ASL m
        // test:  LSR q        ; q = r >> i, bit i → C
        //        BCS add
        //        BNE loop
        let m = self.eval_to_tmp(l);
        let q = self.eval_to_tmp(r);
        self.emit(0xA9);
        self.emit(0x00); // LDA #0
        self.emit_bxx(BEQ, 5); // BEQ test
        let add = self.current_addr();
        self.emit(0x18); // CLC
        self.emit(0x65);
        self.emit(m); // ADC m
        let lp = self.current_addr();
        self.emit(0x06);
        self.emit(m); // ASL m
        self.emit(0x46);
        self.emit(q); // LSR q
        let here = self.current_addr();
        let off = self.branch_offset(here, add).unwrap();
        self.emit_bxx(BCS, off);
        let here = self.current_addr();
        let off = self.branch_offset(here, lp).unwrap();
        self.emit_bxx(BNE, off);
    }

    /// 8-bit unsigned division / remainder, result in A.
    /// Division by zero yields quotient 255 and remainder = dividend.
    pub(super) fn emit_divmod8(&mut self, l: &Expr, r: &Expr, want_mod: bool) {
        if let Expr::Number(n) = r {
            let c = *n as u8;
            if c == 1 {
                if want_mod {
                    if !Self::is_pure(l) {
                        self.eval_expr(l);
                    }
                    self.emit(0xA9);
                    self.emit(0x00); // x mod 1 = 0
                } else {
                    self.eval_expr(l);
                }
                return;
            }
            if c.is_power_of_two() {
                self.eval_expr(l);
                if want_mod {
                    self.emit(0x29);
                    self.emit(c - 1); // AND #(c-1)
                } else {
                    for _ in 0..c.trailing_zeros() {
                        self.emit(0x4A); // LSR A
                    }
                }
                return;
            }
        }

        // Restoring shift-subtract division, 8 rounds:
        //        (q = dividend, becomes the quotient; d = divisor operand)
        //        LDA #8
        //        STA cnt
        //        LDA #0       ; A = remainder
        // loop:  ASL q
        //        ROL A
        //        BCS sub      ; 9-bit remainder ≥ 256 > d
        //        CMP d
        //        BCC skip
        // sub:   SBC d        ; C = 1 here
        //        INC q        ; quotient bit (not needed for mod)
        // skip:  DEC cnt
        //        BNE loop
        //        [LDA q]
        let q = self.eval_to_tmp(l);
        let d = match self.simple_operand(r) {
            Some(op) => op,
            None => Operand::Zp(self.eval_to_tmp(r)),
        };
        let cnt = self.alloc_tmp();
        self.emit(0xA9);
        self.emit(0x08);
        self.emit(0x85);
        self.emit(cnt);
        self.emit(0xA9);
        self.emit(0x00);
        let lp = self.current_addr();
        self.emit(0x06);
        self.emit(q); // ASL q
        self.emit(0x2A); // ROL A
        self.emit_bxx(BCS, 4); // BCS sub
        self.emit_alu(0xC9, d); // CMP d
        let inc_len = if want_mod { 0 } else { 2 };
        self.emit_bxx(BCC, 2 + inc_len); // BCC skip
        self.emit_alu(0xE9, d); // SBC d
        if !want_mod {
            self.emit(0xE6);
            self.emit(q); // INC q
        }
        self.emit(0xC6);
        self.emit(cnt); // DEC cnt
        let here = self.current_addr();
        let off = self.branch_offset(here, lp).unwrap();
        self.emit_bxx(BNE, off);
        if !want_mod {
            self.emit(0xA5);
            self.emit(q); // LDA q
        }
    }

    // ── comparisons ─────────────────────────────────────────────────────────

    /// Emit an 8-bit unsigned comparison of `l` and `r`. On return the flags
    /// satisfy the returned predicate exactly when `l pred r` holds.
    pub(super) fn emit_compare(&mut self, l: &Expr, pred: Pred, r: &Expr) -> Pred {
        // Constant on the left: swap sides so it becomes an immediate operand.
        if matches!(l, Expr::Number(_)) && !matches!(r, Expr::Number(_)) {
            return self.emit_compare(r, pred.mirror(), l);
        }

        if let Expr::Number(n) = r {
            let n = *n as u8;
            // Statically decided comparisons.
            if Self::is_pure(l) {
                match (pred, n) {
                    (Pred::Lt, 0) | (Pred::Gt, 255) => return Pred::Never,
                    (Pred::Ge, 0) | (Pred::Le, 255) => return Pred::Always,
                    _ => {}
                }
            }
            // `>` / `<=` need two branches; `> n` ⇔ `>= n+1`, `<= n` ⇔ `< n+1`.
            let (pred, n) = match pred {
                Pred::Gt if n < 255 => (Pred::Ge, n + 1),
                Pred::Le if n < 255 => (Pred::Lt, n + 1),
                p => (p, n),
            };
            // `var ==/!= 0`: LDA already sets Z.
            if n == 0 && matches!(pred, Pred::Eq | Pred::Ne) && matches!(l, Expr::Var(_)) {
                self.eval_expr(l);
                return pred;
            }
            self.eval_expr(l);
            self.emit_alu(0xC9, Operand::Imm(n)); // CMP #n
            return pred;
        }

        if let Some(op) = self.simple_operand(r) {
            self.eval_expr(l);
            self.emit_alu(0xC9, op); // CMP zp
            return pred;
        }
        if matches!(l, Expr::Var(_)) && Self::is_pure(r) {
            // Complex (side-effect free) right side, variable left side:
            // evaluate r and compare it against l with the mirrored predicate.
            self.eval_expr(r);
            let op = self.simple_operand(l).expect("byte var operand");
            self.emit_alu(0xC9, op);
            return pred.mirror();
        }

        // General case, keeping left-to-right evaluation order:
        // l → scratch, r → A, CMP scratch (mirrored predicate).
        self.eval_expr(l);
        let tmp = self.tmp_zp;
        self.tmp_zp += 1;
        self.emit(0x85);
        self.emit(tmp); // STA tmp
        self.eval_expr(r);
        self.emit(0xC5);
        self.emit(tmp); // CMP tmp
        pred.mirror()
    }

    /// Materialise the outcome of the preceding compare as 1 (true) / 0 in A.
    pub(super) fn emit_bool_from_flags(&mut self, pred: Pred) {
        match pred {
            Pred::Always => {
                self.emit(0xA9);
                self.emit(0x01);
            }
            Pred::Never => {
                self.emit(0xA9);
                self.emit(0x00);
            }
            Pred::Ge => {
                self.emit(0xA9);
                self.emit(0x00); // LDA #0 (keeps C)
                self.emit(0x2A); // ROL A → A = C
            }
            Pred::Lt => {
                self.emit(0xA9);
                self.emit(0x00);
                self.emit(0x2A); // ROL A → A = C
                self.emit(0x49);
                self.emit(0x01); // EOR #1
            }
            _ => {
                // B(!pred) false; LDA #1; BNE end; false: LDA #0; end:
                self.emit_skip(pred.negate(), 4);
                self.emit(0xA9);
                self.emit(0x01);
                self.emit(BNE);
                self.emit(0x02);
                self.emit(0xA9);
                self.emit(0x00);
            }
        }
    }

    // ── branches ────────────────────────────────────────────────────────────

    /// Branch over the next `n` bytes when `pred` holds.
    fn emit_skip(&mut self, pred: Pred, n: u8) {
        match pred {
            Pred::Eq => self.emit_bxx(BEQ, n),
            Pred::Ne => self.emit_bxx(BNE, n),
            Pred::Lt => self.emit_bxx(BCC, n),
            Pred::Ge => self.emit_bxx(BCS, n),
            // C && !Z: BEQ over the BCS
            Pred::Gt => {
                self.emit_bxx(BEQ, 2);
                self.emit_bxx(BCS, n);
            }
            // !C || Z
            Pred::Le => {
                self.emit_bxx(BCC, n + 2);
                self.emit_bxx(BEQ, n);
            }
            Pred::Always | Pred::Never => unreachable!("static predicate in emit_skip"),
        }
    }

    fn emit_bxx(&mut self, op: u8, off: u8) {
        self.emit(op);
        self.emit(off);
    }

    /// Relative offset from the byte after a branch at `at` to `target`, if in range.
    fn branch_offset(&self, at: u16, target: u16) -> Option<u8> {
        let d = target as i32 - (at as i32 + 2);
        (-128..=127).contains(&d).then_some(d as i8 as u8)
    }

    /// Jump to `target` when `pred` holds for the preceding compare.
    /// Returns the operand positions of emitted `JMP`s that must be patched
    /// (only for `Target::Forward`).
    pub(super) fn emit_jump_on(&mut self, pred: Pred, target: Target) -> Vec<usize> {
        match pred {
            Pred::Never => return vec![],
            Pred::Always => {
                self.emit(JMP);
                return match target {
                    Target::Forward => {
                        let p = self.code.len();
                        self.emit16(0);
                        vec![p]
                    }
                    Target::Back(a) => {
                        self.emit16(a);
                        vec![]
                    }
                };
            }
            _ => {}
        }

        if let Target::Back(addr) = target {
            let here = self.current_addr();
            let direct = match pred {
                Pred::Eq | Pred::Ne | Pred::Lt | Pred::Ge => {
                    self.branch_offset(here, addr).map(|o| vec![(o, pred)])
                }
                // BEQ +2; BCS target
                Pred::Gt => self.branch_offset(here + 2, addr).map(|o| vec![(o, pred)]),
                // BCC target; BEQ target
                Pred::Le => match (
                    self.branch_offset(here, addr),
                    self.branch_offset(here + 2, addr),
                ) {
                    (Some(a), Some(b)) => Some(vec![(a, Pred::Lt), (b, Pred::Eq)]),
                    _ => None,
                },
                _ => unreachable!(),
            };
            if let Some(branches) = direct {
                for (off, p) in branches {
                    match p {
                        Pred::Eq => self.emit_bxx(BEQ, off),
                        Pred::Ne => self.emit_bxx(BNE, off),
                        Pred::Lt => self.emit_bxx(BCC, off),
                        Pred::Ge => self.emit_bxx(BCS, off),
                        Pred::Gt => {
                            self.emit_bxx(BEQ, 2);
                            self.emit_bxx(BCS, off);
                        }
                        _ => unreachable!(),
                    }
                }
                return vec![];
            }
            // Out of range: skip-over-JMP with a known destination.
            self.emit_skip(pred.negate(), 3);
            self.emit(JMP);
            self.emit16(addr);
            return vec![];
        }

        self.emit_skip(pred.negate(), 3);
        self.emit(JMP);
        let p = self.code.len();
        self.emit16(0);
        vec![p]
    }

    /// Emit code that jumps to `target` when `cond` evaluates to `jump_when`
    /// and falls through otherwise. Returns the `JMP` operand positions to
    /// patch for `Target::Forward`.
    pub(super) fn gen_cond_jump(
        &mut self,
        cond: &Expr,
        jump_when: bool,
        truth: Truth,
        target: Target,
    ) -> Vec<usize> {
        match cond {
            Expr::Number(n) => {
                let v = match truth {
                    Truth::NonZero => *n as u8 != 0,
                    Truth::EqualsOne => *n as u8 == 1,
                };
                let p = if v == jump_when { Pred::Always } else { Pred::Never };
                self.emit_jump_on(p, target)
            }
            // `not x` is 1 exactly when x == 0.
            Expr::Not(inner) => self.gen_cond_jump(inner, !jump_when, Truth::NonZero, target),
            Expr::BinOp(l, op, r)
                if Pred::from_binop(op).is_some() && self.is_byte_compare(l, r) =>
            {
                let pred = self.emit_compare(l, Pred::from_binop(op).unwrap(), r);
                let pred = if jump_when { pred } else { pred.negate() };
                self.emit_jump_on(pred, target)
            }
            // Logical and/or of 0/1 values; the right side is skipped only if
            // it has no side effects.
            Expr::BinOp(l, op @ (BinOp::And | BinOp::Or), r)
                if Self::is_bool(l) && Self::is_bool(r) && Self::is_pure(r) =>
            {
                // `and` jumps on false as soon as one side is false;
                // `or` jumps on true as soon as one side is true.
                let short_circuit_value = matches!(op, BinOp::Or);
                if jump_when == short_circuit_value {
                    let mut p = self.gen_cond_jump(l, jump_when, truth, target);
                    p.extend(self.gen_cond_jump(r, jump_when, truth, target));
                    p
                } else {
                    // Jump only if both sides decide it: skip r when l already
                    // settles the opposite outcome.
                    let skip = self.gen_cond_jump(l, !jump_when, truth, Target::Forward);
                    let p = self.gen_cond_jump(r, jump_when, truth, target);
                    let here = self.current_addr();
                    for pos in skip {
                        self.patch_abs(pos, here);
                    }
                    p
                }
            }
            _ => {
                self.eval_expr(cond);
                let pred = match truth {
                    Truth::NonZero => {
                        if !matches!(cond, Expr::Var(_)) {
                            self.emit(0xC9);
                            self.emit(0x00); // CMP #0
                        }
                        Pred::Ne
                    }
                    Truth::EqualsOne => {
                        self.emit(0xC9);
                        self.emit(0x01); // CMP #1
                        Pred::Eq
                    }
                };
                let pred = if jump_when { pred } else { pred.negate() };
                self.emit_jump_on(pred, target)
            }
        }
    }
}
