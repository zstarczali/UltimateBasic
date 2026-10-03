//! Runtime of `map view x, y`: pixel-smooth, tear-free scrolling of the loaded map.
//!
//! The 40x25 view uses two screen buffers in VIC bank 0, `$0400` and `$3C00`
//! (the ROM charset stays at `$1000`), XSCROLL/YSCROLL in `$D016`/`$D011` with
//! 38 columns / 24 rows for the fine part, and swaps the buffers at char steps:
//!
//! * every call waits for the frame boundary (raster line 250), applies what the
//!   previous call decided (fine scroll or buffer swap), then takes the new
//!   position;
//! * the position is predicted two frames ahead; when it will enter a new char
//!   cell, the hidden buffer is built for that cell in two halves (13 + 12 rows,
//!   one per call), so the swap itself never has to wait;
//! * color RAM cannot be double-buffered: rows 0..12 are copied in the last old
//!   frame behind the raster beam (from line 70), rows 13..24 right after the swap
//!   ahead of the beam. Both halves run with interrupts masked;
//! * the sprite pointers `$07F8-$07FF` are copied to `$3FF8-$3FFF` every call, so
//!   `sprite` / `sprdef` / `sprite_frame` keep working in both buffers.

use std::collections::HashMap;

/// Second screen buffer (VIC bank 0, `$D018` screen bits = `$F0`).
pub const BUFFER_B: u16 = 0x3C00;

pub struct MapViewParams {
    pub chars: u16,
    pub colors: Option<u16>,
    pub width: u8,
    pub height: u8,
    /// Zero-page bytes holding the requested x (lo, hi) and y (lo, hi).
    pub param_zp: u8,
}

// Offsets in the variable block.
const CAMX: u16 = 0; // 2
const CAMY: u16 = 2; // 2
const REQX: u16 = 4; // 2
const REQY: u16 = 6; // 2
const VX: u16 = 8;
const VY: u16 = 9;
const WX: u16 = 10; // 2
const WY: u16 = 12; // 2
const NCX: u16 = 14;
const NCY: u16 = 15;
const SHX: u16 = 16;
const SHY: u16 = 17;
const TX: u16 = 18;
const TY: u16 = 19;
const BSTATE: u16 = 20;
const DOSWAP: u16 = 21;
const SWAPPED: u16 = 22;
const TLO: u16 = 23;
const THI: u16 = 24;
const TV: u16 = 25;
const MAXL: u16 = 26;
const MAXH: u16 = 27;
const NEXTY: u16 = 28;
const VIEWX: u16 = 29;
const BROW: u16 = 30;
const BEND: u16 = 31;
const BACKHI: u16 = 32;
const BACKD018: u16 = 33;
const CCROW: u16 = 34;
const CCEND: u16 = 35;
const DSTL: u16 = 36;
const DSTH: u16 = 37;
const INIT: u16 = 38;
const CS: u16 = 39;
const VAR_SIZE: usize = 48;

enum Fix {
    Abs(usize, String, u16),
    Rel(usize, String),
}

/// Tiny two-pass assembler for the helper: labels, label+offset operands and
/// range-checked branches.
struct Asm {
    base: u16,
    code: Vec<u8>,
    labels: HashMap<String, u16>,
    fixes: Vec<Fix>,
}

impl Asm {
    fn new(base: u16) -> Self {
        Self { base, code: vec![], labels: HashMap::new(), fixes: vec![] }
    }
    fn here(&self) -> u16 {
        self.base.wrapping_add(self.code.len() as u16)
    }
    fn label(&mut self, name: &str) {
        let addr = self.here();
        self.labels.insert(name.to_string(), addr);
    }
    fn op(&mut self, opcode: u8) {
        self.code.push(opcode);
    }
    fn imm(&mut self, opcode: u8, value: u8) {
        self.code.push(opcode);
        self.code.push(value);
    }
    fn abs(&mut self, opcode: u8, addr: u16) {
        self.code.push(opcode);
        self.code.push(addr as u8);
        self.code.push((addr >> 8) as u8);
    }
    /// Absolute operand `label + offset` (resolved in pass 2).
    fn lab(&mut self, opcode: u8, name: &str, offset: u16) {
        self.code.push(opcode);
        self.fixes.push(Fix::Abs(self.code.len(), name.to_string(), offset));
        self.code.push(0);
        self.code.push(0);
    }
    /// Variable in the variable block.
    fn var(&mut self, opcode: u8, offset: u16) {
        self.lab(opcode, "vars", offset);
    }
    fn branch(&mut self, opcode: u8, name: &str) {
        self.code.push(opcode);
        self.fixes.push(Fix::Rel(self.code.len(), name.to_string()));
        self.code.push(0);
    }
    fn bytes(&mut self, data: &[u8]) {
        self.code.extend_from_slice(data);
    }
    fn finish(mut self) -> Result<Vec<u8>, String> {
        for fix in &self.fixes {
            match fix {
                Fix::Abs(pos, name, offset) => {
                    let target = *self
                        .labels
                        .get(name)
                        .ok_or_else(|| format!("map view helper: unknown label {name}"))?;
                    let value = target.wrapping_add(*offset);
                    self.code[*pos] = value as u8;
                    self.code[*pos + 1] = (value >> 8) as u8;
                }
                Fix::Rel(pos, name) => {
                    let target = *self
                        .labels
                        .get(name)
                        .ok_or_else(|| format!("map view helper: unknown label {name}"))?;
                    let pc = self.base as i32 + *pos as i32 + 1;
                    let rel = target as i32 - pc;
                    if !(-128..=127).contains(&rel) {
                        return Err(format!("map view helper: branch to {name} out of range ({rel})"));
                    }
                    self.code[*pos] = rel as i8 as u8;
                }
            }
        }
        Ok(self.code)
    }
}

// 6502 opcodes used by the helper.
const LDA_IMM: u8 = 0xA9;
const LDA_ABS: u8 = 0xAD;
const LDA_ABX: u8 = 0xBD;
const LDA_ABY: u8 = 0xB9;
const LDA_ZP: u8 = 0xA5;
const STA_ABS: u8 = 0x8D;
const STA_ABX: u8 = 0x9D;
const STA_ABY: u8 = 0x99;
const LDY_IMM: u8 = 0xA0;
const LDX_IMM: u8 = 0xA2;
const CMP_IMM: u8 = 0xC9;
const CMP_ABS: u8 = 0xCD;
const ADC_IMM: u8 = 0x69;
const ADC_ABS: u8 = 0x6D;
const SBC_ABS: u8 = 0xED;
const AND_IMM: u8 = 0x29;
const ORA_IMM: u8 = 0x09;
const ORA_ABS: u8 = 0x0D;
const EOR_IMM: u8 = 0x49;
const INC_ABS: u8 = 0xEE;
const LSR_ABS: u8 = 0x4E;
const ROR_A: u8 = 0x6A;
const DEX: u8 = 0xCA;
const DEY: u8 = 0x88;
const TAX: u8 = 0xAA;
const TAY: u8 = 0xA8;
const TYA: u8 = 0x98;
const CLC: u8 = 0x18;
const SEC: u8 = 0x38;
const PHP: u8 = 0x08;
const PLP: u8 = 0x28;
const SEI: u8 = 0x78;
const RTS: u8 = 0x60;
const JSR: u8 = 0x20;
const JMP: u8 = 0x4C;
const BEQ: u8 = 0xF0;
const BNE: u8 = 0xD0;
const BCC: u8 = 0x90;
const BCS: u8 = 0xB0;
const BPL: u8 = 0x10;
const BMI: u8 = 0x30;

/// Generates the helper at `base`; the entry point is `base`.
pub fn helper(base: u16, p: &MapViewParams) -> Result<Vec<u8>, String> {
    if p.width < 40 || p.height < 25 {
        return Err(format!(
            "map view needs a map of at least 40x25 cells (the loaded map is {}x{})",
            p.width, p.height
        ));
    }
    let max_x = (p.width as u16 - 40) * 8;
    let max_y = (p.height as u16 - 25) * 8;
    let z = p.param_zp;
    let mut a = Asm::new(base);

    // ── entry: take the requested position, clamp it ────────────────────────
    for (k, off) in [(0u8, REQX), (1, REQX + 1), (2, REQY), (3, REQY + 1)] {
        a.imm(LDA_ZP, z + k);
        a.var(STA_ABS, off);
    }
    for (req, max, tag) in [(REQX, max_x, "x"), (REQY, max_y, "y")] {
        // negative (bit 15) → 0, above max → max
        let ok = format!("clamp_ok_{tag}");
        let set = format!("clamp_set_{tag}");
        let zero = format!("clamp_zero_{tag}");
        a.var(LDA_ABS, req + 1);
        a.branch(BMI, &zero);
        a.imm(CMP_IMM, (max >> 8) as u8);
        a.branch(BCC, &ok);
        a.branch(BNE, &set);
        a.var(LDA_ABS, req);
        a.imm(CMP_IMM, max as u8);
        a.branch(BCC, &ok);
        a.label(&set);
        a.imm(LDA_IMM, max as u8);
        a.var(STA_ABS, req);
        a.imm(LDA_IMM, (max >> 8) as u8);
        a.var(STA_ABS, req + 1);
        a.lab(JMP, &ok, 0);
        a.label(&zero);
        a.imm(LDA_IMM, 0);
        a.var(STA_ABS, req);
        a.var(STA_ABS, req + 1);
        a.label(&ok);
    }
    a.var(LDA_ABS, INIT);
    a.branch(BNE, "running");
    a.lab(JMP, "init", 0);

    // ── apply the previous decision at the frame boundary ───────────────────
    a.label("running");
    a.var(LDA_ABS, DOSWAP);
    a.branch(BEQ, "wait_out");
    // color rows 0..12 behind the beam: start between raster lines 70 and 89
    a.label("wait70");
    a.abs(LDA_ABS, 0xD011);
    a.branch(BMI, "wait70");
    a.abs(LDA_ABS, 0xD012);
    a.imm(CMP_IMM, 70);
    a.branch(BCC, "wait70");
    a.imm(CMP_IMM, 90);
    a.branch(BCS, "wait70");
    a.op(PHP);
    a.op(SEI);
    a.imm(LDA_IMM, 0);
    a.var(STA_ABS, CCROW);
    a.imm(LDA_IMM, 13);
    a.var(STA_ABS, CCEND);
    a.lab(JSR, "copy_colors", 0);
    a.op(PLP);
    a.label("wait_out");
    a.lab(JSR, "in_window", 0);
    a.branch(BCS, "wait_out");
    a.label("wait_in");
    a.lab(JSR, "in_window", 0);
    a.branch(BCC, "wait_in");
    a.var(LDA_ABS, DOSWAP);
    a.var(STA_ABS, SWAPPED);
    a.imm(LDA_IMM, 0);
    a.var(STA_ABS, DOSWAP);
    a.var(LDA_ABS, SWAPPED);
    a.branch(BEQ, "apply_fine");
    a.op(PHP);
    a.op(SEI);
    a.var(LDA_ABS, BACKD018);
    a.abs(STA_ABS, 0xD018);
    a.lab(JSR, "set_fine", 0);
    a.imm(LDA_IMM, 13);
    a.var(STA_ABS, CCROW);
    a.imm(LDA_IMM, 25);
    a.var(STA_ABS, CCEND);
    a.lab(JSR, "copy_colors", 0);
    a.op(PLP);
    a.var(LDA_ABS, BACKHI);
    a.imm(EOR_IMM, 0x04 ^ (BUFFER_B >> 8) as u8);
    a.var(STA_ABS, BACKHI);
    a.var(LDA_ABS, BACKD018);
    a.imm(EOR_IMM, 0x10 ^ 0xF0);
    a.var(STA_ABS, BACKD018);
    a.var(LDA_ABS, TX);
    a.var(STA_ABS, SHX);
    a.var(LDA_ABS, TY);
    a.var(STA_ABS, SHY);
    a.imm(LDA_IMM, 0);
    a.var(STA_ABS, BSTATE);
    a.lab(JMP, "sprites", 0);
    a.label("apply_fine");
    a.lab(JSR, "set_fine", 0);
    a.label("sprites");
    a.lab(JSR, "copy_sprite_ptrs", 0);

    // ── decide with the new position ────────────────────────────────────────
    // velocity = requested - current (low bytes: moves below 128 px per frame)
    a.op(SEC);
    a.var(LDA_ABS, REQX);
    a.var(SBC_ABS, CAMX);
    a.var(STA_ABS, VX);
    a.op(SEC);
    a.var(LDA_ABS, REQY);
    a.var(SBC_ABS, CAMY);
    a.var(STA_ABS, VY);
    for (src, dst) in [(REQX, WX), (REQX + 1, WX + 1), (REQY, WY), (REQY + 1, WY + 1)] {
        a.var(LDA_ABS, src);
        a.var(STA_ABS, dst);
    }
    a.lab(JSR, "cells_of_w", 0);
    a.var(LDA_ABS, NCX);
    a.var(CMP_ABS, SHX);
    a.branch(BNE, "crossing");
    a.var(LDA_ABS, NCY);
    a.var(CMP_ABS, SHY);
    a.branch(BNE, "crossing");
    // same char cell: move (shown at the next boundary), prepare the next cell
    a.lab(JSR, "store_cam", 0);
    a.var(LDA_ABS, SWAPPED);
    a.branch(BNE, "done");
    for step in 0..2 {
        a.lab(JSR, "step_w", 0);
        a.var(LDA_ABS, NCX);
        a.var(CMP_ABS, SHX);
        a.branch(BNE, "prepare");
        a.var(LDA_ABS, NCY);
        a.var(CMP_ABS, SHY);
        if step == 0 {
            a.branch(BNE, "prepare");
        } else {
            a.branch(BEQ, "done");
        }
    }
    a.label("prepare");
    a.lab(JMP, "want_cell", 0);
    // new char cell: swap at the next boundary if its buffer is ready, else build
    a.label("crossing");
    a.var(LDA_ABS, BSTATE);
    a.imm(CMP_IMM, 2);
    a.branch(BNE, "cross_build");
    a.var(LDA_ABS, TX);
    a.var(CMP_ABS, NCX);
    a.branch(BNE, "cross_build");
    a.var(LDA_ABS, TY);
    a.var(CMP_ABS, NCY);
    a.branch(BNE, "cross_build");
    a.lab(JSR, "store_cam", 0);
    a.imm(LDA_IMM, 1);
    a.var(STA_ABS, DOSWAP);
    a.op(RTS);
    a.label("cross_build");
    a.var(LDA_ABS, SWAPPED); // after a color copy the view just waits
    a.branch(BNE, "done");
    a.lab(JMP, "want_cell", 0);
    a.label("done");
    a.op(RTS);

    // ── first call: draw the view into $0400 ────────────────────────────────
    a.label("init");
    a.imm(LDA_IMM, 1);
    a.var(STA_ABS, INIT);
    a.abs(LDA_ABS, 0xD018);
    a.imm(AND_IMM, 0x0F); // charset bits
    a.var(STA_ABS, CS);
    a.imm(ORA_IMM, (BUFFER_B >> 6) as u8 & 0xF0);
    a.var(STA_ABS, BACKD018);
    for (src, dst) in [(REQX, WX), (REQX + 1, WX + 1), (REQY, WY), (REQY + 1, WY + 1)] {
        a.var(LDA_ABS, src);
        a.var(STA_ABS, dst);
    }
    a.lab(JSR, "store_cam", 0);
    a.lab(JSR, "cells_of_w", 0);
    for (src, dst) in [(NCX, SHX), (NCX, TX), (NCX, VIEWX), (NCY, SHY), (NCY, TY), (NCY, NEXTY)] {
        a.var(LDA_ABS, src);
        a.var(STA_ABS, dst);
    }
    a.imm(LDA_IMM, 0x04);
    a.var(STA_ABS, BACKHI);
    a.imm(LDA_IMM, 0);
    a.var(STA_ABS, BROW);
    a.var(STA_ABS, CCROW);
    a.var(STA_ABS, BSTATE);
    a.var(STA_ABS, DOSWAP);
    a.var(STA_ABS, SWAPPED);
    a.imm(LDA_IMM, 25);
    a.var(STA_ABS, BEND);
    a.var(STA_ABS, CCEND);
    a.lab(JSR, "build_rows", 0);
    a.lab(JSR, "copy_colors", 0);
    a.var(LDA_ABS, CS);
    a.imm(ORA_IMM, 0x10); // screen $0400
    a.abs(STA_ABS, 0xD018);
    a.lab(JSR, "set_fine", 0);
    a.imm(LDA_IMM, (BUFFER_B >> 8) as u8);
    a.var(STA_ABS, BACKHI);
    a.lab(JMP, "copy_sprite_ptrs", 0);

    // ── subroutines ─────────────────────────────────────────────────────────
    // in_window: C = 1 while the raster is at line 250 or below the 8-bit range
    a.label("in_window");
    a.abs(LDA_ABS, 0xD011);
    a.branch(BMI, "iw_yes");
    a.abs(LDA_ABS, 0xD012);
    a.imm(CMP_IMM, 250);
    a.op(RTS);
    a.label("iw_yes");
    a.op(SEC);
    a.op(RTS);

    a.label("copy_sprite_ptrs");
    a.imm(LDX_IMM, 7);
    a.label("csp_loop");
    a.abs(LDA_ABX, 0x07F8);
    a.abs(STA_ABX, BUFFER_B + 0x3F8);
    a.op(DEX);
    a.branch(BPL, "csp_loop");
    a.op(RTS);

    a.label("store_cam");
    for (src, dst) in [(WX, CAMX), (WX + 1, CAMX + 1), (WY, CAMY), (WY + 1, CAMY + 1)] {
        a.var(LDA_ABS, src);
        a.var(STA_ABS, dst);
    }
    a.op(RTS);

    // step_w: w += v (signed), clamped; then the cells
    a.label("step_w");
    for (w, v, max, tag) in [(WX, VX, max_x, "x"), (WY, VY, max_y, "y")] {
        a.var(LDA_ABS, w);
        a.var(STA_ABS, TLO);
        a.var(LDA_ABS, w + 1);
        a.var(STA_ABS, THI);
        a.var(LDA_ABS, v);
        a.var(STA_ABS, TV);
        a.imm(LDA_IMM, max as u8);
        a.var(STA_ABS, MAXL);
        a.imm(LDA_IMM, (max >> 8) as u8);
        a.var(STA_ABS, MAXH);
        a.lab(JSR, "add_clamp", 0);
        a.var(LDA_ABS, TLO);
        a.var(STA_ABS, w);
        a.var(LDA_ABS, THI);
        a.var(STA_ABS, w + 1);
        let _ = tag;
    }
    // fall through to cells_of_w
    a.label("cells_of_w");
    for (w, cell) in [(WX, NCX), (WY, NCY)] {
        a.var(LDA_ABS, w + 1);
        a.var(STA_ABS, TV);
        a.var(LDA_ABS, w);
        for _ in 0..3 {
            a.var(LSR_ABS, TV);
            a.op(ROR_A);
        }
        a.var(STA_ABS, cell);
    }
    a.op(RTS);

    // add_clamp: t = clamp(t + tv, 0, max)   (tv signed)
    a.label("add_clamp");
    a.var(LDA_ABS, TV);
    a.branch(BEQ, "ac_done");
    a.branch(BPL, "ac_pos");
    a.op(CLC);
    a.var(LDA_ABS, TLO);
    a.var(ADC_ABS, TV);
    a.var(STA_ABS, TLO);
    a.var(LDA_ABS, THI);
    a.imm(ADC_IMM, 0xFF);
    a.var(STA_ABS, THI);
    a.branch(BPL, "ac_done");
    a.imm(LDA_IMM, 0);
    a.var(STA_ABS, TLO);
    a.var(STA_ABS, THI);
    a.op(RTS);
    a.label("ac_pos");
    a.op(CLC);
    a.var(LDA_ABS, TLO);
    a.var(ADC_ABS, TV);
    a.var(STA_ABS, TLO);
    a.var(LDA_ABS, THI);
    a.imm(ADC_IMM, 0);
    a.var(STA_ABS, THI);
    a.var(CMP_ABS, MAXH);
    a.branch(BCC, "ac_done");
    a.branch(BNE, "ac_set");
    a.var(LDA_ABS, TLO);
    a.var(CMP_ABS, MAXL);
    a.branch(BCC, "ac_done");
    a.label("ac_set");
    a.var(LDA_ABS, MAXL);
    a.var(STA_ABS, TLO);
    a.var(LDA_ABS, MAXH);
    a.var(STA_ABS, THI);
    a.label("ac_done");
    a.op(RTS);

    // set_fine: XSCROLL / YSCROLL = 7 - (camera & 7), 38 columns / 24 rows
    a.label("set_fine");
    a.var(LDA_ABS, CAMX);
    a.imm(AND_IMM, 7);
    a.imm(EOR_IMM, 7);
    a.var(STA_ABS, TV);
    a.abs(LDA_ABS, 0xD016);
    a.imm(AND_IMM, 0xF0);
    a.var(ORA_ABS, TV);
    a.abs(STA_ABS, 0xD016);
    a.var(LDA_ABS, CAMY);
    a.imm(AND_IMM, 7);
    a.imm(EOR_IMM, 7);
    a.var(STA_ABS, TV);
    a.abs(LDA_ABS, 0xD011);
    a.imm(AND_IMM, 0x70);
    a.var(ORA_ABS, TV);
    a.abs(STA_ABS, 0xD011);
    a.op(RTS);

    // want_cell: build the hidden buffer for (ncx, ncy), one half per call
    a.label("want_cell");
    a.var(LDA_ABS, BSTATE);
    a.branch(BEQ, "wc_new");
    a.var(LDA_ABS, TX);
    a.var(CMP_ABS, NCX);
    a.branch(BNE, "wc_new");
    a.var(LDA_ABS, TY);
    a.var(CMP_ABS, NCY);
    a.branch(BNE, "wc_new");
    a.var(LDA_ABS, BSTATE);
    a.imm(CMP_IMM, 2);
    a.branch(BEQ, "wc_done");
    a.imm(LDA_IMM, 13);
    a.var(STA_ABS, BROW);
    a.imm(LDA_IMM, 25);
    a.var(STA_ABS, BEND);
    a.lab(JSR, "build_rows", 0);
    a.imm(LDA_IMM, 2);
    a.var(STA_ABS, BSTATE);
    a.op(RTS);
    a.label("wc_new");
    for (src, dst) in [(NCX, TX), (NCX, VIEWX), (NCY, TY), (NCY, NEXTY)] {
        a.var(LDA_ABS, src);
        a.var(STA_ABS, dst);
    }
    a.imm(LDA_IMM, 0);
    a.var(STA_ABS, BROW);
    a.imm(LDA_IMM, 13);
    a.var(STA_ABS, BEND);
    a.lab(JSR, "build_rows", 0);
    a.imm(LDA_IMM, 1);
    a.var(STA_ABS, BSTATE);
    a.label("wc_done");
    a.op(RTS);

    // build_rows: hidden-buffer rows brow .. bend-1 for cell (viewx, nexty)
    a.label("build_rows");
    a.var(LDA_ABS, BROW);
    a.var(CMP_ABS, BEND);
    a.branch(BEQ, "br_done");
    a.op(TAY);
    a.lab(LDA_ABY, "mul40_lo", 0);
    a.var(STA_ABS, DSTL);
    a.op(CLC);
    a.lab(LDA_ABY, "mul40_hi", 0);
    a.var(ADC_ABS, BACKHI);
    a.var(STA_ABS, DSTH);
    a.op(TYA);
    a.op(CLC);
    a.var(ADC_ABS, NEXTY);
    a.op(TAX);
    a.lab(JSR, "copy_row_chars", 0);
    a.var(INC_ABS, BROW);
    a.lab(JMP, "build_rows", 0);
    a.label("br_done");
    a.op(RTS);

    // copy_colors: color RAM rows ccrow .. ccend-1 for cell (viewx, nexty)
    a.label("copy_colors");
    if p.colors.is_some() {
        a.var(LDY_ABS_OP, CCROW);
        a.lab(LDA_ABY, "mul40_lo", 0);
        a.var(STA_ABS, DSTL);
        a.op(CLC);
        a.lab(LDA_ABY, "mul40_hi", 0);
        a.imm(ADC_IMM, 0xD8);
        a.var(STA_ABS, DSTH);
        a.op(TYA);
        a.op(CLC);
        a.var(ADC_ABS, NEXTY);
        a.op(TAX);
        a.lab(JSR, "copy_row_colors", 0);
        a.var(INC_ABS, CCROW);
        a.var(LDA_ABS, CCROW);
        a.var(CMP_ABS, CCEND);
        a.branch(BNE, "copy_colors");
    }
    a.op(RTS);

    // copy_row_*: 40 bytes of map row X from column viewx to (dstl, dsth);
    // source and target are patched into the LDA/STA operands (~14 cycles/byte)
    let tables: Vec<(&str, &str, &str)> = if p.colors.is_some() {
        vec![("chars", "row_lo", "row_hi"), ("colors", "crow_lo", "crow_hi")]
    } else {
        vec![("chars", "row_lo", "row_hi")]
    };
    for (kind, lo, hi) in &tables {
        let name = format!("copy_row_{kind}");
        let src = format!("{name}_src");
        let dst = format!("{name}_dst");
        a.label(&name);
        a.op(CLC);
        a.lab(LDA_ABX, lo, 0);
        a.var(ADC_ABS, VIEWX);
        a.lab(STA_ABS, &src, 1);
        a.lab(LDA_ABX, hi, 0);
        a.imm(ADC_IMM, 0);
        a.lab(STA_ABS, &src, 2);
        a.var(LDA_ABS, DSTL);
        a.lab(STA_ABS, &dst, 1);
        a.var(LDA_ABS, DSTH);
        a.lab(STA_ABS, &dst, 2);
        a.imm(LDY_IMM, 39);
        a.label(&src);
        a.abs(LDA_ABY, 0xFFFF);
        a.label(&dst);
        a.abs(STA_ABY, 0xFFFF);
        a.op(DEY);
        a.branch(BPL, &src);
        a.op(RTS);
    }

    // ── data ────────────────────────────────────────────────────────────────
    let mul40: Vec<u16> = (0..25u16).map(|r| r * 40).collect();
    a.label("mul40_lo");
    a.bytes(&mul40.iter().map(|v| *v as u8).collect::<Vec<_>>());
    a.label("mul40_hi");
    a.bytes(&mul40.iter().map(|v| (*v >> 8) as u8).collect::<Vec<_>>());
    let table = |a: &mut Asm, lo: &str, hi: &str, start: u16| {
        let rows: Vec<u16> = (0..p.height as u16)
            .map(|r| start.wrapping_add(r * p.width as u16))
            .collect();
        a.label(lo);
        a.bytes(&rows.iter().map(|v| *v as u8).collect::<Vec<_>>());
        a.label(hi);
        a.bytes(&rows.iter().map(|v| (*v >> 8) as u8).collect::<Vec<_>>());
    };
    table(&mut a, "row_lo", "row_hi", p.chars);
    if let Some(colors) = p.colors {
        table(&mut a, "crow_lo", "crow_hi", colors);
    }
    a.label("vars");
    a.bytes(&[0u8; VAR_SIZE]);
    a.finish()
}

const LDY_ABS_OP: u8 = 0xAC;

#[cfg(test)]
mod tests {
    use super::*;

    fn params(width: u8, height: u8, colors: bool) -> MapViewParams {
        MapViewParams {
            chars: 0x4000,
            colors: colors.then_some(0x4000 + width as u16 * height as u16),
            width,
            height,
            param_zp: 0x50,
        }
    }

    #[test]
    fn helper_assembles_for_small_and_large_maps() {
        for (w, h) in [(40, 25), (80, 125), (255, 255)] {
            let code = helper(0x1000, &params(w, h, true)).expect("assembles");
            assert!(code.len() > 500, "{w}x{h}: {}", code.len());
        }
        assert!(helper(0x1000, &params(80, 125, false)).is_ok());
    }

    #[test]
    fn helper_rejects_maps_smaller_than_the_view() {
        assert!(helper(0x1000, &params(39, 25, true)).is_err());
        assert!(helper(0x1000, &params(40, 24, true)).is_err());
    }

    #[test]
    fn row_tables_point_at_each_map_row() {
        let p = params(80, 125, true);
        let code = helper(0x1000, &p).unwrap();
        // the tables sit right before the 48 variable bytes:
        // row_lo, row_hi, crow_lo, crow_hi (125 each)
        let vars = code.len() - VAR_SIZE;
        let crow_hi = vars - 125;
        let crow_lo = crow_hi - 125;
        let row_hi = crow_lo - 125;
        let row_lo = row_hi - 125;
        for r in [0usize, 1, 24, 124] {
            let chars = code[row_lo + r] as u16 | (code[row_hi + r] as u16) << 8;
            let colors = code[crow_lo + r] as u16 | (code[crow_hi + r] as u16) << 8;
            assert_eq!(chars, 0x4000 + r as u16 * 80);
            assert_eq!(colors, 0x4000 + 80 * 125 + r as u16 * 80);
        }
    }
}
