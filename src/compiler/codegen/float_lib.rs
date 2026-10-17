//! Floating-point runtime (1.6.4): 5-byte numbers in the C64 BASIC format,
//! assembled from source by the strict internal assembler (`rtasm`) and
//! emitted once, after the program, when a program uses SINGLE / DOUBLE.
//!
//! Packed (memory, 5 bytes): exponent (0 = zero, bias 128, mantissa 0.1xxx…),
//! then 4 mantissa bytes, most significant first; the top bit of the first
//! mantissa byte (always 1 for a normalised number) holds the sign.
//!
//! Unpacked accumulator FAC and operand ARG (7 bytes each, RAM):
//! exponent, 4 mantissa bytes (explicit top bit), sign ($00 / $80) and a
//! rounding byte that extends the mantissa by 8 guard bits.
//!
//! Calling convention: memory operands in A (lo) / Y (hi). Only the 2-byte
//! zero-page pointer `PTR` is needed from the code generator.

use std::collections::HashMap;

/// f64 → packed 5-byte float (round to nearest; saturates; tiny → 0).
pub fn to_mflpt(x: f64) -> [u8; 5] {
    if x == 0.0 || !x.is_finite() && x.is_nan() {
        return [0; 5];
    }
    let neg = x < 0.0;
    let mut a = x.abs();
    if !a.is_finite() {
        a = f64::MAX;
    }
    // a = m * 2^e with m in [0.5, 1)
    let mut e = a.log2().floor() as i32 + 1;
    let mut m = a / 2f64.powi(e);
    while m >= 1.0 {
        m /= 2.0;
        e += 1;
    }
    while m < 0.5 {
        m *= 2.0;
        e -= 1;
    }
    let mut mant = (m * 4294967296.0).round() as u64; // 32 bits
    if mant >= 1 << 32 {
        mant >>= 1;
        e += 1;
    }
    let exp = e + 128;
    if exp <= 0 {
        return [0; 5];
    }
    if exp > 255 {
        return [0xFF, if neg { 0xFF } else { 0x7F }, 0xFF, 0xFF, 0xFF];
    }
    let mb = (mant as u32).to_be_bytes();
    [
        exp as u8,
        (mb[0] & 0x7F) | if neg { 0x80 } else { 0 },
        mb[1],
        mb[2],
        mb[3],
    ]
}

/// Packed 5-byte float → f64 (exact).
pub fn from_mflpt(b: [u8; 5]) -> f64 {
    if b[0] == 0 {
        return 0.0;
    }
    let mant = u32::from_be_bytes([b[1] | 0x80, b[2], b[3], b[4]]) as f64 / 4294967296.0;
    let v = mant * 2f64.powi(b[0] as i32 - 128);
    if b[1] & 0x80 != 0 {
        -v
    } else {
        v
    }
}

fn bytes(name: &str, b: &[u8]) -> String {
    let list: Vec<String> = b.iter().map(|x| format!("${x:02X}")).collect();
    format!("{name}: .byte {}\n", list.join(", "))
}

/// Assemble the whole runtime at `base`, with the 2-byte zero-page pointer
/// `ptr`. Returns the code and every label (entry points `f_*`, data).
pub fn float_runtime(base: u16, ptr: u8) -> (Vec<u8>, HashMap<String, u16>) {
    assemble_runtime(&runtime_source(), base, ptr)
}

/// Assemble only the parts of the runtime that `entries` reach (by calls,
/// jumps, data references and fall-through), so a program that only adds and
/// prints does not carry SIN / LOG / EXP / text input.
pub fn float_runtime_for(base: u16, ptr: u8, entries: &[&str]) -> (Vec<u8>, HashMap<String, u16>) {
    assemble_runtime(&prune(&runtime_source(), entries), base, ptr)
}

fn assemble_runtime(src: &str, base: u16, ptr: u8) -> (Vec<u8>, HashMap<String, u16>) {
    let mut known = HashMap::new();
    known.insert("PTR".to_string(), ptr as u16);
    let a = super::rtasm::assemble(src, base, &known)
        .unwrap_or_else(|e| panic!("float runtime does not assemble: {e}"));
    (a.code, a.labels)
}

/// Symbols used on an (already comment-stripped) line: identifiers outside
/// character literals and numbers. The caller filters for defined names.
fn idents(line: &str) -> Vec<&str> {
    let b = line.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                i += 1;
            }
            i += 1;
        } else if c == b'$' || c == b'%' || c.is_ascii_digit() {
            i += 1;
            while i < b.len() && b[i].is_ascii_alphanumeric() {
                i += 1;
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(&line[st..i]);
        } else {
            i += 1;
        }
    }
    out
}

/// Keep the chunks of `src` (a chunk starts at a label in column 0) that
/// `roots` reach. A chunk whose last instruction is not RTS / JMP / RTI falls
/// through into the next one, which is then needed too. Constant lines
/// (`NAME = expr`) are transparent: using NAME uses what its expression uses,
/// and the line is kept when all of those are kept.
fn prune(src: &str, roots: &[&str]) -> String {
    struct Chunk<'a> {
        lines: Vec<&'a str>,
        refs: Vec<&'a str>,
        falls: bool,
    }
    let mut equs: Vec<(&str, &str, Vec<&str>)> = vec![]; // (line, name, uses)
    let mut chunks: Vec<Chunk> = vec![];
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for raw in src.lines() {
        let line = super::rtasm::strip(raw);
        if line.is_empty() {
            continue;
        }
        if let Some((name, expr)) = line.split_once('=') {
            equs.push((line, name.trim(), idents(expr)));
            continue;
        }
        let mut body = line;
        if !raw.starts_with([' ', '\t']) {
            if let Some((label, rest)) = line.split_once(':') {
                chunks.push(Chunk { lines: vec![], refs: vec![], falls: true });
                owner.insert(label.trim(), chunks.len() - 1);
                body = rest.trim();
            }
        }
        let c = chunks.last_mut().expect("runtime code before the first label");
        c.lines.push(raw);
        if body.is_empty() {
            continue;
        }
        let mnem = body.split_whitespace().next().unwrap_or("");
        c.falls = !matches!(mnem, "RTS" | "JMP" | "RTI" | ".byte" | ".word" | ".res");
        let ops = body[mnem.len()..].trim();
        c.refs.extend(idents(ops));
    }
    let equ_uses: HashMap<&str, &Vec<&str>> = equs.iter().map(|(_, n, u)| (*n, u)).collect();
    let mut keep = vec![false; chunks.len()];
    let mut work: Vec<&str> = roots.to_vec();
    let mut seen_equ: Vec<&str> = vec![];
    while let Some(sym) = work.pop() {
        if let Some(uses) = equ_uses.get(sym) {
            if !seen_equ.contains(&sym) {
                seen_equ.push(sym);
                work.extend(uses.iter());
            }
            continue;
        }
        let Some(&start) = owner.get(sym) else { continue };
        let mut i = start;
        loop {
            if keep[i] {
                break;
            }
            keep[i] = true;
            work.extend(chunks[i].refs.iter());
            if !chunks[i].falls || i + 1 == chunks.len() {
                break;
            }
            i += 1;
        }
    }
    let kept = |n: &str| owner.get(n).is_none_or(|&i| keep[i]) || equ_uses.contains_key(n);
    let mut out = String::new();
    for (line, _, uses) in &equs {
        if uses.iter().all(|u| kept(u)) {
            out += line;
            out.push('\n');
        }
    }
    for (c, _) in chunks.iter().zip(&keep).filter(|(_, k)| **k) {
        for l in &c.lines {
            out += l;
            out.push('\n');
        }
    }
    out
}

/// RAM state of an assembled runtime that an interrupt handler must save:
/// from its first data label up to the float stack (or `end`).
pub fn state_range(labels: &HashMap<String, u16>, end: u16) -> (u16, u16) {
    let data = DATA.lines().filter_map(|l| l.split_once(':').map(|(n, _)| n.trim()));
    let start = data.filter_map(|n| labels.get(n).copied()).min().unwrap_or(end);
    let stop = labels.get("fstk").copied().unwrap_or(end);
    (start, stop.max(start))
}

fn runtime_source() -> String {
    let mut src = String::from(LIB);
    // constants
    src += &bytes("c_one", &to_mflpt(1.0));
    src += &bytes("c_half", &to_mflpt(0.5));
    src += &bytes("c_ten", &to_mflpt(10.0));
    src += &bytes("c_1e8", &to_mflpt(1e8));
    src += &bytes("c_1e9", &to_mflpt(1e9));
    let mut p10 = vec![];
    for k in (0..9).rev() {
        p10.extend_from_slice(&(10u32.pow(k)).to_be_bytes());
    }
    src += &bytes("p10", &p10);
    let u10: Vec<u8> = (0..10).rev().flat_map(|k| 10u32.pow(k).to_be_bytes()).collect();
    src += &bytes("u10", &u10);
    let mut pw = vec![];
    for k in 0..=17 {
        pw.extend_from_slice(&to_mflpt(10f64.powi(k)));
    }
    src += &bytes("pw", &pw);
    for (name, v) in [
        ("c_sqh", 0.5f64.sqrt()),
        ("c_ln2", std::f64::consts::LN_2),
        ("c_log2e", std::f64::consts::LOG2_E),
        ("c_pi2", std::f64::consts::FRAC_PI_2),
        ("c_pi6", std::f64::consts::FRAC_PI_6),
        ("c_2pi", 2.0 * std::f64::consts::PI),
        ("c_i2pi", 0.5 / std::f64::consts::PI),
        ("c_q", 0.25),
        ("c_3q", 0.75),
        ("c_t15", 2.0 - 3f64.sqrt()),
        ("c_sq3", 3f64.sqrt()),
    ] {
        src += &bytes(name, &to_mflpt(v));
    }
    let fact = |n: i32| (1..=n).map(f64::from).product::<f64>();
    // polynomials (degree, then coefficients from the highest power)
    src += &poly("t_log", (0..8).rev().map(|k| 1.0 / (2 * k + 1) as f64));
    src += &poly("t_exp", (0..10).rev().map(|k| 1.0 / fact(k)));
    src += &poly(
        "t_sin",
        (0..8).rev().map(|k| if k % 2 == 0 { 1.0 } else { -1.0 } / fact(2 * k + 1)),
    );
    src += &poly(
        "t_atn",
        (0..8).rev().map(|k| if k % 2 == 0 { 1.0 } else { -1.0 } / (2 * k + 1) as f64),
    );
    src += DATA;
    src
}

/// `name: .byte degree, c_n, …, c_0` (packed coefficients, highest power first).
fn poly(name: &str, coeffs: impl Iterator<Item = f64>) -> String {
    let cs: Vec<f64> = coeffs.collect();
    let mut b = vec![(cs.len() - 1) as u8];
    for c in cs {
        b.extend_from_slice(&to_mflpt(c));
    }
    bytes(name, &b)
}

const DATA: &str = "
pp:     .res 2
mda:    .res 5
mdb:    .res 5
rt:     .res 4
uw:     .res 1
ud:     .res 1
uf:     .res 1
usg:    .res 1
uk:     .res 1
ue:     .res 1
ufst:   .res 1
ulen:   .res 1
ugl:    .res 1
usc:    .res 1
uneed:  .res 1
uidx:   .res 1
ucnt:   .res 1
ux:     .res 5
udg:    .res 10
ubuf:   .res 48
pn:     .res 1
px2:    .res 5
pk:     .res 1
pb:     .res 5
px:     .res 5
py:     .res 5
pneg:   .res 1
sx:     .res 5
sg:     .res 5
sqn:    .res 1
lge:    .res 1
lgm:    .res 5
lgt:    .res 5
lgs:    .res 5
ext:    .res 5
exn:    .res 5
exi:    .res 2
snq:    .res 5
snu:    .res 5
tnx:    .res 5
tnc:    .res 5
atsg:   .res 1
atfl:   .res 1
atx:    .res 5
att:    .res 5
fac:    .res 7
arg:    .res 7
mc:     .res 5
r0:     .res 1
tq:     .res 5
te:     .res 2
fxflag: .res 1
tmask:  .res 1
fsp:    .res 1
dexp:   .res 1
epow:   .res 1
tp:     .res 2
oidx:   .res 1
lastd:  .res 1
ix:     .res 1
neg:    .res 1
dot:    .res 1
ex:     .res 1
exneg:  .res 1
ftmp:   .res 5
digs:   .res 9
obuf:   .res 24
ibuf:   .res 32
fstk:   .res 80
";

const LIB: &str = r#"
FACE = fac
FAC1 = fac+1
FAC2 = fac+2
FAC3 = fac+3
FAC4 = fac+4
FACS = fac+5
FACR = fac+6
ARGE = arg
ARG1 = arg+1
ARG2 = arg+2
ARG3 = arg+3
ARG4 = arg+4
ARGS = arg+5
ARGR = arg+6

; ---------------------------------------------------------------- load / store
f_load:             ; FAC = mem(A/Y)
        STA PTR
        STY PTR+1
        LDY #0
        LDA (PTR),Y
        STA FACE
        INY
        LDA (PTR),Y
        TAX
        AND #$80
        STA FACS
        TXA
        ORA #$80
        STA FAC1
        INY
        LDA (PTR),Y
        STA FAC2
        INY
        LDA (PTR),Y
        STA FAC3
        INY
        LDA (PTR),Y
        STA FAC4
        LDA #0
        STA FACR
        RTS

f_ldarg:            ; ARG = mem(A/Y)
        STA PTR
        STY PTR+1
        LDY #0
        LDA (PTR),Y
        STA ARGE
        INY
        LDA (PTR),Y
        TAX
        AND #$80
        STA ARGS
        TXA
        ORA #$80
        STA ARG1
        INY
        LDA (PTR),Y
        STA ARG2
        INY
        LDA (PTR),Y
        STA ARG3
        INY
        LDA (PTR),Y
        STA ARG4
        LDA #0
        STA ARGR
        RTS

f_store:            ; mem(A/Y) = FAC (rounded)
        STA PTR
        STY PTR+1
        JSR f_round
        LDY #0
        LDA FACE
        STA (PTR),Y
        BEQ st_zero
        INY
        LDA FAC1
        AND #$7F
        ORA FACS
        STA (PTR),Y
        INY
        LDA FAC2
        STA (PTR),Y
        INY
        LDA FAC3
        STA (PTR),Y
        INY
        LDA FAC4
        STA (PTR),Y
        RTS
st_zero:
        INY
        STA (PTR),Y
        CPY #4
        BNE st_zero
        RTS

f_round:            ; round FAC to 32 mantissa bits, FACR = 0
        LDA FACE
        BEQ rn_clr
        LDA FACR
        BPL rn_clr
        INC FAC4
        BNE rn_clr
        INC FAC3
        BNE rn_clr
        INC FAC2
        BNE rn_clr
        INC FAC1
        BNE rn_clr
        LDA #$80
        STA FAC1
        INC FACE
        BNE rn_clr
        JMP f_ovf
rn_clr:
        LDA #0
        STA FACR
        RTS

f_ovf:              ; FAC = +-largest number (sign kept)
        LDA #$FF
        STA FACE
        STA FAC1
        STA FAC2
        STA FAC3
        STA FAC4
        LDA #0
        STA FACR
        RTS

f_zero:             ; FAC = 0
        LDA #0
        STA FACE
        STA FACS
        STA FACR
        RTS

f_norm:             ; normalise FAC1..FACR; zero when empty or too small
        LDA FAC1
        BMI nm_done
        LDX #5
nm_byte:
        LDA FAC1
        BNE nm_bits
        LDA FAC2
        STA FAC1
        LDA FAC3
        STA FAC2
        LDA FAC4
        STA FAC3
        LDA FACR
        STA FAC4
        LDA #0
        STA FACR
        LDA FACE
        SEC
        SBC #8
        BCC nm_zero
        BEQ nm_zero
        STA FACE
        DEX
        BNE nm_byte
        JMP f_zero
nm_bits:
        LDA FAC1
        BMI nm_done
        ASL FACR
        ROL FAC4
        ROL FAC3
        ROL FAC2
        ROL FAC1
        DEC FACE
        BNE nm_bits
nm_zero:
        JMP f_zero
nm_done:
        RTS

f_swap:             ; FAC <-> ARG
        LDX #6
sw_l:   LDA fac,X
        LDY arg,X
        STA arg,X
        TYA
        STA fac,X
        DEX
        BPL sw_l
        RTS

f_argfac:           ; FAC = ARG
        LDX #6
af_l:   LDA arg,X
        STA fac,X
        DEX
        BPL af_l
        RTS

; ---------------------------------------------------------------- + -
ad_r0:  RTS
f_add:              ; FAC = FAC + mem(A/Y)
        JSR f_ldarg
        JMP f_addarg
f_sub:              ; FAC = FAC - mem(A/Y)
        JSR f_ldarg
f_subarg:
        LDA ARGS
        EOR #$80
        STA ARGS
f_addarg:           ; FAC = FAC + ARG
        LDA ARGE
        BEQ ad_r0
        LDA FACE
        BNE ad_go
        JMP f_argfac
ad_go:
        CMP ARGE
        BCS ad_ok
        JSR f_swap
ad_ok:
        LDA FACE
        SEC
        SBC ARGE
        CMP #40
        BCS ad_r0
        TAX
ad_bsh:
        CPX #8
        BCC ad_bits
        LDA ARG4
        STA ARGR
        LDA ARG3
        STA ARG4
        LDA ARG2
        STA ARG3
        LDA ARG1
        STA ARG2
        LDA #0
        STA ARG1
        TXA
        SEC
        SBC #8
        TAX
        JMP ad_bsh
ad_bits:
        CPX #0
        BEQ ad_al
ad_bl:  LSR ARG1
        ROR ARG2
        ROR ARG3
        ROR ARG4
        ROR ARGR
        DEX
        BNE ad_bl
ad_al:
        LDA FACS
        EOR ARGS
        BMI ad_diff
        CLC
        LDA FACR
        ADC ARGR
        STA FACR
        LDA FAC4
        ADC ARG4
        STA FAC4
        LDA FAC3
        ADC ARG3
        STA FAC3
        LDA FAC2
        ADC ARG2
        STA FAC2
        LDA FAC1
        ADC ARG1
        STA FAC1
        BCC ad_ret
        ROR FAC1
        ROR FAC2
        ROR FAC3
        ROR FAC4
        ROR FACR
        INC FACE
        BNE ad_ret
        JMP f_ovf
ad_ret: RTS
ad_diff:
        SEC
        LDA FACR
        SBC ARGR
        STA FACR
        LDA FAC4
        SBC ARG4
        STA FAC4
        LDA FAC3
        SBC ARG3
        STA FAC3
        LDA FAC2
        SBC ARG2
        STA FAC2
        LDA FAC1
        SBC ARG1
        STA FAC1
        BCS ad_n
        LDA FACS
        EOR #$80
        STA FACS
        SEC
        LDA #0
        SBC FACR
        STA FACR
        LDA #0
        SBC FAC4
        STA FAC4
        LDA #0
        SBC FAC3
        STA FAC3
        LDA #0
        SBC FAC2
        STA FAC2
        LDA #0
        SBC FAC1
        STA FAC1
ad_n:   JMP f_norm

; ---------------------------------------------------------------- *
ml_r0:  RTS
f_mul:              ; FAC = FAC * mem(A/Y)
        JSR f_ldarg
f_mularg:
        LDA FACE
        BEQ ml_r0
        LDA ARGE
        BNE ml_go
        JMP f_zero
ml_go:
        LDA FACS
        EOR ARGS
        STA FACS
        LDA FACE
        CLC
        ADC ARGE
        STA te
        LDA #0
        ADC #0
        STA te+1
        LDA te
        SEC
        SBC #128
        STA te
        LDA te+1
        SBC #0
        STA te+1
        LDA FAC1
        STA mc
        LDA FAC2
        STA mc+1
        LDA FAC3
        STA mc+2
        LDA FAC4
        STA mc+3
        LDA FACR            ; 40-bit multiplicand (guard byte included)
        STA mc+4
        LDA #0
        STA FAC1
        STA FAC2
        STA FAC3
        STA FAC4
        STA FACR
        LDX #32
ml_lp:
        LSR ARG1
        ROR ARG2
        ROR ARG3
        ROR ARG4
        BCC ml_sh
        CLC
        LDA FACR
        ADC mc+4
        STA FACR
        LDA FAC4
        ADC mc+3
        STA FAC4
        LDA FAC3
        ADC mc+2
        STA FAC3
        LDA FAC2
        ADC mc+1
        STA FAC2
        LDA FAC1
        ADC mc
        STA FAC1
ml_sh:
        ROR FAC1
        ROR FAC2
        ROR FAC3
        ROR FAC4
        ROR FACR
        DEX
        BNE ml_lp
        JMP f_setexp
ml_ret: RTS

f_setexp:           ; FACE = te (16-bit signed), then normalise
        LDA te+1
        BMI se_zero
        BNE se_ovf
        LDA te
        BEQ se_zero
        STA FACE
        JMP f_norm
se_zero:
        JMP f_zero
se_ovf:
        JMP f_ovf

; ---------------------------------------------------------------- /
dv_r0:  RTS
f_div:              ; FAC = FAC / mem(A/Y)
        JSR f_ldarg
f_divarg:
        LDA ARGE
        BNE dv_nz
        LDA FACS
        EOR ARGS
        STA FACS
        JMP f_ovf
dv_nz:
        LDA FACE
        BEQ dv_r0
        LDA FACS
        EOR ARGS
        STA FACS
        LDA FACE
        CLC
        ADC #129
        STA te
        LDA #0
        ADC #0
        STA te+1
        LDA te
        SEC
        SBC ARGE
        STA te
        LDA te+1
        SBC #0
        STA te+1
        LDA #0
        STA r0
        LDA FAC1
        STA mc
        LDA FAC2
        STA mc+1
        LDA FAC3
        STA mc+2
        LDA FAC4
        STA mc+3
        LDA FACR            ; 40-bit dividend (guard byte included)
        STA mc+4
        LDA mc
        CMP ARG1
        BNE dv_c1
        LDA mc+1
        CMP ARG2
        BNE dv_c1
        LDA mc+2
        CMP ARG3
        BNE dv_c1
        LDA mc+3
        CMP ARG4
        BNE dv_c1
        LDA mc+4
        CMP #0
dv_c1:
        BCS dv_start
        ASL mc+4
        ROL mc+3
        ROL mc+2
        ROL mc+1
        ROL mc
        ROL r0
        LDA te
        SEC
        SBC #1
        STA te
        LDA te+1
        SBC #0
        STA te+1
dv_start:
        LDX #40
dv_lp:
        SEC
        LDA mc+4
        SBC #0
        STA tq+4
        LDA mc+3
        SBC ARG4
        STA tq+3
        LDA mc+2
        SBC ARG3
        STA tq+2
        LDA mc+1
        SBC ARG2
        STA tq+1
        LDA mc
        SBC ARG1
        STA tq
        LDA r0
        SBC #0
        BCC dv_no
        STA r0
        LDA tq
        STA mc
        LDA tq+1
        STA mc+1
        LDA tq+2
        STA mc+2
        LDA tq+3
        STA mc+3
        LDA tq+4
        STA mc+4
dv_no:
        ROL FACR
        ROL FAC4
        ROL FAC3
        ROL FAC2
        ROL FAC1
        ASL mc+4
        ROL mc+3
        ROL mc+2
        ROL mc+1
        ROL mc
        ROL r0
        DEX
        BNE dv_lp
        JMP f_setexp

; ---------------------------------------------------------------- compare
f_cmp:              ; A = $FF / 0 / 1 for FAC < = > mem(A/Y)
        JSR f_ldarg
f_cmparg:
        JSR f_round
f_cmpraw:           ; compare FAC (all 40 bits) with ARG, FAC unchanged
        LDA FACE
        BNE cp_fnz
        LDA ARGE
        BEQ cp_eq
        LDA ARGS
        BMI cp_gt
        BPL cp_lt
cp_fnz:
        LDA ARGE
        BNE cp_both
        LDA FACS
        BMI cp_lt
        BPL cp_gt
cp_both:
        LDA FACS
        CMP ARGS
        BEQ cp_same
        LDA FACS
        BMI cp_lt
        BPL cp_gt
cp_same:
        LDA FACE
        CMP ARGE
        BNE cp_mag
        LDA FAC1
        CMP ARG1
        BNE cp_mag
        LDA FAC2
        CMP ARG2
        BNE cp_mag
        LDA FAC3
        CMP ARG3
        BNE cp_mag
        LDA FAC4
        CMP ARG4
        BNE cp_mag
        LDA FACR
        CMP ARGR
        BNE cp_mag
cp_eq:  LDA #0
        RTS
cp_mag:
        BCC cp_less
        LDA FACS
        BMI cp_lt
        BPL cp_gt
cp_less:
        LDA FACS
        BMI cp_gt
cp_lt:  LDA #$FF
        RTS
cp_gt:  LDA #1
        RTS

; ---------------------------------------------------------------- integers
f_fromu8:           ; FAC = A
        LDY #0
f_fromu16:          ; FAC = A (lo) + 256 * Y, unsigned
        STY FAC1
        STA FAC2
        LDA #0
        STA FACS
        BEQ fi_go
f_froms16:          ; FAC = A (lo) + 256 * Y, signed
        STY FAC1
        STA FAC2
        LDA #0
        STA FACS
        TYA
        BPL fi_go
        LDA #$80
        STA FACS
        SEC
        LDA #0
        SBC FAC2
        STA FAC2
        LDA #0
        SBC FAC1
        STA FAC1
fi_go:
        LDA #0
        STA FAC3
        STA FAC4
        STA FACR
        LDA #144
        STA FACE
        JMP f_norm

f_fix:              ; FAC = trunc(FAC); fxflag <> 0 when a fraction was dropped
        JSR f_round
        LDA #0
        STA fxflag
        LDA FACE
        BEQ fx_ret
        CMP #129
        BCS fx_ge1
        INC fxflag
        JMP f_zero
fx_ge1:
        CMP #160
        BCS fx_ret
        LDA #160
        SEC
        SBC FACE
        TAX
        LDY #4
fx_byte:
        CPX #8
        BCC fx_bits
        LDA fac,Y
        ORA fxflag
        STA fxflag
        LDA #0
        STA fac,Y
        DEY
        TXA
        SEC
        SBC #8
        TAX
        JMP fx_byte
fx_bits:
        CPX #0
        BEQ fx_ret
        LDA #$FF
fx_m:   ASL A
        DEX
        BNE fx_m
        STA tmask
        EOR #$FF
        AND fac,Y
        ORA fxflag
        STA fxflag
        LDA fac,Y
        AND tmask
        STA fac,Y
fx_ret: RTS

f_int:              ; FAC = floor(FAC)
        LDA FACS
        PHA
        JSR f_fix
        PLA
        BPL in_ret
        LDA fxflag
        BEQ in_ret
        LDA #<c_one
        LDY #>c_one
        JMP f_sub
in_ret: RTS

f_tos16:            ; A/Y = trunc(FAC), signed 16-bit, saturating
        JSR f_fix
        LDA FACE
        BNE ts_nz
        LDA #0
        TAY
        RTS
ts_nz:
        CMP #144
        BCC ts_fit
        LDA FACS
        BMI ts_min
        LDA #$FF
        LDY #$7F
        RTS
ts_min: LDA #0
        LDY #$80
        RTS
ts_fit:
        LDA #144
        SEC
        SBC FACE
        TAX
        LDA FAC1
        STA te+1
        LDA FAC2
        STA te
ts_sh:  LSR te+1
        ROR te
        DEX
        BNE ts_sh
        LDA FACS
        BPL ts_ret
        SEC
        LDA #0
        SBC te
        STA te
        LDA #0
        SBC te+1
        STA te+1
ts_ret: LDA te
        LDY te+1
        RTS

f_tou16:            ; A/Y = trunc(FAC), unsigned 16-bit (negative: two's complement)
        LDA FACS
        BPL tu_pos
        JMP f_tos16
tu_pos:
        JSR f_fix
        LDA FACE
        BNE tu_nz
        LDA #0
        TAY
        RTS
tu_nz:  CMP #145
        BCC tu_fit
        LDA #$FF
        TAY
        RTS
tu_fit:
        LDA FAC1
        STA te+1
        LDA FAC2
        STA te
        LDA #144
        SEC
        SBC FACE
        TAX
        BEQ tu_ret
tu_sh:  LSR te+1
        ROR te
        DEX
        BNE tu_sh
tu_ret: LDA te
        LDY te+1
        RTS

f_addhalf:          ; FAC = FAC + 0.5 * sign(FAC)
        LDA FACE
        BEQ ah_ret
        LDA #<c_half
        LDY #>c_half
        JSR f_ldarg
        LDA FACS
        STA ARGS
        JMP f_addarg
ah_ret: RTS

f_rnds16:           ; A/Y = FAC rounded (half away from zero), signed 16-bit
        JSR f_addhalf
        JMP f_tos16
f_rndu16:           ; A/Y = FAC rounded, unsigned 16-bit
        JSR f_addhalf
        JMP f_tou16

; ---------------------------------------------------------------- sign
f_neg:  LDA FACE
        BEQ ng_ret
        LDA FACS
        EOR #$80
        STA FACS
ng_ret: RTS

f_abs:  LDA #0
        STA FACS
        RTS

f_sgn:  LDA FACE            ; FAC = -1 / 0 / 1
        BEQ sg_ret
        LDA #$81
        STA FACE
        LDA #$80
        STA FAC1
        LDA #0
        STA FAC2
        STA FAC3
        STA FAC4
        STA FACR
sg_ret: RTS

; ---------------------------------------------------------------- stack
f_push:             ; push FAC (16 levels)
        JSR f_round
        LDX fsp
        LDA FACE
        STA fstk,X
        LDA FAC1
        AND #$7F
        ORA FACS
        STA fstk+1,X
        LDA FAC2
        STA fstk+2,X
        LDA FAC3
        STA fstk+3,X
        LDA FAC4
        STA fstk+4,X
        TXA
        CLC
        ADC #5
        STA fsp
        RTS

f_poparg:           ; ARG = pop
        LDA fsp
        SEC
        SBC #5
        STA fsp
        TAX
        LDA fstk,X
        STA ARGE
        LDA fstk+1,X
        TAY
        AND #$80
        STA ARGS
        TYA
        ORA #$80
        STA ARG1
        LDA fstk+2,X
        STA ARG2
        LDA fstk+3,X
        STA ARG3
        LDA fstk+4,X
        STA ARG4
        LDA #0
        STA ARGR
        RTS

f_addp: JSR f_poparg        ; FAC = FAC op pop
        JMP f_addarg
f_subp: JSR f_poparg
        JMP f_subarg
f_mulp: JSR f_poparg
        JMP f_mularg
f_divp: JSR f_poparg
        JMP f_divarg
f_cmpp: JSR f_poparg
        JMP f_cmparg
f_cmpr: JSR f_ldarg         ; raw compare with mem(A/Y), FAC not rounded
        JMP f_cmpraw

; ---------------------------------------------------------------- mod
f_mod:  JSR f_ldarg         ; FAC = FAC mod mem(A/Y)
        JMP f_modarg
f_modp: JSR f_poparg        ; FAC = FAC mod pop
f_modarg:                   ; FAC = FAC - ARG * fix(FAC / ARG) (sign of FAC); ARG = 0: FAC
        LDA ARGE
        BEQ md_ret
        LDA #<mda
        LDY #>mda
        JSR f_store
        JSR f_swap
        LDA #<mdb
        LDY #>mdb
        JSR f_store
        LDA #<mda
        LDY #>mda
        JSR f_load
        LDA #<mdb
        LDY #>mdb
        JSR f_div
        JSR f_fix
        LDA #<mdb
        LDY #>mdb
        JSR f_mul
        JSR f_neg
        LDA #<mda
        LDY #>mda
        JMP f_add
md_ret: RTS

; ---------------------------------------------------------------- rnd
f_rnd:              ; FAC = random number, 0 <= x < 1 (xorshift32 13/17/5)
        LDA rnd_f
        BNE rd_go
        INC rnd_f           ; first call: mix in the raster line
        LDA $D012
        EOR rnd_s
        STA rnd_s
rd_go:  LDA #0              ; x ^= x << 13  (t = x << 8, then << 5)
        STA rt
        LDA rnd_s
        STA rt+1
        LDA rnd_s+1
        STA rt+2
        LDA rnd_s+2
        STA rt+3
        LDX #5
        JSR rd_shl
        JSR rd_xor
        LDA rnd_s+2         ; x ^= x >> 17  (t = x >> 16, then >> 1)
        STA rt
        LDA rnd_s+3
        STA rt+1
        LDA #0
        STA rt+2
        STA rt+3
        LSR rt+1
        ROR rt
        JSR rd_xor
        LDX #3              ; x ^= x << 5
rd_cp:  LDA rnd_s,X
        STA rt,X
        DEX
        BPL rd_cp
        LDX #5
        JSR rd_shl
        JSR rd_xor
        LDA rnd_s+3         ; FAC = x / 2^32
        STA FAC1
        LDA rnd_s+2
        STA FAC2
        LDA rnd_s+1
        STA FAC3
        LDA rnd_s
        STA FAC4
        LDA #$80
        STA FACE
        LDA #0
        STA FACS
        STA FACR
        JMP f_norm
rd_shl: ASL rt              ; t <<= X
        ROL rt+1
        ROL rt+2
        ROL rt+3
        DEX
        BNE rd_shl
        RTS
rd_xor: LDX #3              ; x ^= t
rd_xl:  LDA rnd_s,X
        EOR rt,X
        STA rnd_s,X
        DEX
        BPL rd_xl
        RTS
rnd_s:  .byte $5D, $E3, $27, $9A    ; state (little-endian); outside the saved IRQ state
rnd_f:  .byte 0

; ---------------------------------------------------------------- print using
f_using:            ; A/Y = PRINT USING field text of FAC (in ubuf); FAC destroyed.
                    ; A = positions before the point (sign / commas included),
                    ; X = decimals (0-9), Y = flags: 1 commas, 2 '+', 4 trailing '-', 8 point
        STA uw
        STX ud
        STY uf
        LDA #<ux
        LDY #>ux
        JSR f_store         ; (rounds FAC) the value, for an overflow
        LDA FACE
        BEQ us_pz
        LDA FACS
us_pz:  STA usg             ; $80 = negative
        LDA #0
        STA FACS
        LDA ud              ; n = round(|x| * 10^decimals)
        JSR fo_pw
        JSR f_mul
        LDA #<c_half
        LDY #>c_half
        JSR f_add
        JSR f_fix
        LDA FACE
        BEQ us_i0
        CMP #161
        BCC us_sh
        JMP us_big          ; 2^32 or more
us_sh:  CMP #160            ; mantissa -> 32-bit integer
        BEQ us_dg
        LSR FAC1
        ROR FAC2
        ROR FAC3
        ROR FAC4
        INC FACE
        LDA FACE
        JMP us_sh
us_i0:  STA FAC1            ; n = 0: no sign
        STA FAC2
        STA FAC3
        STA FAC4
        STA usg
us_dg:  LDX #0              ; ten digits into udg, 10^9 first
        STX uk
us_dl:  LDA #'0'
        STA udg,X
        LDY uk
us_ds:  SEC
        LDA FAC4
        SBC u10+3,Y
        STA tq+3
        LDA FAC3
        SBC u10+2,Y
        STA tq+2
        LDA FAC2
        SBC u10+1,Y
        STA tq+1
        LDA FAC1
        SBC u10,Y
        BCC us_dn
        STA FAC1
        LDA tq+1
        STA FAC2
        LDA tq+2
        STA FAC3
        LDA tq+3
        STA FAC4
        INC udg,X
        JMP us_ds
us_dn:  LDA uk
        CLC
        ADC #4
        STA uk
        INX
        CPX #10
        BNE us_dl
        LDA #10             ; integer digits: udg[ufst .. ue)
        SEC
        SBC ud
        STA ue
        LDX #0
us_fz:  CPX ue
        BEQ us_f0
        LDA udg,X
        CMP #'0'
        BNE us_f1
        INX
        JMP us_fz
us_f0:  DEX                 ; integer part 0: a single '0'
us_f1:  STX ufst
        LDA ue
        SEC
        SBC ufst
        STA ulen
        LDA uf              ; commas: (ulen - 1) / 3
        AND #1
        BEQ us_g1
        LDX ulen
        DEX
        TXA
        LDX #0
us_g0:  CMP #3
        BCC us_g2
        SBC #3
        INX
        JMP us_g0
us_g2:  TXA
us_g1:  CLC
        ADC ulen
        STA ugl             ; integer part with commas
        LDA #0              ; sign character
        STA usc
        LDA uf
        AND #2
        BEQ us_s1
        LDA #'+'
        STA usc
us_s1:  LDA usg
        BPL us_s2
        LDA uf
        AND #4
        BNE us_s2
        LDA #'-'
        STA usc
us_s2:  LDA usc
        BEQ us_n0
        LDA #1
us_n0:  CLC
        ADC ugl
        STA uneed
        LDA uw
        CMP uneed
        BCS us_fit
        LDA ulen            ; no room: drop a lone leading 0 ("-.50")
        CMP #1
        BNE us_ovf
        LDX ufst
        LDA udg,X
        CMP #'0'
        BNE us_ovf
        LDA #0
        STA ulen
        STA ugl
        DEC uneed
        LDA uw
        CMP uneed
        BCS us_fit
us_ovf: LDA #'%'            ; does not fit: '%' and the whole number
        STA ubuf
        LDA #1
        STA uidx
        JMP us_body
us_fit: LDA #0
        STA uidx
        LDA uw
        SEC
        SBC uneed
        TAX
        BEQ us_body
us_pad: LDA #' '
        JSR us_put
        DEX
        BNE us_pad
us_body:
        LDA usc
        BEQ us_b1
        JSR us_put
us_b1:  LDA ulen
        BEQ us_pt
        STA ucnt            ; integer digits left
        LDX ufst
us_il:  LDA udg,X
        JSR us_put
        INX
        DEC ucnt
        BEQ us_pt
        LDA uf
        AND #1
        BEQ us_il
        LDA ucnt            ; a comma when a multiple of 3 is left
us_m3:  CMP #3
        BCC us_m3e
        SBC #3
        JMP us_m3
us_m3e: CMP #0
        BNE us_il
        LDA #','
        JSR us_put
        JMP us_il
us_pt:  LDA uf
        AND #8
        BEQ us_tm
        LDA #'.'
        JSR us_put
        LDX ue
us_fl:  CPX #10
        BEQ us_tm
        LDA udg,X
        JSR us_put
        INX
        JMP us_fl
us_tm:  LDA uf
        AND #4
        BEQ us_end
        LDA #' '
        LDX usg
        BPL us_tp
        LDA #'-'
us_tp:  JSR us_put
us_end: LDA #0
        JSR us_put
        LDA #<ubuf
        LDY #>ubuf
        RTS
us_put: LDY uidx            ; append A to ubuf, keeps X
        STA ubuf,Y
        INC uidx
        RTS
us_big: LDA #<ux            ; too many digits: '%' and the plain number
        LDY #>ux
        JSR f_load
        JSR f_out
        LDA #'%'
        STA ubuf
        LDX #0
us_bc:  LDA obuf,X
        STA ubuf+1,X
        BEQ us_be
        INX
        BNE us_bc
us_be:  LDA #<ubuf
        LDY #>ubuf
        RTS

; ---------------------------------------------------------------- number -> text
f_out:              ; A/Y = text of FAC (null-terminated, in obuf); FAC destroyed
        JSR f_round
        LDA #0
        STA oidx
        LDA FACE
        BNE fo_nz
        LDA #'0'
        JSR fo_put
        JMP fo_end
fo_nz:
        LDA FACS
        BPL fo_pos
        LDA #'-'
        JSR fo_put
        LDA #0
        STA FACS
fo_pos:
        LDA #0
        STA dexp
fo_c1:  LDA FACE            ; x < 1: x * 1e8
        CMP #129
        BCS fo_c2
        LDA #<c_1e8
        LDY #>c_1e8
        JSR f_mul
        LDA dexp
        SEC
        SBC #8
        STA dexp
        JMP fo_c1
fo_c2:  LDA FACE            ; x >= 2^57: x / 1e8
        CMP #186
        BCC fo_srch
        LDA #<c_1e8
        LDY #>c_1e8
        JSR f_div
        LDA dexp
        CLC
        ADC #8
        STA dexp
        JMP fo_c2
        ; x in [1, 2^57): find E with 10^E <= x < 10^(E+1) (table pw),
        ; then scale by 10^(8-E) in ONE exact operation
fo_srch:
        LDA #<pw
        CLC
        ADC #5
        STA tp
        LDA #>pw
        ADC #0
        STA tp+1
        LDA #0
        STA epow
fo_el:  LDA tp
        LDY tp+1
        JSR f_cmpr
        CMP #$FF
        BEQ fo_ef
        INC epow
        LDA tp
        CLC
        ADC #5
        STA tp
        BCC fo_el2
        INC tp+1
fo_el2: LDA epow
        CMP #17
        BCC fo_el
fo_ef:  LDA epow            ; k = 8 - E
        CMP #8
        BNE fo_ef2
        JMP fo_f1
fo_ef2: BCS fo_big
        LDA #8
        SEC
        SBC epow
        JSR fo_pw           ; A/Y = 10^k
        JSR f_mul
        LDA dexp
        SEC
        SBC #8
        CLC
        ADC epow
        STA dexp
        JMP fo_f1
fo_big: SEC
        SBC #8
        JSR fo_pw
        JSR f_div
        LDA dexp
        CLC
        ADC epow
        SEC
        SBC #8
        STA dexp
        JMP fo_f1
fo_pw:  STA tq              ; A/Y = pw + 5*A
        LDA #<pw
        STA tp
        LDA #>pw
        STA tp+1
        LDX tq
        BEQ fo_pw2
fo_pw1: LDA tp
        CLC
        ADC #5
        STA tp
        BCC fo_pw3
        INC tp+1
fo_pw3: DEX
        BNE fo_pw1
fo_pw2: LDA tp
        LDY tp+1
        RTS
fo_f1:  LDA #<c_1e8         ; x < 1e8: x * 10
        LDY #>c_1e8
        JSR f_cmpr
        CMP #$FF
        BNE fo_f2
        LDA #<c_ten
        LDY #>c_ten
        JSR f_mul
        DEC dexp
        JMP fo_f1
fo_f2:  LDA #<c_1e9         ; x >= 1e9: x / 10
        LDY #>c_1e9
        JSR f_cmpr
        CMP #$FF
        BEQ fo_rnd
        LDA #<c_ten
        LDY #>c_ten
        JSR f_div
        INC dexp
        JMP fo_f2
fo_rnd: LDA #<c_half        ; round to an integer of 9 digits (40-bit value)
        LDY #>c_half
        JSR f_add
fo_int: LDA #160
        SEC
        SBC FACE
        TAX
fo_sh:  LSR FAC1
        ROR FAC2
        ROR FAC3
        ROR FAC4
        DEX
        BNE fo_sh
        LDY #0
        LDX #0
fo_dg:  LDA #'0'
        STA digs,X
fo_ds:  SEC
        LDA FAC4
        SBC p10+3,Y
        STA tq+3
        LDA FAC3
        SBC p10+2,Y
        STA tq+2
        LDA FAC2
        SBC p10+1,Y
        STA tq+1
        LDA FAC1
        SBC p10,Y
        BCC fo_dn
        STA FAC1
        LDA tq+1
        STA FAC2
        LDA tq+2
        STA FAC3
        LDA tq+3
        STA FAC4
        INC digs,X
        JMP fo_ds
fo_dn:  INY
        INY
        INY
        INY
        INX
        CPX #9
        BNE fo_dg
        LDA digs            ; rounding reached 1e9: digits 1000000000
        CMP #':'
        BNE fo_d10
        LDA #'1'
        STA digs
        INC dexp
fo_d10: LDX #8
fo_lz:  LDA digs,X
        CMP #'0'
        BNE fo_lzd
        DEX
        BNE fo_lz
fo_lzd: STX lastd
        LDA dexp
        CLC
        ADC #8
        STA dexp
        BMI fo_nege
        CMP #9
        BCS fo_sci
        LDX #0
fo_ip:  LDA digs,X
        JSR fo_put
        CPX dexp
        BEQ fo_ipd
        INX
        JMP fo_ip
fo_ipd: CPX lastd
        BCC fo_dot
        JMP fo_end
fo_dot: LDA #'.'
        JSR fo_put
fo_fp:  INX
        LDA digs,X
        JSR fo_put
        CPX lastd
        BNE fo_fp
        JMP fo_end
fo_nege:
        CMP #$FE
        BCC fo_sci
        LDA #'.'
        JSR fo_put
        LDA dexp
        CMP #$FE
        BNE fo_n1
        LDA #'0'
        JSR fo_put
fo_n1:  LDX #0
fo_n2:  LDA digs,X
        JSR fo_put
        CPX lastd
        BNE fo_n3
        JMP fo_end
fo_n3:  INX
        JMP fo_n2
fo_sci: LDA digs
        JSR fo_put
        LDX lastd
        BEQ fo_ex
        LDA #'.'
        JSR fo_put
        LDX #0
fo_s1:  INX
        LDA digs,X
        JSR fo_put
        CPX lastd
        BNE fo_s1
fo_ex:  LDA #'E'
        JSR fo_put
        LDA dexp
        BPL fo_ep
        LDA #'-'
        JSR fo_put
        LDA #0
        SEC
        SBC dexp
        JMP fo_e2
fo_ep:  LDA #'+'
        JSR fo_put
        LDA dexp
fo_e2:  LDX #'0'
fo_e3:  CMP #10
        BCC fo_e4
        SBC #10
        INX
        JMP fo_e3
fo_e4:  PHA
        TXA
        JSR fo_put
        PLA
        CLC
        ADC #'0'
        JSR fo_put
fo_end: LDA #0
        JSR fo_put
        LDA #<obuf
        LDY #>obuf
        RTS
fo_put: STX ix              ; append A to obuf, keeps X
        LDX oidx
        STA obuf,X
        INC oidx
        LDX ix
        RTS

; ---------------------------------------------------------------- text -> number
f_in:               ; FAC = number in the text at A/Y
        STA PTR
        STY PTR+1
        LDY #0
fi_cp:  LDA (PTR),Y
        STA ibuf,Y
        BEQ fi_cpd
        INY
        CPY #31
        BNE fi_cp
        LDA #0
        STA ibuf,Y
fi_cpd: JSR f_zero
        LDA #0
        STA neg
        STA dot
        STA dexp
        STA ix
fi_sp:  LDX ix
        LDA ibuf,X
        CMP #' '
        BNE fi_sg
        INC ix
        JMP fi_sp
fi_sg:  CMP #'-'
        BNE fi_pl
        INC neg
        INC ix
        JMP fi_dl
fi_pl:  CMP #'+'
        BNE fi_dl
        INC ix
fi_dl:  LDX ix
        LDA ibuf,X
        CMP #'.'
        BNE fi_dg
        LDA dot
        BNE fi_ex
        INC dot
        INC ix
        JMP fi_dl
fi_dg:  SEC
        SBC #'0'
        CMP #10
        BCS fi_ex
        PHA
        LDA #<c_ten
        LDY #>c_ten
        JSR f_mul
        LDA #<ftmp
        LDY #>ftmp
        JSR f_store
        PLA
        JSR f_fromu8
        LDA #<ftmp
        LDY #>ftmp
        JSR f_add
        LDA dot
        BEQ fi_nd
        DEC dexp
fi_nd:  INC ix
        JMP fi_dl
fi_ex:  LDX ix
        LDA ibuf,X
        CMP #'E'
        BEQ fi_e0
        CMP #'e'
        BNE fi_sc
fi_e0:  INC ix
        LDA #0
        STA ex
        STA exneg
        LDX ix
        LDA ibuf,X
        CMP #'-'
        BNE fi_ep
        INC exneg
        INC ix
        JMP fi_ed
fi_ep:  CMP #'+'
        BNE fi_ed
        INC ix
fi_ed:  LDX ix
        LDA ibuf,X
        SEC
        SBC #'0'
        CMP #10
        BCS fi_ee
        STA tmask
        LDA ex
        ASL A
        ASL A
        CLC
        ADC ex
        ASL A
        CLC
        ADC tmask
        STA ex
        INC ix
        JMP fi_ed
fi_ee:  LDA exneg
        BEQ fi_ea
        LDA dexp
        SEC
        SBC ex
        STA dexp
        JMP fi_sc
fi_ea:  LDA dexp
        CLC
        ADC ex
        STA dexp
fi_sc:  LDA dexp            ; coarse: 10^8 steps (exact constant)
        BMI fi_cd
        CMP #8
        BCC fi_f
        LDA #<c_1e8
        LDY #>c_1e8
        JSR f_mul
        LDA dexp
        SEC
        SBC #8
        STA dexp
        JMP fi_sc
fi_cd:  CMP #$F9            ; dexp <= -8
        BCS fi_f
        LDA #<c_1e8
        LDY #>c_1e8
        JSR f_div
        LDA dexp
        CLC
        ADC #8
        STA dexp
        JMP fi_sc
fi_f:   LDA dexp
        BEQ fi_sg2
        BMI fi_dn
fi_up:  LDA #<c_ten
        LDY #>c_ten
        JSR f_mul
        DEC dexp
        BNE fi_up
        JMP fi_sg2
fi_dn:  LDA #<c_ten
        LDY #>c_ten
        JSR f_div
        INC dexp
        BNE fi_dn
fi_sg2: LDA neg
        BEQ fi_ret
        JSR f_neg
fi_ret: RTS

; ---------------------------------------------------------------- Q8.8
f_q88dn:            ; FAC = FAC / 256
        LDA FACE
        BEQ qd_r
        SEC
        SBC #8
        BCC qd_z
        BEQ qd_z
        STA FACE
qd_r:   RTS
qd_z:   JMP f_zero

f_toq88:            ; A/Y = round(FAC * 256), 16-bit
        LDA FACE
        BEQ tq_go
        CLC
        ADC #8
        BCC tq_st
        JSR f_ovf
        JMP f_rndu16
tq_st:  STA FACE
tq_go:  JMP f_rndu16

; ---------------------------------------------------------------- polynomial
f_poly:             ; FAC = P(FAC); A/Y = table: degree, coefficients high first
        STA pp
        STY pp+1
        LDA #<px2
        LDY #>px2
        JSR f_store
        LDA pp
        STA PTR
        LDA pp+1
        STA PTR+1
        LDY #0
        LDA (PTR),Y
        STA pn
        INC pp
        BNE py_1
        INC pp+1
py_1:   LDA pp
        LDY pp+1
        JSR f_load
py_lp:  LDA pp
        CLC
        ADC #5
        STA pp
        BCC py_2
        INC pp+1
py_2:   LDA #<px2
        LDY #>px2
        JSR f_mul
        LDA pp
        LDY pp+1
        JSR f_add
        DEC pn
        BNE py_lp
        RTS

; ---------------------------------------------------------------- powers
f_powi:             ; FAC = FAC ^ A (A = 0..255)
        STA pk
        LDA #<pb
        LDY #>pb
        JSR f_store
        LDA #<c_one
        LDY #>c_one
        JSR f_load
pi_lp:  LSR pk
        BCC pi_sq
        LDA #<pb
        LDY #>pb
        JSR f_mul
pi_sq:  LDA pk
        BEQ pi_ret
        JSR f_push
        LDA #<pb
        LDY #>pb
        JSR f_load
        LDA #<pb
        LDY #>pb
        JSR f_mul
        LDA #<pb
        LDY #>pb
        JSR f_store
        JSR f_poparg
        JSR f_argfac
        JMP pi_lp
pi_ret: RTS

f_powp:             ; FAC = FAC ^ pop
        LDA #<px
        LDY #>px
        JSR f_store
        JSR f_poparg
        JSR f_argfac
        LDA #<py
        LDY #>py
        JSR f_store
        LDA px
        BNE pw_nz
        JMP f_zero
pw_nz:  LDA #0
        STA pneg
        LDA px+1
        BPL pw_pos
        AND #$7F
        STA px+1
        JSR f_tos16
        AND #1
        STA pneg
pw_pos: LDA #<px
        LDY #>px
        JSR f_load
        JSR f_log
        LDA #<py
        LDY #>py
        JSR f_mul
        JSR f_exp
        LDA pneg
        BEQ pw_ret
        JMP f_neg
pw_ret: RTS

; ---------------------------------------------------------------- sqr
f_sqr:              ; FAC = sqrt(FAC) (negative: 0)
        LDA FACE
        BEQ sq_ret
        LDA FACS
        BMI sq_z
        LDA #<sx
        LDY #>sx
        JSR f_store
        LDA FACE
        CLC
        ADC #129
        ROR A
        STA FACE
        LDA #5
        STA sqn
sq_lp:  LDA #<sg
        LDY #>sg
        JSR f_store
        LDA #<sx
        LDY #>sx
        JSR f_load
        LDA #<sg
        LDY #>sg
        JSR f_div
        LDA #<sg
        LDY #>sg
        JSR f_add
        DEC FACE
        DEC sqn
        BNE sq_lp
sq_ret: RTS
sq_z:   JMP f_zero

; ---------------------------------------------------------------- log / exp
lg_bad: JMP f_zero
f_log:              ; FAC = ln(FAC) (<= 0: 0)
        LDA FACE
        BEQ lg_bad
        LDA FACS
        BMI lg_bad
        LDA FACE
        SEC
        SBC #128
        STA lge
        LDA #128
        STA FACE
        LDA #<c_sqh
        LDY #>c_sqh
        JSR f_cmp
        CMP #$FF
        BNE lg_ok
        INC FACE
        DEC lge
lg_ok:  LDA #<lgm
        LDY #>lgm
        JSR f_store
        LDA #<c_one
        LDY #>c_one
        JSR f_add
        LDA #<lgt
        LDY #>lgt
        JSR f_store
        LDA #<lgm
        LDY #>lgm
        JSR f_load
        LDA #<c_one
        LDY #>c_one
        JSR f_sub
        LDA #<lgt
        LDY #>lgt
        JSR f_div
        LDA #<lgs
        LDY #>lgs
        JSR f_store
        LDA #<lgs
        LDY #>lgs
        JSR f_mul
        LDA #<t_log
        LDY #>t_log
        JSR f_poly
        LDA #<lgs
        LDY #>lgs
        JSR f_mul
        LDA FACE
        BEQ lg_2
        INC FACE
lg_2:   JSR f_push
        LDY #0
        LDA lge
        BPL lg_3
        DEY
lg_3:   JSR f_froms16
        LDA #<c_ln2
        LDY #>c_ln2
        JSR f_mul
        JMP f_addp

f_exp:              ; FAC = e ^ FAC
        LDA #<c_log2e
        LDY #>c_log2e
        JSR f_mul
        LDA #<ext
        LDY #>ext
        JSR f_store
        LDA #<c_half
        LDY #>c_half
        JSR f_add
        JSR f_int
        LDA #<exn
        LDY #>exn
        JSR f_store
        JSR f_tos16
        STA exi
        STY exi+1
        LDA #<ext
        LDY #>ext
        JSR f_load
        LDA #<exn
        LDY #>exn
        JSR f_sub
        LDA #<c_ln2
        LDY #>c_ln2
        JSR f_mul
        LDA #<t_exp
        LDY #>t_exp
        JSR f_poly
        LDA FACE
        CLC
        ADC exi
        STA te
        LDA #0
        ADC exi+1
        STA te+1
        JMP f_setexp

; ---------------------------------------------------------------- trigonometry
f_cos:              ; FAC = cos(FAC) (radians)
        LDA #<c_pi2
        LDY #>c_pi2
        JSR f_add
f_sin:              ; FAC = sin(FAC) (radians)
        LDA #<c_i2pi
        LDY #>c_i2pi
        JSR f_mul
        LDA #<snq
        LDY #>snq
        JSR f_store
        JSR f_int
        JSR f_neg
        LDA #<snq
        LDY #>snq
        JSR f_add
        LDA #<c_3q
        LDY #>c_3q
        JSR f_cmp
        CMP #1
        BNE sn_a
        LDA #<c_one
        LDY #>c_one
        JSR f_sub
        JMP sn_p
sn_a:   LDA #<c_q
        LDY #>c_q
        JSR f_cmp
        CMP #1
        BNE sn_p
        JSR f_neg
        LDA #<c_half
        LDY #>c_half
        JSR f_add
sn_p:   LDA #<c_2pi
        LDY #>c_2pi
        JSR f_mul
        LDA #<snu
        LDY #>snu
        JSR f_store
        LDA #<snu
        LDY #>snu
        JSR f_mul
        LDA #<t_sin
        LDY #>t_sin
        JSR f_poly
        LDA #<snu
        LDY #>snu
        JMP f_mul

f_tan:              ; FAC = tan(FAC)
        LDA #<tnx
        LDY #>tnx
        JSR f_store
        JSR f_cos
        LDA #<tnc
        LDY #>tnc
        JSR f_store
        LDA #<tnx
        LDY #>tnx
        JSR f_load
        JSR f_sin
        LDA #<tnc
        LDY #>tnc
        JMP f_div

f_atn:              ; FAC = atn(FAC)
        LDA FACS
        STA atsg
        LDA #0
        STA FACS
        STA atfl
        LDA FACE
        BNE at_go
        RTS
at_go:  LDA #<c_one
        LDY #>c_one
        JSR f_cmp
        CMP #1
        BNE at_le1
        LDA #<atx
        LDY #>atx
        JSR f_store
        LDA #<c_one
        LDY #>c_one
        JSR f_load
        LDA #<atx
        LDY #>atx
        JSR f_div
        INC atfl
at_le1: LDA #<c_t15
        LDY #>c_t15
        JSR f_cmp
        CMP #1
        BNE at_sm
        LDA #<atx
        LDY #>atx
        JSR f_store
        LDA #<c_sq3
        LDY #>c_sq3
        JSR f_add
        LDA #<att
        LDY #>att
        JSR f_store
        LDA #<atx
        LDY #>atx
        JSR f_load
        LDA #<c_sq3
        LDY #>c_sq3
        JSR f_mul
        LDA #<c_one
        LDY #>c_one
        JSR f_sub
        LDA #<att
        LDY #>att
        JSR f_div
        LDA atfl
        ORA #2
        STA atfl
at_sm:  LDA #<atx
        LDY #>atx
        JSR f_store
        LDA #<atx
        LDY #>atx
        JSR f_mul
        LDA #<t_atn
        LDY #>t_atn
        JSR f_poly
        LDA #<atx
        LDY #>atx
        JSR f_mul
        LDA atfl
        AND #2
        BEQ at_n2
        LDA #<c_pi6
        LDY #>c_pi6
        JSR f_add
at_n2:  LDA atfl
        AND #1
        BEQ at_n1
        JSR f_neg
        LDA #<c_pi2
        LDY #>c_pi2
        JSR f_add
at_n1:  LDA FACE
        BEQ at_ret
        LDA atsg
        STA FACS
at_ret: RTS
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_round_trips() {
        for x in [1.0, 0.5, 10.0, -3.25, 1e8, 1e9, 1.5e-30, 1.7e38, 3.141592653589793] {
            let b = to_mflpt(x);
            let y = from_mflpt(b);
            assert!((x - y).abs() <= x.abs() * 2f64.powi(-32), "{x} -> {b:?} -> {y}");
        }
        assert_eq!(to_mflpt(1.0), [0x81, 0, 0, 0, 0]);
        assert_eq!(to_mflpt(-1.0), [0x81, 0x80, 0, 0, 0]);
        assert_eq!(to_mflpt(10.0), [0x84, 0x20, 0, 0, 0]);
        assert_eq!(to_mflpt(0.0), [0; 5]);
    }

    #[test]
    fn runtime_assembles() {
        let (code, labels) = float_runtime(0x4000, 0x60);
        assert!(code.len() > 500);
        for l in ["f_load", "f_store", "f_add", "f_mul", "f_div", "f_cmp", "f_out", "f_in", "fac"] {
            assert!(labels.contains_key(l), "{l}");
        }
    }

    #[test]
    fn pruned_runtime_keeps_only_reached_parts() {
        let (full, _) = float_runtime(0x4000, 0x60);
        let (code, labels) = float_runtime_for(0x4000, 0x60, &["f_load", "f_store", "f_mul", "f_out"]);
        assert!(code.len() + 1500 < full.len(), "{} vs {}", code.len(), full.len());
        // fall-through and JSR targets come along
        for l in ["f_mularg", "f_round", "f_norm", "obuf", "pw", "fac", "arg"] {
            assert!(labels.contains_key(l), "{l}");
        }
        for l in ["f_sin", "f_log", "f_exp", "f_in", "ibuf", "t_sin", "fstk"] {
            assert!(!labels.contains_key(l), "{l}");
        }
    }

    #[test]
    fn every_entry_assembles_alone() {
        let (_, all) = float_runtime(0x4000, 0x60);
        let mut n = 0;
        for name in all.keys().filter(|k| k.starts_with("f_")) {
            let (_, labels) = float_runtime_for(0x4000, 0x60, &[name.as_str()]);
            assert!(labels.contains_key(name), "{name}");
            n += 1;
        }
        assert!(n > 40);
    }
}
