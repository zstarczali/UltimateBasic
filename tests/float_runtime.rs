// Floating-point runtime (src/compiler/codegen/float_lib.rs) tested routine by
// routine on the 6502 emulator against Rust f64 arithmetic.

mod common;
use common::cpu6502::*;
use std::collections::HashMap;
use ultimate_basic::compiler::codegen::float_lib::{float_runtime, from_mflpt, to_mflpt};

const LIB: u16 = 0x2000;
const PTR: u8 = 0x60;
const A: u16 = 0xC000;
const B: u16 = 0xC010;
const R: u16 = 0xC020;
const TEXT: u16 = 0xC100;

struct Lib {
    code: Vec<u8>,
    labels: HashMap<String, u16>,
}

fn lib() -> Lib {
    let (code, labels) = float_runtime(LIB, PTR);
    Lib { code, labels }
}

enum Op<'a> {
    /// LDA #<addr; LDY #>addr; JSR routine
    Mem(&'a str, u16),
    /// LDA #lo; LDY #hi; JSR routine
    Val(&'a str, u16),
    /// JSR routine; STA R; STY R+1 (A/Y result)
    Ret(&'a str),
    Call(&'a str),
}

impl Lib {
    fn run(&self, ops: &[Op], setup: &[(u16, &[u8])]) -> Cpu {
        let mut code: Vec<u8> = vec![];
        for op in ops {
            let jsr = |name: &str, code: &mut Vec<u8>| {
                let a = *self.labels.get(name).unwrap_or_else(|| panic!("no routine {name}"));
                code.push(0x20);
                code.extend_from_slice(&a.to_le_bytes());
            };
            match op {
                Op::Mem(r, a) | Op::Val(r, a) => {
                    code.extend_from_slice(&[0xA9, *a as u8, 0xA0, (*a >> 8) as u8]);
                    jsr(r, &mut code);
                }
                Op::Ret(r) => {
                    jsr(r, &mut code);
                    code.extend_from_slice(&[0x8D, R as u8, (R >> 8) as u8, 0x8C, (R + 1) as u8, ((R + 1) >> 8) as u8]);
                }
                Op::Call(r) => jsr(r, &mut code),
            }
        }
        code.push(0x60);
        let mut prg = vec![0x01, 0x08];
        prg.extend_from_slice(&code);
        while prg.len() - 2 < (LIB - 0x0801) as usize {
            prg.push(0);
        }
        prg.extend_from_slice(&self.code);
        let mut cpu = Cpu::new(&prg);
        for (addr, bytes) in setup {
            cpu.mem[*addr as usize..*addr as usize + bytes.len()].copy_from_slice(bytes);
        }
        cpu.run(5_000_000);
        cpu
    }

    fn binop(&self, op: &str, a: f64, b: f64) -> f64 {
        let (pa, pb) = (to_mflpt(a), to_mflpt(b));
        let cpu = self.run(
            &[Op::Mem("f_load", A), Op::Mem(op, B), Op::Mem("f_store", R)],
            &[(A, &pa), (B, &pb)],
        );
        from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap())
    }

    fn cmp(&self, a: f64, b: f64) -> u8 {
        let (pa, pb) = (to_mflpt(a), to_mflpt(b));
        let cpu = self.run(&[Op::Mem("f_load", A), Op::Mem("f_cmp", B)], &[(A, &pa), (B, &pb)]);
        cpu.a
    }

    fn unary(&self, routine: &str, a: f64) -> f64 {
        let pa = to_mflpt(a);
        let cpu = self.run(
            &[Op::Mem("f_load", A), Op::Call(routine), Op::Mem("f_store", R)],
            &[(A, &pa)],
        );
        from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap())
    }

    fn to_int(&self, routine: &str, a: f64) -> u16 {
        let pa = to_mflpt(a);
        let cpu = self.run(&[Op::Mem("f_load", A), Op::Ret(routine)], &[(A, &pa)]);
        u16::from_le_bytes([cpu.mem[R as usize], cpu.mem[R as usize + 1]])
    }

    fn from_int(&self, routine: &str, v: u16) -> f64 {
        let cpu = self.run(&[Op::Val(routine, v), Op::Mem("f_store", R)], &[]);
        from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap())
    }

    fn out(&self, a: f64) -> String {
        let pa = to_mflpt(a);
        let cpu = self.run(&[Op::Mem("f_load", A), Op::Ret("f_out")], &[(A, &pa)]);
        let p = u16::from_le_bytes([cpu.mem[R as usize], cpu.mem[R as usize + 1]]) as usize;
        let end = cpu.mem[p..].iter().position(|&c| c == 0).unwrap();
        String::from_utf8(cpu.mem[p..p + end].to_vec()).unwrap()
    }

    fn parse(&self, text: &str) -> f64 {
        let mut t = text.as_bytes().to_vec();
        t.push(0);
        let cpu = self.run(&[Op::Mem("f_in", TEXT), Op::Mem("f_store", R)], &[(TEXT, &t)]);
        from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap())
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// A representable float with a random mantissa, sign and exponent in ±range.
    fn float(&mut self, range: i32) -> f64 {
        let m = (self.next() >> 32) as u32 | 0x8000_0000;
        let e = (self.next() % (2 * range as u64 + 1)) as i32 - range;
        let s = if self.next() & 1 == 1 { -1.0 } else { 1.0 };
        s * m as f64 / 4294967296.0 * 2f64.powi(e)
    }
}

fn ulp(x: f64) -> f64 {
    if x == 0.0 {
        return 2f64.powi(-160);
    }
    2f64.powi(x.abs().log2().floor() as i32 + 1 - 32)
}

fn close(got: f64, want: f64, ulps: f64) -> bool {
    (got - want).abs() <= ulp(want) * ulps
}

#[test]
fn add_sub_mul_div_within_one_ulp() {
    let l = lib();
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut cases: Vec<(f64, f64)> = vec![
        (1.0, 1.0),
        (1.0, -1.0),
        (0.0, 5.0),
        (5.0, 0.0),
        (0.1, 0.2),
        (1e9, 1.0),
        (1.0, 1e-9),
        (123456.789, -123456.788),
        (3.0, 7.0),
        (-2.5, 0.5),
    ];
    for _ in 0..1500 {
        cases.push((rng.float(40), rng.float(40)));
    }
    for _ in 0..300 {
        // near-cancellation
        let a = rng.float(10);
        let b = -a * (1.0 + (rng.float(0) * 1e-6));
        cases.push((a, from_mflpt(to_mflpt(b))));
    }
    for (a, b) in cases {
        let (a, b) = (from_mflpt(to_mflpt(a)), from_mflpt(to_mflpt(b)));
        for (op, want) in [("f_add", a + b), ("f_sub", a - b), ("f_mul", a * b)] {
            let got = l.binop(op, a, b);
            assert!(close(got, want, 1.0), "{op} {a:e} {b:e}: got {got:e} want {want:e}");
        }
        if b != 0.0 {
            let got = l.binop("f_div", a, b);
            assert!(close(got, a / b, 1.0), "f_div {a:e} {b:e}: got {got:e} want {:e}", a / b);
        }
    }
}

#[test]
fn overflow_underflow_and_division_by_zero() {
    let l = lib();
    let big = 1.5e38;
    assert!(l.binop("f_mul", big, big) > 1.7e38, "overflow saturates");
    assert!(l.binop("f_mul", -big, big) < -1.7e38);
    assert_eq!(l.binop("f_mul", 1e-30, 1e-30), 0.0, "underflow is zero");
    assert!(l.binop("f_div", 1.0, 0.0) > 1.7e38);
    assert!(l.binop("f_div", -1.0, 0.0) < -1.7e38);
    assert_eq!(l.binop("f_div", 0.0, 3.0), 0.0);
}

#[test]
fn compare() {
    let l = lib();
    let mut rng = Rng(77);
    let mut cases = vec![(0.0, 0.0), (1.0, 1.0), (-1.0, 1.0), (0.0, -2.0), (0.0, 2.0), (-3.0, -2.0), (2.0, 2.0000001)];
    for _ in 0..500 {
        cases.push((rng.float(20), rng.float(20)));
        let x = rng.float(20);
        cases.push((x, x));
    }
    for (a, b) in cases {
        let (a, b) = (from_mflpt(to_mflpt(a)), from_mflpt(to_mflpt(b)));
        let want = if a < b { 0xFF } else if a > b { 1 } else { 0 };
        assert_eq!(l.cmp(a, b), want, "{a} vs {b}");
    }
}

#[test]
fn integer_conversions() {
    let l = lib();
    for v in [0u16, 1, 2, 3, 100, 255, 256, 1000, 32767, 32768, 40000, 65535] {
        assert_eq!(l.from_int("f_fromu16", v), v as f64, "u16 {v}");
        assert_eq!(l.from_int("f_froms16", v), v as i16 as f64, "s16 {v}");
    }
    for v in (0..65536u32).step_by(97) {
        assert_eq!(l.from_int("f_froms16", v as u16), v as u16 as i16 as f64);
    }
    for (x, t, r) in [
        (0.0, 0, 0),
        (2.7, 2, 3),
        (-2.7, -2, -3),
        (2.5, 2, 3),
        (-2.5, -2, -3),
        (0.4, 0, 0),
        (-0.4, 0, 0),
        (32767.4, 32767, 32767),
        (-32768.0, -32768, -32768),
        (1e6, 32767, 32767),
        (-1e6, -32768, -32768),
        (12345.678, 12345, 12346),
    ] {
        assert_eq!(l.to_int("f_tos16", x) as i16, t, "trunc {x}");
        assert_eq!(l.to_int("f_rnds16", x) as i16, r, "round {x}");
    }
    assert_eq!(l.to_int("f_tou16", 65535.0), 65535);
    assert_eq!(l.to_int("f_tou16", 40000.9), 40000);
    assert_eq!(l.to_int("f_rndu16", 40000.5), 40001);
    assert_eq!(l.to_int("f_tou16", 1e9), 65535);
    assert_eq!(l.to_int("f_tou16", -1.0), 65535, "negative wraps like word arithmetic");
}

#[test]
fn fix_floor_neg_abs_sgn() {
    let l = lib();
    for (x, fix, floor) in [
        (0.0, 0.0, 0.0),
        (2.7, 2.0, 2.0),
        (-2.7, -2.0, -3.0),
        (-0.5, 0.0, -1.0),
        (0.5, 0.0, 0.0),
        (-3.0, -3.0, -3.0),
        (123456.5, 123456.0, 123456.0),
        (-123456.5, -123456.0, -123457.0),
        (1e12, 1e12, 1e12),
        (4294967296.5, 4294967296.0, 4294967296.0),
    ] {
        assert_eq!(l.unary("f_fix", x), fix, "fix {x}");
        assert_eq!(l.unary("f_int", x), floor, "int {x}");
    }
    assert_eq!(l.unary("f_neg", 2.5), -2.5);
    assert_eq!(l.unary("f_neg", 0.0), 0.0);
    assert_eq!(l.unary("f_abs", -2.5), 2.5);
    assert_eq!(l.unary("f_sgn", -2.5), -1.0);
    assert_eq!(l.unary("f_sgn", 0.0), 0.0);
    assert_eq!(l.unary("f_sgn", 1e-20), 1.0);
}

#[test]
fn number_to_text() {
    let l = lib();
    for (x, s) in [
        (0.0, "0"),
        (1.0, "1"),
        (-1.0, "-1"),
        (10.0, "10"),
        (0.5, ".5"),
        (-0.5, "-.5"),
        (3.25, "3.25"),
        (0.1, ".1"),
        (0.01, ".01"),
        (0.001, "1E-03"),
        (123456789.0, "123456789"),
        (1e9, "1E+09"),
        (1.5e10, "1.5E+10"),
        (3.141592653589793, "3.14159265"),
        (1.0 / 3.0, ".333333333"),
        (2.0 / 3.0, ".666666667"),
        (100.0, "100"),
        (12345.678, "12345.678"),
        (-2.5e-20, "-2.5E-20"),
        (1.7e38, "1.7E+38"),
        (65535.0, "65535"),
    ] {
        assert_eq!(l.out(x), s, "{x}");
    }
    // random values: the text reads back within the printed precision
    let mut rng = Rng(5);
    for _ in 0..300 {
        let x = from_mflpt(to_mflpt(rng.float(60)));
        let s = l.out(x);
        let back: f64 = s.replace("E", "e").parse().unwrap_or_else(|_| panic!("{x}: {s}"));
        assert!((back - x).abs() <= x.abs() * 6e-9, "{x:e} printed as {s}");
    }
}

#[test]
fn text_to_number() {
    let l = lib();
    for (s, x) in [
        ("0", 0.0),
        ("1", 1.0),
        ("-1", -1.0),
        ("  42", 42.0),
        ("+7", 7.0),
        ("3.25", 3.25),
        (".5", 0.5),
        ("-.5", -0.5),
        ("12345.678", 12345.678),
        ("1E3", 1000.0),
        ("1e-3", 0.001),
        ("2.5E+10", 2.5e10),
        ("3.14159265", 3.14159265),
        ("12abc", 12.0),
        ("", 0.0),
        ("65536", 65536.0),
    ] {
        let got = l.parse(s);
        let want = from_mflpt(to_mflpt(x));
        assert!(close(got, want, 2.0), "'{s}': got {got} want {want}");
    }
    // print → parse round trip
    let mut rng = Rng(9);
    for _ in 0..200 {
        let x = from_mflpt(to_mflpt(rng.float(30)));
        let s = l.out(x);
        let y = l.parse(&s);
        assert!((y - x).abs() <= x.abs() * 6e-9, "{x:e} -> {s} -> {y:e}");
    }
}

#[test]
fn stack_operations() {
    let l = lib();
    // (a - b) with b pushed first: FAC = a, pop b
    let (pa, pb) = (to_mflpt(10.0), to_mflpt(4.0));
    let cpu = l.run(
        &[
            Op::Mem("f_load", B),
            Op::Call("f_push"),
            Op::Mem("f_load", A),
            Op::Call("f_subp"),
            Op::Mem("f_store", R),
        ],
        &[(A, &pa), (B, &pb)],
    );
    assert_eq!(from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap()), 6.0);
    let cpu = l.run(
        &[
            Op::Mem("f_load", B),
            Op::Call("f_push"),
            Op::Mem("f_load", A),
            Op::Call("f_divp"),
            Op::Mem("f_store", R),
        ],
        &[(A, &pa), (B, &pb)],
    );
    assert_eq!(from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap()), 2.5);
}

/// The 9 printed digits are the correctly rounded ones except for values
/// within ~1e-11 of a rounding boundary (0.2% of random values; the 40-bit
/// intermediate cannot do better).
#[test]
fn printed_digits_are_correctly_rounded() {
    let l = lib();
    let mut rng = Rng(123);
    let (mut bad, n) = (0, 3000);
    for _ in 0..n {
        let x = from_mflpt(to_mflpt(rng.float(100).abs()));
        let s = l.out(x);
        // correctly rounded 9 significant digits
        let r = format!("{:.8e}", x);
        let (mant, _) = r.split_once('e').unwrap();
        let want_digits: String = mant.replace('.', "").trim_end_matches('0').to_string();
        let mut got_digits: String = s.split('E').next().unwrap().replace('.', "").replace('-', "");
        got_digits = got_digits.trim_start_matches('0').trim_end_matches('0').to_string();
        if got_digits != want_digits {
            bad += 1;
            eprintln!("{x:e}: got {s} want {r}");
        }
    }
    assert!(bad * 100 <= n, "{bad} of {n} values printed with a wrong last digit");
}

/// Relative (or, near zero, absolute) error.
fn rel_err(got: f64, want: f64) -> f64 {
    (got - want).abs() / want.abs().max(1.0)
}

#[test]
fn math_functions() {
    let l = lib();
    let mut worst: Vec<(&str, f64, f64, f64)> = vec![];
    let mut check = |name: &'static str, x: f64, got: f64, want: f64, tol: f64| {
        let e = rel_err(got, want);
        if e > tol {
            worst.push((name, x, got, want));
        }
    };
    let xs: Vec<f64> = (0..60).map(|i| (i as f64 - 30.0) * 0.731 + 0.013).collect();
    for &x in &xs {
        check("sin", x, l.unary("f_sin", x), x.sin(), 1e-8);
        check("cos", x, l.unary("f_cos", x), x.cos(), 1e-8);
        check("atn", x, l.unary("f_atn", x), x.atan(), 1e-8);
        check("exp", x, l.unary("f_exp", x), x.exp(), 2e-8 * x.exp().max(1.0));
        if x.cos().abs() > 0.05 {
            check("tan", x, l.unary("f_tan", x), x.tan(), 1e-7 * x.tan().abs().max(1.0));
        }
        if x > 0.0 {
            check("log", x, l.unary("f_log", x), x.ln(), 1e-8);
            check("sqr", x, l.unary("f_sqr", x), x.sqrt(), 1e-9);
        }
    }
    for &x in &[1e-20, 1e-5, 0.1, 0.5, 1.0, 2.0, 1e5, 1e20, 1.7e38] {
        check("log", x, l.unary("f_log", x), x.ln(), 1e-8);
        check("sqr", x, l.unary("f_sqr", x), x.sqrt(), 1e-9 * x.sqrt().max(1.0));
        check("atn", x, l.unary("f_atn", x), x.atan(), 1e-8);
    }
    assert_eq!(l.unary("f_sqr", 0.0), 0.0);
    assert_eq!(l.unary("f_sqr", -4.0), 0.0);
    assert_eq!(l.unary("f_exp", -200.0), 0.0);
    assert!(l.unary("f_exp", 200.0) > 1e38);
    assert_eq!(l.unary("f_sqr", 16.0), 4.0);
    assert!(worst.is_empty(), "{worst:?}");
}

#[test]
fn powers_and_q88() {
    let l = lib();
    let powi = |x: f64, k: u8| {
        let pa = to_mflpt(x);
        let cpu = l.run(
            &[Op::Mem("f_load", A), Op::Val("f_powi", k as u16), Op::Mem("f_store", R)],
            &[(A, &pa)],
        );
        from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap())
    };
    assert_eq!(powi(2.0, 10), 1024.0);
    assert_eq!(powi(3.0, 0), 1.0);
    assert_eq!(powi(-2.0, 3), -8.0);
    assert!(rel_err(powi(1.1, 50), 1.1f64.powi(50)) < 1e-8);
    let powp = |x: f64, y: f64| {
        let (pa, pb) = (to_mflpt(x), to_mflpt(y));
        let cpu = l.run(
            &[
                Op::Mem("f_load", B),
                Op::Call("f_push"),
                Op::Mem("f_load", A),
                Op::Call("f_powp"),
                Op::Mem("f_store", R),
            ],
            &[(A, &pa), (B, &pb)],
        );
        from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap())
    };
    for (x, y) in [(2.0, 0.5), (10.0, 3.0), (2.0, -1.0), (7.5, 2.25), (0.3, 1.7)] {
        let want = f64::powf(x, y);
        assert!(rel_err(powp(x, y), want) < 1e-8, "{x}^{y} = {} want {want}", powp(x, y));
    }
    assert!(rel_err(powp(-2.0, 3.0), -8.0) < 1e-8);
    assert!(rel_err(powp(-2.0, 2.0), 4.0) < 1e-8);
    assert_eq!(powp(0.0, 2.0), 0.0);
    // Q8.8
    assert_eq!(l.to_int("f_toq88", 3.75), 0x03C0);
    assert_eq!(l.to_int("f_toq88", 0.0), 0);
    let cpu = l.run(&[Op::Val("f_fromu16", 0x0380), Op::Call("f_q88dn"), Op::Mem("f_store", R)], &[]);
    assert_eq!(from_mflpt(cpu.mem[R as usize..R as usize + 5].try_into().unwrap()), 3.5);
}
