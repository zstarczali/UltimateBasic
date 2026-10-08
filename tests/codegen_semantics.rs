// Semantic codegen tests: compile UB programs, execute them on a full 6502
// emulator and compare variable values against results computed in Rust.
//
// These tests pin down *behaviour*, not byte patterns, so the code generator
// can be optimised freely as long as they stay green.

mod common;
use common::cpu6502::*;

const VALUES: &[u8] = &[0, 1, 2, 3, 5, 7, 8, 10, 15, 16, 31, 64, 100, 127, 128, 129, 200, 254, 255];

#[derive(Clone, Copy, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

impl Op {
    fn sym(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Mod => "mod",
            Op::And => "and",
            Op::Or => "or",
            Op::Xor => "xor",
            Op::Shl => "shl",
            Op::Shr => "shr",
        }
    }
    fn eval(self, a: u8, b: u8) -> Option<u8> {
        Some(match self {
            Op::Add => a.wrapping_add(b),
            Op::Sub => a.wrapping_sub(b),
            Op::Mul => a.wrapping_mul(b),
            Op::Div => a.checked_div(b)?,
            Op::Mod => a.checked_rem(b)?,
            Op::And => a & b,
            Op::Or => a | b,
            Op::Xor => a ^ b,
            Op::Shl => {
                if b >= 8 {
                    0
                } else {
                    a << b
                }
            }
            Op::Shr => {
                if b >= 8 {
                    0
                } else {
                    a >> b
                }
            }
        })
    }
}

const OPS: &[Op] = &[
    Op::Add,
    Op::Sub,
    Op::Mul,
    Op::Div,
    Op::Mod,
    Op::And,
    Op::Or,
    Op::Xor,
    Op::Shl,
    Op::Shr,
];

/// For every op and value pair: var OP var, var OP const, const OP var.
#[test]
fn binary_ops_match_reference() {
    for &op in OPS {
        for &a in VALUES {
            for &b in VALUES {
                let Some(want) = op.eval(a, b) else { continue };
                // Shift counts beyond 8 are slow in the loop implementation and
                // not interesting; keep them small.
                if matches!(op, Op::Shl | Op::Shr) && b > 9 {
                    continue;
                }
                let s = op.sym();
                let src = format!(
                    "var a = {a}\nvar b = {b}\n\
                     var r1 = a {s} b\nvar r2 = a {s} {b}\nvar r3 = {a} {s} b\n\
                     var r4 = 0\nr4 = a\nr4 = r4 {s} b\n"
                );
                let r = run_src(&src);
                for v in ["r1", "r2", "r3", "r4"] {
                    assert_eq!(r.byte(v), want, "{v}: {a} {s} {b}\n{src}");
                }
                // operands must be untouched
                assert_eq!(r.byte("a"), a, "a clobbered by {a} {s} {b}");
                assert_eq!(r.byte("b"), b, "b clobbered by {a} {s} {b}");
            }
        }
    }
}

/// Nested expressions exercise tmp-zp allocation for both operand sides.
#[test]
fn nested_expressions_match_reference() {
    for &a in VALUES {
        for &b in &[1u8, 3, 7, 16, 200] {
            for &c in &[0u8, 2, 9, 255] {
                let src = format!(
                    "var a = {a}\nvar b = {b}\nvar c = {c}\n\
                     var r1 = (a + b) * (c - a)\n\
                     var r2 = a - (b - c)\n\
                     var r3 = (a xor c) and (b or 1)\n\
                     var r4 = a / b + c mod b\n\
                     var r5 = (a shr 1) + (c shl 2) - b\n\
                     var r6 = a * 3 + b * 2 + c\n"
                );
                let r = run_src(&src);
                let w = |x: u8| x;
                assert_eq!(r.byte("r1"), w(a.wrapping_add(b).wrapping_mul(c.wrapping_sub(a))), "r1 {src}");
                assert_eq!(r.byte("r2"), a.wrapping_sub(b.wrapping_sub(c)), "r2 {src}");
                assert_eq!(r.byte("r3"), (a ^ c) & (b | 1), "r3 {src}");
                assert_eq!(r.byte("r4"), (a / b).wrapping_add(c % b), "r4 {src}");
                assert_eq!(
                    r.byte("r5"),
                    (a >> 1).wrapping_add(c << 2).wrapping_sub(b),
                    "r5 {src}"
                );
                assert_eq!(
                    r.byte("r6"),
                    a.wrapping_mul(3).wrapping_add(b.wrapping_mul(2)).wrapping_add(c),
                    "r6 {src}"
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

const CMPS: &[Cmp] = &[Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Gt, Cmp::Le, Cmp::Ge];

impl Cmp {
    fn sym(self) -> &'static str {
        match self {
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Lt => "<",
            Cmp::Gt => ">",
            Cmp::Le => "<=",
            Cmp::Ge => ">=",
        }
    }
    fn eval(self, a: u8, b: u8) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Gt => a > b,
            Cmp::Le => a <= b,
            Cmp::Ge => a >= b,
        }
    }
}

/// Comparisons as values and as if / if-else / while conditions.
#[test]
fn comparisons_match_reference() {
    for &cmp in CMPS {
        for &a in VALUES {
            for &b in VALUES {
                let s = cmp.sym();
                let want = cmp.eval(a, b);
                let src = format!(
                    "var a = {a}\nvar b = {b}\n\
                     var v1 = a {s} b\nvar v2 = a {s} {b}\nvar v3 = {a} {s} b\n\
                     var t1 = 0\nif a {s} b then\n t1 = 1\nend\n\
                     var t2 = 0\nif a {s} {b} then\n t2 = 1\nelse\n t2 = 2\nend\n\
                     var t3 = 0\nif {a} {s} b then t3 = 7 end\n\
                     var t4 = 0\nif not (a {s} b) then\n t4 = 1\nend\n\
                     var n = 0\nwhile a {s} b\n n = n + 1\n if n == 3 then break end\nend\n"
                );
                let r = run_src(&src);
                let ctx = format!("{a} {s} {b}");
                assert_eq!(r.byte("v1"), want as u8, "v1 {ctx}");
                assert_eq!(r.byte("v2"), want as u8, "v2 {ctx}");
                assert_eq!(r.byte("v3"), want as u8, "v3 {ctx}");
                assert_eq!(r.byte("t1"), want as u8, "t1 {ctx}");
                assert_eq!(r.byte("t2"), if want { 1 } else { 2 }, "t2 {ctx}");
                assert_eq!(r.byte("t3"), if want { 7 } else { 0 }, "t3 {ctx}");
                assert_eq!(r.byte("t4"), (!want) as u8, "t4 {ctx}");
                assert_eq!(r.byte("n"), if want { 3 } else { 0 }, "while {ctx}");
            }
        }
    }
}

/// Combined conditions with bitwise and/or of comparison results, plus
/// plain-value conditions (non-zero = true).
#[test]
fn compound_conditions_match_reference() {
    for &a in VALUES {
        for &b in VALUES {
            let src = format!(
                "var a = {a}\nvar b = {b}\n\
                 var t1 = 0\nif a > 10 and b < 100 then t1 = 1 end\n\
                 var t2 = 0\nif a == 0 or b == 255 then t2 = 1 end\n\
                 var t3 = 0\nif a then t3 = 1 end\n\
                 var t4 = 0\nif a and b then t4 = 1 end\n\
                 var t5 = 0\nif (a < b) xor (b < 50) then t5 = 1 end\n\
                 var t6 = 0\nif a + 1 > b then t6 = 1 end\n"
            );
            let r = run_src(&src);
            let ctx = format!("a={a} b={b}");
            assert_eq!(r.byte("t1"), (a > 10 && b < 100) as u8, "t1 {ctx}");
            assert_eq!(r.byte("t2"), (a == 0 || b == 255) as u8, "t2 {ctx}");
            assert_eq!(r.byte("t3"), (a != 0) as u8, "t3 {ctx}");
            assert_eq!(r.byte("t4"), (a & b != 0) as u8, "t4 {ctx}");
            assert_eq!(r.byte("t5"), ((a < b) ^ (b < 50)) as u8, "t5 {ctx}");
            assert_eq!(r.byte("t6"), (a.wrapping_add(1) > b) as u8, "t6 {ctx}");
        }
    }
}

/// Loop / if bodies longer than a short branch can reach.
#[test]
fn long_bodies_beyond_branch_range() {
    let body = " poke $C000, n\n".repeat(50);
    for &a in &[0u8, 1, 5] {
        let src = format!(
            "var a = {a}\nvar n = 0\nvar m = 0\nvar t = 0\n\
             while n < a\n{body} n = n + 1\nend\n\
             repeat\n{body} m = m + 1\nuntil m >= a\n\
             if a > 0 and a < 5 then\n{body} t = 1\nelse\n{body} t = 2\nend\n"
        );
        let r = run_src(&src);
        assert_eq!(r.byte("n"), a, "while a={a}");
        assert_eq!(r.byte("m"), a.max(1), "repeat a={a}");
        assert_eq!(r.byte("t"), if a > 0 && a < 5 { 1 } else { 2 }, "if a={a}");
    }
    // for loops in both directions; the old count-down loop truncated its
    // back branch when the body exceeded 127 bytes and jumped mid-body.
    let src = format!(
        "var i = 0\nvar n = 0\nvar m = 0\n\
         for i = 107 to 104 step -2\n{body} n = n + 1\nnext\n\
         for i = 3 to 9 step 3\n{body} m = m + 1\nnext\n"
    );
    let r = run_src(&src);
    assert_eq!(r.byte("n"), 2, "count-down loop with long body");
    assert_eq!(r.byte("m"), 3, "count-up loop with long body");
}

/// Right-hand sides with side effects are still evaluated (no short circuit)
/// and the left side is evaluated first.
#[test]
fn side_effects_in_conditions_are_kept() {
    let src = "var g = 0\nvar a = 0\nvar t = 0\n\
               fn bump()\n g = g + 1\n return g\nend\n\
               if a > 10 and bump() == 1 then t = 1 end\n\
               if a == 0 or bump() == 9 then t = t + 2 end\n\
               var u = 0\nif bump() > a then u = 1 end\n\
               a = 3\nvar v = 0\nif a < bump() then v = 1 end\n";
    let r = run_src(src);
    assert_eq!(r.byte("g"), 4, "every bump() call must run");
    assert_eq!(r.byte("t"), 2);
    assert_eq!(r.byte("u"), 1);
    assert_eq!(r.byte("v"), 1);
}

/// `while` / `until` keep their historical "value == 1" semantics for plain
/// values, `if` treats any non-zero value as true.
#[test]
fn truthiness_of_plain_values() {
    let src = "var a = 2\nvar t = 0\nvar n = 0\nvar m = 0\n\
               if a then t = 1 end\n\
               while a\n n = n + 1\n break\nend\n\
               repeat\n m = m + 1\n if m == 3 then break end\nuntil a\n";
    let r = run_src(src);
    assert_eq!(r.byte("t"), 1);
    assert_eq!(r.byte("n"), 0);
    assert_eq!(r.byte("m"), 3);
}

#[test]
fn select_with_expression_cases_and_body_modifying_selector() {
    for &a in &[1u8, 2, 3, 9] {
        let src = format!(
            "var a = {a}\nvar k = 2\nvar s = 0\n\
             select a\n case 1:\n  a = 2\n  s = 10\n case k:\n  s = 20\n case k + 1:\n  s = 30\n else:\n  s = 40\nend\n\
             var s2 = 0\n\
             select a + 1\n case 3:\n  s2 = 1\n else:\n  s2 = 2\nend\n"
        );
        let r = run_src(&src);
        let (want_s, a_after) = match a {
            1 => (10, 2),
            2 => (20, 2),
            3 => (30, 3),
            _ => (40, a),
        };
        assert_eq!(r.byte("s"), want_s, "select a={a}");
        assert_eq!(r.byte("s2"), if a_after + 1 == 3 { 1 } else { 2 }, "select a+1, a={a}");
    }
}

#[test]
fn repeat_until_and_select() {
    for &a in VALUES {
        let src = format!(
            "var a = {a}\nvar n = 0\n\
             repeat\n n = n + 1\nuntil n >= a\n\
             var s = 0\n\
             select a\n case 0:\n  s = 10\n case 7:\n  s = 20\n case 255:\n  s = 30\n else:\n  s = 40\nend\n"
        );
        let r = run_src(&src);
        assert_eq!(r.byte("n"), a.max(1), "repeat a={a}");
        let want = match a {
            0 => 10,
            7 => 20,
            255 => 30,
            _ => 40,
        };
        assert_eq!(r.byte("s"), want, "select a={a}");
    }
}

#[test]
fn inc_dec_and_compound_assign() {
    for &a in VALUES {
        for &b in &[0u8, 1, 3, 100, 255] {
            let src = format!(
                "var a = {a}\nvar b = {b}\n\
                 var r1 = a\nr1 = r1 + 1\n\
                 var r2 = a\nr2 = r2 - 1\n\
                 var r3 = a\nr3 += b\n\
                 var r4 = a\nr4 -= b\n\
                 var r5 = a\nr5 = 1 + r5\n\
                 var r6 = a\nr6 += 5\n\
                 var r7 = a\ninc r7\n\
                 var r8 = a\ndec r8\n\
                 var r9 = a\nr9 = r9 + 2\n\
                 var r10 = a\nr10 = r10 - 2\n"
            );
            let r = run_src(&src);
            let ctx = format!("a={a} b={b}");
            assert_eq!(r.byte("r1"), a.wrapping_add(1), "r1 {ctx}");
            assert_eq!(r.byte("r2"), a.wrapping_sub(1), "r2 {ctx}");
            assert_eq!(r.byte("r3"), a.wrapping_add(b), "r3 {ctx}");
            assert_eq!(r.byte("r4"), a.wrapping_sub(b), "r4 {ctx}");
            assert_eq!(r.byte("r5"), a.wrapping_add(1), "r5 {ctx}");
            assert_eq!(r.byte("r6"), a.wrapping_add(5), "r6 {ctx}");
            assert_eq!(r.byte("r7"), a.wrapping_add(1), "r7 {ctx}");
            assert_eq!(r.byte("r8"), a.wrapping_sub(1), "r8 {ctx}");
            assert_eq!(r.byte("r9"), a.wrapping_add(2), "r9 {ctx}");
            assert_eq!(r.byte("r10"), a.wrapping_sub(2), "r10 {ctx}");
        }
    }
}

#[test]
fn word_inc_and_add() {
    for &w in &[0u16, 1, 255, 256, 1000, 0x7FFF, 0xFFFE, 0xFFFF] {
        for &x in &[0u8, 1, 200, 255] {
            let src = format!(
                "var x = {x}\n\
                 var w1: word = {w}\nw1 = w1 + 1\n\
                 var w2: word = {w}\nw2 = w2 + x\n\
                 var w3: word = {w}\nw3 = w3 - 1\n\
                 var w4: word = {w}\nw4 += 300\n\
                 var w5: word = {w}\ninc w5\n\
                 var w6: word = {w}\ndec w6\n\
                 var w7: word = {w}\nw7 = w7 + 256\n"
            );
            let r = run_src(&src);
            let ctx = format!("w={w} x={x}");
            assert_eq!(r.word("w1"), w.wrapping_add(1), "w1 {ctx}");
            assert_eq!(r.word("w2"), w.wrapping_add(x as u16), "w2 {ctx}");
            assert_eq!(r.word("w3"), w.wrapping_sub(1), "w3 {ctx}");
            assert_eq!(r.word("w4"), w.wrapping_add(300), "w4 {ctx}");
            assert_eq!(r.word("w5"), w.wrapping_add(1), "w5 {ctx}");
            assert_eq!(r.word("w6"), w.wrapping_sub(1), "w6 {ctx}");
            assert_eq!(r.word("w7"), w.wrapping_add(256), "w7 {ctx}");
        }
    }
}

#[test]
fn for_loops_match_reference() {
    // (from, to, step) → iteration count and sum of loop variable (mod 256)
    let cases: &[(u8, u8, i16)] = &[
        (0, 15, 1),
        (1, 10, 1),
        (5, 5, 1),
        (0, 20, 2),
        (0, 21, 2),
        (10, 1, -1),
        (20, 0, -2),
        (255, 0, -1),
        (5, 0, -5),
        (3, 3, -1),
    ];
    for &(from, to, step) in cases {
        let step_src = if step == 1 {
            String::new()
        } else {
            format!(" step {step}")
        };
        let src = format!(
            "var i = 0\nvar n = 0\nvar s = 0\n\
             for i = {from} to {to}{step_src}\n n = n + 1\n s = s + i\nnext\n\
             var lim = {to}\nvar m = 0\n\
             for i = {from} to lim{step_src}\n m = m + 1\nnext\n"
        );
        let r = run_src(&src);
        let mut n: u32 = 0;
        let mut s: u8 = 0;
        let mut i = from as i32;
        loop {
            if step > 0 && i > to as i32 || step < 0 && i < to as i32 {
                break;
            }
            n += 1;
            s = s.wrapping_add(i as u8);
            i += step as i32;
        }
        let ctx = format!("for {from} to {to} step {step}");
        assert_eq!(r.byte("n"), n as u8, "count {ctx}");
        assert_eq!(r.byte("s"), s, "sum {ctx}");
        assert_eq!(r.byte("m"), n as u8, "count (var limit) {ctx}");
    }
}

#[test]
fn for_loop_break_continue_and_nesting() {
    let src = "var i = 0\nvar j = 0\nvar n = 0\nvar c = 0\n\
               for i = 0 to 9\n\
                 if i == 3 then continue end\n\
                 if i == 8 then break end\n\
                 for j = 1 to i\n  n = n + 1\n next\n\
                 c = c + 1\n\
               next\n";
    let r = run_src(src);
    // i in 0..=7 except 3 → c = 7, n = sum(1..=i for those i) (j loop runs i times when i≥1)
    let want_n: u32 = (0..8).filter(|&i| i != 3).map(|i| i as u32).sum();
    assert_eq!(r.byte("c"), 7);
    assert_eq!(r.byte("n"), want_n as u8);
}

#[test]
fn byte_arrays_const_and_var_index() {
    let src = "var a = array(16)\nvar b = array(300)\nvar i = 0\nvar s = 0\nvar k = 5\n\
               for i = 0 to 15\n a[i] = i * 3\nnext\n\
               a[2] = 99\n\
               a[k] = a[k] + 1\n\
               for i = 0 to 15\n s = s + a[i]\nnext\n\
               var p = a[2]\nvar q = a[k]\nvar r = a[k + 1]\n\
               b[0] = 1\nb[255] = 2\nb[299] = 3\n\
               var j = 200\nb[j] = 4\n";
    let r = run_src(src);
    let mut arr: Vec<u8> = (0..16).map(|i: u8| i * 3).collect();
    arr[2] = 99;
    arr[5] += 1;
    let s = arr.iter().fold(0u8, |acc, &x| acc.wrapping_add(x));
    assert_eq!(r.byte("s"), s);
    assert_eq!(r.byte("p"), 99);
    assert_eq!(r.byte("q"), 16);
    assert_eq!(r.byte("r"), 18);
    let a_base = r.res.map.arrays.iter().find(|x| x.name == "a").unwrap().base_addr;
    for (k, &v) in arr.iter().enumerate() {
        assert_eq!(r.mem(a_base + k as u16), v, "a[{k}]");
    }
    let b_base = r.res.map.arrays.iter().find(|x| x.name == "b").unwrap().base_addr;
    assert_eq!(r.mem(b_base), 1);
    assert_eq!(r.mem(b_base + 255), 2);
    assert_eq!(r.mem(b_base + 299), 3);
    assert_eq!(r.mem(b_base + 200), 4);
}

#[test]
fn word_arrays_and_2d_arrays() {
    let src = "var w = array_word(8)\nvar g = array(4, 5)\nvar i = 0\nvar j = 0\n\
               for i = 0 to 7\n w[i] = 1000\nnext\n\
               w[3] = $1234\n\
               var k = 6\nw[k] = $ABCD\n\
               var t: word = w[k]\nvar u: word = w[3]\n\
               for i = 0 to 3\n for j = 0 to 4\n  g[i, j] = i * 10 + j\n next\nnext\n\
               var x = g[2, 3]\nvar r = 3\nvar c = 4\nvar y = g[r, c]\n";
    let r = run_src(src);
    assert_eq!(r.word("t"), 0xABCD);
    assert_eq!(r.word("u"), 0x1234);
    assert_eq!(r.byte("x"), 23);
    assert_eq!(r.byte("y"), 34);
    let wb = r.res.map.arrays.iter().find(|x| x.name == "w").unwrap().base_addr;
    let rd = |k: u16| u16::from_le_bytes([r.mem(wb + 2 * k), r.mem(wb + 2 * k + 1)]);
    assert_eq!(rd(0), 1000);
    assert_eq!(rd(7), 1000);
    assert_eq!(rd(3), 0x1234);
    assert_eq!(rd(6), 0xABCD);
}

#[test]
fn poke_and_peek() {
    let src = "var x = 77\nvar w: word = $C100\n\
               poke $C200, x\npoke $C201, x + 1\npoke w, 5\n\
               var p = peek($C201)\nvar q = peek(w)\n";
    let r = run_src(src);
    assert_eq!(r.mem(0xC200), 77);
    assert_eq!(r.mem(0xC201), 78);
    assert_eq!(r.mem(0xC100), 5);
    assert_eq!(r.byte("p"), 78);
    assert_eq!(r.byte("q"), 5);
}

#[test]
fn subs_and_functions() {
    let src = "var r = 0\nvar g = 0\n\
               fn sq(x)\n return x * x\nend\n\
               fn clampx(v, lo, hi)\n if v < lo then return lo end\n if v > hi then return hi end\n return v\nend\n\
               sub addg(a, b)\n g = g + a + b\nend\n\
               r = sq(12)\n\
               var c1 = clampx(5, 10, 20)\nvar c2 = clampx(15, 10, 20)\nvar c3 = clampx(25, 10, 20)\n\
               addg(3, 4)\naddg(10, 20)\n";
    let r = run_src(src);
    assert_eq!(r.byte("r"), 144);
    assert_eq!(r.byte("c1"), 10);
    assert_eq!(r.byte("c2"), 15);
    assert_eq!(r.byte("c3"), 20);
    assert_eq!(r.byte("g"), 37);
}

#[test]
fn print_numbers() {
    let r = run_src("var x = 200\nvar y = 7\nprint x * 1\nprint y + 3, x / 2");
    let out = r.output();
    assert!(out.contains("200"), "{out:?}");
    assert!(out.contains("10"), "{out:?}");
    assert!(out.contains("100"), "{out:?}");
}

/// Up-counting loops whose variable would wrap past 255 terminate (they used
/// to run forever or restart from the wrapped value).
#[test]
fn for_loops_terminate_on_overflow() {
    let cases: &[(u8, u8, i16, u32)] = &[
        (0, 255, 1, 256),
        (200, 255, 1, 56),
        (1, 254, 3, 85),
        (0, 250, 50, 6),
        (250, 255, 7, 1),
        (255, 0, -1, 256),
        (6, 0, -4, 2),
    ];
    for &(from, to, step, want) in cases {
        let st = if step == 1 { String::new() } else { format!(" step {step}") };
        let src = format!(
            "var i = 0\nvar n = 0\nvar lim = {to}\nvar m = 0\nvar s = {step}\n\
             for i = {from} to {to}{st}\n n = n + 1\nnext\n\
             for i = {from} to lim{st}\n m = m + 1\nnext\n"
        );
        let r = run_src(&src);
        let ctx = format!("for {from} to {to} step {step}");
        assert_eq!(r.byte("n"), want as u8, "{ctx}");
        assert_eq!(r.byte("m"), want as u8, "{ctx} (var limit)");
    }
    // variable (positive) step
    let r = run_src("var i = 0\nvar n = 0\nvar s = 100\nfor i = 0 to 255 step s\n n = n + 1\nnext\n");
    assert_eq!(r.byte("n"), 3); // 0, 100, 200
}

/// The limit is evaluated once; changing it (or the loop variable) inside the
/// body behaves as before.
#[test]
fn for_loop_limit_snapshot_and_body_writes() {
    let r = run_src(
        "var i = 0\nvar lim = 5\nvar n = 0\n\
         for i = 1 to lim\n lim = 2\n n = n + 1\nnext\n\
         var m = 0\nfor i = 0 to 10\n m = m + 1\n i = i + 1\nnext\n",
    );
    assert_eq!(r.byte("n"), 5);
    assert_eq!(r.byte("m"), 6);
}

/// Zero-iteration loops (only possible with variable bounds) skip the body.
#[test]
fn for_loop_zero_iterations() {
    let r = run_src(
        "var i = 0\nvar a = 9\nvar b = 3\nvar n = 0\n\
         for i = a to b\n n = n + 1\nnext\n\
         for i = b to a step -1\n n = n + 1\nnext\n",
    );
    assert_eq!(r.byte("n"), 0);
}

/// Reusing A after `x = …` must never be visible: jump targets, stale flags
/// and nested blocks are all excluded.
#[test]
fn a_register_reuse_is_invisible() {
    // Multiply loop ends with A = 3 but Z = 1 (from LSR q): `if z` must
    // still see a non-zero value.
    let r = run_src("var x = 3\nvar y = 1\nvar t = 0\nvar z = x * y\nif z then t = 1 end\n");
    assert_eq!(r.byte("t"), 1);
    let r = run_src("var x = 7\nvar y = 3\nvar t = 0\nvar z = x mod y\nif z then t = 1 end\n");
    assert_eq!(r.byte("t"), 1);

    // Loop top right after an assignment: the back edge arrives with a
    // different A.
    let r = run_src("var x = 1\nvar n = 0\nloop\n if x == 3 then break end\n x = x + 1\n n = n + 1\nend\n");
    assert_eq!(r.byte("x"), 3);
    assert_eq!(r.byte("n"), 2);
    let r = run_src("var x = 1\nvar n = 0\nrepeat\n n = x\n x = x + 2\nuntil x > 6\n");
    assert_eq!(r.byte("n"), 5);

    // label / goto target right after an assignment
    let r = run_src("var x = 1\nvar y = 0\nlabel top\ny = x\nif x < 5 then\n x = x + 1\n goto top\nend\n");
    assert_eq!(r.byte("y"), 5);

    // assignment at the end of a then-/else-block must not leak past the if
    // (top level and nested, as the main program and blocks are generated by
    // different loops)
    for &a in &[0u8, 1, 5] {
        let r = run_src(&format!(
            "var a = {a}\nvar x = 7\nvar t = 0\nif a then\n x = 5\nend\nif x == 5 then t = 1 end\nvar y = x\n\
             var p = 9\nif a == 5 then\n p = 3\nelse\n p = 4\nend\nvar q = p\n\
             var r = 0\nloop\n  if a then\n   x = 6\n  end\n  r = x\n  break\nend\n"
        ));
        assert_eq!(r.byte("t"), (a != 0) as u8, "a={a}");
        assert_eq!(r.byte("y"), if a != 0 { 5 } else { 7 }, "y, a={a}");
        assert_eq!(r.byte("q"), if a == 5 { 3 } else { 4 }, "q, a={a}");
        assert_eq!(r.byte("r"), if a != 0 { 6 } else { 7 }, "r, a={a}");
    }
    // found by differential fuzzing: `v1 = 65` ends the else-branch; arriving
    // from the then-branch (v0 = 1), A holds 1, not v1.
    // (0/1) + (1+0) = 1 > not (0 and 0) = 1 is false → dec v1 → 255
    let r = run_src(
        "var v0 = 1\nvar v1 = 0\nvar v2 = 0\nvar v5 = 0\nvar a = array(16)\n\
         if v0 then\n  v2 = 1\nelse\n  v1 = 65\nend\n\
         if ((v1 / (v5 or 1)) + (v2 + a[12])) > (not (a[4] and v1)) then\nelse\n  dec v1\nend\n",
    );
    assert_eq!(r.byte("v1"), 255);

    // flag-dependent consumers after a reusable assignment
    for &(a, b) in &[(10u8, 3u8), (3, 10), (5, 5)] {
        let r = run_src(&format!(
            "var a = {a}\nvar b = {b}\nvar x = a - b\nvar y = abs(x)\nvar s = a - b\nvar g = sgn(s)\n\
             var z = a - b\nvar t = 0\nif z then t = 1 end\n"
        ));
        let d = a.wrapping_sub(b);
        let want_abs = if d & 0x80 != 0 { d.wrapping_neg() } else { d };
        assert_eq!(r.byte("y"), want_abs, "abs {a}-{b}");
        let want_sgn = if d == 0 { 0 } else if d & 0x80 != 0 { 0xFF } else { 1 };
        assert_eq!(r.byte("g"), want_sgn, "sgn {a}-{b}");
        assert_eq!(r.byte("t"), (d != 0) as u8, "if {a}-{b}");
    }
}

// 1.6.1: a byte product / shift assigned to a word keeps 16 bits also inside a sum
// (`py = cy * 8 + fy` used to be truncated to 8 bits), and a constant multiplier
// above 255 is no longer cut to 8 bits (`cy * 300`).
#[test]
fn word_assign_of_byte_products_keeps_16_bits() {
    let cases: &[(&str, u16)] = &[
        ("py = cy * 8", 320),
        ("py = cy * 8 + fy", 323),
        ("py = fy + cy * 8", 323),
        ("py = 8 * cy + fy", 323),
        ("py = cy * 9 + fy", 363),
        ("py = cy * 8 - fy", 317),
        ("py = cy * 300", 12000),
        ("py = 300 * cy", 12000),
        ("py = cy shl 3", 320),
        ("py = (cy shl 3) + fy", 323),
        ("py = 100 * cy + 900", 4900),
        ("py = cy + fy", 43),
    ];
    for (stmt, want) in cases {
        let r = run_src(&format!("var cy = 40
var fy = 3
var py: word = 0
{stmt}
"));
        assert_eq!(r.word("py"), *want, "{stmt}");
    }
    // byte targets keep 8-bit semantics
    let r = run_src("var cy = 40
var b = cy * 8
");
    assert_eq!(r.byte("b"), (40u16 * 8) as u8);
}

/// `dim name as type` is a BASIC-style alias for `var name: type`.
#[test]
fn dim_as_type_declarations() {
    let r = run_src(
        "DIM kor AS INTEGER DIM nev AS STRING DIM fizetes AS DOUBLE\n\
         DIM b AS BYTE\n\
         DIM w1, w2 AS INTEGER\n\
         DIM x AS BYTE = 7, y AS INTEGER = 1000\n\
         kor = 300\n\
         nev = \"ZSOLT\"\n\
         fizetes = 3\n\
         b = 200\n\
         w1 = 500\n\
         w2 = w1 + kor\n\
         print nev\n\
         print fizetes\n",
    );
    assert_eq!(r.word("kor"), 300, "INTEGER is 16-bit");
    assert_eq!(r.word("fizetes"), 3 << 8, "DOUBLE is Q8.8 float");
    assert_eq!(r.byte("b"), 200);
    assert_eq!(r.word("w1"), 500);
    assert_eq!(r.word("w2"), 800, "dim a, b as integer types both names");
    assert_eq!(r.byte("x"), 7);
    assert_eq!(r.word("y"), 1000);
    let out = r.output();
    assert!(out.contains("ZSOLT"), "{out:?}");
    assert!(out.contains("3.00"), "{out:?}");
}

/// Uninitialized `dim` vars start at zero / empty string.
#[test]
fn dim_defaults_are_zero_and_empty() {
    let r = run_src(
        "DIM n AS INTEGER\n\
         DIM s AS STRING\n\
         DIM f AS SINGLE\n\
         print \"<\" + s + \">\"\n",
    );
    assert_eq!(r.word("n"), 0);
    assert_eq!(r.word("f"), 0);
    let out = r.output();
    assert!(out.contains("<>"), "empty string prints nothing: {out:?}");
}

/// Bad `dim` forms are compile errors, not silent miscompiles.
#[test]
fn dim_rejects_unsupported_forms() {
    for src in [
        "dim a as long\n",
        "dim a as banana\n",
        "dim a(10) as long\n",
        "dim a(n)\n",
        "dim a(10) = 5\n",
        "dim a(4000) as integer\n",
        "dim a, b as integer = 5\n",
    ] {
        let toks = ultimate_basic::compiler::lexer::Lexer::new(src).tokenize();
        let mut p = ultimate_basic::compiler::parser::Parser::new(toks);
        let _ = p.parse();
        assert!(!p.errors().is_empty(), "expected error for {src:?}");
    }
}

/// `s = "literal"` / `s = t` after declaration repoint the string (it used to
/// store one garbage byte into the pointer's lo byte).
#[test]
fn string_var_reassignment() {
    let r = run_src(
        "var a = \"A\"\n\
         var b: string = \"\"\n\
         a = \"ZSOLT\"\n\
         print a\n\
         b = a\n\
         a = \"X\"\n\
         print b + a\n",
    );
    let out = r.output();
    assert!(out.contains("ZSOLT\r"), "{out:?}");
    assert!(out.contains("ZSOLTX"), "{out:?}");
}


/// QBasic-style FUNCTION / SUB: `AS` types, return by assigning the function
/// name, `END FUNCTION` / `END SUB`, calls with and without parentheses,
/// `;` print separators, accented letters folded to ASCII.
#[test]
fn qbasic_function_and_sub() {
    let r = run_src(
        "FUNCTION Negyzet (szam AS INTEGER) AS INTEGER Negyzet = szam * szam END FUNCTION\n\
         SUB Koszont (nev AS STRING) PRINT \"Üdvözöllek, \"; nev; \"!\" END SUB\n\
         DIM n AS INTEGER\n\
         n = Negyzet(30)\n\
         Koszont \"Zsolt\"\n\
         CALL Koszont(\"Anna\")\n\
         Koszont(\"Bela\")\n",
    );
    assert_eq!(r.word("n"), 900);
    let out = r.output();
    assert!(out.contains("Udvozollek, Zsolt!\r"), "{out:?}");
    assert!(out.contains("Udvozollek, Anna!\r"), "{out:?}");
    assert!(out.contains("Udvozollek, Bela!\r"), "{out:?}");
}

/// `EXIT FUNCTION` returns the value assigned so far; `END IF` closes an if.
#[test]
fn qbasic_exit_function_and_end_if() {
    let r = run_src(
        "FUNCTION Max2(a AS BYTE, b AS BYTE) AS BYTE\n\
           Max2 = a\n\
           IF b > a THEN\n\
             Max2 = b\n\
             EXIT FUNCTION\n\
           END IF\n\
         END FUNCTION\n\
         var p = Max2(3, 9)\n\
         var q = Max2(7, 2)\n",
    );
    assert_eq!(r.byte("p"), 9);
    assert_eq!(r.byte("q"), 7);
}

/// Word parameters receive both bytes; string literals reach string params.
#[test]
fn word_and_string_literal_params() {
    let r = run_src(
        "fn dupla(x: word): word\n  return x + x\nend\n\
         sub k(nev: string)\n  print nev\nend\n\
         var a: word = 0\nvar b: word = 0\n\
         a = dupla(300)\nb = dupla(5)\nk(\"ZSOLT\")\n",
    );
    assert_eq!(r.word("a"), 600);
    assert_eq!(r.word("b"), 10, "hi byte of the param must not stay from the previous call");
    assert!(r.output().contains("ZSOLT\r"), "{:?}", r.output());
}

/// `;` separates print items, a trailing `;` suppresses the newline, and a
/// `;` before an assignment is still a statement separator.
#[test]
fn print_semicolon_forms() {
    let r = run_src("var x = 5\nvar y = 1\nprint \"A\"; x; \"B\"\nprint x;\nprint x; y = 7\nprint y\n");
    assert_eq!(r.byte("y"), 7);
    assert_eq!(r.output(), "A5B\r557\r");
}

/// `:` separates statements, also next to `: type` annotations and the
/// QBasic-style DIM / FUNCTION / SUB / END IF forms.
#[test]
fn colon_statement_separator() {
    let r = run_src(
        "var x: int = 5 : DIM n AS INTEGER : n = 300\n\
         FUNCTION Negyzet(szam AS INTEGER) AS INTEGER : Negyzet = szam * szam : END FUNCTION\n\
         SUB Koszont(nev AS STRING) : PRINT \"HELLO, \"; nev; \"!\" : END SUB\n\
         DIM q AS INTEGER : q = Negyzet(20)\n\
         IF x == 5 THEN : PRINT \"A\" : END IF : PRINT \"B\"\n\
         Koszont \"ZSOLT\" : Koszont \"ANNA\"\n",
    );
    assert_eq!(r.byte("x"), 5);
    assert_eq!(r.word("n"), 300);
    assert_eq!(r.word("q"), 400);
    assert_eq!(r.output(), "A\rB\rHELLO, ZSOLT!\rHELLO, ANNA!\r");
}

/// `dim name(u1, u2, …) as type` — BASIC arrays (bounds = highest index) for
/// every type, indexed with `name(i, j)` or `name[i, j]`.
#[test]
fn dim_arrays_all_types() {
    let r = run_src(
        "DIM tabla(2, 3) AS INTEGER, k(1) AS BYTE\n\
         var r = 0\nvar c = 0\n\
         for r = 0 to 2\n for c = 0 to 3\n  tabla(r, c) = r * 100 + c\n next\nnext\n\
         k(1) = 7\n\
         DIM kocka(1, 2, 3) AS BYTE\nkocka(1, 2, 3) = 77\n\
         DIM ar(3) AS DOUBLE\nar(0) = 3\nar(1) = 2.5\nar(2) = ar(1) + 1.25\n\
         DIM nevek(2) AS STRING\nnevek(0) = \"ZSOLT\"\nnevek(2) = \"ANNA\"\n\
         DIM n AS STRING\nn = nevek(2)\n\
         var w: word = 0\nw = tabla(2, 3) + 5\n\
         print tabla(2, 3); \" \"; tabla(1, 0); \" \"; tabla[0, 2]\n\
         print ar(0); \" \"; ar(1); \" \"; ar(2); \" \"; ar(3)\n\
         print \"<\"; nevek(0); \".\"; nevek(1); \".\"; n; \">\"\n",
    );
    assert_eq!(r.word("w"), 208);
    assert_eq!(
        r.output(),
        "203 100 2\r3.00 2.50 3.75 0.00\r<ZSOLT..ANNA>\r"
    );
}

/// Word-array elements print as 16-bit values and take part in 16-bit
/// arithmetic (both read back as 0 / lo byte only before).
#[test]
fn word_array_print_and_arithmetic() {
    let r = run_src(
        "var t = array_word(4)\nvar w: word = 0\nt[1] = 300\nw = t[1] + 5\nprint t[1]\n",
    );
    assert_eq!(r.word("w"), 305);
    assert_eq!(r.output(), "300\r");
}

/// String-array elements can be passed to string params and concatenated.
#[test]
fn string_array_elements_as_values() {
    let r = run_src(
        "SUB K(x AS STRING) : PRINT \"HI \"; x : END SUB\n\
         DIM t(1, 1) AS STRING\nt(1, 1) = \"EVA\"\nK t(1, 1)\nprint \"A\" + t(1, 1)\n\
         DIM big(200) AS STRING\nbig(200) = \"END\"\nprint \"<\"; big(150); \">\"; big(200)\n",
    );
    assert_eq!(r.output(), "HI EVA\rAEVA\r<>END\r");
}




/// Arrays over 256 bytes with a variable index (8-bit `base,Y` wrapped and
/// wrote into the wrong element before).
#[test]
fn big_arrays_with_variable_index() {
    let r = run_src(
        "DIM t(200) AS INTEGER\nvar i = 150\nt(i) = 1234\nvar a: word = 0\nvar b: word = 0\na = t(150)\nb = t(i)\n\
         DIM g(19, 19) AS BYTE\nvar r = 15\nvar c = 3\ng(r, c) = 99\nvar x = g(15, 3)\nvar y = g(r, c)\n\
         DIM s(300) AS BYTE\nvar k: word = 0\nfor k = 0 to 300\n s(k) = k and 255\nnext\nvar z = s(290)\nvar z0 = s(34)\n",
    );
    assert_eq!(r.word("a"), 1234);
    assert_eq!(r.word("b"), 1234);
    assert_eq!(r.byte("x"), 99);
    assert_eq!(r.byte("y"), 99);
    assert_eq!(r.byte("z"), 290u16 as u8);
    assert_eq!(r.byte("z0"), 34);
}

/// `for` with a 16-bit counter (the 8-bit loop wrapped at 256: `to 300` ran 45 times).
#[test]
fn word_for_loops() {
    let cases: &[(&str, u16, u16)] = &[
        ("for k = 0 to 300\n n = n + 1\nnext", 301, 301),
        ("for k = 1000 to 990 step -1\n n = n + 1\nnext", 11, 989),
        ("for k = 0 to 1000 step 250\n n = n + 1\nnext", 5, 1250),
        ("for k = 65530 to 65535\n n = n + 1\nnext", 6, 0),
        ("for k = 5 to 0 step -1\n n = n + 1\nnext", 6, 65535), // = -1, like QBasic
        ("lim = 5\nfor k = 10 to lim\n n = n + 1\nnext", 0, 10),
        ("lim = 700\nfor k = 600 to lim\n n = n + 1\nnext", 101, 701),
        ("for k = 0 to 500\n if k == 300 then break end\n n = n + 1\nnext", 300, 300),
        ("for k = 0 to 500\n if k < 400 then continue end\n n = n + 1\nnext", 101, 501),
    ];
    for (body, n, k) in cases {
        let src = format!("var k: word = 0\nvar n: word = 0\nvar lim: word = 0\n{body}\n");
        let r = run_src(&src);
        assert_eq!(r.word("n"), *n, "iterations: {body}");
        assert_eq!(r.word("k"), *k, "final k: {body}");
    }
}

/// Recursive subs/fns are compile errors (they silently returned garbage).
#[test]
fn recursion_is_a_compile_error() {
    use ultimate_basic::compiler::{compile, CompileOptions};
    let opts = CompileOptions { basic_stub: false, explicit: false };
    let direct = compile("fn fact(n)\n if n <= 1 then return 1 end\n return n * fact(n - 1)\nend\nprint fact(5)\n", &opts);
    assert!(direct.errors.iter().any(|e| e.contains("recursion") && e.contains("fact → fact")), "{:?}", direct.errors);
    let mutual = compile("sub a()\n b()\nend\nsub b()\n a()\nend\na()\n", &opts);
    assert!(mutual.errors.iter().any(|e| e.contains("a → b → a")), "{:?}", mutual.errors);
    assert_eq!(mutual.errors.iter().filter(|e| e.contains("recursion")).count(), 1);
    // return-by-name and plain calls are not recursion
    let ok = compile("FUNCTION F(x AS BYTE) AS BYTE\n F = x + 1\nEND FUNCTION\nsub s()\n print F(1)\nend\ns()\n", &opts);
    assert!(ok.errors.is_empty(), "{:?}", ok.errors);
}

/// QBasic control structures: `=` / `<>` in conditions, ELSEIF, DO … LOOP,
/// WHILE … WEND, SELECT CASE (lists, ranges, IS, CASE ELSE), EXIT DO / FOR.
#[test]
fn qbasic_control_structures() {
    let grade = |x: u8| {
        let r = run_src(&format!(
            "var x = {x}\nvar g = 0\n\
             IF x = 1 THEN\n g = 10\nELSEIF x < 5 THEN\n g = 20\nELSEIF x <> 9 THEN\n g = 30\nELSE\n g = 40\nEND IF\n"
        ));
        r.byte("g")
    };
    assert_eq!((grade(1), grade(3), grade(7), grade(9)), (10, 20, 30, 40));

    let r = run_src(
        "var a = 0\nvar b = 0\nvar c = 0\nvar d = 0\nvar e = 0\nvar f = 0\nvar i = 0\n\
         DO WHILE a < 5\n a = a + 1\nLOOP\n\
         DO UNTIL b = 7\n b = b + 1\nLOOP\n\
         DO\n c = c + 1\nLOOP UNTIL c >= 3\n\
         DO\n d = d + 1\nLOOP WHILE d < 4\n\
         DO\n e = e + 1\n IF e = 6 THEN EXIT DO\nLOOP\n\
         WHILE f <> 8\n f = f + 1\nWEND\n\
         for i = 1 to 100\n IF i = 12 THEN EXIT FOR\nnext\n",
    );
    assert_eq!(
        [r.byte("a"), r.byte("b"), r.byte("c"), r.byte("d"), r.byte("e"), r.byte("f"), r.byte("i")],
        [5, 7, 3, 4, 6, 8, 12]
    );

    let sel = |x: u16| {
        let r = run_src(&format!(
            "DIM x AS INTEGER = {x}\nvar k = 0\n\
             SELECT CASE x\n CASE 1, 2, 3\n  k = 1\n CASE 10 TO 20\n  k = 2\n CASE IS > 1000\n  k = 3\n CASE ELSE\n  k = 4\nEND SELECT\n"
        ));
        r.byte("k")
    };
    assert_eq!((sel(2), sel(15), sel(5000), sel(50)), (1, 2, 3, 4));
}

/// `END` on its own ends the program (it was an "unexpected 'end'" error).
#[test]
fn end_statement_ends_program() {
    let r = run_src("var x = 7\ngosub kiir\nprint \"VEGE\"\nEND\nlabel kiir\nprint x\nreturn\n");
    assert!(r.output().contains("7\rVEGE\r"), "{:?}", r.output());
}

/// Single-line IF: ends with the line (QBasic), or with a same-line `end`;
/// optional same-line ELSE.
#[test]
fn single_line_if_forms() {
    let r = run_src(
        "var x = 3\nvar a = 0\nvar b = 0\nvar c = 0\nvar d = 0\n\
         IF x = 3 THEN a = 1\n\
         if x == 4 then b = 1 end\n\
         IF x = 4 THEN c = 1 ELSE c = 2\n\
         if x == 3 then d = 5 : d = d + 1 end\n",
    );
    assert_eq!([r.byte("a"), r.byte("b"), r.byte("c"), r.byte("d")], [1, 0, 2, 6]);
}

/// Runtime strings: concatenation into variables (value semantics), numbers
/// in concatenation, left$/right$/mid$, 16-bit str$/val, string = and <>,
/// len() of expressions, computed string arguments, Q8.8 text.
#[test]
fn runtime_strings() {
    let cases: &[(&str, &str)] = &[
        ("DIM s AS STRING\nDIM t AS STRING\ns = \"AB\"\nt = s + \"CD\"\nprint t\n", "ABCD\r"),
        ("DIM s AS STRING\ns = \"X\"\ns = s + \"Y\"\ns = \"<\" + s + \">\"\nprint s\n", "<XY>\r"),
        ("var s = \"HELLO WORLD\"\nprint left$(s, 5); \".\"; right$(s, 5); \".\"; mid$(s, 7, 3); \".\"; mid$(s, 7)\n", "HELLO.WORLD.WOR.WORLD\r"),
        ("DIM n AS INTEGER = 1234\nDIM s AS STRING\ns = \"PONT: \" + n + \"!\"\nprint s\n", "PONT: 1234!\r"),
        ("DIM w AS INTEGER = 0\nprint str$(w); \" \"; str$(w + 300)\n", "0 300\r"),
        ("var s = \"ABC\"\nDIM t AS STRING\nt = s\ns = \"ZZZ\"\nprint t; s\n", "ABCZZZ\r"),
        ("var s = \"ZSOLT\"\nif s = \"ZSOLT\" then print \"EQ\" end\nif s <> \"ANNA\" then print \"NE\" end\nif s == \"X\" then print \"BAD\" end\n", "EQ\rNE\r"),
        ("var s = \"12345\"\nDIM n AS INTEGER\nn = val(s)\nprint n; \" \"; val(\"42\") + 1\n", "12345 43\r"),
        ("var s = \"HELLO\"\nprint len(s + \"XY\"); \" \"; len(left$(s, 2))\n", "7 2\r"),
        ("SUB K(x AS STRING) : print \"[\"; x; \"]\" : END SUB\nvar s = \"AB\"\nK s + \"C\"\nK left$(\"XYZ\", 2) + s\n", "[ABC]\r[XYAB]\r"),
        ("DIM f AS DOUBLE = 3.75\nDIM s AS STRING\ns = \"F=\" + f\nprint s\n", "F=3.75\r"),
        ("var s = \"ABCDEFGHIJ\"\nDIM t AS STRING\nt = left$(s + s, 15)\nprint t; \" \"; len(t)\n", "ABCDEFGHIJABCDE 15\r"),
        ("DIM s AS STRING\ns = \"HI\" + chr$(33)\nprint s\n", "HI!\r"),
        ("var s = \"ABC\"\nprint right$(s, 10); \".\"; left$(s, 0); \".\"; mid$(s, 9); \".\"\n", "ABC...\r"),
    ];
    for (src, want) in cases {
        assert_eq!(run_src(src).output(), *want, "{src}");
    }
    // 80-character cap: longer results are cut, nothing is overwritten
    let r = run_src("DIM s AS STRING\nvar i = 0\nfor i = 1 to 30\n s = s + \"ABC\"\nnext\nprint len(s)\n");
    assert_eq!(r.output(), "80\r");
}

/// `input` into INTEGER (16-bit, signed with '-'), DOUBLE ("12.5"), array
/// elements (string arrays get one slot per element), QBasic `input "p"; x`.
#[test]
fn input_kinds() {
    let cases: &[(&str, &[&str], &str)] = &[
        ("var b = 0\ninput b\nprint \"=\"; b\n", &["42"], "=42\r"),
        ("DIM n AS INTEGER\ninput \"N\"; n\nprint \"=\"; n\n", &["12345"], "=12345\r"),
        ("DIM f AS DOUBLE\ninput f\nprint \"=\"; f\n", &["12.5"], "=12.50\r"),
        ("DIM f AS DOUBLE\ninput f\nprint \"=\"; f\n", &["3.07"], "=3.07\r"),
        ("DIM f AS DOUBLE\ninput f\nprint \"=\"; f\n", &["7"], "=7.00\r"),
        ("DIM t(2) AS INTEGER\nvar i = 0\nfor i = 0 to 2\n input t(i)\nnext\nprint \"=\"; t(0); \",\"; t(1); \",\"; t(2)\n", &["100", "2000", "30000"], "=100,2000,30000\r"),
        ("DIM nev(2) AS STRING\nvar i = 0\nfor i = 0 to 2\n input nev(i)\nnext\nprint \"=\"; nev(0); \",\"; nev(1); \",\"; nev(2)\n", &["ANNA", "BELA", "CILI"], "=ANNA,BELA,CILI\r"),
        ("DIM nev(1) AS STRING\nDIM s AS STRING\ns = \"X\"\nnev(0) = s\ns = \"Y\"\nnev(1) = s + \"Z\"\nprint \"=\"; nev(0); nev(1)\n", &[], "=XYZ\r"),
    ];
    for (src, keys, want) in cases {
        let out = run_src_with_input(src, keys).output();
        assert!(out.ends_with(want), "{src}\n got {out:?}");
    }
}

/// `as integer` is signed 16-bit (QBasic INTEGER): printing, comparisons,
/// division, abs, loops through zero, arrays, functions, select case, val/str$.
#[test]
fn signed_integers() {
    let cases: &[(&str, &str)] = &[
        ("DIM n AS INTEGER\nn = 10 - 20\nprint n\n", "-10\r"),
        ("DIM a AS INTEGER = -5\nDIM b AS INTEGER = 3\nif a < b then print \"LT\" end\nif a > b then print \"GT\" end\nif a < -10 then print \"BAD\" end\nif a >= -5 then print \"GE\" end\n", "LT\rGE\r"),
        ("DIM a AS INTEGER = -100\nprint a / 7; \" \"; abs(a); \" \"; a * 3\n", "-14 100 -300\r"),
        ("DIM i AS INTEGER\nfor i = 3 to -3 step -2\n print i; \" \";\nnext\nprint\n", "3 1 -1 -3 \r"),
        ("DIM i AS INTEGER\nfor i = -2 to 2\n print i; \" \";\nnext\nprint\n", "-2 -1 0 1 2 \r"),
        ("DIM w AS WORD = 40000\nDIM n AS INTEGER = 30000\nprint w; \" \"; n + 2767; \" \"; n + 2768\n", "40000 32767 -32768\r"),
        ("DIM t(2) AS INTEGER\nt(1) = -1234\nprint t(1)\nDIM s AS STRING\ns = \"V\" + t(1)\nprint s\n", "-1234\rV-1234\r"),
        ("FUNCTION Neg(x AS INTEGER) AS INTEGER\n Neg = 0 - x\nEND FUNCTION\nprint Neg(300); \" \"; Neg(-7)\nDIM r AS INTEGER\nr = Neg(1000)\nprint r\n", "-300 7\r-1000\r"),
        ("FUNCTION Negyzet(x AS INTEGER) AS INTEGER\n Negyzet = x * x\nEND FUNCTION\nprint Negyzet(30)\n", "900\r"),
        ("DIM a AS INTEGER = -3\nSELECT CASE a\n CASE IS < 0\n  print \"NEG\"\n CASE ELSE\n  print \"POS\"\nEND SELECT\n", "NEG\r"),
        ("DIM a AS INTEGER\nDIM s AS STRING\ns = \"-42\"\na = val(s)\nprint a; \" \"; str$(a)\n", "-42 -42\r"),
    ];
    for (src, want) in cases {
        assert_eq!(run_src(src).output(), *want, "{src}");
    }
    let r = run_src_with_input("DIM n AS INTEGER\ninput n\nprint \"=\"; n\n", &["-321"]);
    assert!(r.output().ends_with("=-321\r"), "{:?}", r.output());
}

/// Q8.8 × Q8.8 and exact Q8.8 ÷ Q8.8 (the divisor's fraction was ignored).
#[test]
fn float_mul_div() {
    let cases: &[(&str, &str)] = &[
        ("DIM a AS DOUBLE = 2.5\nDIM b AS DOUBLE = 1.5\nDIM c AS DOUBLE\nc = a * b\nprint c\n", "3.75\r"),
        ("DIM a AS DOUBLE = 7.5\nDIM b AS DOUBLE = 2.5\nDIM c AS DOUBLE\nc = a / b\nprint c\n", "3.00\r"),
        ("DIM a AS DOUBLE = 1.0\nDIM b AS DOUBLE = 0.25\nDIM c AS DOUBLE\nc = a / b\nprint c\n", "4.00\r"),
        ("DIM a AS DOUBLE = 10\nDIM b AS DOUBLE = 3\nDIM c AS DOUBLE\nc = a / b\nprint c\n", "3.33\r"),
        ("DIM a AS DOUBLE = 3.5\nprint a * 2; \" \"; a / 2; \" \"; a * a\n", "7.00 1.75 12.25\r"),
        ("DIM a AS DOUBLE = 0.5\nDIM b AS DOUBLE = 0.5\nprint a * b; \" \"; a / b\n", "0.25 1.00\r"),
    ];
    for (src, want) in cases {
        assert_eq!(run_src(src).output(), *want, "{src}");
    }
}





/// examples/qbasic_demo.ub end to end (keyboard input "ZSOLT").
#[test]
fn qbasic_demo_runs() {
    let src = std::fs::read_to_string("examples/qbasic_demo.ub").unwrap();
    let out = run_src_with_input(&src, &["ZSOLT"]).output();
    assert_eq!(
        out,
        "Udvozollek, Zsolt!\r30 negyzete: 900\rnagyobb(3, 9): 9\rNeved: ZSOLT\rUdvozollek, ZSOLT!\r\
         Szia, ZSOLT! Hossz: 5\rZ.SO.T\rIsmerlek!\rOsszeg: 900\r-2 negativ\r-1 negativ\r0 nulla\r\
         1 pozitiv\r2 pozitiv\rharom\r"
    );
}



/// word × word keeps 16 bits of both operands (the right one was cut to a byte).
#[test]
fn word_times_word() {
    for (a, b) in [(300u16, 300u16), (300, 2), (2, 300), (1000, 65), (65535, 65535), (12345, 0)] {
        let r = run_src(&format!(
            "DIM a AS WORD = {a}\nDIM b AS WORD = {b}\nDIM c AS WORD\nc = a * b\n"
        ));
        assert_eq!(r.word("c"), a.wrapping_mul(b), "{a} * {b}");
    }
    let r = run_src("DIM a AS INTEGER = -300\nDIM b AS INTEGER = 7\nDIM c AS INTEGER\nc = a * b\nprint c\n");
    assert_eq!(r.output(), "-2100\r");
}
