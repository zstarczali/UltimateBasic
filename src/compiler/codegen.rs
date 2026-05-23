use std::collections::HashMap;
use super::ast::{Expr, BinOp, Stmt, ColorTarget, VarType, ReuOp};
use super::{MemoryMap, VarEntry, SubEntry, ArrayEntry};

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
    array_sizes: HashMap<String, u16>,      // array_name → size in bytes
    array_ptr: u16,                         // next free array slot
    rnd_seeded: bool,
    plot_zp: Option<u8>,                    // base of 5-byte ZP block for plot helper
    plot_patches: Vec<usize>,               // code positions of JSR targets to patch
    line_zp: Option<u8>,                    // base of 12-byte ZP block for line (Bresenham)
    line_patches: Vec<usize>,               // code positions of JSR targets for drawline helper
    sin_table_patches: Vec<usize>,          // positions of 2-byte address in LDA abs,X for sin/cos
    sin_table_addr: Option<u16>,            // absolute address of the emitted 256-byte sin table
    hex_helper_patches: Vec<usize>,         // JSR targets for print_hex helper
    bin_helper_patches: Vec<usize>,         // JSR targets for print_bin helper
    data_bytes: Vec<u8>,                    // all data-statement bytes (collected in pre_scan)
    data_zp: Option<u8>,                    // ZP pair: lo at zp, hi at zp+1
    data_ptr_lo_patch: Option<usize>,       // code pos of LDA #lo in init sequence
    data_ptr_hi_patch: Option<usize>,       // code pos of LDA #hi in init sequence
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
            array_sizes: HashMap::new(),
            array_ptr: 0xC000,
            rnd_seeded: false,
            plot_zp: None,
            plot_patches: vec![],
            line_zp: None,
            line_patches: vec![],
            sin_table_patches: vec![],
            sin_table_addr: None,
            hex_helper_patches: vec![],
            bin_helper_patches: vec![],
            data_bytes: vec![],
            data_zp: None,
            data_ptr_lo_patch: None,
            data_ptr_hi_patch: None,
        }
    }

    /// Recursively check whether any Plot statement exists anywhere in the AST.
    fn has_plot_stmt(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::Plot(..) => return true,
                Stmt::SubDef(_, _, body) => if Self::has_plot_stmt(body) { return true; }
                Stmt::If(_, then_b, else_b) => {
                    if Self::has_plot_stmt(then_b) { return true; }
                    if let Some(eb) = else_b { if Self::has_plot_stmt(eb) { return true; } }
                }
                Stmt::ForLoop { body, .. } | Stmt::Loop(_, body) | Stmt::WhileLoop(_, body) => {
                    if Self::has_plot_stmt(body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    /// Recursively check whether any Line statement exists anywhere in the AST.
    fn has_line_stmt(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::Line { .. } => return true,
                Stmt::SubDef(_, _, body) => if Self::has_line_stmt(body) { return true; }
                Stmt::If(_, then_b, else_b) => {
                    if Self::has_line_stmt(then_b) { return true; }
                    if let Some(eb) = else_b { if Self::has_line_stmt(eb) { return true; } }
                }
                Stmt::ForLoop { body, .. } | Stmt::Loop(_, body) | Stmt::WhileLoop(_, body) => {
                    if Self::has_line_stmt(body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    fn has_data_or_read(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::Data(_) | Stmt::Read(_) => return true,
                Stmt::SubDef(_, _, body) => if Self::has_data_or_read(body) { return true; }
                Stmt::If(_, then_b, else_b) => {
                    if Self::has_data_or_read(then_b) { return true; }
                    if let Some(eb) = else_b { if Self::has_data_or_read(eb) { return true; } }
                }
                Stmt::ForLoop { body, .. } | Stmt::Loop(_, body) | Stmt::WhileLoop(_, body) => {
                    if Self::has_data_or_read(body) { return true; }
                }
                _ => {}
            }
        }
        false
    }

    fn collect_data_bytes(stmts: &[Stmt]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::Data(items) => {
                    for item in items {
                        if let Expr::Number(n) = item { bytes.push(*n as u8); }
                    }
                }
                Stmt::SubDef(_, _, body) => bytes.extend(Self::collect_data_bytes(body)),
                Stmt::If(_, then_b, else_b) => {
                    bytes.extend(Self::collect_data_bytes(then_b));
                    if let Some(eb) = else_b { bytes.extend(Self::collect_data_bytes(eb)); }
                }
                Stmt::ForLoop { body, .. } | Stmt::Loop(_, body) | Stmt::WhileLoop(_, body) => {
                    bytes.extend(Self::collect_data_bytes(body));
                }
                _ => {}
            }
        }
        bytes
    }

    /// Pre-scan: allocate ZP slots for sub params, register arrays, reserve plot ZP.
    /// Must run before gen_stmt so that reserved slots precede regular vars in ZP.
    fn pre_scan(&mut self, stmts: &[Stmt]) {
        // Reserve 6 ZP bytes for the plot helper (X_lo, X_hi, Y, temp, ptr_lo, ptr_hi).
        // Also needed when line is used (drawline calls the plot helper).
        if Self::has_plot_stmt(stmts) || Self::has_line_stmt(stmts) {
            let zp = self.perm_zp;
            self.perm_zp += 6;
            self.plot_zp = Some(zp);
        }

        // Reserve 12 ZP bytes for the Bresenham line helper.
        // Layout: cx,cy,x2,y2,|dx|,|dy|,sx,sy,err_lo,err_hi,e2_lo,e2_hi
        if Self::has_line_stmt(stmts) {
            let zp = self.perm_zp;
            self.perm_zp += 12;
            self.line_zp = Some(zp);
        }

        // Reserve 2 ZP bytes for the data pointer (lo/hi) if data/read is used.
        if Self::has_data_or_read(stmts) {
            let zp = self.perm_zp;
            self.perm_zp += 2;
            self.data_zp = Some(zp);
            self.data_bytes = Self::collect_data_bytes(stmts);
        }

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
                    self.array_sizes.insert(name.clone(), size);
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
            Expr::ReuPresent => {
                // Write $55 to $DF04, read back; write $AA, read back.
                // Both must match → REU present (returns 1), else returns 0.
                // No ZP scratch needed; result in A.
                //
                // offset  0: A9 55        LDA #$55
                // offset  2: 8D 04 DF     STA $DF04
                // offset  5: AD 04 DF     LDA $DF04
                // offset  8: C9 55        CMP #$55
                // offset 10: D0 0C        BNE fail    (+12 → offset 24)
                // offset 12: A9 AA        LDA #$AA
                // offset 14: 8D 04 DF     STA $DF04
                // offset 17: AD 04 DF     LDA $DF04
                // offset 20: C9 AA        CMP #$AA
                // offset 22: F0 05        BEQ ok      (+5  → offset 29)
                // offset 24: A9 00        LDA #0      (fail)
                // offset 26: 4C ?? ??     JMP done
                // offset 29: A9 01        LDA #1      (ok)
                // offset 31:              (done)
                self.emit(0xA9); self.emit(0x55);        // LDA #$55
                self.emit(0x8D); self.emit16(0xDF04);    // STA $DF04
                self.emit(0xAD); self.emit16(0xDF04);    // LDA $DF04
                self.emit(0xC9); self.emit(0x55);        // CMP #$55
                self.emit(0xD0); self.emit(0x0C);        // BNE fail
                self.emit(0xA9); self.emit(0xAA);        // LDA #$AA
                self.emit(0x8D); self.emit16(0xDF04);    // STA $DF04
                self.emit(0xAD); self.emit16(0xDF04);    // LDA $DF04
                self.emit(0xC9); self.emit(0xAA);        // CMP #$AA
                self.emit(0xF0); self.emit(0x05);        // BEQ ok
                self.emit(0xA9); self.emit(0x00);        // fail: LDA #0
                self.emit(0x4C);                         // JMP done (abs)
                let jmp_patch = self.code.len();
                self.emit(0x00); self.emit(0x00);        // patch later
                self.emit(0xA9); self.emit(0x01);        // ok: LDA #1
                // patch JMP target (current position = done)
                let done_addr = self.current_addr();
                self.code[jmp_patch]     = (done_addr & 0xFF) as u8;
                self.code[jmp_patch + 1] = (done_addr >> 8)   as u8;
            }
            Expr::Getch => {
                let loop_addr = self.current_addr();
                self.emit(0xA9); self.emit(0xFF);     // LDA #$FF
                self.emit(0x85); self.emit(0x91);     // STA $91
                self.emit(0x20); self.emit16(0xFFE4); // JSR $FFE4
                self.emit(0xC9); self.emit(0x00);
                self.emit(0xF0);
                let beq_zero = self.code.len(); self.emit(0x00);
                self.patch_bxx(beq_zero, loop_addr);

                // Ignore RUN/STOP key (GETIN returns $03 on C64).
                self.emit(0xC9); self.emit(0x03);     // CMP #$03
                self.emit(0xF0);
                let beq_stop = self.code.len(); self.emit(0x00);
                self.patch_bxx(beq_stop, loop_addr);

                // Preserve key in A while clearing RUN/STOP flag to avoid BREAK on return.
                self.emit(0xAA);                      // TAX
                self.emit(0xA9); self.emit(0xFF);     // LDA #$FF
                self.emit(0x85); self.emit(0x91);     // STA $91
                self.emit(0x8A);                      // TXA
            }
            Expr::Inkey => {
                // Non-blocking GETIN: single call, returns 0 if no key pressed.
                self.emit(0x20); self.emit16(0xFFE4); // JSR $FFE4
            }
            Expr::StrLen(inner) => {
                let inner = inner.clone();
                match inner.as_ref() {
                    Expr::StringLit(s) => {
                        // compile-time: length known
                        self.emit(0xA9); self.emit(s.len() as u8); // LDA #len
                    }
                    Expr::Var(name) if matches!(self.var_types.get(name), Some(VarType::Str)) => {
                        if let Some(ptr) = self.var_addr(name) {
                            // inline len loop: LDY #$FF; loop: INY; LDA (ptr),Y; BNE loop; TYA
                            self.emit(0xA0); self.emit(0xFF);  // LDY #$FF
                            let loop_top = self.current_addr();
                            self.emit(0xC8);                    // INY
                            self.emit(0xB1); self.emit(ptr);   // LDA (ptr),Y
                            self.emit(0xD0);                    // BNE loop
                            let bne_pos = self.code.len(); self.emit(0x00);
                            self.patch_bxx(bne_pos, loop_top);
                            self.emit(0x98);                    // TYA → A = length
                        } else {
                            self.emit(0xA9); self.emit(0x00);
                        }
                    }
                    _ => { self.eval_expr(&inner); } // fallback: evaluate as numeric
                }
            }
            Expr::Asc(inner) => {
                let inner = inner.clone();
                match inner.as_ref() {
                    Expr::StringLit(s) => {
                        let code = s.chars().next().map(|c| ascii_to_petscii(c)).unwrap_or(0);
                        self.emit(0xA9); self.emit(code);      // LDA #first_char
                    }
                    Expr::Var(name) if matches!(self.var_types.get(name), Some(VarType::Str)) => {
                        if let Some(ptr) = self.var_addr(name) {
                            self.emit(0xA0); self.emit(0x00);  // LDY #0
                            self.emit(0xB1); self.emit(ptr);   // LDA (ptr),Y → first char
                        } else {
                            self.emit(0xA9); self.emit(0x00);
                        }
                    }
                    _ => { self.eval_expr(&inner); }
                }
            }
            Expr::SpriteHit => {
                // Read $D01E — sprite-sprite collision register (cleared on read).
                self.emit(0xAD); self.emit16(0xD01E);
            }
            Expr::SpriteBgHit => {
                // Read $D01F — sprite-background collision register (cleared on read).
                self.emit(0xAD); self.emit16(0xD01F);
            }
            Expr::Joy(port) => {
                // CIA1 joystick: port 2 = $DC00, port 1 = $DC01; bits 0-4 active-low.
                // Return inverted lower 5 bits: bit0=up, bit1=down, bit2=left, bit3=right, bit4=fire.
                let addr: u16 = if *port == 1 { 0xDC01 } else { 0xDC00 };
                self.emit(0xAD); self.emit(addr as u8); self.emit((addr >> 8) as u8); // LDA $DCxx
                self.emit(0x29); self.emit(0x1F);     // AND #$1F  (keep bits 0-4)
                self.emit(0x49); self.emit(0x1F);     // EOR #$1F  (invert: 1 = pressed)
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
            Expr::ChrStr(inner) => {
                // chr$(n) evaluates to the raw byte value n (PETSCII code).
                // In print context print_single_arg handles the JSR CHROUT.
                let inner = inner.clone();
                self.eval_expr(&inner);
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
                self.emit(0x85); self.emit(0xFB);  // STA $FB  (store seed)
                self.emit(0x4D); self.emit(0x12); self.emit(0xD0); // EOR $D012 (post-whiten)
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
            Expr::Sin(e) => {
                // sin(angle): angle 0-255 → lookup table → 0-255 (center=128)
                let e = e.clone();
                self.eval_expr(&e);
                self.emit(0xAA);    // TAX — angle into X
                self.emit(0xBD);    // LDA abs,X
                let patch = self.code.len();
                self.emit(0x00); self.emit(0x00);  // table address (patched later)
                self.sin_table_patches.push(patch);
            }
            Expr::Cos(e) => {
                // cos(angle) = sin(angle + 64) for 256-step circle
                let e = e.clone();
                self.eval_expr(&e);
                self.emit(0x18);                   // CLC
                self.emit(0x69); self.emit(64);    // ADC #64 — quarter period (+90°)
                self.emit(0xAA);                   // TAX
                self.emit(0xBD);                   // LDA abs,X
                let patch = self.code.len();
                self.emit(0x00); self.emit(0x00);
                self.sin_table_patches.push(patch);
            }
            Expr::HexFmt(inner) | Expr::BinFmt(inner) => {
                // In non-print context, evaluate the inner expression (pass-through)
                let inner = inner.clone();
                self.eval_expr(&inner);
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
                    BinOp::Xor => {
                        // Bitwise XOR
                        let tmp = self.tmp_zp; self.tmp_zp += 1;
                        self.eval_expr(l);
                        self.emit(0x85); self.emit(tmp);  // STA tmp (l)
                        self.eval_expr(r);
                        self.emit(0x45); self.emit(tmp);  // EOR tmp → A = l ^ r
                    }
                    BinOp::Shl | BinOp::Shr => {
                        // Shift left/right by variable or constant amount
                        // A = left_val (stored in tmp); shift A left/right by right_val (cnt)
                        let tmp = self.tmp_zp; self.tmp_zp += 1;
                        let cnt = self.tmp_zp; self.tmp_zp += 1;
                        let l = l.clone(); let r = r.clone(); let op = op.clone();
                        self.eval_expr(&l);
                        self.emit(0x85); self.emit(tmp);  // STA tmp (value to shift)
                        self.eval_expr(&r);
                        // if shift count == 0, skip the loop
                        let beq_done = self.code.len();
                        self.emit(0xF0); self.emit(0x00); // BEQ done (patched)
                        self.emit(0x85); self.emit(cnt);  // STA cnt (shift count)
                        let loop_top = self.code.len();
                        if matches!(op, BinOp::Shl) {
                            self.emit(0x06); self.emit(tmp); // ASL tmp
                        } else {
                            self.emit(0x46); self.emit(tmp); // LSR tmp
                        }
                        self.emit(0xC6); self.emit(cnt);  // DEC cnt
                        let bne_pos = self.code.len();
                        self.emit(0xD0); self.emit(0x00); // BNE loop_top (patched)
                        self.emit(0xA5); self.emit(tmp);  // LDA tmp (done)
                        let done_addr = self.current_addr();
                        self.patch_bxx(beq_done + 1, done_addr);
                        self.patch_bxx(bne_pos + 1, self.load_addr + loop_top as u16);
                        return; // result already in A via LDA tmp above
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
                    BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Shl | BinOp::Shr => unreachable!(),
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

    // Fast CLS: fill screen RAM $0400-$07FF with spaces, color RAM with white,
    // then reset cursor position via KERNAL home ($E566).
    fn emit_cls_fast(&mut self) {
        // Fill screen RAM $0400-$07FF (4 × 256 = 1024 bytes) with space ($20).
        // Uses X-register natural overflow: INX from $FF wraps to $00 → BNE exits.
        // This covers all 4 pages in one loop, same as the classic C64 technique.
        // ($07F8-$07FF are sprite pointers; overwriting with $20 is harmless in text mode.)
        self.emit(0xA9); self.emit(0x20); // LDA #$20
        self.emit(0xA2); self.emit(0x00); // LDX #0
        let lp1 = self.current_addr();
        self.emit(0x9D); self.emit16(0x0400); // STA $0400,X
        self.emit(0x9D); self.emit16(0x0500); // STA $0500,X
        self.emit(0x9D); self.emit16(0x0600); // STA $0600,X
        self.emit(0x9D); self.emit16(0x0700); // STA $0700,X
        self.emit(0xE8);                       // INX
        self.emit(0xD0);                       // BNE lp1 (exits when X wraps $FF→$00)
        let bne1 = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne1, lp1);

        // Fill color RAM $D800-$DBFF (4 × 256 = 1024 bytes) with white ($01).
        self.emit(0xA9); self.emit(0x01); // LDA #1 (white)
        self.emit(0xA2); self.emit(0x00); // LDX #0
        let lp2 = self.current_addr();
        self.emit(0x9D); self.emit16(0xD800); // STA $D800,X
        self.emit(0x9D); self.emit16(0xD900); // STA $D900,X
        self.emit(0x9D); self.emit16(0xDA00); // STA $DA00,X
        self.emit(0x9D); self.emit16(0xDB00); // STA $DB00,X
        self.emit(0xE8);                       // INX
        self.emit(0xD0);                       // BNE lp2
        let bne2 = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne2, lp2);

        // Cursor home
        self.emit(0x20); self.emit16(0xE566); // JSR $E566
    }

    // Graphics ON: C64 hires or multicolor bitmap mode at $2000, video matrix at $0400.
    // multi=false → standard hires 320×200 (1bpp); multi=true → multicolor 160×200 (2bpp).
    fn emit_graphics_on(&mut self, multi: bool) {
        // ── 1. Blank display to avoid mode-switch glitch ──────────────────
        self.emit(0xAD); self.emit16(0xD011); // LDA $D011
        self.emit(0x29); self.emit(0xEF);     // AND #$EF  (clear DEN=bit4 → blank)
        self.emit(0x8D); self.emit16(0xD011); // STA $D011

        // ── 2. Set VIC memory layout: bitmap @$2000, matrix @$0400 ────────
        self.emit(0xA9); self.emit(0x18);     // LDA #$18
        self.emit(0x8D); self.emit16(0xD018); // STA $D018

        // ── 3. Set or clear MCM bit ($D016 bit4) ──────────────────────────
        self.emit(0xAD); self.emit16(0xD016); // LDA $D016
        if multi {
            self.emit(0x09); self.emit(0x10); // ORA #$10  (set MCM=bit4)
        } else {
            self.emit(0x29); self.emit(0xEF); // AND #$EF  (clear MCM=bit4)
        }
        self.emit(0x8D); self.emit16(0xD016); // STA $D016

        // ── 4. Set BMM — display stays blanked (DEN=0), user calls `display on` ──
        self.emit(0xAD); self.emit16(0xD011); // LDA $D011
        self.emit(0x09); self.emit(0x20);     // ORA #$20  (set BMM=bit5 only; DEN stays 0)
        self.emit(0x8D); self.emit16(0xD011); // STA $D011
    }

    // Graphics OFF: back to text mode with display blanking around the switch.
    fn emit_graphics_off(&mut self) {
        // ── 1. Blank display ──────────────────────────────────────────────
        self.emit(0xAD); self.emit16(0xD011); // LDA $D011
        self.emit(0x29); self.emit(0xEF);     // AND #$EF  (clear DEN → blank)
        self.emit(0x8D); self.emit16(0xD011); // STA $D011

        // ── 2. Clear MCM ($D016): 40-col text, single-color ───────────────
        self.emit(0xAD); self.emit16(0xD016); // LDA $D016
        self.emit(0x29); self.emit(0xEF);     // AND #$EF  (clear MCM=bit4)
        self.emit(0x09); self.emit(0x08);     // ORA #$08  (set CSEL=bit3 → 40 cols)
        self.emit(0x8D); self.emit16(0xD016); // STA $D016

        // ── 3. Restore $D018: screen @$0400, char @$1000 ─────────────────
        self.emit(0xA9); self.emit(0x14);     // LDA #$14
        self.emit(0x8D); self.emit16(0xD018); // STA $D018

        // ── 4. CIA2 VIC bank: bank 0 ($0000-$3FFF) ───────────────────────
        self.emit(0xAD); self.emit16(0xDD00); // LDA $DD00
        self.emit(0x29); self.emit(0xFC);     // AND #$FC
        self.emit(0x09); self.emit(0x03);     // ORA #$03
        self.emit(0x8D); self.emit16(0xDD00); // STA $DD00

        // ── 5. Unblank in text mode: clear BMM, set DEN + RSEL + YSCROLL=3
        self.emit(0xAD); self.emit16(0xD011); // LDA $D011
        self.emit(0x29); self.emit(0xDF);     // AND #$DF  (clear BMM=bit5)
        self.emit(0x09); self.emit(0x1B);     // ORA #$1B  (DEN+RSEL+YSCROLL=3)
        self.emit(0x8D); self.emit16(0xD011); // STA $D011
    }

    // Gcls: clear bitmap $2000-$3FFF (32 pages with $00) AND fill video matrix
    // $0400-$07FF (4 pages with $10 = white-on-black) so bitmap mode has clean colors.
    fn emit_gcls(&mut self) {
        let ptr_lo = self.tmp_zp; self.tmp_zp += 1;
        let ptr_hi = self.tmp_zp; self.tmp_zp += 1;
        let pg_ctr = self.tmp_zp; self.tmp_zp += 1;

        // ── 1. Zero-fill bitmap $2000-$3FFF (32 pages) ──────────────────────
        self.emit(0xA9); self.emit(0x00);
        self.emit(0x85); self.emit(ptr_lo);   // ptr = $2000
        self.emit(0xA9); self.emit(0x20);
        self.emit(0x85); self.emit(ptr_hi);
        self.emit(0xA9); self.emit(0x20);
        self.emit(0x85); self.emit(pg_ctr);   // 32 pages
        self.emit(0xA9); self.emit(0x00);     // fill value = $00

        let bm_page_top = self.current_addr();
        self.emit(0xA0); self.emit(0x00);     // LDY #0
        let bm_byte_top = self.current_addr();
        self.emit(0x91); self.emit(ptr_lo);   // STA (ptr),Y
        self.emit(0xC8);                       // INY
        self.emit(0xD0);                       // BNE bm_byte_top
        let bne_bm_i = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne_bm_i, bm_byte_top);
        self.emit(0xE6); self.emit(ptr_hi);   // INC ptr_hi
        self.emit(0xC6); self.emit(pg_ctr);  // DEC pg_ctr
        self.emit(0xD0);                       // BNE bm_page_top
        let bne_bm_o = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne_bm_o, bm_page_top);

        // ── 2. Fill video matrix $0400-$07FF with $10 (white/black) ─────────
        // Hires bitmap: high nibble = foreground color, low nibble = background.
        // $10 = foreground 1 (white), background 0 (black).
        self.emit(0xA9); self.emit(0x00);
        self.emit(0x85); self.emit(ptr_lo);   // ptr = $0400
        self.emit(0xA9); self.emit(0x04);
        self.emit(0x85); self.emit(ptr_hi);
        self.emit(0xA9); self.emit(0x04);
        self.emit(0x85); self.emit(pg_ctr);   // 4 pages
        self.emit(0xA9); self.emit(0x10);     // fill value = $10

        let vm_page_top = self.current_addr();
        self.emit(0xA0); self.emit(0x00);     // LDY #0
        let vm_byte_top = self.current_addr();
        self.emit(0x91); self.emit(ptr_lo);   // STA (ptr),Y
        self.emit(0xC8);                       // INY
        self.emit(0xD0);                       // BNE vm_byte_top
        let bne_vm_i = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne_vm_i, vm_byte_top);
        self.emit(0xE6); self.emit(ptr_hi);   // INC ptr_hi
        self.emit(0xC6); self.emit(pg_ctr);  // DEC pg_ctr
        self.emit(0xD0);                       // BNE vm_page_top
        let bne_vm_o = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne_vm_o, vm_page_top);
    }

    // Plot helper subroutine (emitted once, called via JSR).
    // ZP layout: zp+0=X_lo, zp+1=X_hi, zp+2=Y, zp+3=b/mask, zp+4=ptr_lo, zp+5=ptr_hi
    // X: 0-319 (full bitmap width), Y: 0-199
    // Bitmap at $2000: byte = $2000 + (Y>>3)*320 + (X & $1F8) + (Y&7); bit = $80 >> (X&7)
    fn emit_plot_helper(&mut self) {
        let zp = match self.plot_zp { Some(z) => z, None => return };

        // b = Y >> 3  (cell row, 0-24)
        self.emit(0xA5); self.emit(zp + 2);   // LDA Y
        self.emit(0x4A);                       // LSR
        self.emit(0x4A);                       // LSR
        self.emit(0x4A);                       // LSR  → A = b
        self.emit(0x85); self.emit(zp + 3);   // STA b

        // ptr_lo = (b*64) & $FF
        self.emit(0x0A);                       // ASL ×6
        self.emit(0x0A);
        self.emit(0x0A);
        self.emit(0x0A);
        self.emit(0x0A);
        self.emit(0x0A);
        self.emit(0x85); self.emit(zp + 4);   // STA ptr_lo

        // ptr_hi = b + (b>>2) + $20
        self.emit(0xA5); self.emit(zp + 3);   // LDA b
        self.emit(0x4A);                       // LSR
        self.emit(0x4A);                       // LSR  → b>>2
        self.emit(0x18);                       // CLC
        self.emit(0x65); self.emit(zp + 3);   // ADC b
        self.emit(0x69); self.emit(0x20);      // ADC #$20
        self.emit(0x85); self.emit(zp + 5);   // STA ptr_hi

        // ptr_lo += X_lo & $F8  (low part of cell_col*8)
        self.emit(0xA5); self.emit(zp + 0);   // LDA X_lo
        self.emit(0x29); self.emit(0xF8);      // AND #$F8
        self.emit(0x18);                       // CLC
        self.emit(0x65); self.emit(zp + 4);   // ADC ptr_lo
        self.emit(0x85); self.emit(zp + 4);   // STA ptr_lo
        self.emit(0x90);                       // BCC skip_inc1
        let bcc1 = self.code.len(); self.emit(0x00);
        self.emit(0xE6); self.emit(zp + 5);   // INC ptr_hi
        self.patch_bxx(bcc1, self.current_addr());

        // If X_hi != 0 (X >= 256): add 1 to ptr_hi (X & $100 contribution)
        self.emit(0xA5); self.emit(zp + 1);   // LDA X_hi
        self.emit(0xF0);                       // BEQ skip_xhi
        let beq_xhi = self.code.len(); self.emit(0x00);
        self.emit(0xE6); self.emit(zp + 5);   // INC ptr_hi
        self.patch_bxx(beq_xhi, self.current_addr());

        // ptr_lo += Y & 7  (pixel row within cell)
        self.emit(0xA5); self.emit(zp + 2);   // LDA Y
        self.emit(0x29); self.emit(0x07);      // AND #$07
        self.emit(0x18);                       // CLC
        self.emit(0x65); self.emit(zp + 4);   // ADC ptr_lo
        self.emit(0x85); self.emit(zp + 4);   // STA ptr_lo
        self.emit(0x90);                       // BCC skip_inc2
        let bcc2 = self.code.len(); self.emit(0x00);
        self.emit(0xE6); self.emit(zp + 5);   // INC ptr_hi
        self.patch_bxx(bcc2, self.current_addr());

        // bit mask = $80 >> (X_lo & 7)  — pixel column within byte
        self.emit(0xA5); self.emit(zp + 0);   // LDA X_lo
        self.emit(0x29); self.emit(0x07);      // AND #$07
        self.emit(0xAA);                       // TAX  (shift count)
        self.emit(0xA9); self.emit(0x80);      // LDA #$80
        self.emit(0xE0); self.emit(0x00);      // CPX #$00
        self.emit(0xF0);                       // BEQ done_mask
        let beq_mask = self.code.len(); self.emit(0x00);
        let shift_top = self.current_addr();
        self.emit(0x4A);                       // LSR
        self.emit(0xCA);                       // DEX
        self.emit(0xD0);                       // BNE shift_top
        let bne_shift = self.code.len(); self.emit(0x00);
        self.patch_bxx(bne_shift, shift_top);
        self.patch_bxx(beq_mask, self.current_addr());

        // Set the pixel
        self.emit(0x85); self.emit(zp + 3);   // STA mask (reuse b slot)
        self.emit(0xA0); self.emit(0x00);      // LDY #0
        self.emit(0xB1); self.emit(zp + 4);   // LDA (ptr_lo),Y
        self.emit(0x05); self.emit(zp + 3);   // ORA mask
        self.emit(0x91); self.emit(zp + 4);   // STA (ptr_lo),Y
        self.emit(0x60);                       // RTS
    }

    /// Emit code to store a 16-bit address expression to two consecutive REU registers.
    /// Used for C64 address ($DF02/$DF03), REU address ($DF04/$DF05), length ($DF07/$DF08).
    fn emit_addr_to_reu_reg(&mut self, expr: &Expr, lo_reg: u16, hi_reg: u16) {
        match expr {
            Expr::Number(n) => {
                let n = *n;
                self.emit(0xA9); self.emit(n as u8);
                self.emit(0x8D); self.emit16(lo_reg);
                self.emit(0xA9); self.emit((n >> 8) as u8);
                self.emit(0x8D); self.emit16(hi_reg);
            }
            Expr::Var(name) => {
                let name = name.clone();
                if matches!(self.var_types.get(&name), Some(VarType::Word)) {
                    if let Some(zp) = self.var_addr(&name) {
                        self.emit(0xA5); self.emit(zp);       // LDA zp_lo
                        self.emit(0x8D); self.emit16(lo_reg); // STA lo_reg
                        self.emit(0xA5); self.emit(zp + 1);   // LDA zp_hi
                        self.emit(0x8D); self.emit16(hi_reg); // STA hi_reg
                    }
                } else if let Some(zp) = self.var_addr(&name) {
                    self.emit(0xA5); self.emit(zp);
                    self.emit(0x8D); self.emit16(lo_reg);
                    self.emit(0xA9); self.emit(0x00);
                    self.emit(0x8D); self.emit16(hi_reg);
                }
            }
            _ => {
                let expr = expr.clone();
                self.eval_expr(&expr);
                self.emit(0x8D); self.emit16(lo_reg);
                self.emit(0xA9); self.emit(0x00);
                self.emit(0x8D); self.emit16(hi_reg);
            }
        }
    }

    /// Print hex helper: called with value in A, prints as 2 uppercase hex digits.
    /// Layout: print_hex (11 bytes) then print_nibble (11 bytes) = 22 bytes total.
    /// print_hex falls through into print_nibble for the low nibble (tail call to CHROUT).
    fn emit_print_hex_helper(&mut self) -> u16 {
        let base = self.current_addr();
        let nibble_addr = base + 11;
        // print_hex:
        self.emit(0x48);                              // PHA         — save byte
        self.emit(0x4A); self.emit(0x4A);             // LSR; LSR    — shift high nibble
        self.emit(0x4A); self.emit(0x4A);             // LSR; LSR      into bits 0-3
        self.emit(0x20); self.emit16(nibble_addr);    // JSR print_nibble — print high nibble
        self.emit(0x68);                              // PLA         — restore byte
        self.emit(0x29); self.emit(0x0F);             // AND #$0F    — isolate low nibble
        // print_nibble: (A = nibble 0-15)
        // if A >= 10: A+7+$30='A'..'F'; else A+$30='0'..'9'
        self.emit(0xC9); self.emit(0x0A);             // CMP #$0A
        self.emit(0x90); self.emit(0x02);             // BCC +2      — skip ADC #6 if < 10
        self.emit(0x69); self.emit(0x06);             // ADC #$06    — carry=1: +7 total
        self.emit(0x69); self.emit(0x30);             // ADC #$30    — to ASCII '0'-'F'
        self.emit(0x4C); self.emit16(CHROUT);         // JMP $FFD2   — CHROUT tail call
        base
    }

    /// Print bin helper: called with value in A, prints as 8-bit binary (MSB first).
    /// Uses only stack (no extra ZP). 17 bytes.
    fn emit_print_bin_helper(&mut self) -> u16 {
        let base = self.current_addr();
        //             offset  bytes
        self.emit(0xA2); self.emit(0x08); //  0  LDX #8
        // loop: (offset 2)
        self.emit(0x0A);                  //  2  ASL A   — MSB into carry, A shifts left
        self.emit(0x48);                  //  3  PHA     — save shifted value
        self.emit(0xA9); self.emit(0x00); //  4  LDA #0
        self.emit(0x2A);                  //  6  ROL     — A = carry (0 or 1)
        self.emit(0x09); self.emit(0x30); //  7  ORA #$30 → '0' or '1'
        self.emit(0x20); self.emit16(CHROUT); // 9  JSR $FFD2
        self.emit(0x68);                  // 12  PLA
        self.emit(0xCA);                  // 13  DEX
        self.emit(0xD0); self.emit(0xF2); // 14  BNE -14 → loop (target: offset 2)
        self.emit(0x60);                  // 16  RTS
        base
    }

    /// 256-byte sin table: sin(i * 2π/256) * 127 + 128, result 1-255 (center=128).
    fn sin_table() -> Vec<u8> {
        (0u16..256).map(|i| {
            let angle = i as f64 * 2.0 * std::f64::consts::PI / 256.0;
            let v = (angle.sin() * 127.0).round() as i32 + 128;
            v.clamp(0, 255) as u8
        }).collect()
    }

    /// Bresenham line helper. Called via JSR; caller fills line_zp+0..3 (cx,cy,x2,y2).
    /// Internally uses line_zp+4..11 and calls the plot helper for each pixel.
    /// ZP layout: zp+0=cx, zp+1=cy, zp+2=x2, zp+3=y2,
    ///            zp+4=|dx|, zp+5=|dy|, zp+6=sx, zp+7=sy,
    ///            zp+8=err_lo, zp+9=err_hi, zp+10=e2_lo, zp+11=e2_hi
    fn emit_drawline_helper(&mut self, plot_helper_addr: u16) {
        let zp  = match self.line_zp  { Some(z) => z, None => return };
        let pzp = match self.plot_zp  { Some(z) => z, None => return };

        // ── |dx| and sx ────────────────────────────────────────────────────
        self.emit(0xA5); self.emit(zp+2);   // LDA x2
        self.emit(0xC5); self.emit(zp+0);   // CMP cx
        self.emit(0xB0);                     // BCS dl_xpos (x2 >= cx)
        let bcs_xpos = self.code.len(); self.emit(0x00);
        // x2 < cx: |dx| = cx - x2, sx = -1
        self.emit(0x38);                     // SEC
        self.emit(0xA5); self.emit(zp+0);   // LDA cx
        self.emit(0xE5); self.emit(zp+2);   // SBC x2
        self.emit(0x85); self.emit(zp+4);   // STA |dx|
        self.emit(0xA9); self.emit(0xFF);   // LDA #$FF
        self.emit(0x85); self.emit(zp+6);   // STA sx
        self.emit(0x4C);                     // JMP dl_caldy
        let jmp_caldy = self.code.len(); self.emit(0x00); self.emit(0x00);
        // dl_xpos: |dx| = x2 - cx, sx = +1
        let dl_xpos = self.current_addr();
        self.patch_bxx(bcs_xpos, dl_xpos);
        self.emit(0x38);
        self.emit(0xA5); self.emit(zp+2);
        self.emit(0xE5); self.emit(zp+0);
        self.emit(0x85); self.emit(zp+4);
        self.emit(0xA9); self.emit(0x01);
        self.emit(0x85); self.emit(zp+6);

        // ── |dy| and sy ────────────────────────────────────────────────────
        let dl_caldy = self.current_addr();
        self.patch_abs(jmp_caldy, dl_caldy);
        self.emit(0xA5); self.emit(zp+3);   // LDA y2
        self.emit(0xC5); self.emit(zp+1);   // CMP cy
        self.emit(0xB0);                     // BCS dl_ypos (y2 >= cy)
        let bcs_ypos = self.code.len(); self.emit(0x00);
        // y2 < cy: |dy| = cy - y2, sy = -1
        self.emit(0x38);
        self.emit(0xA5); self.emit(zp+1);
        self.emit(0xE5); self.emit(zp+3);
        self.emit(0x85); self.emit(zp+5);
        self.emit(0xA9); self.emit(0xFF);
        self.emit(0x85); self.emit(zp+7);
        self.emit(0x4C);                     // JMP dl_init
        let jmp_init = self.code.len(); self.emit(0x00); self.emit(0x00);
        // dl_ypos: |dy| = y2 - cy, sy = +1
        let dl_ypos = self.current_addr();
        self.patch_bxx(bcs_ypos, dl_ypos);
        self.emit(0x38);
        self.emit(0xA5); self.emit(zp+3);
        self.emit(0xE5); self.emit(zp+1);
        self.emit(0x85); self.emit(zp+5);
        self.emit(0xA9); self.emit(0x01);
        self.emit(0x85); self.emit(zp+7);

        // ── err = |dx| - |dy|  (16-bit signed) ────────────────────────────
        let dl_init = self.current_addr();
        self.patch_abs(jmp_init, dl_init);
        self.emit(0x38);                     // SEC
        self.emit(0xA5); self.emit(zp+4);   // LDA |dx|
        self.emit(0xE5); self.emit(zp+5);   // SBC |dy|
        self.emit(0x85); self.emit(zp+8);   // STA err_lo
        self.emit(0xA9); self.emit(0x00);   // LDA #0
        self.emit(0xE9); self.emit(0x00);   // SBC #0  (borrow → err_hi=$FF if dx<dy)
        self.emit(0x85); self.emit(zp+9);   // STA err_hi

        // ── Main loop ──────────────────────────────────────────────────────
        let dl_loop = self.current_addr();
        // Set up plot ZP: X_lo=cx, X_hi=0, Y=cy
        self.emit(0xA5); self.emit(zp+0);   // LDA cx
        self.emit(0x85); self.emit(pzp+0);  // STA X_lo
        self.emit(0xA9); self.emit(0x00);   // LDA #0
        self.emit(0x85); self.emit(pzp+1);  // STA X_hi
        self.emit(0xA5); self.emit(zp+1);   // LDA cy
        self.emit(0x85); self.emit(pzp+2);  // STA Y
        self.emit(0x20); self.emit(plot_helper_addr as u8); self.emit((plot_helper_addr >> 8) as u8);

        // Check termination: cx==x2 AND cy==y2 → done
        self.emit(0xA5); self.emit(zp+0);   // LDA cx
        self.emit(0xC5); self.emit(zp+2);   // CMP x2
        self.emit(0xD0);                     // BNE dl_step (x differs → keep going)
        let bne_step = self.code.len(); self.emit(0x00);
        self.emit(0xA5); self.emit(zp+1);   // LDA cy
        self.emit(0xC5); self.emit(zp+3);   // CMP y2
        self.emit(0xF0);                     // BEQ dl_done
        let beq_done = self.code.len(); self.emit(0x00);

        // dl_step: compute e2 = err << 1 (16-bit)
        let dl_step = self.current_addr();
        self.patch_bxx(bne_step, dl_step);
        self.emit(0xA5); self.emit(zp+8);   // LDA err_lo
        self.emit(0x0A);                     // ASL A
        self.emit(0x85); self.emit(zp+10);  // STA e2_lo
        self.emit(0xA5); self.emit(zp+9);   // LDA err_hi
        self.emit(0x2A);                     // ROL A
        self.emit(0x85); self.emit(zp+11);  // STA e2_hi

        // X update: if (e2 + |dy|) > 0 → err -= |dy|, cx += sx
        self.emit(0x18);                     // CLC
        self.emit(0xA5); self.emit(zp+10);  // LDA e2_lo
        self.emit(0x65); self.emit(zp+5);   // ADC |dy|
        self.emit(0xAA);                     // TAX (save sum_lo)
        self.emit(0xA5); self.emit(zp+11);  // LDA e2_hi
        self.emit(0x69); self.emit(0x00);   // ADC #0 (carry)
        self.emit(0x10);                     // BPL dl_xchk (sum_hi >= 0)
        let bpl_xchk = self.code.len(); self.emit(0x00);
        self.emit(0x4C);                     // JMP dl_ychk (sum_hi < 0 → skip x update)
        let jmp_ychk = self.code.len(); self.emit(0x00); self.emit(0x00);
        // dl_xchk: sum_hi in 0..127; 0 only if both hi and lo are 0
        let dl_xchk = self.current_addr();
        self.patch_bxx(bpl_xchk, dl_xchk);
        self.emit(0xD0);                     // BNE dl_do_x (hi != 0 → sum > 0)
        let bne_do_x = self.code.len(); self.emit(0x00);
        self.emit(0xE0); self.emit(0x00);   // CPX #0 (check lo)
        self.emit(0xF0);                     // BEQ dl_ychk (hi=0, lo=0 → sum=0 → skip)
        let beq_ychk_zero = self.code.len(); self.emit(0x00);
        // fall through: hi=0, lo>0 → sum > 0 → do x
        let dl_do_x = self.current_addr();
        self.patch_bxx(bne_do_x, dl_do_x);
        self.emit(0x38);                     // SEC
        self.emit(0xA5); self.emit(zp+8);   // LDA err_lo
        self.emit(0xE5); self.emit(zp+5);   // SBC |dy|
        self.emit(0x85); self.emit(zp+8);   // STA err_lo
        self.emit(0xA5); self.emit(zp+9);   // LDA err_hi
        self.emit(0xE9); self.emit(0x00);   // SBC #0 (borrow)
        self.emit(0x85); self.emit(zp+9);   // STA err_hi
        self.emit(0x18);                     // CLC
        self.emit(0xA5); self.emit(zp+0);   // LDA cx
        self.emit(0x65); self.emit(zp+6);   // ADC sx
        self.emit(0x85); self.emit(zp+0);   // STA cx
        // fall through to dl_ychk

        // Y update: if e2 < |dx| → err += |dx|, cy += sy
        let dl_ychk = self.current_addr();
        self.patch_abs(jmp_ychk, dl_ychk);
        self.patch_bxx(beq_ychk_zero, dl_ychk);
        self.emit(0xA5); self.emit(zp+11);  // LDA e2_hi
        self.emit(0x30);                     // BMI dl_do_y (e2 < 0 → < |dx|, always do y)
        let bmi_do_y = self.code.len(); self.emit(0x00);
        self.emit(0xD0);                     // BNE dl_loop (e2_hi > 0 → e2 >= 256 > |dx|)
        let bne_loop1 = self.code.len(); self.emit(0x00);
        // e2_hi == 0: compare |dx| vs e2_lo
        self.emit(0xA5); self.emit(zp+4);   // LDA |dx|
        self.emit(0xC5); self.emit(zp+10);  // CMP e2_lo
        self.emit(0xF0);                     // BEQ dl_loop (|dx|==e2 → skip)
        let beq_loop2 = self.code.len(); self.emit(0x00);
        self.emit(0x90);                     // BCC dl_loop (|dx|<e2 → skip)
        let bcc_loop3 = self.code.len(); self.emit(0x00);
        // fall through: |dx| > e2_lo → e2 < |dx| → do y
        let dl_do_y = self.current_addr();
        self.patch_bxx(bmi_do_y, dl_do_y);
        self.emit(0x18);                     // CLC
        self.emit(0xA5); self.emit(zp+8);   // LDA err_lo
        self.emit(0x65); self.emit(zp+4);   // ADC |dx|
        self.emit(0x85); self.emit(zp+8);   // STA err_lo
        self.emit(0xA5); self.emit(zp+9);   // LDA err_hi
        self.emit(0x69); self.emit(0x00);   // ADC #0
        self.emit(0x85); self.emit(zp+9);   // STA err_hi
        self.emit(0x18);                     // CLC
        self.emit(0xA5); self.emit(zp+1);   // LDA cy
        self.emit(0x65); self.emit(zp+7);   // ADC sy
        self.emit(0x85); self.emit(zp+1);   // STA cy
        self.emit(0x4C); self.emit(dl_loop as u8); self.emit((dl_loop >> 8) as u8); // JMP dl_loop

        // Patch backward branches to dl_loop
        self.patch_bxx(bne_loop1, dl_loop);
        self.patch_bxx(beq_loop2, dl_loop);
        self.patch_bxx(bcc_loop3, dl_loop);

        // dl_done:
        let dl_done = self.current_addr();
        self.patch_bxx(beq_done, dl_done);
        self.emit(0x60);                     // RTS
    }

    /// True if the expression is or contains a string (literal, Str var, or chr$).
    /// Used to decide whether `+` means string concat or numeric add in print.
    fn is_string_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::StringLit(_) => true,
            Expr::ChrStr(_)    => true,
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
            Expr::ChrStr(inner) => {
                // chr$(n): evaluate n into A then output via CHROUT
                let inner = inner.clone();
                self.eval_expr(&inner);
                self.emit(0x20); self.emit16(CHROUT); // JSR CHROUT
            }
            Expr::HexFmt(inner) => {
                // hex(n): print value as 2-digit uppercase hexadecimal
                let inner = inner.clone();
                self.eval_expr(&inner);
                self.emit(0x20);
                let patch = self.code.len();
                self.emit16(0x0000);
                self.hex_helper_patches.push(patch);
            }
            Expr::BinFmt(inner) => {
                // bin(n): print value as 8-bit binary string
                let inner = inner.clone();
                self.eval_expr(&inner);
                self.emit(0x20);
                let patch = self.code.len();
                self.emit16(0x0000);
                self.bin_helper_patches.push(patch);
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
                        // ── 16-bit patterns ─────────────────────────────────────────
                        let handled = match expr {
                            // word_var = number  (already splits lo/hi)
                            Expr::Number(n) => {
                                let n = *n;
                                self.emit(0xA9); self.emit(n as u8);
                                self.emit(0x85); self.emit(zp);
                                self.emit(0xA9); self.emit((n >> 8) as u8);
                                self.emit(0x85); self.emit(zp + 1);
                                true
                            }
                            // word_dst = word_src   (copy both bytes)
                            Expr::Var(src) if matches!(self.var_types.get(src), Some(VarType::Word)) => {
                                if let Some(src_zp) = self.var_addr(src) {
                                    self.emit(0xA5); self.emit(src_zp);     // LDA lo
                                    self.emit(0x85); self.emit(zp);          // STA lo
                                    self.emit(0xA5); self.emit(src_zp + 1); // LDA hi
                                    self.emit(0x85); self.emit(zp + 1);     // STA hi
                                    true
                                } else { false }
                            }
                            // word_dst = word_src + expr  (16-bit add)
                            Expr::BinOp(l, BinOp::Add, r)
                                if matches!(l.as_ref(), Expr::Var(n) if matches!(self.var_types.get(n), Some(VarType::Word))) =>
                            {
                                if let Expr::Var(lname) = l.as_ref() {
                                    if let Some(lzp) = self.var_addr(lname) {
                                        match r.as_ref() {
                                            Expr::Number(n) => {
                                                let n = *n as u16;
                                                self.emit(0x18);                      // CLC
                                                self.emit(0xA5); self.emit(lzp);      // LDA lo_l
                                                self.emit(0x69); self.emit(n as u8);  // ADC #lo_n
                                                self.emit(0x85); self.emit(zp);        // STA lo
                                                self.emit(0xA5); self.emit(lzp + 1);  // LDA hi_l
                                                self.emit(0x69); self.emit((n>>8) as u8); // ADC #hi_n
                                                self.emit(0x85); self.emit(zp + 1);   // STA hi
                                                true
                                            }
                                            Expr::Var(rname)
                                                if matches!(self.var_types.get(rname), Some(VarType::Word)) =>
                                            {
                                                if let Some(rzp) = self.var_addr(rname) {
                                                    self.emit(0x18);                     // CLC
                                                    self.emit(0xA5); self.emit(lzp);     // LDA lo_l
                                                    self.emit(0x65); self.emit(rzp);     // ADC lo_r
                                                    self.emit(0x85); self.emit(zp);       // STA lo
                                                    self.emit(0xA5); self.emit(lzp + 1); // LDA hi_l
                                                    self.emit(0x65); self.emit(rzp + 1); // ADC hi_r
                                                    self.emit(0x85); self.emit(zp + 1);  // STA hi
                                                    true
                                                } else { false }
                                            }
                                            // word + 8-bit-expr: add to lo, propagate carry to hi
                                            other => {
                                                let other = other.clone();
                                                self.eval_expr(&other);
                                                let tmp = self.tmp_zp; self.tmp_zp += 1;
                                                self.emit(0x85); self.emit(tmp);         // STA tmp
                                                self.emit(0x18);                          // CLC
                                                self.emit(0xA5); self.emit(lzp);          // LDA lo_l
                                                self.emit(0x65); self.emit(tmp);          // ADC tmp
                                                self.emit(0x85); self.emit(zp);           // STA lo
                                                self.emit(0xA5); self.emit(lzp + 1);     // LDA hi_l
                                                self.emit(0x69); self.emit(0x00);         // ADC #0 (carry)
                                                self.emit(0x85); self.emit(zp + 1);      // STA hi
                                                true
                                            }
                                        }
                                    } else { false }
                                } else { false }
                            }
                            // word_dst = word_src - expr  (16-bit sub)
                            Expr::BinOp(l, BinOp::Sub, r)
                                if matches!(l.as_ref(), Expr::Var(n) if matches!(self.var_types.get(n), Some(VarType::Word))) =>
                            {
                                if let Expr::Var(lname) = l.as_ref() {
                                    if let Some(lzp) = self.var_addr(lname) {
                                        match r.as_ref() {
                                            Expr::Number(n) => {
                                                let n = *n as u16;
                                                self.emit(0x38);                         // SEC
                                                self.emit(0xA5); self.emit(lzp);         // LDA lo_l
                                                self.emit(0xE9); self.emit(n as u8);     // SBC #lo_n
                                                self.emit(0x85); self.emit(zp);           // STA lo
                                                self.emit(0xA5); self.emit(lzp + 1);     // LDA hi_l
                                                self.emit(0xE9); self.emit((n>>8) as u8); // SBC #hi_n
                                                self.emit(0x85); self.emit(zp + 1);      // STA hi
                                                true
                                            }
                                            Expr::Var(rname)
                                                if matches!(self.var_types.get(rname), Some(VarType::Word)) =>
                                            {
                                                if let Some(rzp) = self.var_addr(rname) {
                                                    self.emit(0x38);                      // SEC
                                                    self.emit(0xA5); self.emit(lzp);      // LDA lo_l
                                                    self.emit(0xE5); self.emit(rzp);      // SBC lo_r
                                                    self.emit(0x85); self.emit(zp);        // STA lo
                                                    self.emit(0xA5); self.emit(lzp + 1);  // LDA hi_l
                                                    self.emit(0xE5); self.emit(rzp + 1);  // SBC hi_r
                                                    self.emit(0x85); self.emit(zp + 1);   // STA hi
                                                    true
                                                } else { false }
                                            }
                                            other => {
                                                let other = other.clone();
                                                self.eval_expr(&other);
                                                let tmp = self.tmp_zp; self.tmp_zp += 1;
                                                self.emit(0x85); self.emit(tmp);          // STA tmp
                                                self.emit(0x38);                           // SEC
                                                self.emit(0xA5); self.emit(lzp);           // LDA lo_l
                                                self.emit(0xE5); self.emit(tmp);           // SBC tmp
                                                self.emit(0x85); self.emit(zp);            // STA lo
                                                self.emit(0xA5); self.emit(lzp + 1);      // LDA hi_l
                                                self.emit(0xE9); self.emit(0x00);          // SBC #0 (borrow)
                                                self.emit(0x85); self.emit(zp + 1);       // STA hi
                                                true
                                            }
                                        }
                                    } else { false }
                                } else { false }
                            }
                            _ => false,
                        };
                        if !handled {
                            // Fallback: 8-bit eval, store lo only
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
                self.emit(0xC9); self.emit(0x00); // CMP #0  (nonzero = true)
                // BNE +3 skip the JMP → execute then_body
                // JMP else/end (absolute, no branch distance limit)
                self.emit(0xD0); self.emit(0x03); // BNE +3
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
            Stmt::Cls { fast } => {
                if *fast {
                    self.emit_cls_fast();
                } else {
                    // KERNAL clear screen
                    self.emit(0x20); self.emit16(0xE544); // JSR $E544
                }
            }
            Stmt::Graphics { on, multi } => {
                if *on {
                    self.emit_graphics_on(*multi);
                } else {
                    self.emit_graphics_off();
                }
            }
            Stmt::Display { on } => {
                // Set or clear DEN (bit4) in $D011.
                // display on  → LDA $D011; ORA #$10; STA $D011
                // display off → LDA $D011; AND #$EF; STA $D011
                self.emit(0xAD); self.emit16(0xD011); // LDA $D011
                if *on {
                    self.emit(0x09); self.emit(0x10);  // ORA #$10  (set DEN)
                } else {
                    self.emit(0x29); self.emit(0xEF);  // AND #$EF  (clear DEN)
                }
                self.emit(0x8D); self.emit16(0xD011); // STA $D011
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
            Stmt::Plot(x_expr, y_expr) => {
                if let Some(zp) = self.plot_zp {
                    let x = x_expr.clone();
                    let y = y_expr.clone();
                    // ZP layout: zp+0=X_lo, zp+1=X_hi, zp+2=Y

                    // Store Y (always 8-bit)
                    self.eval_expr(&y);
                    self.emit(0x85); self.emit(zp + 2);

                    // Store X as 16-bit (supports 0-319 full bitmap width)
                    match &x {
                        Expr::Number(n) => {
                            let xu = *n as u16;
                            self.emit(0xA9); self.emit(xu as u8);        // LDA #lo
                            self.emit(0x85); self.emit(zp);
                            self.emit(0xA9); self.emit((xu >> 8) as u8); // LDA #hi
                            self.emit(0x85); self.emit(zp + 1);
                        }
                        Expr::Var(name) => {
                            let name = name.clone();
                            let is_word = matches!(self.var_types.get(&name), Some(VarType::Word));
                            if is_word {
                                if let Some(vz) = self.var_addr(&name) {
                                    self.emit(0xA5); self.emit(vz);       // LDA var_lo
                                    self.emit(0x85); self.emit(zp);
                                    self.emit(0xA5); self.emit(vz + 1);  // LDA var_hi
                                    self.emit(0x85); self.emit(zp + 1);
                                }
                            } else {
                                self.eval_expr(&Expr::Var(name));
                                self.emit(0x85); self.emit(zp);           // STA X_lo
                                self.emit(0xA9); self.emit(0x00);
                                self.emit(0x85); self.emit(zp + 1);       // STA X_hi = 0
                            }
                        }
                        _ => {
                            self.eval_expr(&x);
                            self.emit(0x85); self.emit(zp);               // STA X_lo
                            self.emit(0xA9); self.emit(0x00);
                            self.emit(0x85); self.emit(zp + 1);           // STA X_hi = 0
                        }
                    }
                    self.emit(0x20);
                    let patch = self.code.len();
                    self.emit16(0x0000);
                    self.plot_patches.push(patch);
                }
            }
            Stmt::Line { x1, y1, x2, y2 } => {
                if let Some(zp) = self.line_zp {
                    let x1 = x1.clone(); let y1 = y1.clone();
                    let x2 = x2.clone(); let y2 = y2.clone();
                    // Load X1,Y1 → cx,cy; X2,Y2 → x2,y2 in ZP block
                    self.eval_expr(&x1);
                    self.emit(0x85); self.emit(zp + 0); // STA cx
                    self.eval_expr(&y1);
                    self.emit(0x85); self.emit(zp + 1); // STA cy
                    self.eval_expr(&x2);
                    self.emit(0x85); self.emit(zp + 2); // STA x2
                    self.eval_expr(&y2);
                    self.emit(0x85); self.emit(zp + 3); // STA y2
                    // JSR drawline helper (address patched after emit)
                    self.emit(0x20);
                    let patch = self.code.len();
                    self.emit16(0x0000);
                    self.line_patches.push(patch);
                }
            }
            Stmt::Gcls => {
                self.emit_gcls();
            }
            Stmt::Bye => {
                self.emit(0x20); self.emit16(0xE544); // JSR $E544 — KERNAL CLS
                self.emit(0xA9); self.emit(0x00);     // LDA #$00
                self.emit(0x85); self.emit(0xC6);     // STA $C6 — clear keyboard buffer length
                // SEI/CLI bracket the $91 clear to prevent IRQ race with STOP key
                self.emit(0x78);                      // SEI
                self.emit(0xA9); self.emit(0xFF);     // LDA #$FF
                self.emit(0x85); self.emit(0x91);     // STA $91 — clear stop-key flag
                self.emit(0x58);                      // CLI
                // Jump into BASIC warm start so we avoid BREAK-line handling path.
                self.emit(0x4C); self.emit16(0xA659); // JMP $A659
            }
            Stmt::Incbin(path) => {
                match std::fs::read(path) {
                    Ok(bytes) => { for b in bytes { self.emit(b); } }
                    Err(e) => eprintln!("incbin: cannot read '{}': {}", path, e),
                }
            }
            Stmt::Data(_) => {
                // Data bytes were collected in pre_scan and will be emitted as a block
                // after all executable code. Nothing to emit here.
            }
            Stmt::Read(varname) => {
                let var_zp = self.alloc_var(varname);
                if let Some(zp) = self.data_zp {
                    self.emit(0xA0); self.emit(0x00);   // LDY #0
                    self.emit(0xB1); self.emit(zp);     // LDA (data_ptr),Y
                    self.emit(0x85); self.emit(var_zp); // STA var_zp
                    // Increment data_ptr (16-bit: INC lo; BNE skip; INC hi)
                    self.emit(0xE6); self.emit(zp);     // INC data_ptr_lo
                    self.emit(0xD0); self.emit(0x02);   // BNE +2 (skip INC hi)
                    self.emit(0xE6); self.emit(zp + 1); // INC data_ptr_hi
                }
            }
            Stmt::Wait { raster_target, value } => {
                let value = value.clone();
                if *raster_target {
                    // wait raster N — spin until $D012 == N
                    let tmp = self.tmp_zp; self.tmp_zp += 1;
                    self.eval_expr(&value);
                    self.emit(0x85); self.emit(tmp); // STA tmp (target line)
                    let loop_top = self.code.len();
                    self.emit(0xAD); self.emit(0x12); self.emit(0xD0); // LDA $D012
                    self.emit(0xC5); self.emit(tmp); // CMP tmp
                    let bne_pos = self.code.len();
                    self.emit(0xD0); self.emit(0x00); // BNE loop_top (patched)
                    self.patch_bxx(bne_pos + 1, self.load_addr + loop_top as u16);
                } else {
                    // wait N — count N raster-line transitions via $D012 polling
                    let fc   = self.tmp_zp; self.tmp_zp += 1;
                    let prev = self.tmp_zp; self.tmp_zp += 1;
                    self.eval_expr(&value);
                    let beq_done = self.code.len();
                    self.emit(0xF0); self.emit(0x00); // BEQ done (N=0, skip)
                    self.emit(0x85); self.emit(fc);   // STA fc
                    let outer_top = self.code.len();
                    self.emit(0xAD); self.emit(0x12); self.emit(0xD0); // LDA $D012
                    self.emit(0x85); self.emit(prev); // STA prev
                    let inner_top = self.code.len();
                    self.emit(0xAD); self.emit(0x12); self.emit(0xD0); // LDA $D012
                    self.emit(0xC5); self.emit(prev); // CMP prev
                    let beq_inner = self.code.len();
                    self.emit(0xF0); self.emit(0x00); // BEQ inner_top (patched)
                    self.emit(0xC6); self.emit(fc);   // DEC fc
                    let bne_outer = self.code.len();
                    self.emit(0xD0); self.emit(0x00); // BNE outer_top (patched)
                    let done_addr = self.current_addr();
                    self.patch_bxx(beq_done  + 1, done_addr);
                    self.patch_bxx(beq_inner + 1, self.load_addr + inner_top as u16);
                    self.patch_bxx(bne_outer + 1, self.load_addr + outer_top as u16);
                }
            }
            Stmt::Sound { channel, freq, duration } => {
                let channel  = channel.clone();
                let freq     = freq.clone();
                let duration = duration.clone();
                // Channel must be a compile-time constant 0, 1, or 2.
                let ch = match &channel {
                    Expr::Number(n) => *n as u16,
                    _ => panic!("sound: channel must be a constant 0, 1, or 2"),
                };
                assert!(ch <= 2, "sound: channel must be 0, 1, or 2");
                let base = 0xD400u16 + ch * 7;
                // $D418 master volume = $0F (all voices audible)
                self.emit(0xA9); self.emit(0x0F);
                self.emit(0x8D); self.emit16(0xD418);
                // ADSR: attack/decay = $09 (fast attack, medium decay),
                //       sustain/release = $F0 (full sustain, fast release)
                self.emit(0xA9); self.emit(0x09);
                self.emit(0x8D); self.emit16(base + 5);
                self.emit(0xA9); self.emit(0xF0);
                self.emit(0x8D); self.emit16(base + 6);
                // Frequency (16-bit)
                match &freq {
                    Expr::Number(n) => {
                        let n = *n as u16;
                        self.emit(0xA9); self.emit(n as u8);
                        self.emit(0x8D); self.emit16(base);       // freq lo
                        self.emit(0xA9); self.emit((n >> 8) as u8);
                        self.emit(0x8D); self.emit16(base + 1);   // freq hi
                    }
                    Expr::Var(name) => {
                        let name = name.clone();
                        if matches!(self.var_types.get(&name), Some(VarType::Word)) {
                            if let Some(zp) = self.var_addr(&name) {
                                self.emit(0xA5); self.emit(zp);
                                self.emit(0x8D); self.emit16(base);
                                self.emit(0xA5); self.emit(zp + 1);
                                self.emit(0x8D); self.emit16(base + 1);
                            }
                        } else {
                            // 8-bit var: lo = var, hi = 0
                            let fe = Expr::Var(name);
                            self.eval_expr(&fe);
                            self.emit(0x8D); self.emit16(base);
                            self.emit(0xA9); self.emit(0x00);
                            self.emit(0x8D); self.emit16(base + 1);
                        }
                    }
                    other => {
                        let other = other.clone();
                        self.eval_expr(&other);
                        self.emit(0x8D); self.emit16(base);
                        self.emit(0xA9); self.emit(0x00);
                        self.emit(0x8D); self.emit16(base + 1);
                    }
                }
                // GATE on: sawtooth waveform + GATE = $11
                self.emit(0xA9); self.emit(0x11);
                self.emit(0x8D); self.emit16(base + 4);
                // Wait `duration` PAL frames (count raster line 0 crossings)
                let fc = self.tmp_zp; self.tmp_zp += 1;
                self.eval_expr(&duration);
                let beq_skip = self.code.len();
                self.emit(0xF0); self.emit(0x00);  // BEQ skip_wait (patched)
                self.emit(0x85); self.emit(fc);    // STA fc
                // wait_not_zero: wait while $D012 == 0 to avoid false-positive
                let wait_nz = self.code.len();
                self.emit(0xAD); self.emit(0x12); self.emit(0xD0); // LDA $D012
                let beq_nz = self.code.len();
                self.emit(0xF0); self.emit(0x00);  // BEQ wait_not_zero (patched)
                // wait_zero: wait until $D012 == 0 (raster line 0 = new frame)
                let wait_z = self.code.len();
                self.emit(0xAD); self.emit(0x12); self.emit(0xD0); // LDA $D012
                let bne_z = self.code.len();
                self.emit(0xD0); self.emit(0x00);  // BNE wait_zero (patched)
                self.emit(0xC6); self.emit(fc);    // DEC fc
                let bne_fc = self.code.len();
                self.emit(0xD0); self.emit(0x00);  // BNE wait_not_zero (patched)
                // skip_wait: GATE off — release note
                let skip_addr = self.current_addr();
                self.patch_bxx(beq_skip + 1, skip_addr);
                self.patch_bxx(beq_nz,       self.load_addr + wait_nz as u16);
                self.patch_bxx(bne_z,        self.load_addr + wait_z  as u16);
                self.patch_bxx(bne_fc + 1,   self.load_addr + wait_nz as u16);
                // GATE off: sawtooth, no gate = $10
                self.emit(0xA9); self.emit(0x10);
                self.emit(0x8D); self.emit16(base + 4);
            }
            Stmt::Sprite { id, x, y, data_addr } => {
                // VIC-II sprite registers:
                //   $D000+id*2 = X low,  $D001+id*2 = Y
                //   $D010 bit id = X bit 8 (MSB) — read-modify-write
                //   $07F8+id = sprite data pointer (screen_ram+$3F8+id; value = data_addr >> 6)
                let sprite_id = match id {
                    Expr::Number(n) => *n as u16,
                    _ => panic!("sprite: id must be a compile-time constant 0-7"),
                };
                assert!(sprite_id <= 7, "sprite: id must be 0-7");
                let x_reg   = 0xD000u16 + sprite_id * 2;
                let y_reg   = 0xD001u16 + sprite_id * 2;
                let msb_bit = 1u8 << (sprite_id as u8);
                let msb_clr = !msb_bit;

                // Set Y (8-bit, no MSB)
                let y = y.clone();
                self.eval_expr(&y);
                self.emit(0x8D); self.emit16(y_reg);

                // Set X — different code paths depending on expression type
                match x {
                    Expr::Number(n) => {
                        let xv = *n as u16;
                        self.emit(0xA9); self.emit((xv & 0xFF) as u8); // LDA #lo
                        self.emit(0x8D); self.emit16(x_reg);            // STA $D000+id*2
                        self.emit(0xAD); self.emit16(0xD010);           // LDA $D010
                        if xv >= 256 {
                            self.emit(0x09); self.emit(msb_bit);        // ORA #bit (set MSB)
                        } else {
                            self.emit(0x29); self.emit(msb_clr);        // AND #~bit (clear MSB)
                        }
                        self.emit(0x8D); self.emit16(0xD010);           // STA $D010
                    }
                    Expr::Var(name) if self.var_types.get(name) == Some(&VarType::Word) => {
                        // Word var: lo byte → X register, hi byte → $D010 bit
                        let zp = *self.vars.get(name).expect("sprite: word var not found");
                        self.emit(0xA5); self.emit(zp);                 // LDA zp_lo
                        self.emit(0x8D); self.emit16(x_reg);            // STA $D000+id*2
                        // Runtime MSB logic
                        self.emit(0xA5); self.emit(zp + 1);             // LDA zp_hi
                        self.emit(0xF0);
                        let beq_pos = self.code.len(); self.emit(0);    // BEQ clear_msb (patched)
                        // hi != 0 → set MSB bit
                        self.emit(0xAD); self.emit16(0xD010);           // LDA $D010
                        self.emit(0x09); self.emit(msb_bit);            // ORA #bit
                        self.emit(0x4C);
                        let jmp_pos = self.code.len(); self.emit16(0);  // JMP done (patched)
                        // clear_msb:
                        let clear_addr = self.current_addr();
                        self.patch_bxx(beq_pos, clear_addr);
                        self.emit(0xAD); self.emit16(0xD010);           // LDA $D010
                        self.emit(0x29); self.emit(msb_clr);            // AND #~bit
                        // done:
                        let done_addr = self.current_addr();
                        self.patch_abs(jmp_pos, done_addr);
                        self.emit(0x8D); self.emit16(0xD010);           // STA $D010
                    }
                    other => {
                        // 8-bit expression: X always < 256 → clear MSB bit
                        let other = other.clone();
                        self.eval_expr(&other);
                        self.emit(0x8D); self.emit16(x_reg);            // STA $D000+id*2
                        self.emit(0xAD); self.emit16(0xD010);           // LDA $D010
                        self.emit(0x29); self.emit(msb_clr);            // AND #~bit
                        self.emit(0x8D); self.emit16(0xD010);           // STA $D010
                    }
                }

                // Set data pointer (optional): ptr = data_addr >> 6 stored at $07F8+id
                let ptr_reg = 0x07F8u16 + sprite_id;
                if let Some(addr_expr) = data_addr {
                    match addr_expr {
                        Expr::Number(n) => {
                            let ptr = (*n as u16) >> 6;
                            self.emit(0xA9); self.emit(ptr as u8);      // LDA #(addr>>6)
                            self.emit(0x8D); self.emit16(ptr_reg);      // STA $07F8+id
                        }
                        Expr::Var(name) if self.var_types.get(name) == Some(&VarType::Word) => {
                            // ptr = (hi<<2) | (lo>>6)
                            let zp = *self.vars.get(name).expect("sprite: word data_addr var");
                            let tmp = self.tmp_zp; self.tmp_zp += 1;
                            self.emit(0xA5); self.emit(zp + 1);         // LDA hi
                            self.emit(0x0A); self.emit(0x0A);           // ASL A; ASL A (hi<<2)
                            self.emit(0x85); self.emit(tmp);            // STA tmp
                            self.emit(0xA5); self.emit(zp);             // LDA lo
                            for _ in 0..6 { self.emit(0x4A); }         // LSR A ×6 (lo>>6)
                            self.emit(0x05); self.emit(tmp);            // ORA tmp
                            self.emit(0x8D); self.emit16(ptr_reg);      // STA $07F8+id
                        }
                        other => {
                            // Other 8-bit expr: treat as already a pointer value (addr>>6)
                            let other = other.clone();
                            self.eval_expr(&other);
                            self.emit(0x8D); self.emit16(ptr_reg);      // STA $07F8+id
                        }
                    }
                }
            }
            Stmt::SpriteOn { id } => {
                // $D015: sprite enable register — set bit for this sprite
                let sprite_id = match id {
                    Expr::Number(n) => *n as u16,
                    _ => panic!("sprite_on: id must be a compile-time constant 0-7"),
                };
                let bit = 1u8 << (sprite_id as u8);
                self.emit(0xAD); self.emit16(0xD015); // LDA $D015
                self.emit(0x09); self.emit(bit);      // ORA #bit
                self.emit(0x8D); self.emit16(0xD015); // STA $D015
            }
            Stmt::SpriteOff { id } => {
                // $D015: sprite enable register — clear bit for this sprite
                let sprite_id = match id {
                    Expr::Number(n) => *n as u16,
                    _ => panic!("sprite_off: id must be a compile-time constant 0-7"),
                };
                let bit = !(1u8 << (sprite_id as u8));
                self.emit(0xAD); self.emit16(0xD015); // LDA $D015
                self.emit(0x29); self.emit(bit);      // AND #~bit
                self.emit(0x8D); self.emit16(0xD015); // STA $D015
            }
            Stmt::SpriteColor { id, color } => {
                // $D027+id: sprite color register
                let sprite_id = match id {
                    Expr::Number(n) => *n as u16,
                    _ => panic!("sprite_color: id must be a compile-time constant 0-7"),
                };
                let color = color.clone();
                self.eval_expr(&color);
                self.emit(0x8D); self.emit16(0xD027 + sprite_id); // STA $D027+id
            }
            Stmt::SpriteMulticolor { id, on } => {
                // $D01C: sprite multicolor enable register
                let sprite_id = match id {
                    Expr::Number(n) => *n as u16,
                    _ => panic!("sprite_multicolor: id must be a compile-time constant 0-7"),
                };
                let on = *on;
                let bit = 1u8 << (sprite_id as u8);
                self.emit(0xAD); self.emit16(0xD01C); // LDA $D01C
                if on {
                    self.emit(0x09); self.emit(bit);  // ORA #bit
                } else {
                    self.emit(0x29); self.emit(!bit); // AND #~bit
                }
                self.emit(0x8D); self.emit16(0xD01C); // STA $D01C
            }
            Stmt::SpriteDef { id, bytes } => {
                let id = *id;
                // After JMP (3 bytes), find the next 64-byte-aligned address.
                let after_jmp  = self.current_addr() + 3;
                let data_addr  = ((after_jmp as u32 + 63) / 64 * 64) as u16;
                let padding    = (data_addr - after_jmp) as usize;
                let page       = (data_addr >> 6) as u8;
                let ptr_reg    = 0x07F8u16 + id as u16;

                // JMP past data block (patched below)
                self.emit(0x4C);
                let jmp_lo = self.code.len();
                self.emit16(0x0000);

                // Zero-padding to reach 64-byte boundary
                for _ in 0..padding { self.emit(0x00); }

                // 63 bytes of sprite data (zero-padded if fewer supplied)
                let mut data = bytes.clone();
                data.resize(63, 0);
                for b in &data { self.emit(*b); }
                self.emit(0x00); // 1 filler byte — completes the 64-byte block

                // Patch JMP to instruction immediately after the data block
                let past_data = self.current_addr();
                self.patch_abs(jmp_lo, past_data);

                // Runtime: register sprite data pointer → $07F8+id
                self.emit(0xA9); self.emit(page);               // LDA #page
                self.emit(0x8D); self.emit(ptr_reg as u8); self.emit((ptr_reg >> 8) as u8); // STA
            }
            Stmt::Reu { op, c64_addr, reu_bank, reu_addr, length } => {
                // $DF02/$DF03 = C64 address, $DF04/$DF05 = REU offset,
                // $DF06 = bank, $DF07/$DF08 = length, $DF01 = command (execute).
                let c64_addr = c64_addr.clone();
                let reu_bank = reu_bank.clone();
                let reu_addr = reu_addr.clone();
                let length   = length.clone();
                let op = op.clone();
                self.emit_addr_to_reu_reg(&c64_addr, 0xDF02, 0xDF03);
                self.emit_addr_to_reu_reg(&reu_addr,  0xDF04, 0xDF05);
                self.eval_expr(&reu_bank);
                self.emit(0x8D); self.emit16(0xDF06); // STA $DF06 — REU bank
                self.emit_addr_to_reu_reg(&length,    0xDF07, 0xDF08);
                let cmd: u8 = match op {
                    ReuOp::Stash => 0xB0, // execute + stash (C64→REU)
                    ReuOp::Fetch => 0xB1, // execute + fetch (REU→C64)
                    ReuOp::Swap  => 0xB2, // execute + swap
                };
                self.emit(0xA9); self.emit(cmd);      // LDA #cmd
                self.emit(0x8D); self.emit16(0xDF01); // STA $DF01 — trigger DMA
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
        // Pre-scan: allocate ZP for sub params, register arrays, data pointer
        self.pre_scan(stmts);

        // Emit data pointer init (forward-patched later when data block address is known)
        if let Some(zp) = self.data_zp {
            self.emit(0xA9);
            self.data_ptr_lo_patch = Some(self.code.len());
            self.emit(0x00);             // lo placeholder
            self.emit(0x85); self.emit(zp);       // STA data_ptr_lo
            self.emit(0xA9);
            self.data_ptr_hi_patch = Some(self.code.len());
            self.emit(0x00);             // hi placeholder
            self.emit(0x85); self.emit(zp + 1);   // STA data_ptr_hi
        }

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

        // Emit plot helper subroutine (once) — needed for direct plot AND for line command
        let mut plot_helper_addr: Option<u16> = None;
        if !self.plot_patches.is_empty() || !self.line_patches.is_empty() {
            let addr = self.current_addr();
            plot_helper_addr = Some(addr);
            self.emit_plot_helper();
            for &pos in &self.plot_patches.clone() {
                self.code[pos]     = addr as u8;
                self.code[pos + 1] = (addr >> 8) as u8;
            }
        }

        // Emit drawline (Bresenham) helper — calls plot helper
        if !self.line_patches.is_empty() {
            if let Some(plot_addr) = plot_helper_addr {
                let dl_addr = self.current_addr();
                self.emit_drawline_helper(plot_addr);
                for &pos in &self.line_patches.clone() {
                    self.code[pos]     = dl_addr as u8;
                    self.code[pos + 1] = (dl_addr >> 8) as u8;
                }
            }
        }

        // Emit data block and patch init code
        if !self.data_bytes.is_empty() {
            let data_addr = self.current_addr();
            if let Some(pos) = self.data_ptr_lo_patch {
                self.code[pos] = data_addr as u8;
            }
            if let Some(pos) = self.data_ptr_hi_patch {
                self.code[pos] = (data_addr >> 8) as u8;
            }
            for &b in &self.data_bytes.clone() {
                self.emit(b);
            }
        }

        // Emit sin/cos lookup table and patch all LDA abs,X references
        if !self.sin_table_patches.is_empty() {
            let table_addr = self.current_addr();
            self.sin_table_addr = Some(table_addr);
            for b in Self::sin_table() {
                self.emit(b);
            }
            for &pos in &self.sin_table_patches.clone() {
                self.code[pos]     = table_addr as u8;
                self.code[pos + 1] = (table_addr >> 8) as u8;
            }
        }

        // Emit print_hex helper and patch all JSR targets
        if !self.hex_helper_patches.is_empty() {
            let hex_addr = self.emit_print_hex_helper();
            for &pos in &self.hex_helper_patches.clone() {
                self.code[pos]     = hex_addr as u8;
                self.code[pos + 1] = (hex_addr >> 8) as u8;
            }
        }

        // Emit print_bin helper and patch all JSR targets
        if !self.bin_helper_patches.is_empty() {
            let bin_addr = self.emit_print_bin_helper();
            for &pos in &self.bin_helper_patches.clone() {
                self.code[pos]     = bin_addr as u8;
                self.code[pos + 1] = (bin_addr >> 8) as u8;
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

    pub fn memory_map(&self) -> MemoryMap {
        let mut variables: Vec<VarEntry> = self
            .vars
            .iter()
            .map(|(name, &zp_addr)| {
                let type_str = match self.var_types.get(name) {
                    Some(VarType::Word) => "word",
                    Some(VarType::Str) => "string",
                    Some(VarType::Array) => "array",
                    _ => "int",
                }
                .to_string();

                VarEntry {
                    name: name.clone(),
                    zp_addr,
                    type_str,
                }
            })
            .collect();
        variables.sort_by_key(|v| v.zp_addr);

        let mut subroutines: Vec<SubEntry> = self
            .subs
            .iter()
            .map(|(name, &addr)| SubEntry {
                name: name.clone(),
                addr,
            })
            .collect();
        subroutines.sort_by_key(|s| s.addr);

        let mut arrays: Vec<ArrayEntry> = self
            .arrays
            .iter()
            .map(|(name, &base_addr)| ArrayEntry {
                name: name.clone(),
                base_addr,
                size: self.array_sizes.get(name).copied().unwrap_or(0),
            })
            .collect();
        arrays.sort_by_key(|a| a.base_addr);

        MemoryMap {
            load_addr: self.load_addr,
            code_size: self.code.len(),
            variables,
            subroutines,
            arrays,
            plot_zp: self.plot_zp,
            line_zp: self.line_zp,
            sin_table_addr: self.sin_table_addr,
            data_zp: self.data_zp,
            code_bytes: self.code.clone(),
        }
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
