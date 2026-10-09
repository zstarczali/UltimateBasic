// SINGLE / DOUBLE: 5-byte floating point (codegen/float.rs + float_lib.rs),
// compiled programs run on the emulator.

mod common;
use common::cpu6502::*;
use ultimate_basic::compiler::codegen::float_lib::from_mflpt;

fn real(r: &Run, name: &str) -> f64 {
    let a = r.var_addr(name) as usize;
    from_mflpt(r.cpu.mem[a..a + 5].try_into().unwrap())
}

fn near(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-8 * want.abs().max(1.0)
}

#[test]
fn smoke() {
    let r = run_src(
        "dim x as double\ndim y as single\nx = 1.5\ny = x * 2 + 0.25\nprint x; \" \"; y\n",
    );
    assert_eq!(real(&r, "x"), 1.5);
    assert_eq!(real(&r, "y"), 3.25);
    assert_eq!(r.output(), "1.5 3.25\r");
}

#[test]
fn arithmetic_and_literals() {
    let r = run_src(
        "dim a as double, b as double, c as double, d as double, e as double, f as double, g as double, h as double
a = 0.1 * 3
b = 7 / 2
c = 2 ^ 10
d = -2 ^ 2
e = 1.5E3 + 2.5e-1
f = (a + b) * (c - 1000) / 4
g = 100000 * 3
h = .5 + 2 ^ 0.5
",
    );
    assert!(near(real(&r, "a"), 0.3));
    assert_eq!(real(&r, "b"), 3.5);
    assert_eq!(real(&r, "c"), 1024.0);
    assert_eq!(real(&r, "d"), -4.0);
    assert_eq!(real(&r, "e"), 1500.25);
    assert!(near(real(&r, "f"), (0.3 + 3.5) * 24.0 / 4.0));
    assert_eq!(real(&r, "g"), 300000.0);
    assert!(near(real(&r, "h"), 0.5 + 2f64.sqrt()));
}

#[test]
fn builtin_functions() {
    let r = run_src(
        "dim s as double, c as double, t as double, q as double, ex as double, lg as double, an as double
dim fx as double, fl as double, ci as double, ab as double, sg as double
s = sin(1)
c = cos(1)
t = tan(0.5)
q = sqr(2)
ex = exp(1)
lg = log(10)
an = atn(1) * 4
fx = fix(-2.7)
fl = int(-2.7)
ci = cint(2.5)
ab = abs(-3.25)
sg = sgn(-0.001)
",
    );
    for (n, v) in [
        ("s", 1f64.sin()),
        ("c", 1f64.cos()),
        ("t", 0.5f64.tan()),
        ("q", 2f64.sqrt()),
        ("ex", 1f64.exp()),
        ("lg", 10f64.ln()),
        ("an", std::f64::consts::PI),
        ("fx", -2.0),
        ("fl", -3.0),
        ("ci", 3.0),
        ("ab", 3.25),
        ("sg", -1.0),
    ] {
        assert!(near(real(&r, n), v), "{n} = {} want {v}", real(&r, n));
    }
}

#[test]
fn conversions_to_and_from_integers() {
    let r = run_src(
        "dim x as double
dim i as integer, w as word, b as byte
dim f as float
x = -2.5
i = x
x = 40000.0
w = x
b = 3.7 + x - x
f = 1.75 * 2
x = f + 0.125
dim y as double = i
dim z as double = w + 1
",
    );
    assert_eq!(r.word("i") as i16, -3);
    assert_eq!(r.word("w"), 40000);
    assert_eq!(r.byte("b"), 4);
    assert_eq!(r.word("f"), 0x0380); // Q8.8 3.5
    assert_eq!(real(&r, "x"), 3.625);
    assert_eq!(real(&r, "y"), -3.0);
    assert_eq!(real(&r, "z"), 40001.0);
}

#[test]
fn comparisons_and_conditions() {
    let r = run_src(
        "dim x as double
dim n as byte, k as byte
x = 0.5
if x > 0.25 then n = n + 1
if x < 0.25 then n = n + 10
if x = 0.5 then n = n + 2
if x <> 0.5 then n = n + 20
if x >= 0.5 and x <= 0.5 then n = n + 4
if x then n = n + 8
x = 0
while x < 1
  x = x + 0.125
  k = k + 1
wend
dim t as byte
t = (x > 0.9)
",
    );
    assert_eq!(r.byte("n"), 15);
    assert_eq!(r.byte("k"), 8);
    assert_eq!(r.byte("t"), 1);
}

#[test]
fn for_loops() {
    let r = run_src(
        "dim x as double, s as double, st as double
dim n as byte, m as byte, p as byte
for x = 0 to 1 step 0.25
  n = n + 1
  s = s + x
next
for x = 1 to 0 step -0.5
  m = m + 1
next
st = -1
for x = 3 to 1 step st
  p = p + 1
next
",
    );
    assert_eq!(r.byte("n"), 5);
    assert_eq!(real(&r, "s"), 2.5);
    assert_eq!(r.byte("m"), 3);
    assert_eq!(r.byte("p"), 3);
}

#[test]
fn arrays() {
    let r = run_src(
        "dim a(9) as double
dim g(2, 3) as single
dim i as byte, j as byte
dim s as double
for i = 0 to 9
  a(i) = i / 4
next
for i = 0 to 2
  for j = 0 to 3
    g(i, j) = i * 10 + j + 0.5
  next
next
for i = 0 to 9
  s = s + a(i)
next
s = s + g(2, 3) + a(3)
dim f(3) as float
f(1) = 1.5 * 1.5
",
    );
    assert_eq!(real(&r, "s"), 45.0 / 4.0 + 23.5 + 0.75);
    let base = r.res.map.arrays.iter().find(|a| a.name == "a").unwrap().base_addr as usize;
    assert_eq!(from_mflpt(r.cpu.mem[base + 15..base + 20].try_into().unwrap()), 0.75);
    let f = r.res.map.arrays.iter().find(|a| a.name == "f").unwrap().base_addr as usize;
    assert_eq!(u16::from_le_bytes([r.cpu.mem[f + 2], r.cpu.mem[f + 3]]), 0x0240); // 2.25
}

#[test]
fn functions_and_subs() {
    let r = run_src(
        "FUNCTION Hyp (a AS DOUBLE, b AS DOUBLE) AS DOUBLE
    Hyp = SQR(a * a + b * b)
END FUNCTION
FUNCTION Half (v AS DOUBLE) AS DOUBLE
    RETURN v / 2
END FUNCTION
SUB Add (v AS DOUBLE)
    total = total + v
END SUB
dim total as double
dim h as double, k as double
h = Hyp(3, 4)
k = Half(Hyp(6, 8)) + Half(1)
Add 1.25
Add h
",
    );
    assert_eq!(real(&r, "h"), 5.0);
    assert_eq!(real(&r, "k"), 5.5);
    assert_eq!(real(&r, "total"), 6.25);
}

#[test]
fn printing_and_strings() {
    let r = run_src(
        "dim x as double
dim s as string
x = 1 / 3
print x
print 1E10; \" \"; -0.001; \" \"; 123456789; \" \"; 2.5
s = \"X=\" + x
print s
s = str$(x * 3)
print s
x = val(\"-12.75\") * 2
print x
",
    );
    assert_eq!(
        r.output(),
        ".333333333\r1E+10 -1E-03 123456789 2.5\rX=.333333333\r1\r-25.5\r"
    );
}

#[test]
fn input_read_inc_dec() {
    let r = run_src_with_input(
        "dim x as double, y as double
input \"? \"; x
read y
inc y
dec x
data 7
",
        &["3.25E2"],
    );
    assert_eq!(real(&r, "x"), 324.0);
    assert_eq!(real(&r, "y"), 8.0);
}

#[test]
fn untyped_var_and_mixed() {
    let r = run_src(
        "var big = 1E6
var half = big / 2
var b = 10
var w: word = 0
w = half / 100
",
    );
    assert_eq!(real(&r, "big"), 1e6);
    assert_eq!(real(&r, "half"), 5e5);
    assert_eq!(r.word("w"), 5000);
}

#[test]
fn large_real_arrays_under_basic_rom() {
    let r = run_src(
        "dim a(1500) as double
dim i as word
dim s as double
for i = 0 to 1500
  a(i) = i
next
for i = 0 to 1500
  s = s + a(i)
next
",
    );
    assert_eq!(real(&r, "s"), 1500.0 * 1501.0 / 2.0);
    assert!(r.cpu.basic_visible());
}

#[test]
fn programs_without_reals_do_not_change() {
    // a plain decimal literal stays Q8.8, no runtime is appended
    let r = run_src("var f: float = 1.5\nf = f * 2\nprint f\n");
    assert_eq!(r.output(), "3.00\r");
    assert!(r.code_size() < 400, "{}", r.code_size());
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Random expression over a, b, c (DOUBLE), byte n and word w → (source, value).
fn gen_expr(rng: &mut Rng, depth: u32, env: &[(&str, f64)]) -> (String, f64) {
    if depth == 0 || rng.below(4) == 0 {
        return match rng.below(4) {
            0 => {
                let (n, v) = env[rng.below(env.len() as u64) as usize];
                (n.to_string(), v)
            }
            1 => {
                let v = rng.below(1000) as f64 / 8.0;
                (format!("{v}"), v)
            }
            2 => {
                let m = rng.below(9000) as f64 + 1000.0;
                let e = rng.below(9) as i32 - 4;
                let v: f64 = format!("{m}E{e}").parse().unwrap();
                (format!("{m}E{e}"), v)
            }
            _ => {
                let v = rng.below(200) as f64;
                (format!("{v}"), v)
            }
        };
    }
    let (ls, lv) = gen_expr(rng, depth - 1, env);
    let (rs, rv) = gen_expr(rng, depth - 1, env);
    match rng.below(8) {
        0 | 1 => (format!("({ls} + {rs})"), lv + rv),
        2 | 3 => (format!("({ls} - {rs})"), lv - rv),
        4 | 5 => (format!("({ls} * {rs})"), lv * rv),
        6 if rv.abs() > 1e-3 => (format!("({ls} / {rs})"), lv / rv),
        _ => match rng.below(4) {
            0 => (format!("abs({ls})"), lv.abs()),
            1 => (format!("sqr(abs({ls}))"), lv.abs().sqrt()),
            2 => (format!("int({ls})"), lv.floor()),
            _ => (format!("-{ls}"), -lv),
        },
    }
}

#[test]
fn random_expressions_match_f64() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut bad = vec![];
    for case in 0..150 {
        let env = [
            ("a", (rng.below(20000) as f64 - 10000.0) / 64.0),
            ("b", rng.below(5000) as f64 / 1000.0 + 0.5),
            ("c", -(rng.below(300) as f64) / 7.0),
            ("n", rng.below(256) as f64),
            ("w", rng.below(60000) as f64),
        ];
        let mut src = format!(
            "dim a as double, b as double, c as double, r as double, k as byte\n\
             dim n as byte = {}\ndim w as word = {}\na = {}\nb = {}\nc = {}\n",
            env[3].1, env[4].1, env[0].1, env[1].1, env[2].1
        );
        let (e, want) = gen_expr(&mut rng, 4, &env);
        if !want.is_finite() || want.abs() > 1e30 || (want != 0.0 && want.abs() < 1e-30) {
            continue;
        }
        let (e2, want2) = gen_expr(&mut rng, 2, &env);
        src += &format!("r = {e}\nif c * 0 + {e} < {e2} then k = 1\n");
        let r = run_src(&src);
        let got = real(&r, "r");
        // a few roundings per operation: compare relative to the operand sizes
        let tol = 1e-6 * want.abs().max(1.0) * 50.0;
        if (got - want).abs() > tol.max(1e-6) {
            bad.push(format!("case {case}: {e} = {got}, want {want}"));
        }
        let k = r.byte("k");
        if (want - want2).abs() > tol && k != (want < want2) as u8 {
            bad.push(format!("case {case}: {e} < {e2}: got {k}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn parser_changes_of_1_6_4() {
    // leading minus binds to its operand: -5 - 3 = -8 (was -(5 - 3))
    let r = run_src("dim n as integer\ndim m as integer\nn = -5 - 3\nm = -n * 2 + 1\n");
    assert_eq!(r.word("n") as i16, -8);
    assert_eq!(r.word("m") as i16, 17);
    // binary literals, big integer constants
    let r = run_src("var b = %1010\nconst K = 200 * 200\nvar w: word = K\nvar v: word = 0\nv = 300 * 200 / 2\n");
    assert_eq!(r.byte("b"), 10);
    assert_eq!(r.word("w"), 40000);
    assert_eq!(r.word("v"), ((300i32 * 200) as i16 / 2) as u16); // integer rules unchanged
}

#[test]
fn select_case_and_spilled_variables() {
    let src = "dim x as double
dim n as byte, i as byte, j as integer, k as word
x = 2.75
SELECT CASE x
  CASE IS < 1 : n = 1
  CASE 2 TO 3 : n = 2
  CASE ELSE : n = 3
END SELECT
for i = 1 to 10
  j = j + x * i
  k = k + sqr(i * i)
next
x = x + j + k
";
    let plain = run_src(src);
    ultimate_basic::compiler::set_force_spill(true);
    let spilled = run_src(src);
    ultimate_basic::compiler::set_force_spill(false);
    for r in [&plain, &spilled] {
        assert_eq!(r.byte("n"), 2);
        // j: rounded after every step (2.75, 8.25 → 8, …)
        let mut j = 0i32;
        for i in 1..=10 {
            j = (j as f64 + 2.75 * i as f64).round() as i32;
        }
        assert_eq!(r.word("j") as i32, j);
        assert_eq!(r.word("k"), 55);
        assert_eq!(real(r, "x"), 2.75 + j as f64 + 55.0);
    }
    assert!(spilled.res.map.variables.iter().any(|v| v.ram_addr.is_some() && v.type_str != "single"));
}

#[test]
fn float_demo_runs() {
    let src = std::fs::read_to_string("examples/float_demo.ub").unwrap();
    let r = run_src_budget(&src, 50_000_000);
    let out = r.output();
    for want in [
        "PI = 3.14159265\r",
        "R=1.5  T=7.06858347\r",
        "SIN(PI/6) = .5\r",
        "ATFOGO(3,4) = 5\r",
        "TOKE 10 EV UTAN: 162889.46\r",
        "1.2676506E+30 3.33333333E-09 -1.5E-07\r",
        "PI*1000 -> INTEGER: 3142\r",
    ] {
        assert!(out.contains(want), "{want:?} missing in {out:?}");
    }
}
