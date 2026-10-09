// Differential fuzzing of the code generator against a reference compiler.
//
// Random programs are compiled with this crate and with a reference `ub`
// executable (e.g. a build of an older commit), both results are executed on
// the 6502 emulator, and the final variable / array state must match.
//
//     UB_REFERENCE=path/to/old/ub.exe cargo test --release --test differential_fuzz -- --ignored
//
// Optional: UB_FUZZ_CASES (default 2000), UB_FUZZ_SEED (default 1).
//
// The generator avoids constructs that the reference compiler (1.5.9) is known
// to get wrong or to hang on (division by zero, shift by 0, 8-bit for-loop
// overflow, count-down loops with bodies over 127 bytes), so any difference
// points at the new code generator.

mod common;
use common::cpu6502::*;
use std::process::Command;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn chance(&mut self, pct: u32) -> bool {
        self.below(100) < pct
    }
}

const VARS: usize = 6;
const INTERESTING: &[u32] = &[0, 1, 2, 3, 7, 8, 15, 16, 100, 127, 128, 200, 254, 255];

struct Gen {
    rng: Rng,
    guards: usize,
    loop_vars: usize,
}

impl Gen {
    fn num(&mut self) -> String {
        if self.rng.chance(50) {
            INTERESTING[self.rng.below(INTERESTING.len() as u32) as usize].to_string()
        } else {
            self.rng.below(256).to_string()
        }
    }

    fn var(&mut self) -> String {
        format!("v{}", self.rng.below(VARS as u32))
    }

    fn atom(&mut self) -> String {
        match self.rng.below(5) {
            0 | 1 => self.var(),
            2 => self.num(),
            3 => format!("a[{} and 15]", self.var()),
            _ => format!("a[{}]", self.rng.below(16)),
        }
    }

    fn expr(&mut self, depth: u32) -> String {
        if depth == 0 || self.rng.chance(30) {
            return self.atom();
        }
        let l = self.expr(depth - 1);
        let r = self.expr(depth - 1);
        match self.rng.below(14) {
            0 | 1 => format!("({l} + {r})"),
            2 | 3 => format!("({l} - {r})"),
            4 => format!("({l} * {r})"),
            5 => format!("({l} / ({r} or 1))"),
            6 => format!("({l} mod ({r} or 1))"),
            7 => format!("({l} and {r})"),
            8 => format!("({l} or {r})"),
            9 => format!("({l} xor {r})"),
            10 => format!("({l} shl (({r} and 7) or 1))"),
            11 => format!("({l} shr (({r} and 7) or 1))"),
            12 => self.cmp(depth - 1),
            _ => format!("(not {l})"),
        }
    }

    fn cmp(&mut self, depth: u32) -> String {
        let l = self.expr(depth);
        let r = self.expr(depth);
        let op = ["==", "!=", "<", ">", "<=", ">="][self.rng.below(6) as usize];
        format!("({l} {op} {r})")
    }

    fn cond(&mut self) -> String {
        match self.rng.below(6) {
            0 => format!("{} and {}", self.cmp(1), self.cmp(1)),
            1 => format!("{} or {}", self.cmp(1), self.cmp(1)),
            2 => format!("not {}", self.cmp(1)),
            3 => self.var(),
            _ => self.cmp(2),
        }
    }

    fn block(&mut self, depth: u32, out: &mut String, indent: &str) {
        let n = 1 + self.rng.below(4);
        for _ in 0..n {
            self.stmt(depth, out, indent);
        }
    }

    fn stmt(&mut self, depth: u32, out: &mut String, ind: &str) {
        let inner = format!("{ind}  ");
        let kind = if depth == 0 { self.rng.below(5) } else { self.rng.below(12) };
        match kind {
            0 | 1 => {
                let v = self.var();
                let e = self.expr(3);
                out.push_str(&format!("{ind}{v} = {e}\n"));
            }
            2 => {
                let i = self.expr(2);
                let e = self.expr(2);
                out.push_str(&format!("{ind}a[{i} and 15] = {e}\n"));
            }
            3 => {
                let v = self.var();
                let op = ["inc", "dec"][self.rng.below(2) as usize];
                out.push_str(&format!("{ind}{op} {v}\n"));
            }
            4 => {
                let v = self.var();
                let op = ["+=", "-=", "and=", "or=", "xor="][self.rng.below(5) as usize];
                let e = self.expr(1);
                out.push_str(&format!("{ind}{v} {op} {e}\n"));
            }
            5 | 6 => {
                let c = self.cond();
                out.push_str(&format!("{ind}if {c} then\n"));
                self.block(depth - 1, out, &inner);
                if self.rng.chance(40) {
                    out.push_str(&format!("{ind}else\n"));
                    self.block(depth - 1, out, &inner);
                }
                out.push_str(&format!("{ind}end\n"));
            }
            7 | 8 if self.loop_vars < 2 => {
                let lv = format!("l{}", self.loop_vars);
                self.loop_vars += 1;
                let from = self.rng.below(120);
                let to = from + self.rng.below(12);
                let step = 1 + self.rng.below(3);
                // (No count-down loops: the reference compiler truncates their
                // back branch when the body exceeds 127 bytes; they are
                // covered by codegen_semantics.rs instead.)
                if step == 1 {
                    out.push_str(&format!("{ind}for {lv} = {from} to {to}\n"));
                } else {
                    out.push_str(&format!("{ind}for {lv} = {from} to {to} step {step}\n"));
                }
                self.block(depth - 1, out, &inner);
                if self.rng.chance(15) {
                    let c = self.cmp(1);
                    out.push_str(&format!("{inner}if {c} then continue end\n"));
                }
                out.push_str(&format!("{ind}next\n"));
                self.loop_vars -= 1;
            }
            9 if self.guards < 3 => {
                // bounded while: the guard counter always terminates it
                let g = format!("g{}", self.guards);
                self.guards += 1;
                let c = self.cmp(1);
                out.push_str(&format!("{ind}{g} = 0\n{ind}while {g} < 6 and {c}\n{inner}inc {g}\n"));
                self.block(depth - 1, out, &inner);
                out.push_str(&format!("{ind}end\n"));
            }
            10 if self.guards < 3 => {
                let g = format!("g{}", self.guards);
                self.guards += 1;
                let c = self.cmp(1);
                out.push_str(&format!("{ind}{g} = 0\n{ind}repeat\n{inner}inc {g}\n"));
                self.block(depth - 1, out, &inner);
                if self.rng.chance(20) {
                    out.push_str(&format!("{inner}if {g} == 4 then break end\n"));
                }
                out.push_str(&format!("{ind}until {g} >= 6 or {c}\n"));
            }
            _ => {
                let v = self.var();
                out.push_str(&format!("{ind}select {v}\n"));
                for _ in 0..1 + self.rng.below(3) {
                    let n = self.num();
                    out.push_str(&format!("{ind}  case {n}:\n"));
                    self.block(depth - 1, out, &format!("{ind}    "));
                }
                if self.rng.chance(50) {
                    out.push_str(&format!("{ind}  else:\n"));
                    self.block(depth - 1, out, &format!("{ind}    "));
                }
                out.push_str(&format!("{ind}end\n"));
            }
        }
    }

    fn program(&mut self) -> String {
        let mut s = String::new();
        // declare everything first so both compilers use the same ZP layout
        for k in 0..VARS {
            let n = self.num();
            s.push_str(&format!("var v{k} = {n}\n"));
        }
        s.push_str("var l0 = 0\nvar l1 = 0\nvar g0 = 0\nvar g1 = 0\nvar g2 = 0\nvar a = array(16)\n");
        self.guards = 0;
        self.loop_vars = 0;
        for _ in 0..3 + self.rng.below(5) {
            self.stmt(3, &mut s, "");
        }
        s
    }
}

fn compile_reference(exe: &str, src: &str, dir: &std::path::Path, n: usize) -> Option<Vec<u8>> {
    let ub = dir.join(format!("case{n}.ub"));
    let prg = dir.join(format!("case{n}.prg"));
    std::fs::write(&ub, src).unwrap();
    let out = Command::new(exe)
        .args(["build", ub.to_str().unwrap(), "-o", prg.to_str().unwrap(), "--no-stub"])
        .output()
        .expect("run reference compiler");
    if !out.status.success() || !prg.exists() {
        return None;
    }
    Some(std::fs::read(&prg).unwrap())
}

fn final_state(prg: &[u8]) -> Result<Vec<u8>, String> {
    let mut cpu = Cpu::new(prg);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cpu.run(20_000_000)));
    if r.is_err() {
        return Err(format!("crashed/hung at ${:04X}", cpu.pc));
    }
    // variables v0..v5, l0, l1, g0..g2 are at $02, $04, … (2 bytes each)
    let mut state: Vec<u8> = (0..11).map(|k| cpu.mem[0x02 + 2 * k]).collect();
    state.extend_from_slice(&cpu.mem[0xC000..0xC010]);
    Ok(state)
}

#[test]
#[ignore = "needs UB_REFERENCE=<path to reference ub executable>"]
fn differential_against_reference_compiler() {
    let exe = std::env::var("UB_REFERENCE").expect("set UB_REFERENCE");
    let cases: usize = std::env::var("UB_FUZZ_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
    let seed: u64 = std::env::var("UB_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let dir = std::env::temp_dir().join(format!("ub_fuzz_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut g = Gen { rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1), guards: 0, loop_vars: 0 };
    let (mut compared, mut skipped) = (0, 0);
    for n in 0..cases {
        let src = g.program();
        let new_prg = compile_src(&src).prg;
        let Some(ref_prg) = compile_reference(&exe, &src, &dir, n % 8) else {
            skipped += 1;
            continue;
        };
        let want = match final_state(&ref_prg) {
            Ok(s) => s,
            Err(_) => {
                skipped += 1; // reference itself misbehaves on this program
                continue;
            }
        };
        let got = final_state(&new_prg).unwrap_or_else(|e| panic!("case {n}: new code {e}\n{src}"));
        assert_eq!(got, want, "case {n} differs (v0..v5, l0, l1, g0..g2, a[0..16])\n{src}");
        compared += 1;
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("compared {compared} programs, skipped {skipped}");
    assert!(compared > cases / 2, "too many skipped programs ({skipped})");
}

/// Compare one source file (UB_FUZZ_FILE): prints SAME, DIFF or ERR.
/// Used to minimise a failing case.
#[test]
#[ignore = "needs UB_REFERENCE and UB_FUZZ_FILE"]
fn differential_single_file() {
    let exe = std::env::var("UB_REFERENCE").expect("set UB_REFERENCE");
    let file = std::env::var("UB_FUZZ_FILE").expect("set UB_FUZZ_FILE");
    let src = std::fs::read_to_string(&file).unwrap();
    let dir = std::env::temp_dir().join(format!("ub_fuzz1_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let res = ultimate_basic::compiler::compile(
        &src,
        &ultimate_basic::compiler::CompileOptions { basic_stub: false, explicit: false },
    );
    let verdict = match (res.errors.is_empty(), compile_reference(&exe, &src, &dir, 0)) {
        (true, Some(ref_prg)) => match (final_state(&res.prg), final_state(&ref_prg)) {
            (Ok(a), Ok(b)) if a == b => "SAME".to_string(),
            (Ok(a), Ok(b)) => format!("DIFF new={a:?} ref={b:?}"),
            _ => "ERR".to_string(),
        },
        _ => "ERR".to_string(),
    };
    let _ = std::fs::remove_dir_all(&dir);
    println!("VERDICT {verdict}");
}

/// Print every change of zero-page byte UB_WATCH (hex) for both compilers.
#[test]
#[ignore = "debug helper: needs UB_REFERENCE, UB_FUZZ_FILE, UB_WATCH"]
fn differential_trace_file() {
    let exe = std::env::var("UB_REFERENCE").expect("set UB_REFERENCE");
    let file = std::env::var("UB_FUZZ_FILE").expect("set UB_FUZZ_FILE");
    let watch = usize::from_str_radix(&std::env::var("UB_WATCH").expect("set UB_WATCH"), 16).unwrap();
    let src = std::fs::read_to_string(&file).unwrap();
    let dir = std::env::temp_dir().join(format!("ub_fuzz2_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let new_prg = compile_src(&src).prg;
    let ref_prg = compile_reference(&exe, &src, &dir, 0).unwrap();
    for (name, prg) in [("new", new_prg), ("ref", ref_prg)] {
        let mut cpu = Cpu::new(&prg);
        let mut last = cpu.mem[watch];
        let mut log = vec![];
        loop {
            let pc = cpu.pc;
            if !cpu.step() || cpu.steps > 2_000_000 {
                break;
            }
            if cpu.mem[watch] != last {
                last = cpu.mem[watch];
                log.push(format!("${pc:04X}:{last}"));
            }
        }
        println!("{name}: {}", log.join(" "));
    }
}

/// Final state by variable name (works for zero-page and RAM variables).
fn named_state(res: &ultimate_basic::compiler::CompileResult) -> Result<Vec<u8>, String> {
    let mut cpu = Cpu::new(&res.prg);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cpu.run(20_000_000)));
    if r.is_err() {
        return Err(format!("crashed/hung at ${:04X}", cpu.pc));
    }
    let mut state = vec![];
    for name in ["v0", "v1", "v2", "v3", "v4", "v5", "l0", "l1", "g0", "g1", "g2"] {
        let v = res.map.variables.iter().find(|v| v.name == name);
        state.push(v.map(|v| cpu.mem[v.addr() as usize]).unwrap_or(0));
    }
    state.extend_from_slice(&cpu.mem[0xC000..0xC010]);
    Ok(state)
}

/// Variables moved to RAM (1.6.4 spilling) must not change behaviour: every
/// random program is compiled normally and with *all* eligible variables in
/// RAM, and both runs must end in the same state.
#[test]
fn spilled_variables_match_zero_page() {
    let cases: usize = std::env::var("UB_FUZZ_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let seed: u64 = std::env::var("UB_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let mut g = Gen { rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1), guards: 0, loop_vars: 0 };
    let mut spilled_total = 0;
    for n in 0..cases {
        let src = g.program();
        ultimate_basic::compiler::set_force_spill(false);
        let plain = compile_src(&src);
        ultimate_basic::compiler::set_force_spill(true);
        let spilled = compile_src(&src);
        ultimate_basic::compiler::set_force_spill(false);
        spilled_total += spilled.map.variables.iter().filter(|v| v.ram_addr.is_some()).count();
        let want = match named_state(&plain) {
            Ok(s) => s,
            Err(_) => continue, // the program itself hangs (budget) — nothing to compare
        };
        let got = named_state(&spilled).unwrap_or_else(|e| panic!("case {n}: spilled code {e}\n{src}"));
        assert_eq!(got, want, "case {n} differs (v0..v5, l0, l1, g0..g2, a[0..16])\n{src}");
    }
    assert!(spilled_total > cases, "spilling was hardly exercised ({spilled_total})");
    // more: UB_FUZZ_CASES=3000 cargo test --release --test differential_fuzz spilled
}
