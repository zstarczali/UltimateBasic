//! Variables in RAM when zero page runs out (1.6.4).
//!
//! The code generator addresses every variable as a zero-page byte pair. When
//! a program needs more than the 78 bytes of permanent zero page, `compile()`
//! runs a second pass with a [`SpillPlan`]: the least used *safe* variables get
//! a 2-byte home in RAM (in the array area), and every code *fragment* that
//! mentions one copies it into a zero-page proxy slot first:
//!
//! * a plain statement is one fragment: load the spilled variables it mentions,
//!   generate the statement against the proxies, store them back;
//! * a compound statement's header — the condition of `if` / `while` /
//!   `until`, the `for` bounds, the `select` value and case values — is a
//!   fragment that only loads (headers never write variables).
//!
//! A fragment's proxies are only live while the fragment runs. The only code
//! that can run in the middle of a fragment is a function called from an
//! expression, so routines get a *pool level*: main is 0 and a function called
//! from an expression at level L runs at level L+1 or deeper, with its own
//! proxy slots. Statement calls (`sub` calls, `gosub`) end their fragment, so
//! they do not need a new level.
//!
//! A variable is never spilled when moving it could change behaviour:
//! `for` counters, parameters, variables touched by interrupt handlers, and
//! variables written by any routine reachable from an expression call (the
//! caller's fragment would read a stale proxy).

use super::ast::{Expr, Stmt, VarType};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What the second compile pass needs to know.
#[derive(Clone, Debug, Default)]
pub struct SpillPlan {
    /// Spilled variables; the i-th one lives at home_base + 2*i.
    pub vars: Vec<String>,
    /// Pool level of every routine (main = 0, not listed).
    pub levels: HashMap<String, u8>,
    /// Highest level that has a fragment with spilled variables.
    pub max_level: u8,
    /// Zero-page bytes per level (2 per variable of the largest fragment).
    pub slot_bytes: u8,
}

/// Result of analysing a program once; [`Analysis::plan`] picks how many
/// variables to move.
pub struct Analysis {
    /// Spill candidates, cheapest (least used) first.
    candidates: Vec<String>,
    levels: HashMap<String, u8>,
    /// Every fragment: (level, names it mentions).
    fragments: Vec<(u8, BTreeSet<String>)>,
}

/// Quoted strings in a `Debug` rendering — every identifier the AST stores
/// (variable, sub, label and string-literal texts; extra matches only cost a
/// harmless load).
pub fn quoted(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut word = String::new();
        let mut esc = false;
        for c in chars.by_ref() {
            if esc {
                word.push(c);
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                break;
            } else {
                word.push(c);
            }
        }
        out.push(word);
    }
    out
}

/// Names after `marker` (e.g. `FnCall("`) in a Debug rendering.
fn calls_after(s: &str, marker: &str, not_after_alnum: bool) -> Vec<String> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(i) = s[from..].find(marker) {
        let at = from + i;
        from = at + marker.len();
        if not_after_alnum && at > 0 && s.as_bytes()[at - 1].is_ascii_alphanumeric() {
            continue;
        }
        if let Some(end) = s[from..].find('"') {
            out.push(s[from..from + end].to_string());
        }
    }
    out
}

fn is_compound(s: &Stmt) -> bool {
    matches!(
        s,
        Stmt::If(..)
            | Stmt::Loop(..)
            | Stmt::ForLoop { .. }
            | Stmt::WhileLoop(..)
            | Stmt::RepeatLoop(..)
            | Stmt::Select { .. }
            | Stmt::SubDef(..)
            | Stmt::FnDef(..)
            | Stmt::Block(..)
    )
}

/// Variables a statement tree writes (assignment targets, `for` counters…).
fn writes(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Assign(n, _) | Stmt::Inc(n) | Stmt::Dec(n) | Stmt::Read(n) => {
                out.insert(n.clone());
            }
            Stmt::VarDecl { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::Input { var, .. } => {
                out.insert(var.clone());
            }
            Stmt::ForLoop { var, body, .. } => {
                out.insert(var.clone());
                writes(body, out);
            }
            Stmt::If(_, t, e) => {
                writes(t, out);
                if let Some(e) = e {
                    writes(e, out);
                }
            }
            Stmt::Loop(_, b) | Stmt::WhileLoop(_, b) | Stmt::RepeatLoop(b, _) | Stmt::Block(b) => {
                writes(b, out)
            }
            Stmt::Select { cases, else_body, .. } => {
                for (_, b) in cases {
                    writes(b, out);
                }
                if let Some(e) = else_body {
                    writes(e, out);
                }
            }
            _ => {}
        }
    }
}

/// Visit every fragment of a statement list (same split as the code generator).
fn fragments(stmts: &[Stmt], depth: u32, f: &mut dyn FnMut(String, u32)) {
    for s in stmts {
        match s {
            Stmt::SubDef(..) | Stmt::FnDef(..) => {} // separate routines
            Stmt::If(c, t, e) => {
                f(format!("{c:?}"), depth);
                fragments(t, depth, f);
                if let Some(e) = e {
                    fragments(e, depth, f);
                }
            }
            Stmt::WhileLoop(c, b) => {
                f(format!("{c:?}"), depth + 1);
                fragments(b, depth + 1, f);
            }
            Stmt::RepeatLoop(b, c) => {
                fragments(b, depth + 1, f);
                f(format!("{c:?}"), depth + 1);
            }
            Stmt::ForLoop { from, to, step, body, .. } => {
                f(format!("{from:?}{to:?}{step:?}"), depth);
                fragments(body, depth + 1, f);
            }
            Stmt::Loop(_, b) | Stmt::Block(b) => fragments(b, depth + 1, f),
            Stmt::Select { expr, cases, else_body } => {
                let vals: Vec<&Expr> = cases.iter().map(|(v, _)| v).collect();
                f(format!("{expr:?}{vals:?}"), depth);
                for (_, b) in cases {
                    fragments(b, depth, f);
                }
                if let Some(e) = else_body {
                    fragments(e, depth, f);
                }
            }
            leaf => {
                debug_assert!(!is_compound(leaf));
                f(format!("{leaf:?}"), depth);
            }
        }
    }
}

/// Interrupt handler names installed anywhere (`irq`, `nmi`, `cia_timer`).
fn handlers(stmts: &[Stmt], out: &mut Vec<Expr>) {
    for s in stmts {
        match s {
            Stmt::Irq { handler, .. } | Stmt::Nmi { handler } | Stmt::CiaTimer { handler, .. } => {
                out.push(handler.clone())
            }
            Stmt::If(_, t, e) => {
                handlers(t, out);
                if let Some(e) = e {
                    handlers(e, out);
                }
            }
            Stmt::Loop(_, b)
            | Stmt::WhileLoop(_, b)
            | Stmt::RepeatLoop(b, _)
            | Stmt::Block(b)
            | Stmt::SubDef(_, _, b)
            | Stmt::FnDef(_, _, _, b) => handlers(b, out),
            Stmt::ForLoop { body, .. } => handlers(body, out),
            Stmt::Select { cases, else_body, .. } => {
                for (_, b) in cases {
                    handlers(b, out);
                }
                if let Some(e) = else_body {
                    handlers(e, out);
                }
            }
            _ => {}
        }
    }
}

/// Scalar variable declarations and `for` counters anywhere.
fn declared(stmts: &[Stmt], vars: &mut BTreeSet<String>, counters: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::VarDecl { name, vtype, .. } => {
                let array = matches!(
                    vtype,
                    Some(
                        VarType::Array
                            | VarType::WordArray
                            | VarType::FloatArray
                            | VarType::StrArray
                            | VarType::Int16Array
                            | VarType::StructArray(_)
                    )
                );
                if !array {
                    vars.insert(name.clone());
                }
            }
            Stmt::ForLoop { var, body, .. } => {
                counters.insert(var.clone());
                declared(body, vars, counters);
            }
            Stmt::If(_, t, e) => {
                declared(t, vars, counters);
                if let Some(e) = e {
                    declared(e, vars, counters);
                }
            }
            Stmt::Loop(_, b)
            | Stmt::WhileLoop(_, b)
            | Stmt::RepeatLoop(b, _)
            | Stmt::Block(b)
            | Stmt::SubDef(_, _, b)
            | Stmt::FnDef(_, _, _, b) => declared(b, vars, counters),
            Stmt::Select { cases, else_body, .. } => {
                for (_, b) in cases {
                    declared(b, vars, counters);
                }
                if let Some(e) = else_body {
                    declared(e, vars, counters);
                }
            }
            _ => {}
        }
    }
}

/// Analyse a program for spilling. `None` when the program uses a construct
/// spilling cannot handle safely (an interrupt handler that is a label, or
/// `gosub` inside code reachable from an expression call).
pub fn analyse(stmts: &[Stmt]) -> Option<Analysis> {
    // routines
    let mut bodies: HashMap<String, &[Stmt]> = HashMap::new();
    let mut params: HashSet<String> = HashSet::new();
    let mut main: Vec<Stmt> = vec![];
    for s in stmts {
        match s {
            Stmt::SubDef(n, p, b) | Stmt::FnDef(n, p, _, b) => {
                bodies.insert(n.clone(), b.as_slice());
                params.extend(p.iter().map(|(n, _)| n.clone()));
            }
            other => main.push(other.clone()),
        }
    }
    let main_dbg = format!("{main:?}");
    // call edges: (caller, callee, is expression call)
    let mut edges: Vec<(String, String, bool)> = vec![];
    let mut gosub_in: HashSet<String> = HashSet::new();
    let add_edges = |caller: &str, dbg: &str, edges: &mut Vec<(String, String, bool)>| {
        for c in calls_after(dbg, "FnCall(\"", false) {
            if bodies.contains_key(&c) {
                edges.push((caller.to_string(), c, true));
            }
        }
        for c in calls_after(dbg, "Call(\"", true) {
            if bodies.contains_key(&c) {
                edges.push((caller.to_string(), c, false));
            }
        }
    };
    add_edges("", &main_dbg, &mut edges);
    let mut dbgs: HashMap<String, String> = HashMap::new();
    for (n, b) in &bodies {
        let d = format!("{b:?}");
        add_edges(n, &d, &mut edges);
        if d.contains("Gosub(\"") {
            gosub_in.insert(n.clone());
        }
        dbgs.insert(n.clone(), d);
    }

    let reach = |roots: Vec<String>| -> HashSet<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut todo = roots;
        while let Some(r) = todo.pop() {
            if seen.insert(r.clone()) {
                for (c, t, _) in &edges {
                    if *c == r {
                        todo.push(t.clone());
                    }
                }
            }
        }
        seen
    };

    // interrupt handlers and everything they call
    let mut hs = vec![];
    handlers(stmts, &mut hs);
    let mut irq_roots = vec![];
    for h in hs {
        match h {
            Expr::Var(n) if bodies.contains_key(&n) => irq_roots.push(n),
            Expr::Number(_) => {}
            _ => return None, // a label in main code as handler
        }
    }
    let irq = reach(irq_roots);

    // routines reachable from an expression call
    let fn_roots: Vec<String> = edges.iter().filter(|e| e.2).map(|e| e.1.clone()).collect();
    let fn_reach = reach(fn_roots);
    if fn_reach.iter().any(|r| gosub_in.contains(r)) {
        return None;
    }

    // pool levels: longest path, +1 per expression call
    let mut levels: HashMap<String, u8> = HashMap::new();
    for _ in 0..=bodies.len() {
        let mut changed = false;
        for (c, t, expr) in &edges {
            let base = if c.is_empty() { 0 } else { *levels.get(c).unwrap_or(&0) };
            let l = base.saturating_add(*expr as u8);
            let e = levels.entry(t.clone()).or_insert(0);
            if l > *e {
                *e = l;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // candidates
    let mut vars = BTreeSet::new();
    let mut counters = HashSet::new();
    declared(stmts, &mut vars, &mut counters);
    let mut excluded: HashSet<String> = counters;
    excluded.extend(params.iter().cloned());
    for r in &irq {
        if let Some(d) = dbgs.get(r) {
            excluded.extend(quoted(d));
        }
    }
    for r in &fn_reach {
        if let Some(b) = bodies.get(r) {
            writes(b, &mut excluded);
        }
    }

    // usage weight (8× per loop nesting level) and fragments
    let mut weight: HashMap<String, u64> = HashMap::new();
    let mut frags: Vec<(u8, BTreeSet<String>)> = vec![];
    let mut visit = |level: u8, body: &[Stmt]| {
        fragments(body, 0, &mut |dbg, depth| {
            let names: BTreeSet<String> =
                quoted(&dbg).into_iter().filter(|n| vars.contains(n)).collect();
            for n in quoted(&dbg) {
                if vars.contains(&n) {
                    *weight.entry(n).or_insert(0) += 8u64.pow(depth.min(6));
                }
            }
            if !names.is_empty() {
                frags.push((level, names));
            }
        });
    };
    visit(0, &main);
    let mut names: Vec<&String> = bodies.keys().collect();
    names.sort();
    for n in names {
        visit(*levels.get(n).unwrap_or(&0), bodies[n]);
    }

    let mut candidates: Vec<String> =
        vars.iter().filter(|v| !excluded.contains(*v)).cloned().collect();
    candidates.sort_by_key(|v| (*weight.get(v).unwrap_or(&0), v.clone()));
    Some(Analysis { candidates, levels, fragments: frags })
}

impl Analysis {
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// Plan that moves the `k` cheapest candidates to RAM.
    pub fn plan(&self, k: usize) -> SpillPlan {
        let vars: Vec<String> = self.candidates.iter().take(k).cloned().collect();
        let set: HashSet<&String> = vars.iter().collect();
        let mut widest = 0usize;
        let mut max_level = 0u8;
        for (level, names) in &self.fragments {
            let n = names.iter().filter(|v| set.contains(v)).count();
            if n > 0 {
                widest = widest.max(n);
                max_level = max_level.max(*level);
            }
        }
        SpillPlan {
            vars,
            levels: self.levels.clone(),
            max_level,
            slot_bytes: (widest * 2).min(255) as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast(src: &str) -> Vec<Stmt> {
        let toks = crate::compiler::lexer::Lexer::new(src).tokenize();
        let mut p = crate::compiler::parser::Parser::new(toks);
        let a = crate::compiler::ast::flatten_blocks(p.parse());
        assert!(p.errors().is_empty(), "{:?}", p.errors());
        a
    }

    #[test]
    fn interrupt_handlers_keep_their_variables() {
        let a = ast(
            "var tick = 0\nvar other = 0\nsub h()\n  tick = tick + 1\n  irq_exit\nend\n\
             irq h, 100\nother = 5\n",
        );
        let an = analyse(&a).expect("analysable");
        assert_eq!(an.candidates, vec!["other".to_string()]);
    }

    #[test]
    fn label_as_handler_disables_spilling() {
        let a = ast("var x = 1\nirq tick, 100\nlabel tick\nx = 2\nirq_exit\n");
        assert!(analyse(&a).is_none());
    }

    #[test]
    fn functions_get_deeper_levels_and_keep_what_they_write() {
        let a = ast(
            "var g = 0\nvar r = 0\nvar w = 0\n\
             fn inner(x)\n  w = x\n  return x + g\nend\n\
             fn outer(y)\n  return inner(y) + 1\nend\n\
             sub s()\n  r = outer(2)\nend\n\
             s()\n",
        );
        let an = analyse(&a).unwrap();
        assert_eq!(an.levels.get("s"), Some(&0));
        assert_eq!(an.levels.get("outer"), Some(&1));
        assert_eq!(an.levels.get("inner"), Some(&2));
        assert!(!an.candidates.contains(&"w".to_string()), "written in a function");
        assert!(an.candidates.contains(&"g".to_string()));
        assert!(an.candidates.contains(&"r".to_string()));
        let plan = an.plan(an.candidate_count());
        assert_eq!(plan.max_level, 2, "inner reads g at level 2");
    }

    #[test]
    fn least_used_variables_go_first() {
        let a = ast(
            "var hot = 0\nvar cold = 0\nvar i = 0\nfor i = 1 to 9\n  hot = hot + 1\nnext\ncold = 1\n",
        );
        let an = analyse(&a).unwrap();
        assert_eq!(an.candidates, vec!["cold".to_string(), "hot".to_string()]);
    }
}
