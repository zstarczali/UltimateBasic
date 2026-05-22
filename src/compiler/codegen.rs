use std::collections::HashMap;
use super::ast::{Expr, BinOp, Stmt, ColorTarget, VarType};

const ZP_BASE: u8 = 0x02;
const TMP_BASE: u8 = 0x50;
const CHROUT: u16 = 0xFFD2;
const VIC_BORDER: u16 = 0xD020;
const VIC_BG: u16 = 0xD021;

pub struct Codegen {
    code: Vec<u8>,
    load_addr: u16,
    vars: HashMap<String, u8>,
    var_types: HashMap<String, VarType>,
    subs: HashMap<String, u16>,
    sub_patches: Vec<(usize, String)>,
    sub_params: HashMap<String, Vec<u8>>,   // sub_name → [zp_addr per param]
    labels: HashMap<String, u16>,
    goto_patches: Vec<(usize, String)>,
    perm_zp: u8,
    tmp_zp: u8,
    break_patches: Vec<Vec<usize>>,
    arrays: HashMap<String, u16>,           // array_name → base address ($C000+)
    array_ptr: u16,                         // next free array slot
    rnd_seeded: bool,
}

impl Codegen {
    pub fn new(load_addr: u16) -> Self {
        Self {
            code: vec![],
            load_addr,
            vars: HashMap::new(),
            var_types: HashMap::new(),
            subs: HashMap::new(),
            sub_patches: vec![],
            sub_params: HashMap::new(),
            labels: HashMap::new(),
            goto_patches: vec![],
            perm_zp: ZP_BASE,
            tmp_zp: TMP_BASE,
            break_patches: vec![],
            arrays: HashMap::new(),
            array_ptr: 0xC000,
            rnd_seeded: false,
        }
    }

    /// Pre-scan: allocate ZP slots for sub params and register arrays.
    /// Must run before gen_stmt so that param slots precede regular vars in ZP.
    fn pre_scan(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::SubDef(name, params, _) => {
                    let mut addrs = vec![];
                    for _ in params {
                        let addr = self.perm_zp;
                        self.perm_zp += 2; // 2 bytes per slot (consistent with other vars)
                        addrs.push(addr);
                    }
                    self.sub_params.insert(name.clone(), addrs);
                }
                Stmt::VarDecl { name, vtype: Some(VarType::Array), expr, .. } => {
                    let size = if let Expr::Number(n) = expr { *n as u16 } else { 0 };
                    self.arrays.insert(name.clone(), self.array_ptr);
                    self.array_ptr += size;
                }
                _ => {}
            }
        }
    }

    fn emit(&mut self, byte: u8) {
        self.code.push(byte);
    }

    fn emit16(&mut self, val: u16) {
        self.emit(val as u8);
        self.emit((val >> 8) as u8);
    }

    fn current_addr(&self) -> u16 {
        self.load_addr + self.code.len() as u16
    }

    fn alloc_var(&mut self, name: &str) -> u8 {
        if let Some(&addr) = self.vars.get(name) {
            return addr;
        }
        let addr = self.perm_zp;
        self.perm_zp += 2; // 16-bit vars (lo/hi)
        self.vars.insert(name.to_string(), addr);
        addr
    }

    fn var_addr(&self, name: &str) -> Option<u8> {
        self.vars.get(name).copied()
    }

    // Helpers for 16-bit register operations (reserved for future use)
    #[allow(dead_code)]
    fn load_imm16(&mut self, val: i16) {
        let lo = val as u8;
        let hi = (val >> 8) as u8;
        self.emit(0xA9); self.emit(lo); // LDA #lo
        self.emit(0xAA);                 // TAX -> now A=lo, but we need A=lo X=hi
        self.emit(0xA9); self.emit(lo); // LDA #lo
        self.emit(0xA2); self.emit(hi); // LDX #hi
    }

    #[allow(dead_code)]
    fn store_ax_to_var(&mut self, zp: u8) {
        self.emit(0x85); self.emit(zp);       // STA zp (lo)
        self.emit(0x86); self.emit(zp + 1);   // STX zp+1 (hi)
    }

    #[allow(dead_code)]
    fn load_var_to_ax(&mut self, zp: u8) {
        self.emit(0xA5); self.emit(zp);       // LDA zp
        self.emit(0xA6); self.emit(zp + 1);   // LDX zp+1
    }

    // Evaluate expression, result in A (lo byte only for simplicity)
    fn eval_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Number(n) => {
                self.emit(0xA9); self.emit(*n as u8); // LDA #n
            }
            Expr::StringLit(_) => {
                // strings handled separately in print
                self.emit(0xA9); self.emit(0x00);
            }
            Expr::Var(name) => {
                if let Some(zp) = self.var_addr(name) {
                    self.emit(0xA5); self.emit(zp); // LDA zp
                } else {
                    self.emit(0xA9); self.emit(0x00);
                }
            }
            Expr::Not(expr) => {
                // not expr → 1 if expr==0, 0 otherwise
                let expr = expr.clone();
                self.eval_expr(&expr);
                self.emit(0xC9); self.emit(0x01);  // CMP #1
                self.emit(0xF0);                    // BEQ → false (expr is 1, return 0)
                self.emit(0x05);                    // +5 to true
                // expr is false
                self.emit(0xA9); self.emit(0x01);  // LDA #1
                self.emit(0x4C);
                let jmp = self.code.len(); self.emit16(0x0000);
                // expr is true → return 0
                self.emit(0xA9); self.emit(0x00);
                let end = self.current_addr();
                self.patch_abs(jmp, end);
            }
            Expr::Getch => {
                let loop_addr = self.current_addr();
                self.emit(0x20); self.emit16(0xFFE4); // JSR $FFE4
                self.emit(0xC9); self.emit(0x00);
                self.emit(0xF0);
                let bne_pos = self.code.len(); self.emit(0x00);
                self.patch_bxx(bne_pos, loop_addr);
            }
            Expr::Peek(addr) => {
                match addr.as_ref() {
                    Expr::Number(n) => {
                        let n = *n;
                        self.emit(0xAD); self.emit(n as u8); self.emit((n >> 8) as u8); // LDA abs
                    }
                    Expr::Var(name) => {
                        let name = name.clone();
                        if matches!(self.var_types.get(&name), Some(VarType::Word)) {
                            // word var = 16-bit ZP pointer → LDA (zp),Y  Y=0
                            if let Some(zp) = self.var_addr(&name) {
                                self.emit(0xA0); self.emit(0x00); // LDY #0
                                self.emit(0xB1); self.emit(zp);   // LDA (zp),Y
                            }
                        } else if let Some(zp) = self.var_addr(&name) {
                            // 8-bit var used as ZP address
                            self.emit(0xA5); self.emit(zp); // LDA zp
                        }
                    }
                    _ => {
                        let addr = addr.clone();
                        let ptr = self.tmp_zp; self.tmp_zp += 2;
                        self.eval_expr(&addr);
                        self.emit(0x85); self.emit(ptr);       // STA ptr_lo
                        self.emit(0xA9); self.emit(0x00);
                        self.emit(0x85); self.emit(ptr + 1);   // STA ptr_hi = 0
                        self.emit(0xA0); self.emit(0x00);      // LDY #0
                        self.emit(0xB1); self.emit(ptr);       // LDA (ptr),Y
                    }
                }
            }
            Expr::ArrayGet(arr_name, idx_expr) => {
                let base = self.arrays.get(arr_name).copied().unwrap_or(0xC000);
                match idx_expr.as_ref() {
                    Expr::Number(n) => {
                        let addr = base.wrapping_add(*n as u16);
                        self.emit(0xAD); self.emit16(addr); // LDA abs
                    }
                    _ => {
                        let idx = idx_expr.clone();
                        let ptr = self.tmp_zp; self.tmp_zp += 2;
                        self.emit(0xA9); self.emit(base as u8);
                        self.emit(0x85); self.emit(ptr);
                        self.emit(0xA9); self.emit((base >> 8) as u8);
                        self.emit(0x85); self.emit(ptr + 1);
                        self.eval_expr(&idx);
                        self.emit(0xA8);             // TAY
                        self.emit(0xB1); self.emit(ptr); // LDA (ptr),Y
                    }
                }
            }
            Expr::Rnd => {
                // LCG: seed = seed*5 + 1 mod 256  (full period, Hull-Dobell)
                // Seed at $FB – free ZP, not used by KERNAL or BASIC
                if !self.rnd_seeded {
                    // Seed with raster line for variety across runs
                    self.emit(0xAD); self.emit(0x12); self.emit(0xD0); // LDA $D012
                    self.emit(0x85); self.emit(0xFB);                   // STA $FB
                    self.rnd_seeded = true;
                }
                self.emit(0xA5); self.emit(0xFB); // LDA $FB
                self.emit(0x0A);                   // ASL A  (×2)
                self.emit(0x0A);                   // ASL A  (×4)
                self.emit(0x18);                   // CLC
                self.emit(0x65); self.emit(0xFB); // ADC $FB (×5)
                self.emit(0x18);                   // CLC
                self.emit(0x69); self.emit(0x01);  // ADC #1
                self.emit(0x85); self.emit(0xFB);  // STA $FB
            }
            Expr::Abs(expr) => {
                let expr = expr.clone();
                self.eval_expr(&expr);
                self.emit(0x10);                    // BPL + (skip negate if positive)
                let bpl_pos = self.code.len(); self.emit(0x00);
                self.emit(0x49); self.emit(0xFF);   // EOR #$FF
                self.emit(0x18);
                self.emit(0x69); self.emit(0x01);   // ADC #1 (two's complement)
                let after = self.current_addr();
                self.patch_bxx(bpl_pos, after);
            }
            Expr::Min(a, b) => {
                let (a, b) = (a.clone(), b.clone());
                let t = self.tmp_zp; self.tmp_zp += 1;
                self.eval_expr(&b);
                self.emit(0x85); self.emit(t);
                self.eval_expr(&a);
                self.emit(0xC5); self.emit(t);
                self.emit(0x90); self.emit(0x05); // BCC +5
                self.emit(0xA5); self.emit(t);
                self.emit(0x4C);
                let skip = self.code.len(); self.emit16(0x0000);
                let end = self.current_addr();
                self.patch_abs(skip, end);
            }
            Expr::Max(a, b) => {
                let (a, b) = (a.clone(), b.clone());
                let t = self.tmp_zp; self.tmp_zp += 1;
                self.eval_expr(&b);
                self.emit(0x85); self.emit(t);
                self.eval_expr(&a);
                self.emit(0xC5); self.emit(t);
                self.emit(0xB0); self.emit(0x05); // BCS +5
                self.emit(0xA5); self.emit(t);
                self.emit(0x4C);
                let skip = self.code.len(); self.emit16(0x0000);
                let end = self.current_addr();
                self.patch_abs(skip, end);
            }
            Expr::Sgn(expr) => {
                let expr = expr.clone();
                self.eval_expr(&expr);
                self.emit(0xC9); self.emit(0x01);  // CMP #1
                self.emit(0xB0);                    // BCS → return 1 (non-zero)
                self.emit(0x04);                    // +4
                self.emit(0xA9); self.emit(0x00);   // LDA #0 (zero)
                self.emit(0x4C);
                let jmp = self.code.len(); self.emit16(0x0000);
                self.emit(0xA9); self.emit(0x01);   // LDA #1 (non-zero)
                let end = self.current_addr();
                self.patch_abs(jmp, end);
            }
            Expr::BinOp(l, op, r) => {
                match op {
                    BinOp::And => {
                        // Bitwise AND – matches BASIC's AND semantics (e.g. color and 15)
                        let tmp = self.tmp_zp; self.tmp_zp += 1;
                        self.eval_expr(l);
                        self.emit(0x85); self.emit(tmp);  // STA tmp (l)
                        self.eval_expr(r);
                        self.emit(0x25); self.emit(tmp);  // AND tmp → A = l & r
                    }
                    BinOp::Or => {
                        // Bitwise OR
                        let tmp = self.tmp_zp; self.tmp_zp += 1;
                        self.eval_expr(l);
                        self.emit(0x85); self.emit(tmp);  // STA tmp (l)
                        self.eval_expr(r);
                        self.emit(0x05); self.emit(tmp);  // ORA tmp → A = l | r
                    }
                    _ => {
                let tmp = self.tmp_zp;
                self.tmp_zp += 1;
                self.eval_expr(l);
                self.emit(0x85); self.emit(tmp); // STA tmp
                self.eval_expr(r);
                match op {
                    BinOp::Add => {
                        self.emit(0x18);              // CLC
                        self.emit(0x65); self.emit(tmp); // ADC tmp
                    }
                    BinOp::Sub => {
                        let tmp2 = self.tmp_zp;
                        self.tmp_zp += 1;
                        self.emit(0x85); self.emit(tmp2); // STA tmp2 (r)
                        self.emit(0xA5); self.emit(tmp);  // LDA tmp (l)
                        self.emit(0x38);                   // SEC
                        self.emit(0xE5); self.emit(tmp2);  // SBC tmp2
                    }
                    BinOp::Mul => {
                        let cnt = self.tmp_zp; self.tmp_zp += 1;
                        let res = self.tmp_zp; self.tmp_zp += 1;
                        self.emit(0x85); self.emit(cnt); // STA cnt (r = count)
                        self.emit(0xA9); self.emit(0x00);
                        self.emit(0x85); self.emit(res); // STA res = 0
                        let loop_addr = self.current_addr();
                        self.emit(0xA5); self.emit(res);
                        self.emit(0x18);
                        self.emit(0x65); self.emit(tmp); // ADC tmp (l)
                        self.emit(0x85); self.emit(res);
                        self.emit(0xC6); self.emit(cnt); // DEC cnt
                        self.emit(0xD0);                  // BNE
                        let offset = loop_addr as i32 - self.current_addr() as i32 - 1;
                        self.emit(offset as u8);
                        self.emit(0xA5); self.emit(res); // LDA res
                    }
                    BinOp::Div => {
                        let divisor = self.tmp_zp; self.tmp_zp += 1;
                        let quot = self.tmp_zp; self.tmp_zp += 1;
                        self.emit(0x85); self.emit(divisor);
                        self.emit(0xA9); self.emit(0x00);
                        self.emit(0x85); self.emit(quot);
                        let loop_addr = self.current_addr();
                        self.emit(0xA5); self.emit(tmp);
                        self.emit(0x38);
                        self.emit(0xE5); self.emit(divisor);
                        self.emit(0x85); self.emit(tmp);
                        self.emit(0xB0); self.emit(0x04); // BCS +4 (continue)
                        self.emit(0x4C); // JMP end
                        let patch = self.code.len();
                        self.emit16(0x0000);
                        self.emit(0xE6); self.emit(quot); // INC quot
                        let back = loop_addr as i32 - self.current_addr() as i32 - 2;
                        self.emit(0x90); self.emit(back as u8); // BCC loop
                        let end_addr = self.current_addr();
                        let p = patch;
                        self.code[p] = end_addr as u8;
                        self.code[p+1] = (end_addr >> 8) as u8;
                        self.emit(0xA5); self.emit(quot);
                    }
                    BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                        // Compare: returns 1 (true) or 0 (false) in A
                        let tmp2 = self.tmp_zp; self.tmp_zp += 1;
                        self.emit(0x85); self.emit(tmp2); // STA tmp2 (r)
                        self.emit(0xA5); self.emit(tmp);  // LDA tmp (l)
                        self.emit(0xC5); self.emit(tmp2); // CMP tmp2
                        let branch_op: u8 = match op {
                            BinOp::Eq    => 0xF0, // BEQ
                            BinOp::NotEq => 0xD0, // BNE
                            BinOp::Lt    => 0x90, // BCC
                            BinOp::GtEq  => 0xB0, // BCS
                            BinOp::Gt    => 0x00, // special
                            BinOp::LtEq  => 0x00, // special
                            _ => 0xF0,
                        };
                        if matches!(op, BinOp::Gt) {
                            // l > r  ->  r < l  -> swap and BCC
                            self.emit(0xA5); self.emit(tmp2);
                            self.emit(0xC5); self.emit(tmp);
                            self.emit(0x90); // BCC true
                        } else if matches!(op, BinOp::LtEq) {
                            self.emit(0xA5); self.emit(tmp2);
                            self.emit(0xC5); self.emit(tmp);
                            self.emit(0xB0); // BCS true
                        } else {
                            self.emit(branch_op);
                        }
                        self.emit(0x05); // branch +5 to true (skip LDA#0(2) + JMP(3) = 5 bytes)
                        self.emit(0xA9); self.emit(0x00); // LDA #0 (false)
                        self.emit(0x4C); // JMP past true
                        let patch = self.code.len();
                        self.emit16(0x0000);
                        self.emit(0xA9); self.emit(0x01); // LDA #1 (true)
                        let end = self.current_addr();
                        self.code[patch] = end as u8;
                        self.code[patch+1] = (end >> 8) as u8;
                    }
                    BinOp::And | BinOp::Or => unreachable!(),
                } // end _ => { inner match
                    } // end outer match op
                } // end BinOp
            }
        }
    }

    // Print string literal, no trailing newline
    fn print_str_inline(&mut self, s: &str) {
        for c in s.chars() {
            self.emit(0xA9); self.emit(ascii_to_petscii(c));
            self.emit(0x20); self.emit16(CHROUT);
        }
    }

    fn print_newline(&mut self) {
        self.emit(0xA9); self.emit(0x0D);
        self.emit(0x20); self.emit16(CHROUT);
    }

    /// Print null-terminated PETSCII string whose address is in ZP pair (ptr, ptr+1).
    fn print_str_via_ptr(&mut self, ptr: u8) {
        self.emit(0xA0); self.emit(0x00);    // LDY #0
        let loop_top = self.current_addr();
        self.emit(0xB1); self.emit(ptr);     // LDA (ptr),Y
        self.emit(0xF0);                      // BEQ done (null terminator)
        let beq_pos = self.code.len(); self.emit(0x00);
        self.emit(0x20); self.emit16(CHROUT); // JSR CHROUT
        self.emit(0xC8);                      // INY
        self.emit(0x4C); self.emit16(loop_top); // JMP loop_top
        let done = self.current_addr();
        self.patch_bxx(beq_pos, done);
    }

    // Print 8-bit decimal value from ZP address, no trailing newline.
    // Uses 3 ZP temps. Suppresses leading zeros.
    fn print_decimal(&mut self, zp: u8) {
        let t_val = self.tmp_zp; self.tmp_zp += 1; // working copy
        let t_lz  = self.tmp_zp; self.tmp_zp += 1; // leading-zero flag (1=suppress)

        // t_val = zp;  t_lz = 1
        self.emit(0xA5); self.emit(zp);
        self.emit(0x85); self.emit(t_val);
        self.emit(0xA9); self.emit(0x01);
        self.emit(0x85); self.emit(t_lz);

        self.print_digit_loop(t_val, 100, t_lz);
        self.print_digit_loop(t_val, 10,  t_lz);

        // Ones: always print (no suppression)
        self.emit(0xA5); self.emit(t_val);
        self.emit(0x09); self.emit(0x30); // ORA #'0'
        self.emit(0x20); self.emit16(CHROUT);
    }

    // Emit code that divides t_val by `div`, prints the quotient digit
    // (with leading-zero suppression via t_lz), leaves remainder in t_val.
    fn print_digit_loop(&mut self, t_val: u8, div: u8, t_lz: u8) {
        let t_digit = self.tmp_zp; self.tmp_zp += 1;

        // t_digit = 0
        self.emit(0xA9); self.emit(0x00);
        self.emit(0x85); self.emit(t_digit);

        // loop: while t_val >= div { t_val -= div; t_digit++ }
        let loop_top = self.current_addr();
        self.emit(0xA5); self.emit(t_val);
        self.emit(0xC9); self.emit(div);       // CMP #div
        self.emit(0x90);                        // BCC → done
        let bcc_pos = self.code.len(); self.emit(0x00);
        self.emit(0x38);
        self.emit(0xE9); self.emit(div);        // SBC #div
        self.emit(0x85); self.emit(t_val);
        self.emit(0xE6); self.emit(t_digit);    // INC t_digit
        self.emit(0x4C); self.emit16(loop_top); // JMP loop_top
        let loop_done = self.current_addr();
        self.patch_bxx(bcc_pos, loop_done);

        // if t_digit == 0 && t_lz == 1: skip (leading zero)
        self.emit(0xA5); self.emit(t_digit);
        self.emit(0xD0);                        // BNE → print
        let bne_pos = self.code.len(); self.emit(0x00);
        // digit is 0 — check leading zero flag
        self.emit(0xA5); self.emit(t_lz);
        self.emit(0xD0);                        // BNE → skip
        let bne_skip_pos = self.code.len(); self.emit(0x00);
        // fall through: digit != 0 path joins here
        let print_pos = self.current_addr();
        self.patch_bxx(bne_pos, print_pos);

        // print digit: ORA #'0', JSR CHROUT, clear lz flag
        self.emit(0xA5); self.emit(t_digit);
        self.emit(0x09); self.emit(0x30);       // ORA #'0'
        self.emit(0x20); self.emit16(CHROUT);
        self.emit(0xA9); self.emit(0x00);
        self.emit(0x85); self.emit(t_lz);       // t_lz = 0 (printed something)
        self.emit(0x4C);                        // JMP → after_skip
        let jmp_pos = self.code.len(); self.emit16(0x0000);

        let skip_pos = self.current_addr();
        self.patch_bxx(bne_skip_pos, skip_pos);
        let after_skip = self.current_addr();
        self.patch_abs(jmp_pos, after_skip);
    }

    fn patch_bxx(&mut self, offset_pos: usize, target: u16) {
        // offset_pos = index of the branch offset byte in self.code
        // after branch instr = load_addr + offset_pos + 1
        let after = self.load_addr as i32 + offset_pos as i32 + 1;
        self.code[offset_pos] = (target as i32 - after) as u8;
    }

    fn patch_abs(&mut self, lo_pos: usize, target: u16) {
        self.code[lo_pos]     = target as u8;
        self.code[lo_pos + 1] = (target >> 8) as u8;
    }

    // Convert 8-bit ZP value to decimal ASCII string stored at dest_addr.
    // Always writes 3 chars + null terminator: "042\0"
    fn emit_int_to_str(&mut self, zp_src: u8, dest_addr: u16) {
        let t_val = self.tmp_zp; self.tmp_zp += 1;

        self.emit(0xA5); self.emit(zp_src);
        self.emit(0x85); self.emit(t_val);

        self.store_digit(t_val, 100, dest_addr);
        self.store_digit(t_val, 10,  dest_addr.wrapping_add(1));

        // ones = remainder
        self.emit(0xA5); self.emit(t_val);
        self.emit(0x09); self.emit(0x30);              // ORA #'0'
        self.emit(0x8D); self.emit16(dest_addr.wrapping_add(2)); // STA dest+2

        // null terminator
        self.emit(0xA9); self.emit(0x00);
        self.emit(0x8D); self.emit16(dest_addr.wrapping_add(3)); // STA dest+3
    }

    fn store_digit(&mut self, t_val: u8, div: u8, dest: u16) {
        let t_digit = self.tmp_zp; self.tmp_zp += 1;

        self.emit(0xA9); self.emit(0x00);
        self.emit(0x85); self.emit(t_digit);             // t_digit = 0

        let lp = self.current_addr();
        self.emit(0xA5); self.emit(t_val);
        self.emit(0xC9); self.emit(div);                 // CMP #div
        self.emit(0x90);
        let bcc = self.code.len(); self.emit(0x00);      // BCC done
        self.emit(0x38);
        self.emit(0xE9); self.emit(div);                 // SBC #div
        self.emit(0x85); self.emit(t_val);
        self.emit(0xE6); self.emit(t_digit);             // INC t_digit
        self.emit(0x4C); self.emit16(lp);
        let done = self.current_addr();
        self.patch_bxx(bcc, done);

        self.emit(0xA5); self.emit(t_digit);
        self.emit(0x09); self.emit(0x30);                // ORA #'0'
        self.emit(0x8D); self.emit16(dest);              // STA dest
    }

    // Manual CLS: fill screen RAM $0400-$07E7 with spaces, color RAM with white,
    // then reset cursor position via KERNAL home ($E566).
    fn emit_cls_manual(&mut self) {
        // Fill $0400, $0500, $0600 (3 × 256 = 768 bytes) with space ($20)
        self.emit(0xA9); self.emit(0x20); // LDA #$20
        self.emit(0xA2); self.emit(0x00); // LDX #0
        // loop: STA $0400,X / $0500,X / $0600,X; INX; BNE loop
        let lp1 = self.current_addr();
        self.emit(0x9D); self.emit16(0x0400); // STA $0400,X
        self.emit(0x9D); self.emit16(0x0500); // STA $0500,X
        self.emit(0x9D); self.emit16(0x0600); // STA $0600,X
        self.emit(0xE8);                       // INX
        self.emit(0xD0);                       // BNE lp1
        let bne1 = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne1, lp1);

        // Fill $0700-$07E7 (232 bytes = $E8)
        self.emit(0xA2); self.emit(0x00); // LDX #0
        let lp2 = self.current_addr();
        self.emit(0x9D); self.emit16(0x0700); // STA $0700,X
        self.emit(0xE8);                       // INX
        self.emit(0xE0); self.emit(0xE8);      // CPX #$E8
        self.emit(0xD0);                       // BNE lp2
        let bne2 = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne2, lp2);

        // Color RAM $D800-$DAff (3 pages) = white ($01)
        self.emit(0xA9); self.emit(0x01); // LDA #1 (white)
        self.emit(0xA2); self.emit(0x00); // LDX #0
        let lp3 = self.current_addr();
        self.emit(0x9D); self.emit16(0xD800);
        self.emit(0x9D); self.emit16(0xD900);
        self.emit(0x9D); self.emit16(0xDA00);
        self.emit(0xE8);
        self.emit(0xD0);
        let bne3 = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne3, lp3);

        // Color RAM $DB00-$DBE7
        self.emit(0xA2); self.emit(0x00);
        let lp4 = self.current_addr();
        self.emit(0x9D); self.emit16(0xDB00);
        self.emit(0xE8);
        self.emit(0xE0); self.emit(0xE8);
        self.emit(0xD0);
        let bne4 = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne4, lp4);

        // Cursor home
        self.emit(0x20); self.emit16(0xE566); // JSR $E566
    }

    // Graphics ON: C64 hires bitmap mode (320×200) at $2000, video matrix at $0400
    fn emit_graphics_on(&mut self) {
        // Set BMM bit in $D011
        self.emit(0xAD); self.emit16(0xD011); // LDA $D011
        self.emit(0x09); self.emit(0x20);      // ORA #$20  (set bit 5 = BMM)
        self.emit(0x8D); self.emit16(0xD011); // STA $D011
        // $D018: bitmap at $2000 (bit3=1), video matrix at $0400 (bits7-4=0001)
        self.emit(0xA9); self.emit(0x18);      // LDA #$18
        self.emit(0x8D); self.emit16(0xD018); // STA $D018
    }

    // Graphics OFF: back to text mode
    fn emit_graphics_off(&mut self) {
        // Clear BMM bit in $D011
        self.emit(0xAD); self.emit16(0xD011); // LDA $D011
        self.emit(0x29); self.emit(0xDF);      // AND #$DF  (clear bit 5)
        self.emit(0x8D); self.emit16(0xD011); // STA $D011
        // $D018: restore default (video at $0400, charset ROM at $1000)
        self.emit(0xA9); self.emit(0x15);      // LDA #$15
        self.emit(0x8D); self.emit16(0xD018); // STA $D018
    }

    /// True if the expression is or contains a string (literal or Str var).
    /// Used to decide whether `+` means string concat or numeric add in print.
    fn is_string_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::StringLit(_) => true,
            Expr::Var(name)    => matches!(self.var_types.get(name), Some(VarType::Str)),
            Expr::BinOp(l, BinOp::Add, r) =>
                self.is_string_expr(l) || self.is_string_expr(r),
            _ => false,
        }
    }

    /// Print a single argument. Handles the `+` operator as string concat
    /// when at least one operand is a string; otherwise evaluates numerically.
    fn print_single_arg(&mut self, arg: &Expr) {
        match arg {
            Expr::StringLit(s) => {
                let s = s.clone();
                self.print_str_inline(&s);
            }
            Expr::Var(name) => {
                let name = name.clone();
                if matches!(self.var_types.get(&name), Some(VarType::Str)) {
                    if let Some(zp) = self.var_addr(&name) {
                        self.print_str_via_ptr(zp);
                    }
                } else if let Some(zp) = self.var_addr(&name) {
                    self.print_decimal(zp);
                }
            }
            // String-side `+`: print left part then right part (no separator)
            Expr::BinOp(l, BinOp::Add, r)
                if self.is_string_expr(l) || self.is_string_expr(r) =>
            {
                let (l, r) = (l.clone(), r.clone());
                self.print_single_arg(&l);
                self.print_single_arg(&r);
            }
            _ => {
                // Numeric expression: evaluate → print as decimal
                let tmp = self.tmp_zp; self.tmp_zp += 1;
                let arg = arg.clone();
                self.eval_expr(&arg);
                self.emit(0x85); self.emit(tmp);
                self.print_decimal(tmp);
            }
        }
    }

    fn gen_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.tmp_zp = TMP_BASE; // reset scratch pool – prevents ZP overflow into BASIC/KERNAL vars
            self.gen_stmt(stmt);
        }
    }

    fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::VarDecl { name, vtype, expr } => {
                // Infer type from expr when not annotated
                let effective = vtype.clone().or_else(|| match expr {
                    Expr::StringLit(_) => Some(VarType::Str),
                    _ => None,
                });
                match effective {
                    Some(VarType::Array) => {
                        // Already registered in pre_scan — no ZP, no code
                        self.var_types.insert(name.clone(), VarType::Array);
                    }
                    Some(VarType::Word) => {
                        let zp = self.alloc_var(name);
                        self.var_types.insert(name.clone(), VarType::Word);
                        if let Expr::Number(n) = expr {
                            let n = *n;
                            self.emit(0xA9); self.emit(n as u8);
                            self.emit(0x85); self.emit(zp);
                            self.emit(0xA9); self.emit((n >> 8) as u8);
                            self.emit(0x85); self.emit(zp + 1);
                        } else {
                            let expr = expr.clone();
                            self.eval_expr(&expr);
                            self.emit(0x85); self.emit(zp);
                            self.emit(0xA9); self.emit(0x00);
                            self.emit(0x85); self.emit(zp + 1);
                        }
                    }
                    Some(VarType::Str) => {
                        let zp = self.alloc_var(name);
                        self.var_types.insert(name.clone(), VarType::Str);
                        if let Expr::StringLit(s) = expr {
                            let s = s.clone();
                            // JMP over inline string data
                            self.emit(0x4C);
                            let jmp_patch = self.code.len();
                            self.emit16(0x0000);
                            // Emit PETSCII string + null terminator
                            let str_addr = self.current_addr();
                            for c in s.chars() { self.emit(ascii_to_petscii(c)); }
                            self.emit(0x00);
                            let after = self.current_addr();
                            self.patch_abs(jmp_patch, after);
                            // Store pointer in ZP pair
                            self.emit(0xA9); self.emit(str_addr as u8);
                            self.emit(0x85); self.emit(zp);
                            self.emit(0xA9); self.emit((str_addr >> 8) as u8);
                            self.emit(0x85); self.emit(zp + 1);
                        }
                    }
                    _ => {
                        let zp = self.alloc_var(name);
                        let expr = expr.clone();
                        self.eval_expr(&expr);
                        self.emit(0x85); self.emit(zp);
                    }
                }
            }
            Stmt::Assign(name, expr) => {
                if matches!(self.var_types.get(name), Some(VarType::Word)) {
                    if let Some(zp) = self.var_addr(name) {
                        if let Expr::Number(n) = expr {
                            let n = *n;
                            self.emit(0xA9); self.emit(n as u8);
                            self.emit(0x85); self.emit(zp);
                            self.emit(0xA9); self.emit((n >> 8) as u8);
                            self.emit(0x85); self.emit(zp + 1);
                        } else {
                            let expr = expr.clone();
                            self.eval_expr(&expr);
                            self.emit(0x85); self.emit(zp);
                        }
                    }
                } else {
                    let zp = self.alloc_var(name);
                    let expr = expr.clone();
                    self.eval_expr(&expr);
                    self.emit(0x85); self.emit(zp);
                }
            }
            Stmt::Print(args) => {
                for arg in args {
                    let arg = arg.clone();
                    self.print_single_arg(&arg);
                }
                self.print_newline();
            }
            Stmt::If(cond, then_body, else_body) => {
                self.eval_expr(cond);
                self.emit(0xC9); self.emit(0x01); // CMP #1
                // BEQ +3 skip the JMP → execute then_body
                // JMP else/end (absolute, no branch distance limit)
                self.emit(0xF0); self.emit(0x03); // BEQ +3
                self.emit(0x4C);                   // JMP skip
                let skip_patch = self.code.len(); self.emit16(0x0000);

                self.gen_stmts(then_body);

                if let Some(eb) = else_body {
                    self.emit(0x4C); // JMP past else
                    let patch_else = self.code.len();
                    self.emit16(0x0000);

                    let else_start = self.current_addr();
                    self.patch_abs(skip_patch, else_start);

                    self.gen_stmts(eb);
                    let end = self.current_addr();
                    self.code[patch_else]   = end as u8;
                    self.code[patch_else+1] = (end >> 8) as u8;
                } else {
                    let end = self.current_addr();
                    self.patch_abs(skip_patch, end);
                }
            }
            Stmt::Loop(count, body) => {
                self.break_patches.push(vec![]);

                if *count == 0 {
                    // Infinite loop: JMP back unconditionally
                    let loop_start = self.current_addr();
                    self.gen_stmts(body);
                    self.emit(0x4C); self.emit16(loop_start);
                } else {
                    let cnt = self.perm_zp; self.perm_zp += 1; // permanent: persists across iterations
                    self.emit(0xA9); self.emit(*count);
                    self.emit(0x85); self.emit(cnt);
                    let loop_start = self.current_addr();
                    self.gen_stmts(body);
                    self.emit(0xC6); self.emit(cnt);   // DEC cnt
                    self.emit(0xD0);                    // BNE loop_start
                    let back = loop_start as i32 - self.current_addr() as i32 - 1;
                    self.emit(back as u8);
                }

                let loop_end = self.current_addr();
                let breaks = self.break_patches.pop().unwrap_or_default();
                for pos in breaks { self.patch_abs(pos, loop_end); }
            }
            Stmt::ForLoop { var, from, to, step, body } => {
                let zp = self.alloc_var(var);
                // eval from → var
                self.eval_expr(from);
                self.emit(0x85); self.emit(zp);

                // eval 'to' once into a permanent ZP temp (must survive loop body resets)
                let zp_to = self.perm_zp; self.perm_zp += 1;
                self.eval_expr(to);
                self.emit(0x85); self.emit(zp_to);

                // eval 'step' once into a permanent ZP temp (default 1)
                let zp_step = self.perm_zp; self.perm_zp += 1;
                match step {
                    Some(expr) => { self.eval_expr(expr); }
                    None       => { self.emit(0xA9); self.emit(0x01); }
                }
                self.emit(0x85); self.emit(zp_step);

                self.break_patches.push(vec![]);
                let loop_top = self.current_addr();

                // if var > zp_to → exit  (unsigned: zp_to < var → C=0 after CMP zp_to,var? no)
                // LDA var; CMP zp_to; BEQ body (equal → run once more); BCS exit (var > to)
                self.emit(0xA5); self.emit(zp);
                self.emit(0xC5); self.emit(zp_to);  // CMP zp_to
                self.emit(0x90);                      // BCC → continue (var < to)
                let bcc_pos = self.code.len(); self.emit(0x00);
                self.emit(0xF0);                      // BEQ → continue (var == to)
                let beq_pos = self.code.len(); self.emit(0x00);
                // var > to → exit
                self.emit(0x4C);
                let exit_pos = self.code.len(); self.emit16(0x0000);

                let body_start = self.current_addr();
                self.patch_bxx(bcc_pos, body_start);
                self.patch_bxx(beq_pos, body_start);

                self.gen_stmts(body);

                // var += step
                self.emit(0xA5); self.emit(zp);
                self.emit(0x18);
                self.emit(0x75); self.emit(zp_step); // ADC zp_step (BUG: indexed, should be 0x65)
                // Fix: undo last 5 bytes (2+1+2) and redo correctly
                let len = self.code.len();
                self.code.truncate(len - 5);
                self.emit(0xA5); self.emit(zp);
                self.emit(0x18);
                self.emit(0x65); self.emit(zp_step); // ADC zp_step
                self.emit(0x85); self.emit(zp);

                self.emit(0x4C); self.emit16(loop_top);

                let loop_end = self.current_addr();
                self.patch_abs(exit_pos, loop_end);
                let breaks = self.break_patches.pop().unwrap_or_default();
                for pos in breaks { self.patch_abs(pos, loop_end); }
            }
            Stmt::WhileLoop(cond, body) => {
                self.break_patches.push(vec![]);
                let loop_top = self.current_addr();
                self.eval_expr(cond);
                self.emit(0xC9); self.emit(0x01); // CMP #1
                // BEQ continue → skip JMP exit (3 bytes)
                // JMP exit (absolute, no distance limit for large bodies)
                self.emit(0xF0); self.emit(0x03); // BEQ +3
                self.emit(0x4C);                   // JMP exit
                let exit_patch = self.code.len(); self.emit16(0x0000);
                // continue:
                self.gen_stmts(body);
                self.emit(0x4C); self.emit16(loop_top);
                let loop_end = self.current_addr();
                self.patch_abs(exit_patch, loop_end);
                let breaks = self.break_patches.pop().unwrap_or_default();
                for pos in breaks { self.patch_abs(pos, loop_end); }
            }
            Stmt::Break => {
                self.emit(0x4C); // JMP (address patched later)
                let pos = self.code.len();
                self.emit16(0x0000);
                if let Some(list) = self.break_patches.last_mut() {
                    list.push(pos);
                }
            }
            Stmt::Sys(addr) => {
                self.emit(0x20); self.emit16(*addr); // JSR addr
            }
            Stmt::AsmBytes(bytes) => {
                for &b in bytes { self.emit(b); }
            }
            Stmt::IntToStr { var, addr } => {
                let addr = *addr;
                if let Some(zp) = self.var_addr(var) {
                    self.emit_int_to_str(zp, addr);
                }
            }
            Stmt::Color { target, expr } => {
                let expr = expr.clone();
                self.eval_expr(&expr);
                let addr = match target {
                    ColorTarget::Text   => 0x0286,
                    ColorTarget::Border => VIC_BORDER,
                    ColorTarget::Bg     => VIC_BG,
                };
                self.emit(0x8D); self.emit16(addr); // STA addr
            }
            Stmt::Cls { manual } => {
                if *manual {
                    self.emit_cls_manual();
                } else {
                    // KERNAL clear screen
                    self.emit(0x20); self.emit16(0xE544); // JSR $E544
                }
            }
            Stmt::Graphics { on } => {
                if *on {
                    self.emit_graphics_on();
                } else {
                    self.emit_graphics_off();
                }
            }
            Stmt::SubDef(name, params, body) => {
                let addr = self.current_addr();
                self.subs.insert(name.clone(), addr);
                // Register params as vars with their pre-allocated ZP addresses
                if let Some(param_addrs) = self.sub_params.get(name).cloned() {
                    for (i, param_name) in params.iter().enumerate() {
                        if let Some(&zp) = param_addrs.get(i) {
                            self.vars.insert(param_name.clone(), zp);
                        }
                    }
                }
                self.gen_stmts(body);
                self.emit(0x60); // RTS
            }
            Stmt::Call(name, args) => {
                // Store args into the sub's parameter ZP slots before calling
                if let Some(param_addrs) = self.sub_params.get(name).cloned() {
                    for (i, arg) in args.iter().enumerate() {
                        if let Some(&zp) = param_addrs.get(i) {
                            let arg = arg.clone();
                            self.eval_expr(&arg);
                            self.emit(0x85); self.emit(zp); // STA param_zp
                        }
                    }
                }
                self.emit(0x20); // JSR
                if let Some(&addr) = self.subs.get(name) {
                    self.emit16(addr);
                } else {
                    // Forward reference — patch later
                    let patch = self.code.len();
                    self.emit16(0x0000);
                    self.sub_patches.push((patch, name.clone()));
                }
            }
            Stmt::Return => {
                self.emit(0x60); // RTS
            }
            Stmt::Const(..) => {
                // Constants are handled at parse time (stored in parser.consts)
                // No code generation needed
            }
            Stmt::Label(name) => {
                let addr = self.current_addr();
                self.labels.insert(name.clone(), addr);
            }
            Stmt::Goto(name) => {
                self.emit(0x4C); // JMP
                if let Some(&addr) = self.labels.get(name) {
                    self.emit16(addr);
                } else {
                    let pos = self.code.len();
                    self.emit16(0x0000);
                    self.goto_patches.push((pos, name.clone()));
                }
            }
            Stmt::ArraySet(arr_name, idx_expr, val_expr) => {
                let base = self.arrays.get(arr_name).copied().unwrap_or(0xC000);
                let val  = val_expr.clone();
                let idx  = idx_expr.clone();
                self.eval_expr(&val);
                let tmp = self.tmp_zp; self.tmp_zp += 1;
                self.emit(0x85); self.emit(tmp); // STA tmp (value)
                match &idx {
                    Expr::Number(n) => {
                        let addr = base.wrapping_add(*n as u16);
                        self.emit(0xA5); self.emit(tmp);    // LDA tmp
                        self.emit(0x8D); self.emit16(addr); // STA base+n
                    }
                    _ => {
                        let ptr = self.tmp_zp; self.tmp_zp += 2;
                        self.emit(0xA9); self.emit(base as u8);
                        self.emit(0x85); self.emit(ptr);
                        self.emit(0xA9); self.emit((base >> 8) as u8);
                        self.emit(0x85); self.emit(ptr + 1);
                        self.eval_expr(&idx);               // index → A
                        self.emit(0xA8);                    // TAY
                        self.emit(0xA5); self.emit(tmp);    // LDA tmp (value)
                        self.emit(0x91); self.emit(ptr);    // STA (ptr),Y
                    }
                }
            }
            Stmt::Poke(addr, val) => {
                let val = val.clone();
                let addr = addr.clone();
                self.eval_expr(&val);
                let tmp_val = self.tmp_zp; self.tmp_zp += 1;
                self.emit(0x85); self.emit(tmp_val); // STA tmp_val
                if let Expr::Number(n) = &addr {
                    // Constant address: direct STA abs
                    self.emit(0xA5); self.emit(tmp_val); // LDA tmp_val
                    self.emit(0x8D); self.emit(*n as u8); self.emit((n >> 8) as u8);
                } else if let Expr::Var(ref vname) = addr {
                    if matches!(self.var_types.get(vname), Some(VarType::Word)) {
                        // Word var already holds 16-bit address in ZP pair → STA (zp),Y
                        if let Some(zp) = self.var_addr(vname) {
                            self.emit(0xA0); self.emit(0x00);    // LDY #0
                            self.emit(0xA5); self.emit(tmp_val); // LDA tmp_val
                            self.emit(0x91); self.emit(zp);      // STA (zp),Y
                        }
                    } else {
                        // 8-bit var used as lo-byte address (rare but valid)
                        let ptr = self.tmp_zp; self.tmp_zp += 2;
                        self.eval_expr(&addr);
                        self.emit(0x85); self.emit(ptr);
                        self.emit(0xA9); self.emit(0x00);
                        self.emit(0x85); self.emit(ptr + 1);
                        self.emit(0xA8);
                        self.emit(0xA5); self.emit(tmp_val);
                        self.emit(0x91); self.emit(ptr);
                    }
                } else {
                    // General expression address: compute then indirect
                    let ptr = self.tmp_zp; self.tmp_zp += 2;
                    self.eval_expr(&addr);
                    self.emit(0x85); self.emit(ptr);     // STA ptr_lo
                    self.emit(0xA9); self.emit(0x00);    // LDA #0
                    self.emit(0x85); self.emit(ptr + 1); // STA ptr_hi
                    self.emit(0xA8);                      // TAY (Y=0)
                    self.emit(0xA5); self.emit(tmp_val); // LDA tmp_val
                    self.emit(0x91); self.emit(ptr);     // STA (ptr),Y
                }
            }
        }
    }

    fn patch_forward_refs(&mut self) {
        for (offset, name) in self.sub_patches.clone() {
            if let Some(&addr) = self.subs.get(&name) {
                self.code[offset] = addr as u8;
                self.code[offset + 1] = (addr >> 8) as u8;
            }
        }
        for (offset, name) in self.goto_patches.clone() {
            if let Some(&addr) = self.labels.get(&name) {
                self.code[offset] = addr as u8;
                self.code[offset + 1] = (addr >> 8) as u8;
            }
        }
    }

    /// Returns raw machine code bytes (no PRG header).
    /// Two-pass: main code first, subroutines after.
    /// This prevents sub bodies from executing as inline code at startup.
    pub fn compile(&mut self, stmts: &[Stmt]) -> Vec<u8> {
        // Pre-scan: allocate ZP for sub params, register arrays
        self.pre_scan(stmts);

        // Pass 1: everything except SubDef
        for stmt in stmts {
            if !matches!(stmt, Stmt::SubDef(..)) {
                self.gen_stmt(stmt);
            }
        }
        self.emit(0x60); // RTS — end of main program

        // Pass 2: subroutine definitions (after main, so they aren't executed at startup)
        for stmt in stmts {
            if matches!(stmt, Stmt::SubDef(..)) {
                self.gen_stmt(stmt);
            }
        }

        self.patch_forward_refs();
        self.code.clone()
    }

    pub fn errors(&self) -> Vec<String> {
        let mut errs = vec![];
        for (_, name) in &self.sub_patches {
            if !self.subs.contains_key(name) {
                errs.push(format!("Undefined subroutine: {name}"));
            }
        }
        for (_, name) in &self.goto_patches {
            if !self.labels.contains_key(name) {
                errs.push(format!("Undefined label: {name}"));
            }
        }
        errs
    }
}

fn ascii_to_petscii(c: char) -> u8 {
    // C64 default mode: uppercase/graphics
    // PETSCII $41-$5A = uppercase A-Z (same codes as ASCII uppercase)
    // lowercase input → convert to uppercase
    match c {
        'A'..='Z' => c as u8,
        'a'..='z' => c as u8 - 0x20,
        '0'..='9' => c as u8,
        ' '       => 0x20,
        '!'       => 0x21,
        '"'       => 0x22,
        '#'       => 0x23,
        '$'       => 0x24,
        '%'       => 0x25,
        '&'       => 0x26,
        '\''      => 0x27,
        '('       => 0x28,
        ')'       => 0x29,
        '*'       => 0x2A,
        '+'       => 0x2B,
        ','       => 0x2C,
        '-'       => 0x2D,
        '.'       => 0x2E,
        '/'       => 0x2F,
        ':'       => 0x3A,
        ';'       => 0x3B,
        '<'       => 0x3C,
        '='       => 0x3D,
        '>'       => 0x3E,
        '?'       => 0x3F,
        '@'       => 0x40,
        _         => 0x3F, // '?' for unknown
    }
}
