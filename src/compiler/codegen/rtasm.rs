//! Strict two-pass 6502 assembler for the compiler's own runtime libraries
//! (the floating-point package). Unlike `assemble_inline` (user `asm { }`
//! blocks, lenient on purpose) every unknown label, mnemonic or addressing
//! mode and every out-of-range branch is an error.
//!
//! Syntax, one statement per line, `;` comments:
//!
//! ```text
//! label:  LDA #<table      ; #<expr / #>expr / #expr
//!         STA ptr+1        ; expr, expr,X, expr,Y, (expr), (expr,X), (expr),Y
//!         .byte 1, 2, <x   ; data;  .word expr, …;  .res n (n zero bytes)
//! name = expr              ; constant (may use labels)
//! ```
//!
//! `expr` is a sum of numbers (`$hex`, `%bin`, decimal, `'c'`), symbols and
//! `*` joined by `+` / `-`. An operand is zero page only when its value is
//! known in pass 1 (numbers and predefined symbols) and below $100; anything
//! that involves a label of the source is absolute.

use super::{asm_opcode, AMode};
use std::collections::HashMap;

#[derive(Debug)]
pub struct Assembled {
    pub code: Vec<u8>,
    pub labels: HashMap<String, u16>,
}

#[derive(Clone, Debug)]
enum Operand {
    None,
    Acc,
    Imm(String, ByteSel),
    Addr(String),
    AddrX(String),
    AddrY(String),
    Ind(String),
    IndX(String),
    IndY(String),
}

#[derive(Clone, Copy, Debug)]
enum ByteSel {
    Full,
    Lo,
    Hi,
}

enum Item {
    Ins { line: usize, mnem: String, op: Operand, mode: AMode, addr: u16 },
    Bytes { line: usize, exprs: Vec<(String, ByteSel)>, addr: u16 },
    Words { line: usize, exprs: Vec<String>, addr: u16 },
    Res { n: u16 },
}

fn strip(line: &str) -> &str {
    // `;` starts a comment unless it is inside a character literal
    let mut in_chr = false;
    for (i, c) in line.char_indices() {
        match c {
            '\'' => in_chr = !in_chr,
            ';' if !in_chr => return line[..i].trim(),
            _ => {}
        }
    }
    line.trim()
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_chr = false;
    for c in s.chars() {
        match c {
            '\'' => {
                in_chr = !in_chr;
                cur.push(c);
            }
            ',' if !in_chr => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

struct Ctx<'a> {
    labels: HashMap<String, u16>,
    equs: HashMap<String, String>,
    known: &'a HashMap<String, u16>,
}

impl Ctx<'_> {
    /// Value of `expr` at `pc`; `Ok(None)` when it needs a label that is not
    /// known yet (pass 1).
    fn eval(&self, expr: &str, pc: u16, depth: u32) -> Result<Option<u16>, String> {
        if depth > 16 {
            return Err(format!("constant definitions loop: {expr}"));
        }
        let mut total: i32 = 0;
        let mut unknown = false;
        let mut sign = 1;
        let mut term = String::new();
        let mut terms: Vec<(i32, String)> = vec![];
        let mut in_chr = false;
        for c in expr.chars() {
            if c == '\'' {
                in_chr = !in_chr;
                term.push(c);
            } else if (c == '+' || c == '-') && !in_chr && !term.trim().is_empty() {
                terms.push((sign, term.trim().to_string()));
                term.clear();
                sign = if c == '+' { 1 } else { -1 };
            } else if (c == '+' || c == '-') && !in_chr {
                if c == '-' {
                    sign = -sign;
                }
            } else {
                term.push(c);
            }
        }
        if term.trim().is_empty() {
            return Err(format!("empty expression '{expr}'"));
        }
        terms.push((sign, term.trim().to_string()));
        for (s, t) in terms {
            let v: Option<i32> = if t == "*" {
                Some(pc as i32)
            } else if let Some(h) = t.strip_prefix('$') {
                Some(i32::from_str_radix(h, 16).map_err(|_| format!("bad number '{t}'"))?)
            } else if let Some(b) = t.strip_prefix('%') {
                Some(i32::from_str_radix(b, 2).map_err(|_| format!("bad number '{t}'"))?)
            } else if t.starts_with('\'') && t.ends_with('\'') && t.chars().count() == 3 {
                Some(t.chars().nth(1).unwrap() as i32)
            } else if t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                Some(t.parse::<i32>().map_err(|_| format!("bad number '{t}'"))?)
            } else if let Some(&v) = self.known.get(&t) {
                Some(v as i32)
            } else if let Some(&v) = self.labels.get(&t) {
                Some(v as i32)
            } else if let Some(e) = self.equs.get(&t) {
                self.eval(e, pc, depth + 1)?.map(|v| v as i32)
            } else {
                None
            };
            match v {
                Some(v) => total += s * v,
                None => unknown = true,
            }
        }
        Ok(if unknown { None } else { Some(total as u16) })
    }

    /// Is `expr` built only from numbers and predefined symbols (so its size
    /// can be decided in pass 1)?
    fn is_const(&self, expr: &str) -> bool {
        expr.split(['+', '-'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .all(|t| {
                t.starts_with('$')
                    || t.starts_with('%')
                    || t.starts_with('\'')
                    || t.chars().next().is_some_and(|c| c.is_ascii_digit())
                    || self.known.contains_key(t)
            })
    }
}

fn parse_operand(s: &str) -> Operand {
    let s = s.trim();
    if s.is_empty() {
        return Operand::None;
    }
    if s.eq_ignore_ascii_case("A") {
        return Operand::Acc;
    }
    if let Some(r) = s.strip_prefix('#') {
        let r = r.trim();
        return if let Some(x) = r.strip_prefix('<') {
            Operand::Imm(x.trim().into(), ByteSel::Lo)
        } else if let Some(x) = r.strip_prefix('>') {
            Operand::Imm(x.trim().into(), ByteSel::Hi)
        } else {
            Operand::Imm(r.into(), ByteSel::Full)
        };
    }
    let up = s.to_ascii_uppercase();
    if s.starts_with('(') {
        if up.ends_with("),Y") {
            return Operand::IndY(s[1..s.len() - 3].trim().into());
        }
        if up.ends_with(",X)") {
            return Operand::IndX(s[1..s.len() - 3].trim().into());
        }
        if s.ends_with(')') {
            return Operand::Ind(s[1..s.len() - 1].trim().into());
        }
    }
    if up.ends_with(",X") {
        return Operand::AddrX(s[..s.len() - 2].trim().into());
    }
    if up.ends_with(",Y") {
        return Operand::AddrY(s[..s.len() - 2].trim().into());
    }
    Operand::Addr(s.into())
}

fn is_branch(m: &str) -> bool {
    matches!(m, "BCC" | "BCS" | "BEQ" | "BMI" | "BNE" | "BPL" | "BVC" | "BVS")
}

fn size(mode: AMode) -> u16 {
    use AMode::*;
    match mode {
        Imp | Acc => 1,
        Imm | Zp | Zpx | Zpy | Izx | Izy | Rel => 2,
        Abs | Abx | Aby | Ind => 3,
    }
}

/// Assemble `src` at `base`. `known` are predefined symbols (e.g. zero-page
/// addresses chosen by the code generator).
pub fn assemble(src: &str, base: u16, known: &HashMap<String, u16>) -> Result<Assembled, String> {
    use AMode::*;
    let mut ctx = Ctx { labels: HashMap::new(), equs: HashMap::new(), known };
    let mut items: Vec<Item> = vec![];
    let mut pc = base;
    for (ln, raw) in src.lines().enumerate() {
        let line = ln + 1;
        let mut s = strip(raw);
        if s.is_empty() {
            continue;
        }
        // label
        if let Some(cp) = s.find(':') {
            let name = s[..cp].trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                if ctx.labels.insert(name.to_string(), pc).is_some() {
                    return Err(format!("line {line}: label '{name}' defined twice"));
                }
                s = s[cp + 1..].trim();
                if s.is_empty() {
                    continue;
                }
            }
        }
        // constant
        if let Some(eq) = s.find('=') {
            let name = s[..eq].trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                ctx.equs.insert(name.to_string(), s[eq + 1..].trim().to_string());
                continue;
            }
        }
        let (word, rest) = match s.find(char::is_whitespace) {
            Some(i) => (&s[..i], s[i..].trim()),
            None => (s, ""),
        };
        let lw = word.to_ascii_lowercase();
        if lw == ".byte" {
            let exprs: Vec<(String, ByteSel)> = split_args(rest)
                .into_iter()
                .map(|a| {
                    if let Some(x) = a.strip_prefix('<') {
                        (x.trim().to_string(), ByteSel::Lo)
                    } else if let Some(x) = a.strip_prefix('>') {
                        (x.trim().to_string(), ByteSel::Hi)
                    } else {
                        (a, ByteSel::Full)
                    }
                })
                .collect();
            let n = exprs.len() as u16;
            items.push(Item::Bytes { line, exprs, addr: pc });
            pc = pc.wrapping_add(n);
            continue;
        }
        if lw == ".word" {
            let exprs = split_args(rest);
            let n = exprs.len() as u16 * 2;
            items.push(Item::Words { line, exprs, addr: pc });
            pc = pc.wrapping_add(n);
            continue;
        }
        if lw == ".res" {
            let n = ctx
                .eval(rest, pc, 0)?
                .ok_or_else(|| format!("line {line}: .res size must be a constant"))?;
            items.push(Item::Res { n });
            pc = pc.wrapping_add(n);
            continue;
        }
        let mnem = word.to_ascii_uppercase();
        let op = parse_operand(rest);
        let mode = if is_branch(&mnem) {
            Rel
        } else {
            match &op {
                Operand::None => Imp,
                Operand::Acc => Acc,
                Operand::Imm(..) => Imm,
                Operand::Ind(_) => Ind,
                Operand::IndX(_) => Izx,
                Operand::IndY(_) => Izy,
                Operand::Addr(e) | Operand::AddrX(e) | Operand::AddrY(e) => {
                    let zp = ctx.is_const(e)
                        && ctx.eval(e, pc, 0)?.is_some_and(|v| v < 0x100);
                    let (z, a) = match &op {
                        Operand::Addr(_) => (Zp, Abs),
                        Operand::AddrX(_) => (Zpx, Abx),
                        _ => (Zpy, Aby),
                    };
                    if zp && asm_opcode(&mnem, z).is_some() {
                        z
                    } else {
                        a
                    }
                }
            }
        };
        // `ASL` without operand = accumulator
        let mode = if mode == Imp && asm_opcode(&mnem, Imp).is_none() && asm_opcode(&mnem, Acc).is_some() {
            Acc
        } else {
            mode
        };
        if asm_opcode(&mnem, mode).is_none() {
            return Err(format!("line {line}: no such instruction '{mnem}' ({mode:?})"));
        }
        items.push(Item::Ins { line, mnem, op, mode, addr: pc });
        pc = pc.wrapping_add(size(mode));
    }

    // pass 2
    let mut code = vec![];
    let need = |ctx: &Ctx, e: &str, pc: u16, line: usize| -> Result<u16, String> {
        ctx.eval(e, pc, 0)?
            .ok_or_else(|| format!("line {line}: unknown symbol in '{e}'"))
    };
    for it in &items {
        match it {
            Item::Res { n } => code.extend(std::iter::repeat_n(0u8, *n as usize)),
            Item::Bytes { line, exprs, addr } => {
                for (k, (e, sel)) in exprs.iter().enumerate() {
                    let v = need(&ctx, e, addr + k as u16, *line)?;
                    code.push(match sel {
                        ByteSel::Full => {
                            if v > 0xFF && (v as i16) < -128 {
                                return Err(format!("line {line}: byte value {v} out of range"));
                            }
                            v as u8
                        }
                        ByteSel::Lo => v as u8,
                        ByteSel::Hi => (v >> 8) as u8,
                    });
                }
            }
            Item::Words { line, exprs, addr } => {
                for (k, e) in exprs.iter().enumerate() {
                    let v = need(&ctx, e, addr + 2 * k as u16, *line)?;
                    code.extend_from_slice(&v.to_le_bytes());
                }
            }
            Item::Ins { line, mnem, op, mode, addr } => {
                let opc = asm_opcode(mnem, *mode).unwrap();
                code.push(opc);
                match (mode, op) {
                    (Rel, Operand::Addr(e)) => {
                        let t = need(&ctx, e, *addr, *line)?;
                        let off = t as i32 - (*addr as i32 + 2);
                        if !(-128..=127).contains(&off) {
                            return Err(format!("line {line}: branch to '{e}' out of range ({off})"));
                        }
                        code.push(off as i8 as u8);
                    }
                    (Rel, _) => return Err(format!("line {line}: bad branch operand")),
                    (Imp | Acc, _) => {}
                    (_, Operand::Imm(e, sel)) => {
                        let v = need(&ctx, e, *addr, *line)?;
                        code.push(match sel {
                            ByteSel::Full => {
                                if v > 0xFF {
                                    return Err(format!("line {line}: immediate {v} > 255"));
                                }
                                v as u8
                            }
                            ByteSel::Lo => v as u8,
                            ByteSel::Hi => (v >> 8) as u8,
                        });
                    }
                    (_, Operand::Addr(e) | Operand::AddrX(e) | Operand::AddrY(e) | Operand::Ind(e) | Operand::IndX(e) | Operand::IndY(e)) => {
                        let v = need(&ctx, e, *addr, *line)?;
                        if size(*mode) == 2 {
                            if v > 0xFF {
                                return Err(format!("line {line}: zero-page operand {v:#X} > $FF"));
                            }
                            code.push(v as u8);
                        } else {
                            code.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                    _ => return Err(format!("line {line}: bad operand for {mnem}")),
                }
            }
        }
    }
    let _ = &mut ctx;
    let mut labels = ctx.labels.clone();
    for (k, e) in &ctx.equs {
        if let Some(v) = ctx.eval(e, 0, 0)? {
            labels.insert(k.clone(), v);
        }
    }
    Ok(Assembled { code, labels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asm(src: &str) -> Result<Assembled, String> {
        let mut k = HashMap::new();
        k.insert("ZP".to_string(), 0x50);
        assemble(src, 0x1000, &k)
    }

    #[test]
    fn modes_and_labels() {
        let a = asm(
            "start: LDA #<data\n LDY #>data\n STA ZP\n STA ZP+1\n LDA (ZP),Y\n LDA data+1,X\n \
             ASL\n ROR A\n JMP start\n BNE start\ndata: .byte 1, $FF, <start, >start, 'A'\n .word data\n .res 2\nK = data - start\n",
        )
        .unwrap();
        assert_eq!(
            a.code,
            vec![
                0xA9, 0x14, 0xA0, 0x10, 0x85, 0x50, 0x85, 0x51, 0xB1, 0x50, 0xBD, 0x15, 0x10, 0x0A, 0x6A,
                0x4C, 0x00, 0x10, 0xD0, 0xEC, 1, 0xFF, 0x00, 0x10, 0x41, 0x14, 0x10, 0, 0
            ]
        );
        assert_eq!(a.labels["K"], 0x14); // data at $1014 (20 bytes of code)
    }

    #[test]
    fn errors_are_reported() {
        assert!(asm("LDA nowhere\n").is_err());
        assert!(asm("FOO #1\n").is_err());
        assert!(asm("x: NOP\nx: NOP\n").is_err());
        assert!(asm("LDA #300\n").is_err());
        let mut far = String::from("top: NOP\n");
        for _ in 0..130 {
            far += "NOP\n";
        }
        far += "BNE top\n";
        assert!(asm(&far).unwrap_err().contains("out of range"));
    }
}
