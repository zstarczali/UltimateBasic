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
