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
//!
//! Every frame the player runs, per voice, the instrument's wave/arpeggio
//! table, vibrato, pulse width modulation and gate timer / hard restart, plus
//! the filter cutoff sweep of the voice that last started a filter-routed
//! instrument. It is the same algorithm as the Visual Assembler SID editor's
//! exported player (and its preview engine), frame for frame.

use super::codegen::assemble_inline;

pub const ROWS: usize = 32;
pub const MAX_PATTERNS: usize = 8;
pub const DEFAULT_ADDR: u16 = 0x8000;
pub const MAX_TABLE_STEPS: usize = 32;

#[derive(Clone, Debug)]
pub struct Instrument {
    /// `inst id, ctrl, ad, sr, pw, cutoff, resfilt, modevol`
    pub ctrl: u8,
    pub ad: u8,
    pub sr: u8,
    pub pw: u16,
    pub cutoff: u16,
    pub res_filt: u8,
    pub mode_vol: u8,
    /// `imod id, vibdelay, vibspeed, vibdepth, pwmspeed, pwmmin, pwmmax`
    pub vib_delay: u8,
    pub vib_speed: u8,
    pub vib_depth: u8,
    pub pwm_speed: u8,
    pub pwm_min: u16,
    pub pwm_max: u16,
    /// `ifilt id, cutend, sweep, pingpong` (no line: the cutoff stays put)
    pub cut_end: Option<u16>,
    pub cut_speed: u8,
    pub cut_ping: bool,
    /// `igate id, gatetimer, hardrestart, firstwave`
    pub gate_timer: u8,
    pub hard_restart: bool,
    pub first_wave: u8,
    /// `itab id, speed, loop, "WW +N" ...`: (waveform, 0 = keep; note; absolute)
    pub table: Vec<(u8, i16, bool)>,
    pub table_loop: u8,
    pub table_speed: u8,
}

impl Default for Instrument {
    fn default() -> Self {
        Instrument {
            ctrl: 0x41,
            ad: 0x09,
            sr: 0xF0,
            pw: 0x800,
            cutoff: 0,
            res_filt: 0,
            mode_vol: 0x0F,
            vib_delay: 0,
            vib_speed: 0,
            vib_depth: 0,
            pwm_speed: 0,
            pwm_min: 0,
            pwm_max: 0,
            cut_end: None,
            cut_speed: 0,
            cut_ping: false,
            gate_timer: 0,
            hard_restart: false,
            first_wave: 0,
            table: Vec::new(),
            table_loop: 0xFF,
            table_speed: 1,
        }
    }
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

/// Parse one `itab` step: `"41 +4"`, `".. -12"`, `"81 =60"` (absolute note).
pub fn parse_table_step(text: &str) -> Result<(u8, i16, bool), String> {
    let toks: Vec<&str> = text.split_whitespace().collect();
    if toks.is_empty() || toks.len() > 2 {
        return Err(format!("bad table step '{}' (use \"WW +N\", \".. -N\" or \"WW =N\")", text));
    }
    let wave = if toks[0] == ".." || toks[0] == "--" {
        0
    } else {
        u8::from_str_radix(toks[0].trim_start_matches('$'), 16).map_err(|_| format!("bad waveform '{}'", toks[0]))?
    };
    let note_tok = toks.get(1).copied().unwrap_or("+0");
    let (abs, digits) = match note_tok.as_bytes()[0] {
        b'=' => (true, &note_tok[1..]),
        b'+' => (false, &note_tok[1..]),
        b'-' => (false, note_tok),
        _ => (false, note_tok),
    };
    let n: i16 = digits.parse().map_err(|_| format!("bad note '{}'", note_tok))?;
    if abs && !(0..96).contains(&n) {
        return Err(format!("absolute note must be 0-95: '{}'", note_tok));
    }
    if !abs && !(-96..96).contains(&n) {
        return Err(format!("note offset must be -96..95: '{}'", note_tok));
    }
    Ok((wave, n, abs))
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

fn emit(out: &mut String, lines: &[&str]) {
    for l in lines {
        if l.ends_with(':') {
            out.push_str(l);
        } else {
            out.push_str("    ");
            out.push_str(l);
        }
        out.push('\n');
    }
}

/// PAL frequency table (C-0..B-7) and the semitone step above each note.
pub fn freq_tables() -> (Vec<u16>, Vec<u16>) {
    let f: Vec<u16> = (0..96)
        .map(|n| {
            let hz = 440.0f64 * 2f64.powf((n as f64 - 57.0) / 12.0);
            ((hz * 16777216.0 / 985248.0).round() as u64).min(0xFFFF) as u16
        })
        .collect();
    let mut semi: Vec<u16> = Vec::with_capacity(96);
    for n in 0..96 {
        let d = f[(n + 1).min(95)].saturating_sub(f[n]);
        semi.push(if d > 0 { d } else if n > 0 { semi[n - 1] } else { 0 });
    }
    (f, semi)
}

/// One byte table per instrument parameter, indexed by instrument number,
/// plus the concatenated wave/arpeggio table steps (at most 255 in all).
struct InstTables {
    cols: Vec<(&'static str, Vec<u8>)>,
    tw: Vec<u8>,
    tn: Vec<u8>,
    ta: Vec<u8>,
}

fn inst_tables(insts: &[Instrument]) -> InstTables {
    let mut t = InstTables { cols: Vec::new(), tw: Vec::new(), tn: Vec::new(), ta: Vec::new() };
    let names = [
        "ictl", "iad", "isr", "ipwl", "ipwh", "icul", "icuh", "icel", "iceh", "icsp", "ifl", "ires", "imode", "ivol", "ivdl",
        "ivsp", "ivdp", "ipms", "ipmnl", "ipmnh", "ipmxl", "ipmxh", "igt", "ihr", "iffw", "itst", "itln", "itlp", "itsp",
    ];
    let mut cols: Vec<Vec<u8>> = vec![Vec::new(); names.len()];
    for i in insts {
        let pw = i.pw.min(4095);
        let cut = i.cutoff & 0x7FF;
        let end = i.cut_end.unwrap_or(cut).min(2047);
        let sweep = i.cut_speed.min(127);
        let csp = if sweep == 0 { 0 } else if end >= cut { sweep } else { 0u8.wrapping_sub(sweep) };
        let filt = i.res_filt & 7 != 0;
        let (pmin, pmax) = (i.pwm_min.min(4095).min(i.pwm_max.min(4095)), i.pwm_min.min(4095).max(i.pwm_max.min(4095)));
        let steps: Vec<(u8, i16, bool)> = i.table.iter().copied().take(MAX_TABLE_STEPS).take(255 - t.tw.len()).collect();
        let start = t.tw.len() as u8;
        let tloop = if (i.table_loop as usize) < steps.len() { i.table_loop } else { 0xFF };
        let vals = [
            i.ctrl & 0xFE,
            i.ad,
            i.sr,
            (pw & 0xFF) as u8,
            (pw >> 8) as u8,
            (cut & 0xFF) as u8,
            (cut >> 8) as u8,
            (end & 0xFF) as u8,
            (end >> 8) as u8,
            csp,
            filt as u8 | if i.cut_ping { 2 } else { 0 },
            i.res_filt & 0xF0,
            i.mode_vol & 0xF0,
            i.mode_vol & 0x0F,
            i.vib_delay,
            i.vib_speed.min(15),
            i.vib_depth.min(15),
            i.pwm_speed,
            (pmin & 0xFF) as u8,
            (pmin >> 8) as u8,
            (pmax & 0xFF) as u8,
            (pmax >> 8) as u8,
            i.gate_timer.min(15),
            i.hard_restart as u8,
            i.first_wave,
            start,
            steps.len() as u8,
            tloop,
            i.table_speed.clamp(1, 15),
        ];
        for (c, v) in cols.iter_mut().zip(vals) {
            c.push(v);
        }
        for (w, n, abs) in steps {
            t.tw.push(w);
            t.tn.push(n as u8);
            t.ta.push(abs as u8);
        }
    }
    t.cols = names.iter().copied().zip(cols).collect();
    t
}

/// Generate the player + data as assembly text for `assemble_inline`.
/// Entry points: `base` = init, `base + 3` = play (once per frame).
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
    let mut insts = def.insts.clone();
    if insts.is_empty() {
        insts.push(Instrument::default());
    }
    let ninst = insts.len();
    for p in &def.patterns {
        for v in p.iter() {
            for c in v.iter() {
                if c.note > 0 && c.inst as usize >= ninst {
                    return Err(format!("tune: pattern uses undefined instrument {:02X}", c.inst));
                }
            }
        }
    }

    const STATE_V: [&str; 28] = [
        "on", "inst", "note", "gate", "jon", "ffr", "ctl", "sll", "slh", "fvol", "fvoh", "fvs", "fvc", "vdl", "vcn", "vdr",
        "vofl", "vofh", "vstl", "vsth", "pwl", "pwh", "pwd", "tpos", "tcnt", "wav", "nof", "nab",
    ];
    const STATE_G: [&str; 30] = [
        "fown", "fcl", "fch", "fspd", "ftl", "fth", "fsl", "fsh", "fel", "feh", "fping", "fres", "fmode", "froute", "vol", "cv",
        "ci", "fxv", "tmp", "t0", "t1", "t3", "m0", "m1", "tick", "spd", "row", "opos", "poff", "idx",
    ];
    let state_size = STATE_V.len() * 3 + STATE_G.len();

    let mut a = String::new();
    a.push_str("    JMP sid_init\n    JMP sid_play\n");

    // ---- init ---------------------------------------------------------------
    emit(&mut a, &["sid_init:", "LDA #$00", "STA $D418", "LDX #$18", "sid_clr:", "STA $D400,X", "DEX", "BPL sid_clr"]);
    a.push_str(&format!("    LDX #${:02X}\n", state_size - 1));
    emit(&mut a, &[
        "sid_clr_state:", "STA sid_state,X", "DEX", "BPL sid_clr_state", "LDA #$FF", "STA sid_fown",
        "LDA sid_speed", "STA sid_tick", "STA sid_spd", "LDX sid_order", "JSR sid_setpat", "RTS",
        "sid_setpat:", "TXA", "ASL A", "ASL A", "ASL A", "ASL A", "ASL A", "STA sid_poff", "RTS",
        // one frame: a row step every sid_spd frames, then the per-frame instrument work
        "sid_play:", "DEC sid_tick", "BNE sid_play_norow", "LDA sid_spd", "STA sid_tick", "JSR sid_rowstep",
        "sid_play_norow:", "JSR sid_hr_check", "JMP sid_update",
    ]);

    // ---- row step ---------------------------------------------------------------
    emit(&mut a, &["sid_rowstep:", "LDA sid_row", "CLC", "ADC sid_poff", "STA sid_idx"]);
    for v in 0..3usize {
        a.push_str(&format!(
            "    LDY sid_idx\n    LDA sid_n{v},Y\n    BEQ sid_skip{v}\n    STA sid_tmp\n    LDA sid_s{v},Y\n    TAX\n    LDA #${v:02X}\n    STA sid_cv\n    LDA sid_tmp\n    JSR sid_set_voice\nsid_skip{v}:\n"
        ));
        // effect column: any effect but V stops the effect vibrato
        a.push_str(&format!(
            "    LDY sid_idx\n    LDA sid_x{v},Y\n    STA sid_fxv\n    LDX #${v:02X}\n    LDA sid_f{v},Y\n    STA sid_tmp\n    CMP #$01\n    BEQ sid_fxv{v}\n    LDA #$00\n    STA sid_fvol,X\n    STA sid_fvoh,X\n    LDA sid_tmp\n    BEQ sid_fd{v}\n"
        ));
        a.push_str(&format!(
            "    CMP #$04\n    BNE sid_fxnc{v}\n    LDA #$00\n    STA sid_gate,X\n    JMP sid_fd{v}\nsid_fxnc{v}:\n    CMP #$05\n    BNE sid_fxnf{v}\n    LDA sid_fxv\n    BEQ sid_fd{v}\n    STA sid_spd\n    JMP sid_fd{v}\nsid_fxnf{v}:\n    CMP #$02\n    BNE sid_fxnu{v}\n    JSR sid_fx_up\n    JMP sid_fd{v}\nsid_fxnu{v}:\n    CMP #$03\n    BNE sid_fd{v}\n    JSR sid_fx_down\n    JMP sid_fd{v}\nsid_fxv{v}:\n    JSR sid_fx_vib\nsid_fd{v}:\n"
        ));
    }
    a.push_str(&format!(
        "    INC sid_row\n    LDA sid_row\n    CMP #$20\n    BCC sid_rdone\n    LDA #$00\n    STA sid_row\n    INC sid_opos\n    LDX sid_opos\n    CPX #${:02X}\n    BCC sid_ook\n    LDX #$00\n    STX sid_opos\nsid_ook:\n    LDA sid_order,X\n    TAX\n    JSR sid_setpat\nsid_rdone:\n    RTS\n",
        def.order.len()
    ));

    // ---- effect column routines (X = voice, sid_fxv = value) ------------------
    emit(&mut a, &[
        "sid_fx_vib:", "LDA sid_fxv", "LSR A", "LSR A", "LSR A", "LSR A", "BNE sid_fxvib_sp", "LDA #$01",
        "sid_fxvib_sp:", "CMP sid_fvc,X", "BNE sid_fxvib_hold", "LDA #$00", "STA sid_fvc,X", "LDA sid_fvs,X", "EOR #$FF",
        "STA sid_fvs,X", "JMP sid_fxvib_apply", "sid_fxvib_hold:", "INC sid_fvc,X", "sid_fxvib_apply:", "LDA sid_fxv",
        "AND #$0F", "ASL A", "ASL A", "STA sid_tmp", "LDA sid_fvs,X", "BEQ sid_fxvib_neg", "LDA sid_tmp", "STA sid_fvol,X",
        "LDA #$00", "STA sid_fvoh,X", "RTS", "sid_fxvib_neg:", "LDA #$00", "SEC", "SBC sid_tmp", "STA sid_fvol,X", "LDA #$00",
        "SBC #$00", "STA sid_fvoh,X", "RTS",
        "sid_fx_up:", "CLC", "LDA sid_sll,X", "ADC sid_fxv", "STA sid_sll,X", "LDA sid_slh,X", "ADC #$00", "STA sid_slh,X", "RTS",
        "sid_fx_down:", "SEC", "LDA sid_sll,X", "SBC sid_fxv", "STA sid_sll,X", "LDA sid_slh,X", "SBC #$00", "STA sid_slh,X", "RTS",
    ]);

    // ---- note on (A = note 1..96, X = instrument, sid_cv = voice) --------------
    emit(&mut a, &[
        "sid_set_voice:", "SEC", "SBC #$01", "STA sid_tmp", "STX sid_ci", "LDX sid_cv", "LDY sid_vbase,X", "LDA sid_ctl,X",
        "AND #$FE", "STA sid_ctl,X", "STA $D404,Y", "LDA sid_tmp", "STA sid_note,X", "LDA sid_ci", "STA sid_inst,X", "LDA #$01",
        "STA sid_on,X", "STA sid_gate,X", "STA sid_jon,X", "STA sid_vdr,X", "LDA #$00", "STA sid_sll,X", "STA sid_slh,X",
        "STA sid_fvol,X", "STA sid_fvoh,X", "STA sid_fvs,X", "STA sid_fvc,X", "STA sid_pwd,X", "STA sid_vofl,X", "STA sid_vofh,X",
        "STA sid_tpos,X", "STA sid_nof,X", "STA sid_nab,X", "LDY sid_ci", "LDA sid_iffw,Y", "STA sid_ffr,X", "LDA sid_ipwl,Y",
        "STA sid_pwl,X", "LDA sid_ipwh,Y", "STA sid_pwh,X", "LDA sid_ivdl,Y", "STA sid_vdl,X", "LDA sid_ivsp,Y", "CLC", "ADC #$01",
        "LSR A", "BNE sid_sv_vcn", "LDA #$01", "sid_sv_vcn:", "STA sid_vcn,X", "LDA sid_itsp,Y", "STA sid_tcnt,X", "LDA sid_ictl,Y",
        "STA sid_wav,X",
        // vibrato step = (semitone(note) * depth) >> 5
        "LDA sid_ivdp,Y", "STA sid_t3", "LDA #$00", "STA sid_vstl,X", "STA sid_vsth,X", "LDY sid_tmp", "LDA sid_slo,Y",
        "STA sid_m0", "LDA sid_shi,Y", "STA sid_m1", "sid_sv_mul:", "LSR sid_t3", "BCC sid_sv_noadd", "CLC", "LDA sid_vstl,X",
        "ADC sid_m0", "STA sid_vstl,X", "LDA sid_vsth,X", "ADC sid_m1", "STA sid_vsth,X", "sid_sv_noadd:", "ASL sid_m0",
        "ROL sid_m1", "LDA sid_t3", "BNE sid_sv_mul", "LDY #$05", "sid_sv_shr:", "LSR sid_vsth,X", "ROR sid_vstl,X", "DEY",
        "BNE sid_sv_shr", "LDY sid_ci", "LDA sid_itln,Y", "BEQ sid_sv_notab", "LDA sid_itst,Y", "JSR sid_tab_apply", "LDY sid_ci",
        "sid_sv_notab:", "LDA sid_iad,Y", "STA sid_t0", "LDA sid_isr,Y", "LDY sid_vbase,X", "STA $D406,Y", "LDA sid_t0",
        "STA $D405,Y", "LDY sid_ci", "LDA sid_ivol,Y", "STA sid_vol", "LDA sid_ifl,Y", "AND #$01", "BEQ sid_sv_nofilt",
        // this voice takes over the filter: cutoff sweep, resonance, mode
        "STX sid_fown", "LDA sid_icul,Y", "STA sid_fcl", "STA sid_fsl", "LDA sid_icuh,Y", "STA sid_fch", "STA sid_fsh",
        "LDA sid_icel,Y", "STA sid_fel", "STA sid_ftl", "LDA sid_iceh,Y", "STA sid_feh", "STA sid_fth", "LDA sid_icsp,Y",
        "STA sid_fspd", "LDA sid_ifl,Y", "AND #$02", "STA sid_fping", "LDA sid_ires,Y", "STA sid_fres", "LDA sid_imode,Y",
        "STA sid_fmode", "LDA sid_vbit,X", "ORA sid_froute", "STA sid_froute", "RTS", "sid_sv_nofilt:", "LDA sid_vbit,X",
        "EOR #$FF", "AND sid_froute", "STA sid_froute", "RTS",
        // A = table step, X = voice: waveform (0 = keep) and note offset
        "sid_tab_apply:", "TAY", "LDA sid_tw,Y", "BEQ sid_ta_keep", "AND #$FE", "STA sid_wav,X", "sid_ta_keep:", "LDA sid_tn,Y",
        "STA sid_nof,X", "LDA sid_ta,Y", "STA sid_nab,X", "RTS",
    ]);

    // ---- gate timer / hard restart: gt frames before the next new note --------
    a.push_str("sid_hr_check:\n");
    for v in 0..3usize {
        let ad = 0xD405 + 7 * v;
        a.push_str(&format!(
            "    LDX #${v:02X}\n    LDA sid_on,X\n    BEQ sid_hr{v}\n    LDY sid_inst,X\n    LDA sid_igt,Y\n    BEQ sid_hr{v}\n    CMP sid_tick\n    BNE sid_hr{v}\n    CMP sid_spd\n    BEQ sid_hr{v}\n    LDA sid_row\n    CLC\n    ADC sid_poff\n    TAY\n    LDA sid_n{v},Y\n    BEQ sid_hr{v}\n    LDA #$00\n    STA sid_gate,X\n    LDY sid_inst,X\n    LDA sid_ihr,Y\n    BEQ sid_hr{v}\n    LDA #$00\n    STA ${ad:04X}\n    STA ${ad1:04X}\nsid_hr{v}:\n",
            ad1 = ad + 1
        ));
    }
    a.push_str("    RTS\n");

    // ---- per-frame update: table, pitch, vibrato, pulse width, filter ---------
    emit(&mut a, &[
        "sid_update:", "LDX #$00", "sid_up_loop:", "LDA sid_on,X", "BNE sid_up_on", "JMP sid_up_next", "sid_up_on:",
        "LDY sid_inst,X", "STY sid_ci", "LDA sid_jon,X", "BEQ sid_up_tab", "LDA #$00", "STA sid_jon,X", "LDA sid_ffr,X",
        "BEQ sid_up_wgate", "PHA", "LDA #$00", "STA sid_ffr,X", "PLA", "JMP sid_up_wctl", "sid_up_tab:", "LDA sid_itln,Y",
        "BEQ sid_up_wgate", "DEC sid_tcnt,X", "BNE sid_up_wgate", "LDA sid_itsp,Y", "STA sid_tcnt,X", "INC sid_tpos,X",
        "LDA sid_tpos,X", "CMP sid_itln,Y", "BCC sid_up_tstep", "LDA sid_itlp,Y", "CMP #$FF", "BNE sid_up_tloop",
        "LDA sid_itln,Y", "SEC", "SBC #$01", "sid_up_tloop:", "STA sid_tpos,X", "sid_up_tstep:", "LDA sid_tpos,X", "CLC",
        "ADC sid_itst,Y", "JSR sid_tab_apply", "LDY sid_ci", "sid_up_wgate:", "LDA sid_wav,X", "ORA sid_gate,X",
        "sid_up_wctl:", "STA sid_ctl,X", "LDY sid_vbase,X", "STA $D404,Y",
        // note index: absolute table note, or note + signed offset, clamped 0..95
        "LDA sid_nab,X", "BEQ sid_up_rel", "LDA sid_nof,X", "JMP sid_up_nclamp", "sid_up_rel:", "LDA sid_nof,X",
        "BMI sid_up_nneg", "CLC", "ADC sid_note,X", "JMP sid_up_nclamp", "sid_up_nneg:", "CLC", "ADC sid_note,X",
        "BCS sid_up_nclamp", "LDA #$00", "sid_up_nclamp:", "CMP #$60", "BCC sid_up_nok", "LDA #$5F", "sid_up_nok:", "TAY",
        // frequency = table + slide + effect vibrato + instrument vibrato
        "CLC", "LDA sid_flo,Y", "ADC sid_sll,X", "STA sid_t0", "LDA sid_fhi,Y", "ADC sid_slh,X", "STA sid_t1", "CLC",
        "LDA sid_t0", "ADC sid_fvol,X", "STA sid_t0", "LDA sid_t1", "ADC sid_fvoh,X", "STA sid_t1", "CLC", "LDA sid_t0",
        "ADC sid_vofl,X", "STA sid_t0", "LDA sid_t1", "ADC sid_vofh,X", "LDY sid_vbase,X", "STA $D401,Y", "LDA sid_t0",
        "STA $D400,Y",
        // instrument vibrato: after the delay a triangle LFO, flipping every speed frames
        "LDY sid_ci", "LDA sid_ivsp,Y", "BEQ sid_up_novib", "LDA sid_ivdp,Y", "BEQ sid_up_novib", "LDA sid_vdl,X",
        "BEQ sid_up_vrun", "DEC sid_vdl,X", "JMP sid_up_novib", "sid_up_vrun:", "LDA sid_vdr,X", "BEQ sid_up_vdown", "CLC",
        "LDA sid_vofl,X", "ADC sid_vstl,X", "STA sid_vofl,X", "LDA sid_vofh,X", "ADC sid_vsth,X", "STA sid_vofh,X",
        "JMP sid_up_vcnt", "sid_up_vdown:", "SEC", "LDA sid_vofl,X", "SBC sid_vstl,X", "STA sid_vofl,X", "LDA sid_vofh,X",
        "SBC sid_vsth,X", "STA sid_vofh,X", "sid_up_vcnt:", "DEC sid_vcn,X", "BNE sid_up_novib", "LDA sid_ivsp,Y",
        "STA sid_vcn,X", "LDA sid_vdr,X", "EOR #$01", "STA sid_vdr,X", "sid_up_novib:",
        // pulse width out, then the PWM sweep bouncing between min and max
        "LDY sid_vbase,X", "LDA sid_pwl,X", "STA $D402,Y", "LDA sid_pwh,X", "STA $D403,Y", "LDY sid_ci", "LDA sid_ipms,Y",
        "BEQ sid_up_next", "STA sid_t0", "LDA sid_pwd,X", "BNE sid_up_pwdn", "CLC", "LDA sid_pwl,X", "ADC sid_t0",
        "STA sid_pwl,X", "LDA sid_pwh,X", "ADC #$00", "STA sid_pwh,X", "CMP sid_ipmxh,Y", "BCC sid_up_next",
        "BNE sid_up_pwtop", "LDA sid_pwl,X", "CMP sid_ipmxl,Y", "BCC sid_up_next", "sid_up_pwtop:", "LDA sid_ipmxl,Y",
        "STA sid_pwl,X", "LDA sid_ipmxh,Y", "STA sid_pwh,X", "LDA #$01", "STA sid_pwd,X", "JMP sid_up_next", "sid_up_pwdn:",
        "SEC", "LDA sid_pwl,X", "SBC sid_t0", "STA sid_pwl,X", "LDA sid_pwh,X", "SBC #$00", "STA sid_pwh,X",
        "BCC sid_up_pwbot", "CMP sid_ipmnh,Y", "BCC sid_up_pwbot", "BNE sid_up_next", "LDA sid_pwl,X", "CMP sid_ipmnl,Y",
        "BEQ sid_up_pwbot", "BCS sid_up_next", "sid_up_pwbot:", "LDA sid_ipmnl,Y", "STA sid_pwl,X", "LDA sid_ipmnh,Y",
        "STA sid_pwh,X", "LDA #$00", "STA sid_pwd,X", "sid_up_next:", "INX", "CPX #$03", "BCS sid_up_filter",
        "JMP sid_up_loop",
        // filter registers, then the cutoff sweep (bounce with ping-pong)
        "sid_up_filter:", "LDA sid_fcl", "AND #$07", "STA $D415", "LDA sid_fch", "STA sid_t1", "LDA sid_fcl", "LSR sid_t1",
        "ROR A", "LSR sid_t1", "ROR A", "LSR sid_t1", "ROR A", "STA $D416", "LDA sid_fres", "ORA sid_froute", "STA $D417",
        "LDA sid_fmode", "ORA sid_vol", "STA $D418", "LDA sid_fown", "BMI sid_fl_ret", "LDA sid_fspd", "BNE sid_fl_go",
        "sid_fl_ret:", "RTS", "sid_fl_go:", "BMI sid_fl_neg", "CLC", "ADC sid_fcl", "STA sid_fcl", "LDA sid_fch", "ADC #$00",
        "STA sid_fch", "CMP sid_fth", "BCC sid_fl_ret", "BNE sid_fl_reach", "LDA sid_fcl", "CMP sid_ftl", "BCC sid_fl_ret",
        "JMP sid_fl_reach", "sid_fl_neg:", "CLC", "ADC sid_fcl", "STA sid_fcl", "LDA sid_fch", "ADC #$FF", "STA sid_fch",
        "BMI sid_fl_reach", "CMP sid_fth", "BCC sid_fl_reach", "BNE sid_fl_ret", "LDA sid_fcl", "CMP sid_ftl",
        "BEQ sid_fl_reach", "BCS sid_fl_ret", "sid_fl_reach:", "LDA sid_ftl", "STA sid_fcl", "LDA sid_fth", "STA sid_fch",
        "LDA sid_fping", "BNE sid_fl_bounce", "LDA #$00", "STA sid_fspd", "RTS", "sid_fl_bounce:", "LDA sid_ftl",
        "CMP sid_fel", "BNE sid_fl_toend", "LDA sid_fth", "CMP sid_feh", "BNE sid_fl_toend", "LDA sid_fsl", "STA sid_ftl",
        "LDA sid_fsh", "STA sid_fth", "JMP sid_fl_flip", "sid_fl_toend:", "LDA sid_fel", "STA sid_ftl", "LDA sid_feh",
        "STA sid_fth", "sid_fl_flip:", "LDA #$00", "SEC", "SBC sid_fspd", "STA sid_fspd", "RTS",
    ]);

    // ---- data -------------------------------------------------------------------
    hexlines(&mut a, "sid_speed", &[def.speed.max(1)]);
    hexlines(&mut a, "sid_order", &def.order);
    hexlines(&mut a, "sid_vbase", &[0, 7, 14]);
    hexlines(&mut a, "sid_vbit", &[1, 2, 4]);
    let tables = inst_tables(&insts);
    for (name, col) in &tables.cols {
        hexlines(&mut a, &format!("sid_{}", name), col);
    }
    hexlines(&mut a, "sid_tw", &tables.tw);
    hexlines(&mut a, "sid_tn", &tables.tn);
    hexlines(&mut a, "sid_ta", &tables.ta);
    for v in 0..3 {
        let mut n = Vec::new();
        let mut s = Vec::new();
        let mut f = Vec::new();
        let mut x = Vec::new();
        for p in &def.patterns {
            for c in p[v].iter() {
                n.push(c.note);
                s.push(if (c.inst as usize) < ninst { c.inst } else { 0 });
                f.push(c.fx);
                x.push(c.fxval);
            }
        }
        hexlines(&mut a, &format!("sid_n{}", v), &n);
        hexlines(&mut a, &format!("sid_s{}", v), &s);
        hexlines(&mut a, &format!("sid_f{}", v), &f);
        hexlines(&mut a, &format!("sid_x{}", v), &x);
    }
    let (freq, semi) = freq_tables();
    hexlines(&mut a, "sid_flo", &freq.iter().map(|f| (f & 0xFF) as u8).collect::<Vec<_>>());
    hexlines(&mut a, "sid_fhi", &freq.iter().map(|f| (f >> 8) as u8).collect::<Vec<_>>());
    hexlines(&mut a, "sid_slo", &semi.iter().map(|f| (f & 0xFF) as u8).collect::<Vec<_>>());
    hexlines(&mut a, "sid_shi", &semi.iter().map(|f| (f >> 8) as u8).collect::<Vec<_>>());
    a.push_str("sid_state:\n");
    for n in STATE_V {
        hexlines(&mut a, &format!("sid_{}", n), &[0, 0, 0]);
    }
    for n in STATE_G {
        hexlines(&mut a, &format!("sid_{}", n), &[0]);
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
    fn table_steps() {
        assert_eq!(parse_table_step("41 +4").unwrap(), (0x41, 4, false));
        assert_eq!(parse_table_step(".. -12").unwrap(), (0, -12, false));
        assert_eq!(parse_table_step("81 =60").unwrap(), (0x81, 60, true));
        assert_eq!(parse_table_step("21").unwrap(), (0x21, 0, false));
        assert!(parse_table_step("81 =99").is_err());
        assert!(parse_table_step("zz +1").is_err());
    }

    #[test]
    fn blob_assembles_with_all_labels_resolved() {
        let mut def = TuneDef::default();
        def.insts.push(Instrument { ctrl: 0x40, ad: 0x09, sr: 0xF0, pw: 0x800, cutoff: 0x100, res_filt: 0x21, mode_vol: 0x1F, ..Instrument::default() });
        def.order = vec![0, 1, 0];
        let mut p = [[Cell::default(); ROWS]; 3];
        p[0][0] = Cell { note: 49, inst: 0, fx: 1, fxval: 0x24 };
        def.patterns = vec![p, p];
        let asm = build_asm(&def).unwrap();
        let bytes = build(&def, 0x8000).unwrap();
        assert_eq!(&bytes[0..3], &[0x4C, 0x06, 0x80]); // JMP sid_init (right after the two JMPs)
        // no JMP/JSR may point at $0000 (unresolved label)
        let mut i = 0;
        while i + 2 < bytes.len() && i < 1800 {
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
