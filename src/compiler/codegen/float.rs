//! Floating point in the code generator (1.6.4): `SINGLE` / `DOUBLE` values are
//! 5-byte numbers in RAM, computed with the runtime in `float_lib.rs`.
//!
//! An expression is evaluated in floating point when it involves a SINGLE /
//! DOUBLE variable, array element or function, a real-only built-in (`sqr`,
//! `fix`, …), `^`, or a literal only floating point can hold (`1E6`, `300.5`).
//! Plain decimal literals keep their Q8.8 meaning elsewhere, so existing
//! programs compile unchanged.
//!
//! Evaluation leaves the result in the runtime's accumulator (FAC). A simple
//! right operand (variable, constant, constant-index element) is used straight
//! from memory; anything else is evaluated first and pushed on the runtime's
//! float stack. Calls go through `flt_call` and are patched when the runtime
//! is appended after the program; constants are pooled the same way.

use super::opt::Pred;
use super::Codegen;
use crate::compiler::ast::{BinOp, Expr, Stmt, VarType};

/// Built-in functions that compute in floating point (a user function with
/// the same name wins).
pub(super) fn is_real_builtin(name: &str) -> bool {
    matches!(
        name,
        "fix" | "cint" | "csng" | "cdbl" | "sqr" | "exp" | "log" | "atn" | "tan"
    )
}

/// An operand used straight from memory.
#[derive(Clone, Copy)]
enum ROp {
    Addr(u16),
    Const(f64),
}

impl Codegen {
    // ── runtime calls and data ───────────────────────────────────────────────

    /// Zero-page pointer of the runtime (allocated on first use).
    fn flt_ptr(&mut self) -> u8 {
        // allocated in pre_scan (`program_uses_reals`)
        self.flt_zp.expect("floating point used but not detected by the pre-scan")
    }

    /// `JSR routine` (patched when the runtime is appended).
    pub(super) fn flt_call(&mut self, routine: &'static str) {
        self.flt_ptr();
        self.emit(0x20);
        let pos = self.code.len();
        self.emit16(0);
        self.flt_patches.push((pos, routine));
        self.a_cache = None;
    }

    fn flt_const_ay(&mut self, v: f64) {
        let b = super::float_lib::to_mflpt(v);
        let idx = match self.flt_consts.iter().position(|c| *c == b) {
            Some(i) => i,
            None => {
                self.flt_consts.push(b);
                self.flt_consts.len() - 1
            }
        };
        self.emit(0xA9);
        let lo = self.code.len();
        self.emit(0);
        self.emit(0xA0);
        let hi = self.code.len();
        self.emit(0);
        self.flt_const_patches.push((lo, hi, idx));
    }

    fn emit_ay(&mut self, op: ROp) {
        match op {
            ROp::Addr(a) => {
                self.emit(0xA9);
                self.emit(a as u8);
                self.emit(0xA0);
                self.emit((a >> 8) as u8);
            }
            ROp::Const(v) => self.flt_const_ay(v),
        }
    }

    /// FAC → memory at `addr`.
    pub(super) fn flt_store(&mut self, addr: u16) {
        self.emit_ay(ROp::Addr(addr));
        self.flt_call("f_store");
    }

    /// Append the runtime and its constants (end of compile).
    pub(super) fn emit_float_runtime(&mut self) {
        if self.flt_patches.is_empty() {
            return;
        }
        let base = self.current_addr();
        let zp = self.flt_zp.expect("float pointer");
        let (code, labels) = super::float_lib::float_runtime(base, zp);
        self.listing_symbol("ub_float_runtime");
        let start = self.code.len();
        self.code.extend_from_slice(&code);
        self.listing_data(start, "ub_float_runtime");
        for (pos, name) in std::mem::take(&mut self.flt_patches) {
            let a = labels[name];
            self.code[pos] = a as u8;
            self.code[pos + 1] = (a >> 8) as u8;
        }
        let cbase = self.current_addr();
        let cstart = self.code.len();
        for c in std::mem::take(&mut self.flt_consts) {
            self.code.extend_from_slice(&c);
        }
        if self.code.len() > cstart {
            self.listing_data(cstart, "ub_float_constants");
        }
        for (lo, hi, idx) in std::mem::take(&mut self.flt_const_patches) {
            let a = cbase + 5 * idx as u16;
            self.code[lo] = a as u8;
            self.code[hi] = (a >> 8) as u8;
        }
    }

    // ── typing ───────────────────────────────────────────────────────────────

    fn is_user_routine(&self, name: &str) -> bool {
        self.sub_params.contains_key(name)
    }

    /// Does `e` evaluate in floating point?
    pub(super) fn is_real_expr(&self, e: &Expr) -> bool {
        match e {
            Expr::Var(n) => self.real_vars.contains_key(n),
            Expr::ArrayGet(n, _) => self.real_arrays.contains(n.as_str()),
            // once a program uses floating point, every decimal literal is one
            Expr::FixedLit(..) => self.flt_zp.is_some(),
            Expr::FnCall(n, _) => {
                self.real_fns.contains_key(n) || (is_real_builtin(n) && !self.is_user_routine(n))
            }
            Expr::BinOp(_, BinOp::Pow, _) => true,
            Expr::BinOp(l, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, r) => {
                !self.is_string_expr(e) && (self.is_real_expr(l) || self.is_real_expr(r))
            }
            Expr::Abs(x) | Expr::Sgn(x) | Expr::FixedToInt(x) | Expr::Sin(x) | Expr::Cos(x) => {
                self.is_real_expr(x)
            }
            _ => false,
        }
    }

    fn real_operand(&self, e: &Expr) -> Option<ROp> {
        match e {
            Expr::Var(n) => self.real_vars.get(n).map(|&a| ROp::Addr(a)),
            Expr::Number(n) => Some(ROp::Const(*n as f64)),
            Expr::FixedLit(_, v, _) => Some(ROp::Const(*v)),
            Expr::ArrayGet(a, idx) if self.real_arrays.contains(a.as_str()) => match idx.as_ref() {
                Expr::Number(k) => {
                    let base = self.arrays.get(a).copied().unwrap_or(0xC000);
                    Some(ROp::Addr(base.wrapping_add(5 * *k as u16)))
                }
                _ => None,
            },
            _ => None,
        }
    }

    // ── evaluation ───────────────────────────────────────────────────────────

    /// Evaluate `e` in floating point into FAC.
    pub(super) fn eval_real(&mut self, e: &Expr) {
        if let Some(op) = self.real_operand(e) {
            self.emit_ay(op);
            self.flt_call("f_load");
            return;
        }
        match e {
            Expr::BinOp(l, BinOp::Sub, r) if matches!(l.as_ref(), Expr::Number(0)) => {
                let r = (**r).clone();
                self.eval_real(&r);
                self.flt_call("f_neg");
            }
            Expr::BinOp(l, op @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div), r) => {
                let (l, r) = ((**l).clone(), (**r).clone());
                let (mem, pop) = match op {
                    BinOp::Add => ("f_add", "f_addp"),
                    BinOp::Sub => ("f_sub", "f_subp"),
                    BinOp::Mul => ("f_mul", "f_mulp"),
                    _ => ("f_div", "f_divp"),
                };
                let commutative = matches!(op, BinOp::Add | BinOp::Mul);
                if let Some(rop) = self.real_operand(&r) {
                    self.eval_real(&l);
                    self.emit_ay(rop);
                    self.flt_call(mem);
                } else if let (true, Some(lop)) = (commutative, self.real_operand(&l)) {
                    self.eval_real(&r);
                    self.emit_ay(lop);
                    self.flt_call(mem);
                } else {
                    self.eval_real(&r);
                    self.flt_call("f_push");
                    self.eval_real(&l);
                    self.flt_call(pop);
                }
            }
            Expr::BinOp(l, BinOp::Pow, r) => {
                let (l, r) = ((**l).clone(), (**r).clone());
                self.eval_pow(&l, &r);
            }
            Expr::ArrayGet(a, idx) if self.real_arrays.contains(a.as_str()) => {
                let (a, idx) = (a.clone(), (**idx).clone());
                self.emit_real_elem_addr(&a, &idx);
                self.flt_call("f_load");
            }
            Expr::FnCall(name, args) if self.real_fns.contains_key(name) => {
                let (name, args) = (name.clone(), args.clone());
                self.emit_param_stores(&name, &args);
                self.emit(0x20); // JSR fn
                if let Some(&addr) = self.subs.get(&name) {
                    self.emit16(addr);
                } else {
                    let patch = self.code.len();
                    self.emit16(0);
                    self.sub_patches.push((patch, name.clone(), 0));
                }
                let slot = self.real_fns[&name];
                self.emit_ay(ROp::Addr(slot));
                self.flt_call("f_load");
            }
            Expr::FnCall(name, args) if is_real_builtin(name) && !self.is_user_routine(name) => {
                let name = name.clone();
                let arg = args.first().cloned().unwrap_or(Expr::Number(0));
                self.eval_real(&arg);
                match name.as_str() {
                    "fix" => self.flt_call("f_fix"),
                    "cint" => {
                        self.flt_call("f_addhalf");
                        self.flt_call("f_fix");
                    }
                    "csng" | "cdbl" => {}
                    "sqr" => self.flt_call("f_sqr"),
                    "exp" => self.flt_call("f_exp"),
                    "log" => self.flt_call("f_log"),
                    "atn" => self.flt_call("f_atn"),
                    "tan" => self.flt_call("f_tan"),
                    _ => {}
                }
            }
            Expr::Abs(x) => {
                let x = (**x).clone();
                self.eval_real(&x);
                self.flt_call("f_abs");
            }
            Expr::Sgn(x) => {
                let x = (**x).clone();
                self.eval_real(&x);
                self.flt_call("f_sgn");
            }
            Expr::FixedToInt(x) => {
                let x = (**x).clone();
                self.eval_real(&x);
                self.flt_call("f_int");
            }
            Expr::Sin(x) => {
                let x = (**x).clone();
                self.eval_real(&x);
                self.flt_call("f_sin");
            }
            Expr::Cos(x) => {
                let x = (**x).clone();
                self.eval_real(&x);
                self.flt_call("f_cos");
            }
            Expr::Val(s) => {
                let s = (**s).clone();
                let src = self.str_text_ptr(&s);
                self.emit(0xA5);
                self.emit(src);
                self.emit(0xA4);
                self.emit(src + 1); // LDA src; LDY src+1
                self.flt_call("f_in");
            }
            _ => self.real_from_int(e),
        }
    }

    /// An integer / Q8.8 expression converted into FAC.
    fn real_from_int(&mut self, e: &Expr) {
        let t = self.tmp_zp;
        self.tmp_zp += 2;
        if self.is_float_like(e) {
            // Q8.8: raw 16-bit value / 256
            if !self.gen_word_assign(t, e) {
                self.eval_expr_word(e, t, t + 1);
            }
            self.emit(0xA5);
            self.emit(t);
            self.emit(0xA4);
            self.emit(t + 1);
            self.flt_call("f_fromu16");
            self.flt_call("f_q88dn");
        } else if self.is_signed_expr(e) || self.can_be_word_result(e) {
            if !self.gen_word_assign(t, e) {
                self.eval_expr_word(e, t, t + 1);
            }
            self.emit(0xA5);
            self.emit(t);
            self.emit(0xA4);
            self.emit(t + 1);
            self.flt_call(if self.is_signed_expr(e) { "f_froms16" } else { "f_fromu16" });
        } else {
            self.eval_expr(e);
            self.flt_call("f_fromu8");
        }
    }

    /// `l ^ r`: small integer constants by repeated squaring (exact), the rest
    /// via exp(r * log(l)).
    fn eval_pow(&mut self, l: &Expr, r: &Expr) {
        let k = match r {
            Expr::Number(n) => Some(*n as i32),
            Expr::FixedLit(_, v, _) if v.fract() == 0.0 && v.abs() <= 255.0 => Some(*v as i32),
            _ => None,
        };
        match k {
            Some(k) if (-255..=255).contains(&k) => {
                self.eval_real(l);
                self.emit(0xA9);
                self.emit(k.unsigned_abs() as u8); // LDA #|k|
                self.flt_call("f_powi");
                if k < 0 {
                    self.flt_call("f_push");
                    self.emit_ay(ROp::Const(1.0));
                    self.flt_call("f_load");
                    self.flt_call("f_divp"); // 1 / x^|k|
                }
            }
            _ => {
                self.eval_real(r);
                self.flt_call("f_push");
                self.eval_real(l);
                self.flt_call("f_powp");
            }
        }
    }

    /// A/Y = address of `arr(idx)` (5-byte elements).
    pub(super) fn emit_real_elem_addr(&mut self, arr: &str, idx: &Expr) {
        let base = self.arrays.get(arr).copied().unwrap_or(0xC000);
        if let Expr::Number(k) = idx {
            let a = base.wrapping_add(5 * *k as u16);
            self.emit_ay(ROp::Addr(a));
            return;
        }
        let t = self.tmp_zp;
        self.tmp_zp += 4;
        if !self.gen_word_assign(t, idx) {
            self.eval_expr_word(idx, t, t + 1);
        }
        // t*5 = t*4 + t
        for k in 0..2 {
            self.emit(0xA5);
            self.emit(t + k);
            self.emit(0x85);
            self.emit(t + 2 + k);
        }
        for _ in 0..2 {
            self.emit(0x06);
            self.emit(t); // ASL lo
            self.emit(0x26);
            self.emit(t + 1); // ROL hi
        }
        self.emit(0x18); // CLC
        self.emit(0xA5);
        self.emit(t);
        self.emit(0x65);
        self.emit(t + 2);
        self.emit(0x85);
        self.emit(t);
        self.emit(0xA5);
        self.emit(t + 1);
        self.emit(0x65);
        self.emit(t + 3);
        self.emit(0x85);
        self.emit(t + 1);
        self.emit(0x18); // CLC
        self.emit(0xA5);
        self.emit(t);
        self.emit(0x69);
        self.emit(base as u8); // ADC #<base
        self.emit(0x48); // PHA (lo)
        self.emit(0xA5);
        self.emit(t + 1);
        self.emit(0x69);
        self.emit((base >> 8) as u8); // ADC #>base
        self.emit(0xA8); // TAY
        self.emit(0x68); // PLA
    }

    // ── conversions and comparisons ──────────────────────────────────────────

    /// Compare two expressions of which at least one is floating point.
    /// Leaves the flags for the returned predicate (A = f_cmp result).
    pub(super) fn real_compare(&mut self, l: &Expr, op: &BinOp, r: &Expr) -> Pred {
        if let Some(rop) = self.real_operand(r) {
            self.eval_real(l);
            self.emit_ay(rop);
            self.flt_call("f_cmp");
        } else {
            self.eval_real(r);
            self.flt_call("f_push");
            self.eval_real(l);
            self.flt_call("f_cmpp");
        }
        let (v, p) = match op {
            BinOp::Eq => (0x00, Pred::Eq),
            BinOp::NotEq => (0x00, Pred::Ne),
            BinOp::Lt => (0xFF, Pred::Eq),
            BinOp::GtEq => (0xFF, Pred::Ne),
            BinOp::Gt => (0x01, Pred::Eq),
            _ => (0x01, Pred::Ne), // LtEq
        };
        self.emit(0xC9);
        self.emit(v); // CMP #v
        p
    }

    /// Evaluate a floating-point expression and leave it rounded in A (lo) / Y (hi).
    pub(super) fn real_to_ay(&mut self, e: &Expr, _signed: bool) {
        self.eval_real(e);
        self.flt_call("f_rndu16"); // negative values: two's complement
    }

    /// Floating-point expression → Q8.8 raw value in A (lo) / Y (hi).
    pub(super) fn real_to_q88_ay(&mut self, e: &Expr) {
        self.eval_real(e);
        self.flt_call("f_toq88");
    }

    /// `print` / text: A/Y = pointer to the decimal text of `e`.
    pub(super) fn real_text_ay(&mut self, e: &Expr) {
        self.eval_real(e);
        self.flt_call("f_out");
    }

    // ── statements ───────────────────────────────────────────────────────────

    /// `x = e` / `var x = e` for a floating-point variable.
    pub(super) fn real_assign(&mut self, name: &str, e: &Expr) {
        let addr = self.real_vars[name];
        if let Expr::StringLit(_) = e {
            return; // (type error caught elsewhere)
        }
        self.eval_real(e);
        self.flt_store(addr);
    }

    /// `arr(i) = e` for a floating-point array.
    pub(super) fn real_array_set(&mut self, arr: &str, idx: &Expr, val: &Expr) {
        // address first (it may need integer temps), kept on the CPU stack
        // while the value is computed (the value may call functions)
        self.emit_real_elem_addr(arr, idx);
        self.emit(0x48); // PHA lo
        self.emit(0x98); // TYA
        self.emit(0x48); // PHA hi
        self.eval_real(val);
        self.emit(0x68); // PLA hi
        self.emit(0xA8); // TAY
        self.emit(0x68); // PLA lo
        self.flt_call("f_store");
    }

    /// `inc x` / `dec x` on a floating-point variable.
    pub(super) fn real_inc(&mut self, name: &str, delta: f64) {
        let addr = self.real_vars[name];
        self.emit_ay(ROp::Addr(addr));
        self.flt_call("f_load");
        self.emit_ay(ROp::Const(delta));
        self.flt_call("f_add");
        self.flt_store(addr);
    }

    /// `for x = a to b [step c]` with a floating-point counter.
    pub(super) fn gen_for_loop_real(
        &mut self,
        var: &str,
        from: &Expr,
        to: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
    ) {
        let ctr = self.real_vars[var];
        let header_frame = self.spill_enter(&format!("{from:?}{to:?}{step:?}"));
        let slots = self.emit_inline_zeros(10);
        let (lim, stp) = (slots, slots + 5);
        self.eval_real(from);
        self.flt_store(ctr);
        self.eval_real(to);
        self.flt_store(lim);
        let step_sign = match step {
            None => Some(true),
            Some(Expr::Number(n)) => Some(*n >= 0),
            Some(Expr::FixedLit(_, v, _)) => Some(*v >= 0.0),
            _ => None,
        };
        match step {
            Some(s) => {
                let s = s.clone();
                self.eval_real(&s);
            }
            None => {
                self.emit_ay(ROp::Const(1.0));
                self.flt_call("f_load");
            }
        }
        self.flt_store(stp);
        self.spill_exit(header_frame, false);

        self.emit(0x4C); // JMP test
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
        // x = x + step
        self.emit_ay(ROp::Addr(ctr));
        self.flt_call("f_load");
        self.emit_ay(ROp::Addr(stp));
        self.flt_call("f_add");
        self.flt_store(ctr);
        // test: continue while x <= limit (x >= limit when counting down)
        let test = self.current_addr();
        self.patch_abs(entry, test);
        self.emit_ay(ROp::Addr(ctr));
        self.flt_call("f_load");
        self.emit_ay(ROp::Addr(lim));
        self.flt_call("f_cmp"); // A = $FF / 0 / 1
        match step_sign {
            Some(up) => {
                self.emit(0xC9);
                self.emit(if up { 0x01 } else { 0xFF }); // CMP #past
                self.emit(0xF0);
                self.emit(0x03); // BEQ +3 (past the limit: leave)
                self.emit(0x4C);
                self.emit16(body_top); // JMP body
            }
            None => {
                // sign of the step at run time: bit 7 of its first mantissa byte
                self.emit(0xAA); // TAX (compare result)
                self.emit(0xAD);
                self.emit16(stp + 1); // LDA stp+1
                self.emit(0x30);
                self.emit(0x07); // BMI down
                self.emit(0xE0);
                self.emit(0x01); // CPX #1
                self.emit(0xF0);
                self.emit(0x0A); // BEQ end (x > limit)
                self.emit(0x4C);
                self.emit16(body_top); // JMP body
                // down:
                self.emit(0xE0);
                self.emit(0xFF); // CPX #$FF
                self.emit(0xF0);
                self.emit(0x03); // BEQ end (x < limit)
                self.emit(0x4C);
                self.emit16(body_top); // JMP body
            }
        }
        let end = self.current_addr();
        for pos in self.break_patches.pop().unwrap_or_default() {
            self.patch_abs(pos, end);
        }
    }

    /// `input x` for a floating-point variable: read a line into a hidden
    /// string and convert it.
    pub(super) fn real_input(&mut self, prompt: &Option<String>, var: &str) {
        let hidden = "input__real".to_string();
        self.var_types.insert(hidden.clone(), VarType::Str);
        let st = Stmt::Input { prompt: prompt.clone(), var: hidden.clone() };
        self.gen_stmt_inner(&st);
        let zp = self.alloc_var(&hidden);
        self.emit(0xA5);
        self.emit(zp);
        self.emit(0xA4);
        self.emit(zp + 1); // LDA ptr; LDY ptr+1
        self.flt_call("f_in");
        let addr = self.real_vars[var];
        self.flt_store(addr);
    }
}

/// Does the program use floating point at all? (Debug-text scan: real types,
/// `^`, real-only literals, real built-ins.) Over-approximation only costs the
/// runtime's 2-byte zero-page pointer.
pub(super) fn program_uses_reals(stmts: &[Stmt]) -> bool {
    let text = format!("{stmts:?}");
    if text.contains("Some(Real)") || text.contains("Some(RealArray)") || text.contains(", Pow, ") {
        return true;
    }
    if text.contains(", true)") && text.contains("FixedLit(") {
        // a real-only literal: FixedLit(q, v, true)
        let mut rest = text.as_str();
        while let Some(i) = rest.find("FixedLit(") {
            rest = &rest[i + 9..];
            if let Some(end) = rest.find(')') {
                if rest[..end].ends_with("true") {
                    return true;
                }
            }
        }
    }
    ["sqr", "exp", "log", "atn", "tan", "fix", "cint", "csng", "cdbl"]
        .iter()
        .any(|f| text.contains(&format!("FnCall(\"{f}\"")))
}

/// Visit every statement, nested bodies included.
fn each_stmt(stmts: &[Stmt], f: &mut dyn FnMut(&Stmt)) {
    for st in stmts {
        f(st);
        match st {
            Stmt::If(_, t, e) => {
                each_stmt(t, f);
                if let Some(e) = e {
                    each_stmt(e, f);
                }
            }
            Stmt::Loop(_, b)
            | Stmt::WhileLoop(_, b)
            | Stmt::RepeatLoop(b, _)
            | Stmt::Block(b)
            | Stmt::SubDef(_, _, b)
            | Stmt::FnDef(_, _, _, b) => each_stmt(b, f),
            Stmt::ForLoop { body, .. } => each_stmt(body, f),
            Stmt::Select { cases, else_body, .. } => {
                for (_, b) in cases {
                    each_stmt(b, f);
                }
                if let Some(e) = else_body {
                    each_stmt(e, f);
                }
            }
            _ => {}
        }
    }
}

impl Codegen {
    /// Pre-scan: floating-point variables (typed, and untyped ones initialised
    /// with a floating-point value), function results and parameters. Fills the
    /// maps with placeholder addresses and returns the number of 5-byte slots.
    pub(super) fn scan_reals(&mut self, stmts: &[Stmt]) -> u16 {
        let mut slots = 0u16;
        let mut typed = vec![];
        let mut fns = vec![];
        let mut params: Vec<(String, usize)> = vec![];
        each_stmt(stmts, &mut |st| match st {
            Stmt::VarDecl { name, vtype: Some(VarType::Real), .. } => typed.push(name.clone()),
            Stmt::FnDef(name, ps, _, _) | Stmt::SubDef(name, ps, _) => {
                if ret_is_real(st) {
                    fns.push(name.clone());
                }
                for (i, (_, t)) in ps.iter().enumerate() {
                    if matches!(t, Some(VarType::Real)) {
                        params.push((name.clone(), i));
                    }
                }
            }
            _ => {}
        });
        for n in typed {
            self.real_vars.entry(n).or_insert(0);
        }
        for n in fns {
            self.real_fns.entry(n).or_insert(0);
        }
        // untyped `var x = <floating-point value>` (twice: `var b = a * 2` after `var a = 1E3`)
        for _ in 0..2 {
            let mut found = vec![];
            each_stmt(stmts, &mut |st| {
                if let Stmt::VarDecl { name, vtype: None, expr } = st {
                    if !self.real_vars.contains_key(name) && self.is_real_expr(expr) {
                        found.push(name.clone());
                    }
                }
            });
            for n in found {
                self.real_vars.insert(n, 0);
            }
        }
        slots += self.real_vars.len() as u16 + self.real_fns.len() as u16 + params.len() as u16;
        for (sub, i) in params {
            let v = self.real_params.entry(sub).or_default();
            if v.len() <= i {
                v.resize(i + 1, None);
            }
            v[i] = Some(0);
        }
        slots
    }

    /// Give the scanned slots their addresses, from `self.array_ptr` up.
    pub(super) fn place_reals(&mut self) {
        let start = self.array_ptr;
        let mut names: Vec<String> = self.real_vars.keys().cloned().collect();
        names.sort();
        for n in names {
            self.real_vars.insert(n, self.array_ptr);
            self.array_ptr = self.array_ptr.wrapping_add(5);
        }
        let mut fns: Vec<String> = self.real_fns.keys().cloned().collect();
        fns.sort();
        for n in fns {
            self.real_fns.insert(n, self.array_ptr);
            self.array_ptr = self.array_ptr.wrapping_add(5);
        }
        let mut subs: Vec<String> = self.real_params.keys().cloned().collect();
        subs.sort();
        for s in subs {
            let v = self.real_params.get_mut(&s).unwrap();
            for slot in v.iter_mut().flatten() {
                *slot = self.array_ptr;
                self.array_ptr = self.array_ptr.wrapping_add(5);
            }
        }
        if self.array_ptr != start {
            self.real_range = Some((start, self.array_ptr));
        }
    }

    fn reals_on(&self) -> bool {
        self.flt_zp.is_some()
    }

    fn is_real_compare(&self, e: &Expr) -> bool {
        match e {
            Expr::BinOp(l, op, r) if Pred::from_binop(op).is_some() => {
                !self.is_string_expr(l)
                    && !self.is_string_expr(r)
                    && (self.is_real_expr(l) || self.is_real_expr(r))
            }
            _ => false,
        }
    }

    // ── hooks into the integer code generator ───────────────────────────────

    /// `eval_expr` (A = byte result): floating point rounded, or a comparison.
    pub(super) fn real_eval_byte(&mut self, e: &Expr) -> bool {
        if !self.reals_on() {
            return false;
        }
        if let Expr::BinOp(l, op, r) = e {
            if self.is_real_compare(e) {
                let (l, op, r) = ((**l).clone(), op.clone(), (**r).clone());
                let p = self.real_compare(&l, &op, &r);
                self.emit_bool_from_flags(p);
                return true;
            }
        }
        if self.is_real_expr(e) {
            self.real_to_ay(e, true);
            return true;
        }
        false
    }

    /// 16-bit result into `lo` / `hi`.
    pub(super) fn real_eval_word(&mut self, e: &Expr, lo: u8, hi: u8) -> bool {
        if !self.reals_on() {
            return false;
        }
        if self.is_real_compare(e) {
            self.real_eval_byte(e);
            self.emit(0x85);
            self.emit(lo);
            self.emit(0xA9);
            self.emit(0x00);
            self.emit(0x85);
            self.emit(hi);
            return true;
        }
        if self.is_real_expr(e) {
            self.real_to_ay(e, true);
            self.emit(0x85);
            self.emit(lo);
            self.emit(0x84);
            self.emit(hi);
            return true;
        }
        false
    }

    /// Q8.8 destination: raw value into `zp` / `zp+1`.
    pub(super) fn real_to_q88_zp(&mut self, e: &Expr, zp: u8) -> bool {
        if !self.reals_on() || !self.is_real_expr(e) {
            return false;
        }
        self.real_to_q88_ay(e);
        self.emit(0x85);
        self.emit(zp);
        self.emit(0x84);
        self.emit(zp + 1);
        true
    }

    /// `print` of a floating-point value.
    pub(super) fn real_print(&mut self, arg: &Expr) -> bool {
        if !self.reals_on() {
            return false;
        }
        if let Expr::StrN(inner) = arg {
            if self.is_real_expr(inner) {
                let arg = arg.clone();
                self.emit_str_print(&arg);
                return true;
            }
            return false;
        }
        // a decimal constant prints exactly (`print -0.001`) once floating point is in
        let literal = match arg {
            Expr::FixedLit(..) => true,
            Expr::BinOp(z, BinOp::Sub, l) => {
                matches!(z.as_ref(), Expr::Number(0)) && matches!(l.as_ref(), Expr::FixedLit(..))
            }
            _ => false,
        };
        if !literal && (self.is_real_compare(arg) || !self.is_real_expr(arg)) {
            return false;
        }
        self.real_text_ay(arg);
        let t = self.tmp_zp;
        self.tmp_zp += 2;
        self.emit(0x85);
        self.emit(t);
        self.emit(0x84);
        self.emit(t + 1);
        self.print_str_via_ptr(t);
        true
    }

    /// A string-engine number part: SRC (`src_zp`) ← text of `e`.
    pub(super) fn real_number_text(&mut self, e: &Expr, src_zp: u8) -> bool {
        if !self.reals_on() || !self.is_real_expr(e) {
            return false;
        }
        self.real_text_ay(e);
        self.emit(0x85);
        self.emit(src_zp);
        self.emit(0x84);
        self.emit(src_zp + 1);
        true
    }

    pub(super) fn is_real_number(&self, e: &Expr) -> bool {
        self.reals_on() && self.is_real_expr(e)
    }

    /// Store one argument into a floating-point (or Q8.8) parameter.
    pub(super) fn real_param_store(&mut self, sub: &str, i: usize, zp: u8, ptype: &Option<VarType>, arg: &Expr) -> bool {
        match ptype {
            Some(VarType::Real) => {
                let slot = self.real_params.get(sub).and_then(|v| v.get(i).copied().flatten());
                if let Some(addr) = slot {
                    self.eval_real(arg);
                    self.flt_store(addr);
                }
                true
            }
            Some(VarType::Float) => self.real_to_q88_zp(arg, zp),
            _ => false,
        }
    }

    /// Statements with a floating-point variable or value. Returns false when
    /// the integer code generator should handle the statement.
    pub(super) fn gen_real_stmt(&mut self, stmt: &Stmt) -> bool {
        if !self.reals_on() {
            return false;
        }
        match stmt {
            Stmt::VarDecl { name, expr, .. } | Stmt::Assign(name, expr)
                if self.real_vars.contains_key(name) =>
            {
                self.var_types.insert(name.clone(), VarType::Real);
                self.mark_used(name);
                let (name, expr) = (name.clone(), expr.clone());
                self.real_assign(&name, &expr);
                true
            }
            Stmt::VarDecl { name, vtype, expr } if self.is_real_expr(expr) => {
                let (name, expr) = (name.clone(), expr.clone());
                let zp = self.alloc_var(&name);
                let ty = match vtype {
                    Some(VarType::Word) => VarType::Word,
                    Some(VarType::Float) => VarType::Float,
                    _ => VarType::Int,
                };
                self.var_types.insert(name.clone(), ty.clone());
                self.store_real_into(&ty, zp, &expr);
                true
            }
            Stmt::Assign(name, expr)
                if self.is_real_expr(expr)
                    && !matches!(self.var_types.get(name), Some(VarType::Str)) =>
            {
                let (name, expr) = (name.clone(), expr.clone());
                self.mark_used(&name);
                let zp = self.alloc_var(&name);
                let ty = match self.var_types.get(&name) {
                    Some(VarType::Word) => VarType::Word,
                    Some(VarType::Float) => VarType::Float,
                    _ => VarType::Int,
                };
                self.store_real_into(&ty, zp, &expr);
                true
            }
            Stmt::ArraySet(arr, idx, val) if self.real_arrays.contains(arr.as_str()) => {
                let (arr, idx, val) = (arr.clone(), idx.clone(), val.clone());
                self.real_array_set(&arr, &idx, &val);
                true
            }
            Stmt::ArraySet(arr, idx, val)
                if self.float_arrays.contains(arr.as_str()) && self.is_real_expr(val) =>
            {
                // Q8.8 element from a floating-point value: through a hidden Q8.8 var
                let hidden = "q88__real".to_string();
                let zp = self.alloc_var(&hidden);
                self.var_types.insert(hidden.clone(), VarType::Float);
                self.real_to_q88_zp(val, zp);
                let st = Stmt::ArraySet(arr.clone(), idx.clone(), Expr::Var(hidden));
                self.gen_stmt_inner(&st);
                true
            }
            Stmt::Inc(name) if self.real_vars.contains_key(name) => {
                let name = name.clone();
                self.real_inc(&name, 1.0);
                true
            }
            Stmt::Dec(name) if self.real_vars.contains_key(name) => {
                let name = name.clone();
                self.real_inc(&name, -1.0);
                true
            }
            Stmt::Input { prompt, var } if self.real_vars.contains_key(var) => {
                let (prompt, var) = (prompt.clone(), var.clone());
                self.real_input(&prompt, &var);
                true
            }
            Stmt::Read(var) if self.real_vars.contains_key(var) => {
                let var = var.clone();
                let hidden = "read__real".to_string();
                self.gen_stmt_inner(&Stmt::Read(hidden.clone()));
                self.var_types.insert(hidden.clone(), VarType::Int);
                self.real_assign(&var, &Expr::Var(hidden));
                true
            }
            Stmt::ForLoop { var, from, to, step, body } if self.real_vars.contains_key(var) => {
                let (var, from, to, step, body) =
                    (var.clone(), from.clone(), to.clone(), step.clone(), body.clone());
                self.gen_for_loop_real(&var, &from, &to, step.as_ref(), &body);
                true
            }
            Stmt::Return(Some(e)) if matches!(self.fn_ret_type, Some(VarType::Real)) => {
                let e = e.clone();
                let slot = self.cur_fn.as_ref().and_then(|f| self.real_fns.get(f).copied());
                self.eval_real(&e);
                if let Some(slot) = slot {
                    self.flt_store(slot);
                }
                self.emit(0x60); // RTS
                true
            }
            Stmt::Return(Some(e))
                if matches!(self.fn_ret_type, Some(VarType::Float)) && self.is_real_expr(e) =>
            {
                let e = e.clone();
                if let Some(z) = self.fn_ret_zp {
                    self.real_to_q88_zp(&e, z);
                }
                self.emit(0x60); // RTS
                true
            }
            _ => false,
        }
    }

    /// Integer / Q8.8 variable at `zp` ← floating-point `e` (rounded).
    fn store_real_into(&mut self, ty: &VarType, zp: u8, e: &Expr) {
        match ty {
            VarType::Float => {
                self.real_to_q88_zp(e, zp);
            }
            VarType::Word => {
                self.real_to_ay(e, true);
                self.emit(0x85);
                self.emit(zp);
                self.emit(0x84);
                self.emit(zp + 1);
            }
            _ => {
                self.real_to_ay(e, true);
                self.emit(0x85);
                self.emit(zp);
            }
        }
        self.a_cache = None;
    }
}

fn ret_is_real(st: &Stmt) -> bool {
    matches!(st, Stmt::FnDef(_, _, Some(VarType::Real), _))
}

impl Codegen {
    /// Undo the parameter bindings of a sub / fn body.
    pub(super) fn restore_real_params(&mut self, shadowed: Vec<(String, Option<u16>)>) {
        for (name, prev) in shadowed.into_iter().rev() {
            match prev {
                Some(a) => {
                    self.real_vars.insert(name, a);
                }
                None => {
                    self.real_vars.remove(&name);
                }
            }
        }
    }
}
