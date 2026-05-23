#[derive(Debug, Clone)]
pub enum Expr {
    Number(i16),
    StringLit(String),
    Var(String),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    Not(Box<Expr>),
    Getch,
    Inkey,    // inkey() — non-blocking $FFE4; 0 = no key, else PETSCII code
    ReuPresent,  // reu_present() — 1 if REU detected, 0 otherwise
    Joy(u8),  // joy(1) or joy(2) — read joystick port, returns inverted bits 0-4
    Sin(Box<Expr>),   // sin(angle) — 8-bit angle 0-255, returns 0-255 (center=128)
    Cos(Box<Expr>),   // cos(angle) — same as sin with +64 offset
    HexFmt(Box<Expr>), // hex(n) — in print: shows value as 2-digit uppercase hex
    BinFmt(Box<Expr>), // bin(n) — in print: shows value as 8-bit binary string
    Peek(Box<Expr>),
    Rnd,
    Abs(Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    Sgn(Box<Expr>),
    ArrayGet(String, Box<Expr>), // arr[idx]
    ChrStr(Box<Expr>),           // chr$(n) — character with PETSCII code n
    SpriteHit,                   // sprhit()    — read $D01E (sprite–sprite collision, cleared on read)
    SpriteBgHit,                 // sprbghit() — read $D01F (sprite–background collision, cleared on read)
    StrLen(Box<Expr>),           // len(s)  — length of null-terminated string var, 0–255
    Asc(Box<Expr>),              // asc(s)  — PETSCII code of first character (0 if empty)
}

#[derive(Debug, Clone)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    And, Or, Xor,
    Shl, Shr,
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

/// REU (RAM Expansion Unit) transfer type.
#[derive(Debug, Clone)]
pub enum ReuOp { Stash, Fetch, Swap }

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
    Cls { fast: bool },
    Graphics { on: bool, multi: bool },  // multi=true → multicolor bitmap mode ($D016.b4 set)
    Display { on: bool },  // display on/off — controls VIC DEN bit ($D011 bit4)
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
    Plot(Expr, Expr), // plot x, y — set pixel in bitmap mode
    Circle { x: Expr, y: Expr, radius: Expr }, // circle x,y,r — midpoint circle using bitmap plot helper
    Line { x1: Expr, y1: Expr, x2: Expr, y2: Expr }, // line x1,y1,x2,y2 — Bresenham line
    Gcls,             // gcls — clear bitmap screen
    Bye,              // bye/exit — cls then RTS back to BASIC
    Incbin(String),   // incbin "file" — embed raw binary file bytes inline
    Data(Vec<Expr>),  // data 1,2,3 — constant byte table (read with 'read')
    Read(String),     // read varname — load next data byte into variable
    Load { filename: String, addr: Option<Expr> }, // load "file" [, addr] — KERNAL LOAD from device 8
    Input { prompt: Option<String>, var: String }, // input ["prompt",] var — BASIN line input
    Fill { addr: Expr, len: Expr, val: Expr },     // fill addr, len, val — memory block fill
    Memcopy { src: Expr, dst: Expr, len: Expr },   // memcopy src, dst, len — memory block copy
    Irq { handler: Expr, line: Option<Expr> },     // irq handler [, raster_line] — raster IRQ setup
    Save { filename: String, addr: Option<Expr>, len: Option<Expr> }, // save "file" [, addr, len] — KERNAL SAVE
    Cursor { x: Expr, y: Expr },                   // cursor x, y — KERNAL PLOT set cursor position
    RepeatLoop(Vec<Stmt>, Expr),                   // repeat ... until cond — do-while loop
    PlotErase(Expr, Expr),                         // plot erase x, y — clear pixel in bitmap
    PlotXor(Expr, Expr),                           // plot xor x, y — XOR pixel in bitmap
    SpriteExpandX { id: Expr, on: bool },          // sprite expand x id, on/off — $D01D
    SpriteExpandY { id: Expr, on: bool },          // sprite expand y id, on/off — $D017
    SpritePriority { id: Expr, on: bool },         // sprite priority id, on/off — $D01B
    Reu { op: ReuOp, c64_addr: Expr, reu_bank: Expr, reu_addr: Expr, length: Expr },
    // reu stash/fetch/swap c64_addr, bank, reu_addr, length — DMA transfer to/from REU
    Wait { raster_target: bool, value: Expr }, // wait N (raster lines) / wait raster N (specific line)
    Sound { channel: Expr, freq: Expr, duration: Expr }, // SID: sound ch, freq(16-bit), frames
    Sprite { id: Expr, x: Expr, y: Expr, data_addr: Option<Expr> },
    // sprite id, x, y [, data_addr] — VIC-II hardware sprite position + optional data pointer
    // id/x/y: 0-7; x supports word vars for 9-bit range; data_addr = 64-byte-aligned address
    SpriteOn  { id: Expr },          // sprite_on id  — set bit in $D015 (sprite enable)
    SpriteOff { id: Expr },          // sprite_off id — clear bit in $D015
    SpriteColor { id: Expr, color: Expr },        // sprite_color id, color — write $D027+id
    SpriteMulticolor { id: Expr, on: bool },       // sprite_multicolor id, on/off — bit in $D01C
    /// Align to 64-byte boundary, embed 63 sprite bytes, emit `LDA #page; STA $07F8+id`.
    SpriteDef { id: u8, bytes: Vec<u8> },
}
