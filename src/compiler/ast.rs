#[derive(Debug, Clone)]
pub enum Expr {
    Number(i16),
    StringLit(String),
    Var(String),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    Not(Box<Expr>),
    Getch,
    Peek(Box<Expr>),
    Rnd,
    Abs(Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    Sgn(Box<Expr>),
    ArrayGet(String, Box<Expr>), // arr[idx]
}

#[derive(Debug, Clone)]
pub enum BinOp {
    Add, Sub, Mul, Div,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    And, Or,
}

#[derive(Debug, Clone)]
pub enum ColorTarget { Text, Border, Bg }

/// Variable type annotation.
/// `Word`  = 16-bit unsigned  (`var x: word = $0400`)
/// `Str`   = string pointer   (`var s = "text"` – inferred)
/// `Array` = byte array       (`var arr = array(10)`)
/// `Int`   = 8-bit (default)
#[derive(Debug, Clone, PartialEq)]
pub enum VarType { Int, Str, Float, Word, Array }

#[derive(Debug, Clone)]
pub enum Stmt {
    VarDecl { name: String, vtype: Option<VarType>, expr: Expr },
    Assign(String, Expr),
    ArraySet(String, Expr, Expr),  // arr[idx] = val
    Print(Vec<Expr>),
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    Loop(u8, Vec<Stmt>),
    ForLoop { var: String, from: Expr, to: Expr, step: Option<Expr>, body: Vec<Stmt> },
    WhileLoop(Expr, Vec<Stmt>),
    Break,
    Cls { manual: bool },
    Graphics { on: bool },
    Sys(u16),
    AsmBytes(Vec<u8>),
    IntToStr { var: String, addr: u16 },
    Color { target: ColorTarget, expr: Expr },
    SubDef(String, Vec<String>, Vec<Stmt>), // name, params, body
    Call(String, Vec<Expr>),                // name, args
    Return,
    Const(String, Expr),
    Label(String),
    Goto(String),
    Poke(Expr, Expr),
}
