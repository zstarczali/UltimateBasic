//! `tune ... end` — an inline SID tracker tune.
//!
//! The block describes instruments, an order list and per-voice pattern rows in
//! plain text (the same cell notation the Visual Assembler SID editor shows,
//! e.g. `C-4 01 V24`). The compiler turns it into one self-contained machine
//! code blob (player + data) placed at a fixed address, exactly like
//! `load sid`, and defines `sid_init` / `sid_play` so `music play|stop|pause|
//! resume` work unchanged.
//!
//! The player keeps ALL of its state inside the blob (absolute addressing) and
//! uses no zero page at all, so it cannot collide with UltimateBasic variables
//! or temporaries and survives a return to BASIC.

use super::codegen::assemble_inline;

pub const ROWS: usize = 32;
pub const MAX_PATTERNS: usize = 8;
pub const DEFAULT_ADDR: u16 = 0x8000;

#[derive(Clone, Debug, Default)]
pub struct Instrument {
    pub ctrl: u8,
    pub ad: u8,
    pub sr: u8,
    pub pw: u16,
    pub cutoff: u16,
    pub res_filt: u8,
    pub mode_vol: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    /// 1..=96 (C-0 = 1), 0 = no note
    pub note: u8,
    pub inst: u8,
    /// 0 none, 1 V, 2 U, 3 D, 4 C, 5 F
    pub fx: u8,
    pub fxval: u8,
}

#[derive(Clone, Debug)]
pub struct TuneDef {
    pub speed: u8,
    pub insts: Vec<Instrument>,
    pub order: Vec<u8>,
    /// [pattern][voice][row]
    pub patterns: Vec<[[Cell; ROWS]; 3]>,
}

impl Default for TuneDef {
    fn default() -> Self {
        TuneDef {
            speed: 6,
            insts: Vec::new(),
            order: Vec::new(),
            patterns: Vec::new(),
        }
    }
}

/// Parse one tracker cell: `C-4 01 V24`, `... .. F06`, `D#3 02`, `...`.
pub fn parse_cell(text: &str) -> Result<Cell, String> {
    let mut cell = Cell::default();
    let toks: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    if toks.is_empty() {
        return Ok(cell);
    }
    let b = toks[0].as_bytes();
    if toks[0] == "..." || toks[0] == "---" {
        i = 1;
    } else if b.len() == 3
        && matches!(b[0].to_ascii_lowercase(), b'a'..=b'g')
        && matches!(b[1], b'-' | b'#' | b'b')
        && b[2].is_ascii_digit()
    {
        let semi: i32 = match b[0].to_ascii_lowercase() {
            b'c' => 0,
            b'd' => 2,
            b'e' => 4,
            b'f' => 5,
            b'g' => 7,
            b'a' => 9,
            b'b' => 11,
            _ => return Err(format!("bad note '{}'", toks[0])),
        };
        let semi = match b[1] {
            b'-' => semi,
            b'#' => semi + 1,
            b'b' => semi - 1,
            _ => return Err(format!("bad note '{}'", toks[0])),
        };
        let n = (b[2] - b'0') as i32 * 12 + semi.rem_euclid(12);
        if !(0..96).contains(&n) {
            return Err(format!("note out of range '{}'", toks[0]));
        }
        cell.note = n as u8 + 1;
        i = 1;
    }
    // Positional after the note: [inst] [fx]. A lone token that looks like an
    // effect (e.g. `F06`) is accepted for effect-only rows.
    let fx_of = |t: &str| -> Option<(u8, u8)> {
        let tb = t.as_bytes();
        if tb.is_empty() || tb.len() > 3 || !tb[1..].iter().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let code = match tb[0].to_ascii_uppercase() {
            b'V' => 1,
            b'U' => 2,
            b'D' => 3,
            b'C' => 4,
            b'F' => 5,
            _ => return None,
        };
        let val = if t.len() > 1 { u8::from_str_radix(&t[1..], 16).ok()? } else { 0 };
        Some((code, val))
    };
    if i == 0 && toks.len() == 1 {
        if let Some((c, v)) = fx_of(toks[0]) {
            cell.fx = c;
            cell.fxval = v;
            return Ok(cell);
        }
        return Err(format!("bad cell '{}'", text));
    }
    if i < toks.len() {
        let t = toks[i];
        i += 1;
        if t != ".." && t != "..." {
            if t.len() <= 2 && t.chars().all(|c| c.is_ascii_hexdigit()) {
                cell.inst = u8::from_str_radix(t, 16).unwrap_or(0);
            } else if let Some((c, v)) = fx_of(t) {
                cell.fx = c;
                cell.fxval = v;
            } else {
                return Err(format!("bad cell entry '{}'", t));
            }
        }
    }
    if i < toks.len() {
        let t = toks[i];
        if t != ".." && t != "..." {
            match fx_of(t) {
                Some((c, v)) => {
                    cell.fx = c;
                    cell.fxval = v;
                }
                None => return Err(format!("bad effect '{}'", t)),
            }
        }
    }
    Ok(cell)
}

fn hexlines(out: &mut String, label: &str, data: &[u8]) {
    out.push_str(&format!("{}:\n", label));
    if data.is_empty() {
        out.push_str("    $00\n");
    }
    for chunk in data.chunks(16) {
        let v: Vec<String> = chunk.iter().map(|b| format!("${:02X}", b)).collect();
        out.push_str(&format!("    {}\n", v.join(", ")));
    }
}

/// Generate the player + data as assembly text for `assemble_inline`.
/// Entry points: `base` = init, `base + 3` = play.
pub fn build_asm(def: &TuneDef) -> Result<String, String> {
    if def.order.is_empty() {
        return Err("tune: no 'order' line".into());
    }
    if def.patterns.is_empty() {
        return Err("tune: no 'pat' lines".into());
    }
    if def.patterns.len() > MAX_PATTERNS {
        return Err(format!("tune: at most {} patterns are supported", MAX_PATTERNS));
    }
    if def.order.len() > 255 {
        return Err("tune: order list is limited to 255 steps".into());
    }
    if let Some(&bad) = def.order.iter().find(|&&p| p as usize >= def.patterns.len()) {
        return Err(format!("tune: order refers to undefined pattern {}", bad));
    }
    let ninst = def.insts.len().max(1);
    let mut insts = def.insts.clone();
    if insts.is_empty() {
        insts.push(Instrument { ctrl: 0x41, ad: 0x09, sr: 0xF0, pw: 0x800, cutoff: 0, res_filt: 0, mode_vol: 0x0F });
    }
    for p in &def.patterns {
        for v in p.iter() {
            for c in v.iter() {
                if c.note > 0 && c.inst as usize >= ninst {
                    return Err(format!("tune: pattern uses undefined instrument {:02X}", c.inst));
                }
            }
        }
    }

    let mut a = String::new();
    a.push_str("    JMP sid_init\n    JMP sid_play\n");

    // ---- init -------------------------------------------------------------
    a.push_str("sid_init:\n    LDA #$00\n    STA $D418\n    LDX #$18\nsid_clr:\n    STA $D400,X\n    DEX\n    BPL sid_clr\n");
    a.push_str("    LDA sid_speed\n    STA sid_tick\n    STA sid_spd\n    LDA #$00\n    STA sid_row\n    STA sid_opos\n");
    for v in 0..3 {
        for n in ["bl", "bh", "cl", "ch", "vs", "vc"] {
            a.push_str(&format!("    STA sid_{}{}\n", n, v));
        }
    }
    a.push_str("    LDX sid_order\n    JSR sid_setpat\n    RTS\n");
    a.push_str("sid_setpat:\n    TXA\n    ASL A\n    ASL A\n    ASL A\n    ASL A\n    ASL A\n    STA sid_poff\n    RTS\n");

    // ---- play (once per frame) --------------------------------------------
    a.push_str("sid_play:\n    DEC sid_tick\n    BNE sid_pdone\n    LDA sid_spd\n    STA sid_tick\n    JSR sid_rowstep\nsid_pdone:\n    RTS\n");

    // ---- row step -----------------------------------------------------------
    a.push_str("sid_rowstep:\n    LDA sid_row\n    CLC\n    ADC sid_poff\n    STA sid_idx\n");
    for v in 0..3usize {
        let d = 0xD400u16 + 7 * v as u16;
        let reg = |o: u16| format!("${:04X}", d + o);
        a.push_str(&format!("    LDY sid_idx\n    LDA sid_n{v},Y\n    BNE sid_t{v}\n    JMP sid_nn{v}\nsid_t{v}:\n    STA sid_tmp\n    LDA sid_s{v},Y\n    TAX\n"));
        a.push_str(&format!("    LDA {g}\n    AND #$FE\n    STA {g}\n", g = reg(4)));
        a.push_str(&format!("    LDA sid_ipl,X\n    STA {}\n    LDA sid_iph,X\n    STA {}\n", reg(2), reg(3)));
        a.push_str(&format!("    LDA sid_iad,X\n    STA {}\n    LDA sid_isr,X\n    STA {}\n", reg(5), reg(6)));
        a.push_str("    LDA sid_ict,X\n    STA sid_ctl\n    LDA sid_icl,X\n    STA $D415\n    LDA sid_ich,X\n    STA $D416\n    LDA sid_irf,X\n    STA $D417\n    LDA sid_imv,X\n    STA $D418\n");
        a.push_str(&format!("    LDX sid_tmp\n    DEX\n    LDA sid_flo,X\n    STA {}\n    STA sid_bl{v}\n    STA sid_cl{v}\n", reg(0)));
        a.push_str(&format!("    LDA sid_fhi,X\n    STA {}\n    STA sid_bh{v}\n    STA sid_ch{v}\n", reg(1)));
        a.push_str(&format!("    LDA #$00\n    STA sid_vs{v}\n    STA sid_vc{v}\n    LDA sid_ctl\n    STA {}\n", reg(4)));
        // effect column
        a.push_str(&format!("sid_nn{v}:\n    LDY sid_idx\n    LDA sid_f{v},Y\n    BNE sid_fx{v}\n    JMP sid_fd{v}\nsid_fx{v}:\n    STA sid_tmp\n    LDA sid_x{v},Y\n    TAX\n    LDA sid_tmp\n"));
        a.push_str(&format!("    CMP #$04\n    BNE sid_c1_{v}\n    LDA {g}\n    AND #$FE\n    STA {g}\n    JMP sid_fd{v}\n", g = reg(4)));
        a.push_str(&format!("sid_c1_{v}:\n    CMP #$05\n    BNE sid_c2_{v}\n    STX sid_spd\n    JMP sid_fd{v}\n"));
        a.push_str(&format!("sid_c2_{v}:\n    CMP #$01\n    BNE sid_c3_{v}\n    JSR sid_vib{v}\n    JMP sid_fd{v}\n"));
        a.push_str(&format!("sid_c3_{v}:\n    CMP #$02\n    BNE sid_c4_{v}\n    JSR sid_up{v}\n    JMP sid_fd{v}\n"));
        a.push_str(&format!("sid_c4_{v}:\n    JSR sid_dn{v}\nsid_fd{v}:\n"));
    }
    a.push_str(&format!(
        "    INC sid_row\n    LDA sid_row\n    CMP #$20\n    BCC sid_rdone\n    LDA #$00\n    STA sid_row\n    INC sid_opos\n    LDX sid_opos\n    CPX #${:02X}\n    BCC sid_ook\n    LDX #$00\n    STX sid_opos\nsid_ook:\n    LDA sid_order,X\n    TAX\n    JSR sid_setpat\nsid_rdone:\n    RTS\n",
        def.order.len()
    ));

    // ---- per-voice effect routines -------------------------------------------
    for v in 0..3usize {
        let d = 0xD400u16 + 7 * v as u16;
        let (flo, fhi) = (format!("${:04X}", d), format!("${:04X}", d + 1));
        a.push_str(&format!(
            "sid_vib{v}:\n    STX sid_tmp\n    LDA sid_tmp\n    LSR A\n    LSR A\n    LSR A\n    LSR A\n    BNE sid_vp{v}\n    LDA #$01\nsid_vp{v}:\n    CMP sid_vc{v}\n    BNE sid_vh{v}\n    LDA #$00\n    STA sid_vc{v}\n    LDA sid_vs{v}\n    EOR #$FF\n    STA sid_vs{v}\n    JMP sid_va{v}\nsid_vh{v}:\n    INC sid_vc{v}\nsid_va{v}:\n    LDA sid_tmp\n    AND #$0F\n    ASL A\n    ASL A\n    STA sid_tmp\n    LDA sid_vs{v}\n    BEQ sid_vn{v}\n    CLC\n    LDA sid_bl{v}\n    ADC sid_tmp\n    STA sid_cl{v}\n    LDA sid_bh{v}\n    ADC #$00\n    STA sid_ch{v}\n    JMP sid_vw{v}\nsid_vn{v}:\n    SEC\n    LDA sid_bl{v}\n    SBC sid_tmp\n    STA sid_cl{v}\n    LDA sid_bh{v}\n    SBC #$00\n    STA sid_ch{v}\nsid_vw{v}:\n    LDA sid_cl{v}\n    STA {flo}\n    LDA sid_ch{v}\n    STA {fhi}\n    RTS\n"
        ));
        a.push_str(&format!(
            "sid_up{v}:\n    CLC\n    TXA\n    ADC sid_cl{v}\n    STA sid_cl{v}\n    LDA sid_ch{v}\n    ADC #$00\n    STA sid_ch{v}\n    LDA sid_cl{v}\n    STA {flo}\n    LDA sid_ch{v}\n    STA {fhi}\n    RTS\n"
        ));
        a.push_str(&format!(
            "sid_dn{v}:\n    SEC\n    LDA sid_cl{v}\n    STX sid_tmp\n    SBC sid_tmp\n    STA sid_cl{v}\n    LDA sid_ch{v}\n    SBC #$00\n    STA sid_ch{v}\n    LDA sid_cl{v}\n    STA {flo}\n    LDA sid_ch{v}\n    STA {fhi}\n    RTS\n"
        ));
    }

    // ---- data -----------------------------------------------------------------
    hexlines(&mut a, "sid_speed", &[def.speed.max(1)]);
    hexlines(&mut a, "sid_order", &def.order);
    let col = |f: &dyn Fn(&Instrument) -> u8| -> Vec<u8> { insts.iter().map(f).collect() };
    hexlines(&mut a, "sid_ict", &col(&|i| i.ctrl | 1));
    hexlines(&mut a, "sid_iad", &col(&|i| i.ad));
    hexlines(&mut a, "sid_isr", &col(&|i| i.sr));
    hexlines(&mut a, "sid_ipl", &col(&|i| (i.pw & 0xFF) as u8));
    hexlines(&mut a, "sid_iph", &col(&|i| ((i.pw >> 8) & 0x0F) as u8));
    hexlines(&mut a, "sid_icl", &col(&|i| (i.cutoff & 7) as u8));
    hexlines(&mut a, "sid_ich", &col(&|i| ((i.cutoff >> 3) & 0xFF) as u8));
    hexlines(&mut a, "sid_irf", &col(&|i| i.res_filt));
    hexlines(&mut a, "sid_imv", &col(&|i| i.mode_vol));
    for v in 0..3 {
        let mut n = Vec::new();
        let mut s = Vec::new();
        let mut f = Vec::new();
        let mut x = Vec::new();
        for p in &def.patterns {
            for c in p[v].iter() {
                n.push(c.note);
                s.push(c.inst);
                f.push(c.fx);
                x.push(c.fxval);
            }
        }
        hexlines(&mut a, &format!("sid_n{}", v), &n);
        hexlines(&mut a, &format!("sid_s{}", v), &s);
        hexlines(&mut a, &format!("sid_f{}", v), &f);
        hexlines(&mut a, &format!("sid_x{}", v), &x);
    }
    let (mut lo, mut hi) = (Vec::new(), Vec::new());
    for n in 0..96 {
        let hz = 440.0f64 * 2f64.powf((n as f64 - 57.0) / 12.0);
        let f = ((hz * 16777216.0 / 985248.0).round() as u64).min(0xFFFF) as u16;
        lo.push((f & 0xFF) as u8);
        hi.push((f >> 8) as u8);
    }
    hexlines(&mut a, "sid_flo", &lo);
    hexlines(&mut a, "sid_fhi", &hi);
    for n in ["tick", "spd", "row", "opos", "poff", "idx", "tmp", "ctl"] {
        hexlines(&mut a, &format!("sid_{}", n), &[0]);
    }
    for v in 0..3 {
        for n in ["bl", "bh", "cl", "ch", "vs", "vc"] {
            hexlines(&mut a, &format!("sid_{}{}", n, v), &[0]);
        }
    }
    Ok(a)
}

/// Assemble the tune at `base`. Entry points: init = base, play = base + 3.
pub fn build(def: &TuneDef, base: u16) -> Result<Vec<u8>, String> {
    let asm = build_asm(def)?;
    let bytes = assemble_inline(&asm, base);
    if base as usize + bytes.len() > 0xFFFF {
        return Err("tune: player and data do not fit below $FFFF at that address".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells() {
        assert_eq!(parse_cell("C-4 01 V24").unwrap(), Cell { note: 49, inst: 1, fx: 1, fxval: 0x24 });
        assert_eq!(parse_cell("... .. F06").unwrap(), Cell { note: 0, inst: 0, fx: 5, fxval: 6 });
        assert_eq!(parse_cell("D#3 0A").unwrap().note, 3 * 12 + 3 + 1);
        assert_eq!(parse_cell("").unwrap(), Cell::default());
        assert!(parse_cell("H-4").is_err());
    }

    #[test]
    fn blob_assembles_with_all_labels_resolved() {
        let mut def = TuneDef::default();
        def.insts.push(Instrument { ctrl: 0x40, ad: 0x09, sr: 0xF0, pw: 0x800, cutoff: 0x100, res_filt: 0x21, mode_vol: 0x1F });
        def.order = vec![0, 1, 0];
        let mut p = [[Cell::default(); ROWS]; 3];
        p[0][0] = Cell { note: 49, inst: 0, fx: 1, fxval: 0x24 };
        def.patterns = vec![p, p];
        let asm = build_asm(&def).unwrap();
        let bytes = build(&def, 0x8000).unwrap();
        assert_eq!(&bytes[0..3], &[0x4C, 0x06, 0x80]); // JMP sid_init (right after the two JMPs)
        // no JMP/JSR may point at $0000 (unresolved label)
        let mut i = 0;
        while i + 2 < bytes.len() && i < 900 {
            if (bytes[i] == 0x4C || bytes[i] == 0x20) && bytes[i + 1] == 0 && bytes[i + 2] == 0 {
                panic!("unresolved label near offset {} in:\n{}", i, asm);
            }
            i += 1;
        }
    }

    #[test]
    fn inline_asm_keeps_indexed_label_modes() {
        let out = assemble_inline("LDA tbl,X
LDA tbl,Y
JMP (tbl)
tbl:
$01", 0x1000);
        assert_eq!(out, vec![0xBD, 0x09, 0x10, 0xB9, 0x09, 0x10, 0x6C, 0x09, 0x10, 0x01]);
    }
}
