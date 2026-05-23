use super::lexer::Token;
use super::ast::{Expr, BinOp, Stmt, ColorTarget, VarType};
use super::lexer::Token::{LBracket, RBracket};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    consts: std::collections::HashMap<String, i16>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0, consts: std::collections::HashMap::new() }
    }

    pub fn new_with_consts(tokens: Vec<Token>, consts: std::collections::HashMap<String, i16>) -> Self {
        Self { tokens, pos: 0, consts }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        t
    }

    fn skip_newlines(&mut self) {
        while self.peek() == &Token::Newline {
            self.advance();
        }
    }

    fn expect_newline(&mut self) {
        while matches!(self.peek(), Token::Newline | Token::Eof) {
            self.advance();
            if self.tokens.get(self.pos.saturating_sub(1)) == Some(&Token::Eof) { break; }
        }
    }

    pub fn parse(&mut self) -> Vec<Stmt> {
        let mut stmts = vec![];
        self.skip_newlines();
        while self.peek() != &Token::Eof {
            if self.peek() == &Token::Include {
                self.advance(); // consume 'include'
                if let Token::StringLit(path) = self.peek().clone() {
                    self.advance();
                    match std::fs::read_to_string(&path) {
                        Ok(src) => {
                            let mut lex = super::lexer::Lexer::new(&src);
                            let toks = lex.tokenize();
                            let consts = self.consts.clone();
                            let mut sub = Parser::new_with_consts(toks, consts);
                            let sub_stmts = sub.parse();
                            self.consts.extend(sub.consts.into_iter());
                            stmts.extend(sub_stmts);
                        }
                        Err(e) => eprintln!("include '{}': {}", path, e),
                    }
                }
                self.expect_newline();
            } else if let Some(s) = self.parse_stmt() {
                stmts.push(s);
            }
            self.skip_newlines();
        }
        stmts
    }

    fn parse_addr(&mut self) -> u16 {
        match self.peek().clone() {
            Token::Addr(a)   => { self.advance(); a }
            Token::Number(n) => { self.advance(); n as u16 }
            _ => 0,
        }
    }

    fn parse_asm_line(&mut self) -> Vec<u8> {
        let mut bytes = vec![];
        loop {
            match self.peek().clone() {
                Token::Addr(v)   => { self.advance(); bytes.push(v as u8); bytes.push((v >> 8) as u8); }
                Token::Number(n) => { self.advance(); bytes.push(n as u8); }
                Token::Comma     => { self.advance(); }
                Token::Newline | Token::Eof => { self.advance(); break; }
                _ => break,
            }
        }
        bytes
    }

    fn parse_asm_block(&mut self) -> Vec<u8> {
        let mut bytes = vec![];
        loop {
            match self.peek().clone() {
                Token::RBrace | Token::Eof => { self.advance(); break; }
                Token::Newline             => { self.advance(); }
                Token::Addr(v)             => { self.advance(); bytes.push(v as u8); bytes.push((v >> 8) as u8); }
                Token::Number(n)           => { self.advance(); bytes.push(n as u8); }
                Token::Comma               => { self.advance(); }
                _                          => { self.advance(); }
            }
        }
        bytes
    }

    fn parse_body(&mut self) -> Vec<Stmt> {
        let mut body = vec![];
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::End | Token::Eof) { break; }
            if let Some(s) = self.parse_stmt() { body.push(s); }
        }
        if self.peek() == &Token::End { self.advance(); }
        self.expect_newline();
        body
    }

    /// For loop body: stops at `next` (or `end` for backward compat).
    /// Optionally consumes the loop variable name after `next` (e.g. `next i`).
    fn parse_for_body(&mut self) -> Vec<Stmt> {
        let mut body = vec![];
        loop {
            self.skip_newlines();
            if matches!(self.peek(), Token::Next | Token::End | Token::Eof) { break; }
            if let Some(s) = self.parse_stmt() { body.push(s); }
        }
        if matches!(self.peek(), Token::Next | Token::End) { self.advance(); }
        // consume optional loop var after 'next': `next i`
        if let Token::Ident(_) = self.peek().clone() { self.advance(); }
        self.expect_newline();
        body
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        self.skip_newlines();
        match self.peek().clone() {
            Token::Var => {
                self.advance();
                let name = if let Token::Ident(n) = self.advance() { n } else { return None; };
                let vtype = if self.peek() == &Token::Colon {
                    self.advance();
                    match self.peek() {
                        Token::Int   => { self.advance(); Some(VarType::Int)   }
                        Token::Str   => { self.advance(); Some(VarType::Str)   }
                        Token::Float => { self.advance(); Some(VarType::Float) }
                        Token::Word  => { self.advance(); Some(VarType::Word)  }
                        _ => None,
                    }
                } else { None };
                if self.peek() == &Token::Assign { self.advance(); }
                // array(N) initializer
                if matches!(self.peek(), Token::Array) {
                    self.advance(); // consume 'array'
                    if self.peek() == &Token::LParen { self.advance(); }
                    let size = self.parse_expr();
                    if self.peek() == &Token::RParen { self.advance(); }
                    self.expect_newline();
                    return Some(Stmt::VarDecl { name, vtype: Some(VarType::Array), expr: size });
                }
                let expr = self.parse_expr();
                self.expect_newline();
                Some(Stmt::VarDecl { name, vtype, expr })
            }
            Token::Ident(name) => {
                self.advance();
                if self.peek() == &LBracket {
                    // arr[idx] = val
                    self.advance(); // [
                    let idx = self.parse_expr();
                    if self.peek() == &RBracket { self.advance(); } // ]
                    if self.peek() == &Token::Assign { self.advance(); } // =
                    let val = self.parse_expr();
                    self.expect_newline();
                    Some(Stmt::ArraySet(name, idx, val))
                } else if self.peek() == &Token::Assign {
                    self.advance();
                    let expr = self.parse_expr();
                    self.expect_newline();
                    Some(Stmt::Assign(name, expr))
                } else if self.peek() == &Token::LParen {
                    // name(arg1, arg2, ...) → Call with args
                    self.advance(); // (
                    let mut args = vec![];
                    while !matches!(self.peek(), Token::RParen | Token::Eof | Token::Newline) {
                        args.push(self.parse_expr());
                        if self.peek() == &Token::Comma { self.advance(); }
                    }
                    if self.peek() == &Token::RParen { self.advance(); } // )
                    self.expect_newline();
                    Some(Stmt::Call(name, args))
                } else {
                    self.expect_newline();
                    Some(Stmt::Call(name, vec![]))
                }
            }
            Token::Print => {
                self.advance();
                let mut args = vec![];
                // Collect comma-separated exprs until end of line
                if !matches!(self.peek(), Token::Newline | Token::Eof) {
                    args.push(self.parse_expr());
                    while self.peek() == &Token::Comma {
                        self.advance();
                        if !matches!(self.peek(), Token::Newline | Token::Eof) {
                            args.push(self.parse_expr());
                        }
                    }
                }
                self.expect_newline();
                Some(Stmt::Print(args))
            }
            Token::If => {
                self.advance();
                let cond = self.parse_expr();
                // consume 'then' if present
                if self.peek() == &Token::Then { self.advance(); }
                self.expect_newline();
                let mut then_body = vec![];
                let mut else_body: Option<Vec<Stmt>> = None;
                loop {
                    self.skip_newlines();
                    if matches!(self.peek(), Token::End | Token::Else | Token::Eof) { break; }
                    if let Some(s) = self.parse_stmt() { then_body.push(s); }
                }
                if self.peek() == &Token::Else {
                    self.advance();
                    self.expect_newline();
                    let mut eb = vec![];
                    loop {
                        self.skip_newlines();
                        if matches!(self.peek(), Token::End | Token::Eof) { break; }
                        if let Some(s) = self.parse_stmt() { eb.push(s); }
                    }
                    else_body = Some(eb);
                }
                if self.peek() == &Token::End { self.advance(); }
                self.expect_newline();
                Some(Stmt::If(cond, then_body, else_body))
            }
            Token::For => {
                // for i = expr to expr [step expr]
                //   body
                // next [i]
                self.advance();
                let var = if let Token::Ident(n) = self.advance() { n }
                          else { return None; };
                if self.peek() == &Token::Assign { self.advance(); }
                let from = self.parse_expr();
                if self.peek() == &Token::To { self.advance(); }
                let to = self.parse_expr();
                let step = if self.peek() == &Token::Step {
                    self.advance();
                    Some(self.parse_expr())
                } else { None };
                self.expect_newline();
                let body = self.parse_for_body();
                Some(Stmt::ForLoop { var, from, to, step, body })
            }
            Token::Loop => {
                self.advance();
                // loop i = expr to expr [step expr]
                if let Token::Ident(var) = self.peek().clone() {
                    self.advance();
                    if self.peek() == &Token::Assign {
                        self.advance();
                        let from = self.parse_expr();
                        // expect 'to'
                        if self.peek() == &Token::To { self.advance(); }
                        let to = self.parse_expr();
                        let step = if self.peek() == &Token::Step {
                            self.advance();
                            Some(self.parse_expr())
                        } else { None };
                        self.expect_newline();
                        let body = self.parse_body();
                        return Some(Stmt::ForLoop { var, from, to, step, body });
                    }
                    // fallthrough: treat ident as a statement inside loop 1
                    self.pos -= 1;
                }
                let count = if let Token::Number(n) = self.peek().clone() {
                    self.advance();
                    n.clamp(0, 255) as u8
                } else { 0 }; // 0 = infinite loop
                self.expect_newline();
                let body = self.parse_body();
                Some(Stmt::Loop(count, body))
            }
            Token::While => {
                self.advance();
                let cond = self.parse_expr();
                self.expect_newline();
                let body = self.parse_body();
                Some(Stmt::WhileLoop(cond, body))
            }
            Token::Break => {
                self.advance();
                self.expect_newline();
                Some(Stmt::Break)
            }
            Token::Cls => {
                self.advance();
                let fast = self.peek() == &Token::Fast;
                if fast { self.advance(); }
                self.expect_newline();
                Some(Stmt::Cls { fast })
            }
            Token::Graphics => {
                self.advance();
                let on = match self.peek() {
                    Token::On  => { self.advance(); true  }
                    Token::Off => { self.advance(); false }
                    _          => true,
                };
                let multi = if on && self.peek() == &Token::Multi {
                    self.advance(); true
                } else { false };
                self.expect_newline();
                Some(Stmt::Graphics { on, multi })
            }
            Token::Display => {
                self.advance();
                let on = match self.peek() {
                    Token::On  => { self.advance(); true  }
                    Token::Off => { self.advance(); false }
                    _          => true,
                };
                self.expect_newline();
                Some(Stmt::Display { on })
            }
            Token::Sys => {
                self.advance();
                let addr = self.parse_addr();
                self.expect_newline();
                Some(Stmt::Sys(addr))
            }
            Token::Asm => {
                self.advance();
                let bytes = if self.peek() == &Token::LBrace {
                    self.advance(); // consume '{'
                    self.parse_asm_block()
                } else {
                    self.parse_asm_line()
                };
                Some(Stmt::AsmBytes(bytes))
            }
            Token::NumStr => {
                self.advance();
                let var = if let Token::Ident(n) = self.advance() { n } else { return None; };
                if self.peek() == &Token::Comma { self.advance(); }
                let addr = self.parse_addr();
                self.expect_newline();
                Some(Stmt::IntToStr { var, addr })
            }
            Token::Color => {
                self.advance();
                let target = match self.peek() {
                    Token::Text   => { self.advance(); ColorTarget::Text   }
                    Token::Border => { self.advance(); ColorTarget::Border }
                    Token::Bg     => { self.advance(); ColorTarget::Bg     }
                    _             => ColorTarget::Text,
                };
                let expr = self.parse_expr();
                self.expect_newline();
                Some(Stmt::Color { target, expr })
            }
            Token::Sub => {
                self.advance();
                let name = if let Token::Ident(n) = self.advance() { n } else { return None; };
                // parse optional parameter list: (p1, p2, ...)
                let mut params = vec![];
                if self.peek() == &Token::LParen {
                    self.advance(); // (
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        if let Token::Ident(p) = self.peek().clone() {
                            self.advance();
                            params.push(p);
                        } else { self.advance(); }
                        if self.peek() == &Token::Comma { self.advance(); }
                    }
                    if self.peek() == &Token::RParen { self.advance(); } // )
                }
                self.expect_newline();
                let mut body = vec![];
                loop {
                    self.skip_newlines();
                    if matches!(self.peek(), Token::End | Token::Eof) { break; }
                    if let Some(s) = self.parse_stmt() { body.push(s); }
                }
                if self.peek() == &Token::End { self.advance(); }
                self.expect_newline();
                Some(Stmt::SubDef(name, params, body))
            }
            Token::Return => {
                self.advance();
                self.expect_newline();
                Some(Stmt::Return)
            }
            Token::Const => {
                self.advance();
                let name = if let Token::Ident(n) = self.advance() { n } else { return None; };
                if self.peek() == &Token::Assign { self.advance(); }
                let val = self.parse_expr();
                if let Expr::Number(v) = &val {
                    self.consts.insert(name.clone(), *v);
                }
                self.expect_newline();
                Some(Stmt::Const(name, val))
            }
            Token::Label => {
                self.advance();
                let name = if let Token::Ident(n) = self.advance() { n } else { return None; };
                self.expect_newline();
                Some(Stmt::Label(name))
            }
            Token::Goto => {
                self.advance();
                let name = if let Token::Ident(n) = self.advance() { n } else { return None; };
                self.expect_newline();
                Some(Stmt::Goto(name))
            }
            Token::Poke => {
                self.advance();
                let addr = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let val = self.parse_expr();
                self.expect_newline();
                Some(Stmt::Poke(addr, val))
            }
            Token::Plot => {
                self.advance();
                let x = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let y = self.parse_expr();
                self.expect_newline();
                Some(Stmt::Plot(x, y))
            }
            Token::Line => {
                self.advance();
                let x1 = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let y1 = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let x2 = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let y2 = self.parse_expr();
                self.expect_newline();
                Some(Stmt::Line { x1, y1, x2, y2 })
            }
            Token::Gcls => {
                self.advance();
                self.expect_newline();
                Some(Stmt::Gcls)
            }
            Token::Bye => {
                self.advance();
                self.expect_newline();
                Some(Stmt::Bye)
            }
            Token::Incbin => {
                self.advance();
                if let Token::StringLit(path) = self.peek().clone() {
                    self.advance();
                    self.expect_newline();
                    Some(Stmt::Incbin(path))
                } else {
                    self.expect_newline();
                    None
                }
            }
            Token::Data => {
                self.advance();
                let mut items = vec![];
                loop {
                    match self.peek().clone() {
                        Token::Number(n) => { self.advance(); items.push(Expr::Number(n)); }
                        Token::Addr(n)   => { self.advance(); items.push(Expr::Number(n as i16)); }
                        Token::Comma     => { self.advance(); }
                        Token::Newline | Token::Eof => break,
                        _ => break,
                    }
                }
                self.expect_newline();
                Some(Stmt::Data(items))
            }
            Token::Read => {
                self.advance();
                if let Token::Ident(name) = self.peek().clone() {
                    self.advance();
                    self.expect_newline();
                    Some(Stmt::Read(name))
                } else {
                    self.expect_newline();
                    None
                }
            }
            Token::Wait => {
                self.advance();
                let raster_target = matches!(self.peek(), Token::Raster);
                if raster_target { self.advance(); } // consume 'raster'
                let value = self.parse_expr();
                self.expect_newline();
                Some(Stmt::Wait { raster_target, value })
            }
            Token::Sound => {
                self.advance();
                let channel = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let freq = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let duration = self.parse_expr();
                self.expect_newline();
                Some(Stmt::Sound { channel, freq, duration })
            }
            Token::Sprite => {
                self.advance();
                match self.peek() {
                    Token::On => {
                        self.advance();
                        let id = self.parse_expr();
                        self.expect_newline();
                        Some(Stmt::SpriteOn { id })
                    }
                    Token::Off => {
                        self.advance();
                        let id = self.parse_expr();
                        self.expect_newline();
                        Some(Stmt::SpriteOff { id })
                    }
                    Token::Color => {
                        self.advance();
                        let id = self.parse_expr();
                        if self.peek() == &Token::Comma { self.advance(); }
                        let color = self.parse_expr();
                        self.expect_newline();
                        Some(Stmt::SpriteColor { id, color })
                    }
                    Token::Multi => {
                        self.advance();
                        let id = self.parse_expr();
                        if self.peek() == &Token::Comma { self.advance(); }
                        let on = matches!(self.advance(), Token::On);
                        self.expect_newline();
                        Some(Stmt::SpriteMulticolor { id, on })
                    }
                    _ => {
                        let id = self.parse_expr();
                        if self.peek() == &Token::Comma { self.advance(); }
                        let x = self.parse_expr();
                        if self.peek() == &Token::Comma { self.advance(); }
                        let y = self.parse_expr();
                        let data_addr = if self.peek() == &Token::Comma {
                            self.advance();
                            Some(self.parse_expr())
                        } else {
                            None
                        };
                        self.expect_newline();
                        Some(Stmt::Sprite { id, x, y, data_addr })
                    }
                }
            }
            Token::Sprdef => {
                self.advance();
                let id = match self.parse_expr() {
                    Expr::Number(n) => n as u8,
                    _ => panic!("sprdef: id must be a constant"),
                };
                self.expect_newline();
                // bytes: newline-separated rows, comma-separated within rows, until 'end'
                let mut bytes: Vec<u8> = Vec::new();
                loop {
                    while self.peek() == &Token::Newline { self.advance(); }
                    if self.peek() == &Token::End { self.advance(); break; }
                    match self.parse_expr() {
                        Expr::Number(n) => bytes.push(n as u8),
                        _ => panic!("sprdef: byte values must be constants"),
                    }
                    if self.peek() == &Token::Comma { self.advance(); }
                }
                self.expect_newline();
                Some(Stmt::SpriteDef { id, bytes })
            }
            Token::Reu => {
                self.advance();
                // parse op: stash / fetch / swap
                let op = match self.advance() {
                    Token::Stash => crate::compiler::ast::ReuOp::Stash,
                    Token::Fetch => crate::compiler::ast::ReuOp::Fetch,
                    _            => crate::compiler::ast::ReuOp::Swap,
                };
                let c64_addr = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let reu_bank = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let reu_addr = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let length = self.parse_expr();
                self.expect_newline();
                Some(Stmt::Reu { op, c64_addr, reu_bank, reu_addr, length })
            }
            Token::Call => {
                self.advance();
                let name = if let Token::Ident(n) = self.advance() { n } else { return None; };
                let mut args = vec![];
                if self.peek() == &Token::LParen {
                    self.advance(); // (
                    while !matches!(self.peek(), Token::RParen | Token::Eof | Token::Newline) {
                        args.push(self.parse_expr());
                        if self.peek() == &Token::Comma { self.advance(); }
                    }
                    if self.peek() == &Token::RParen { self.advance(); } // )
                }
                self.expect_newline();
                Some(Stmt::Call(name, args))
            }
            _ => { self.advance(); None }
        }
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        loop {
            let op = match self.peek() {
                Token::Or  => BinOp::Or,
                Token::Xor => BinOp::Xor,
                _ => break,
            };
            self.advance();
            let right = self.parse_and();
            left = Expr::BinOp(Box::new(left), op, Box::new(right));
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_shift();
        loop {
            if matches!(self.peek(), Token::And) {
                self.advance();
                let right = self.parse_shift();
                left = Expr::BinOp(Box::new(left), BinOp::And, Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_shift(&mut self) -> Expr {
        let mut left = self.parse_unary();
        loop {
            let op = match self.peek() {
                Token::Shl => BinOp::Shl,
                Token::Shr => BinOp::Shr,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary();
            left = Expr::BinOp(Box::new(left), op, Box::new(right));
        }
        left
    }

    fn parse_unary(&mut self) -> Expr {
        if matches!(self.peek(), Token::Not) {
            self.advance();
            return Expr::Not(Box::new(self.parse_unary()));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Expr {
        let mut left = self.parse_additive();
        loop {
            let op = match self.peek() {
                Token::Eq    => BinOp::Eq,
                Token::NotEq => BinOp::NotEq,
                Token::Lt    => BinOp::Lt,
                Token::Gt    => BinOp::Gt,
                Token::LtEq  => BinOp::LtEq,
                Token::GtEq  => BinOp::GtEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_additive();
            left = Expr::BinOp(Box::new(left), op, Box::new(right));
        }
        left
    }

    fn parse_additive(&mut self) -> Expr {
        let mut left = self.parse_multiplicative();
        loop {
            let op = match self.peek() {
                Token::Plus  => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative();
            // Compile-time string concatenation: "A" + "B" → "AB"
            if let (Expr::StringLit(a), BinOp::Add, Expr::StringLit(b)) = (&left, &op, &right) {
                left = Expr::StringLit(a.clone() + b);
            } else {
                left = Expr::BinOp(Box::new(left), op, Box::new(right));
            }
        }
        left
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut left = self.parse_primary();
        loop {
            let op = match self.peek() {
                Token::Star  => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            let right = self.parse_primary();
            left = Expr::BinOp(Box::new(left), op, Box::new(right));
        }
        left
    }

    fn parse_primary(&mut self) -> Expr {
        match self.advance() {
            Token::Number(n)    => Expr::Number(n),
            Token::Addr(a)      => Expr::Number(a as i16),
            Token::StringLit(s) => Expr::StringLit(s),
            Token::Ident(n) => {
                if let Some(&v) = self.consts.get(&n) {
                    Expr::Number(v)
                } else if self.peek() == &LBracket {
                    // arr[idx] expression
                    self.advance(); // [
                    let idx = self.parse_expr();
                    if self.peek() == &RBracket { self.advance(); } // ]
                    Expr::ArrayGet(n, Box::new(idx))
                } else {
                    Expr::Var(n)
                }
            }
            Token::StrToInt => {
                // str_to_int("123") — compile-time conversion
                if self.peek() == &Token::LParen { self.advance(); }
                let val = if let Token::StringLit(s) = self.advance() {
                    s.trim().parse::<i16>().unwrap_or(0)
                } else { 0 };
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Number(val)
            }
            Token::LParen => {
                let e = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                e
            }
            Token::Getch => {
                if self.peek() == &Token::LParen { self.advance(); } // skip (
                if self.peek() == &Token::RParen { self.advance(); } // skip )
                Expr::Getch
            }
            Token::Inkey => {
                if self.peek() == &Token::LParen { self.advance(); } // skip (
                if self.peek() == &Token::RParen { self.advance(); } // skip )
                Expr::Inkey
            }
            Token::StrLen => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::StrLen(Box::new(arg))
            }
            Token::Asc => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Asc(Box::new(arg))
            }
            Token::ReuDet => {
                if self.peek() == &Token::LParen { self.advance(); } // skip (
                if self.peek() == &Token::RParen { self.advance(); } // skip )
                Expr::ReuPresent
            }
            Token::SpriteHit => {
                if self.peek() == &Token::LParen { self.advance(); }
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::SpriteHit
            }
            Token::SpriteBgHit => {
                if self.peek() == &Token::LParen { self.advance(); }
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::SpriteBgHit
            }
            Token::Joy => {
                if self.peek() == &Token::LParen { self.advance(); } // skip (
                let port = match self.advance() {
                    Token::Number(1) => 1u8,
                    Token::Number(2) => 2u8,
                    _ => 2u8, // default to port 2
                };
                if self.peek() == &Token::RParen { self.advance(); } // skip )
                Expr::Joy(port)
            }
            Token::Peek => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Peek(Box::new(arg))
            }
            Token::Rnd => {
                if self.peek() == &Token::LParen { self.advance(); self.advance(); } // skip ()
                Expr::Rnd
            }
            Token::Abs => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Abs(Box::new(arg))
            }
            Token::Min => {
                if self.peek() == &Token::LParen { self.advance(); }
                let a = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let b = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Min(Box::new(a), Box::new(b))
            }
            Token::Max => {
                if self.peek() == &Token::LParen { self.advance(); }
                let a = self.parse_expr();
                if self.peek() == &Token::Comma { self.advance(); }
                let b = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Max(Box::new(a), Box::new(b))
            }
            Token::Sgn => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Sgn(Box::new(arg))
            }
            Token::Chr => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::ChrStr(Box::new(arg))
            }
            Token::Sin => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Sin(Box::new(arg))
            }
            Token::Cos => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::Cos(Box::new(arg))
            }
            Token::Hex => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::HexFmt(Box::new(arg))
            }
            Token::Bin => {
                if self.peek() == &Token::LParen { self.advance(); }
                let arg = self.parse_expr();
                if self.peek() == &Token::RParen { self.advance(); }
                Expr::BinFmt(Box::new(arg))
            }
            _ => Expr::Number(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::lexer::Lexer;

    fn parse(src: &str) -> Vec<Stmt> {
        let tokens = Lexer::new(src).tokenize();
        Parser::new(tokens).parse()
    }

    fn first_expr(src: &str) -> Expr {
        let tokens = Lexer::new(src).tokenize();
        let mut p = Parser::new(tokens);
        p.parse_expr()
    }

    // ── Variable declarations ────────────────────────────────────────────

    #[test]
    fn var_decl_simple() {
        let stmts = parse("var x = 5");
        assert!(matches!(&stmts[0], Stmt::VarDecl { name, vtype: None, .. } if name == "x"));
    }

    #[test]
    fn var_decl_with_type() {
        let stmts = parse("var x: int = 5");
        if let Stmt::VarDecl { name, vtype, .. } = &stmts[0] {
            assert_eq!(name, "x");
            assert!(matches!(vtype, Some(VarType::Int)));
        } else { panic!("Expected VarDecl"); }
    }

    #[test]
    fn var_decl_string_type() {
        let stmts = parse("var s: string = \"hi\"");
        if let Stmt::VarDecl { name, vtype, .. } = &stmts[0] {
            assert_eq!(name, "s");
            assert!(matches!(vtype, Some(VarType::Str)));
        } else { panic!("Expected VarDecl"); }
    }

    #[test]
    fn var_decl_float_type() {
        let stmts = parse("var f: float = 3");
        if let Stmt::VarDecl { name, vtype, .. } = &stmts[0] {
            assert_eq!(name, "f");
            assert!(matches!(vtype, Some(VarType::Float)));
        } else { panic!("Expected VarDecl"); }
    }

    // ── Assignments ──────────────────────────────────────────────────────

    #[test]
    fn assign_statement() {
        let stmts = parse("x = 5");
        assert!(matches!(&stmts[0], Stmt::Assign(name, ..) if name == "x"));
    }

    // ── Expressions ──────────────────────────────────────────────────────

    #[test]
    fn expr_number() {
        assert!(matches!(first_expr("42"), Expr::Number(42)));
    }

    #[test]
    fn expr_var() {
        assert!(matches!(first_expr("myvar"), Expr::Var(n) if n == "myvar"));
    }

    #[test]
    fn expr_string() {
        assert!(matches!(first_expr("\"hello\""), Expr::StringLit(s) if s == "hello"));
    }

    #[test]
    fn expr_add() {
        let e = first_expr("1 + 2");
        assert!(matches!(e, Expr::BinOp(_, BinOp::Add, _)));
    }

    #[test]
    fn string_concat_folds() {
        // "Hello " + "World" → single StringLit at parse time
        let e = first_expr("\"Hello \" + \"World\"");
        assert!(matches!(e, Expr::StringLit(s) if s == "Hello World"));
    }

    #[test]
    fn string_concat_triple() {
        let e = first_expr("\"A\" + \"B\" + \"C\"");
        assert!(matches!(e, Expr::StringLit(s) if s == "ABC"));
    }

    #[test]
    fn expr_mul() {
        let e = first_expr("3 * 4");
        assert!(matches!(e, Expr::BinOp(_, BinOp::Mul, _)));
    }

    #[test]
    fn expr_precedence() {
        // 1 + 2 * 3 should parse as 1 + (2 * 3)
        let e = first_expr("1 + 2 * 3");
        assert!(matches!(e, Expr::BinOp(_, BinOp::Add, _)));
        if let Expr::BinOp(left, BinOp::Add, right) = e {
            assert!(matches!(*left, Expr::Number(1)));
            assert!(matches!(*right, Expr::BinOp(_, BinOp::Mul, _)));
        }
    }

    #[test]
    fn expr_eq() {
        let e = first_expr("a == b");
        assert!(matches!(e, Expr::BinOp(_, BinOp::Eq, _)));
    }

    #[test]
    fn expr_logic() {
        let e = first_expr("a == 1 and b == 2");
        assert!(matches!(e, Expr::BinOp(_, BinOp::And, _)));
    }

    #[test]
    fn expr_not() {
        let e = first_expr("not a");
        assert!(matches!(e, Expr::Not(_)));
    }

    #[test]
    fn expr_parens() {
        let e = first_expr("(1 + 2)");
        assert!(matches!(e, Expr::BinOp(_, BinOp::Add, _)));
    }

    #[test]
    fn expr_getch() {
        assert!(matches!(first_expr("getch()"), Expr::Getch));
    }

    // ── Print ────────────────────────────────────────────────────────────

    #[test]
    fn print_string() {
        let stmts = parse("print \"hello\"");
        assert!(matches!(&stmts[0], Stmt::Print(args) if args.len() == 1));
    }

    #[test]
    fn print_multiple_args() {
        let stmts = parse("print \"X=\", x");
        assert!(matches!(&stmts[0], Stmt::Print(args) if args.len() == 2));
    }

    #[test]
    fn print_empty() {
        // bare `print` = just newline
        let stmts = parse("print");
        assert!(matches!(&stmts[0], Stmt::Print(args) if args.is_empty()));
    }

    #[test]
    fn print_var_var_string() {
        // print x, y, "text"  – all orders work
        let stmts = parse("print x, y, \"hello\"");
        assert!(matches!(&stmts[0], Stmt::Print(args) if args.len() == 3));
    }

    #[test]
    fn print_string_var_string() {
        let stmts = parse("print \"A=\", a, \" B=\", b");
        assert!(matches!(&stmts[0], Stmt::Print(args) if args.len() == 4));
    }

    // ── Control flow ─────────────────────────────────────────────────────

    #[test]
    fn if_then() {
        let stmts = parse("if x == 1 then\n  x = 2\nend");
        assert!(matches!(&stmts[0], Stmt::If(_, then_body, None) if then_body.len() == 1));
    }

    #[test]
    fn if_else() {
        let stmts = parse("if x == 1 then\n  x = 2\nelse\n  x = 3\nend");
        if let Stmt::If(_, _, else_body) = &stmts[0] {
            assert!(else_body.is_some());
            assert_eq!(else_body.as_ref().unwrap().len(), 1);
        } else { panic!("Expected If"); }
    }

    // ── Loops ────────────────────────────────────────────────────────────

    #[test]
    fn loop_infinite() {
        let stmts = parse("loop\n  x = 1\nend");
        assert!(matches!(&stmts[0], Stmt::Loop(0, _))); // 0 = infinite
    }

    #[test]
    fn loop_counted() {
        let stmts = parse("loop 5\n  x = 1\nend");
        assert!(matches!(&stmts[0], Stmt::Loop(5, _)));
    }

    #[test]
    fn for_loop() {
        let stmts = parse("loop i = 1 to 5\n  print i\nend");
        assert!(matches!(&stmts[0], Stmt::ForLoop { var, .. } if var == "i"));
    }

    #[test]
    fn for_loop_with_step() {
        let stmts = parse("loop i = 0 to 10 step 2\n  print i\nend");
        if let Stmt::ForLoop { step, .. } = &stmts[0] {
            assert!(step.is_some());
        } else { panic!("Expected ForLoop"); }
    }

    #[test]
    fn for_next_syntax() {
        let stmts = parse("for i = 1 to 5\n  print i\nnext");
        assert!(matches!(&stmts[0], Stmt::ForLoop { var, .. } if var == "i"));
    }

    #[test]
    fn for_next_with_var() {
        // `next i` — variable name after next is optional/cosmetic
        let stmts = parse("for i = 1 to 10\n  print i\nnext i");
        assert!(matches!(&stmts[0], Stmt::ForLoop { var, .. } if var == "i"));
    }

    #[test]
    fn for_next_with_step() {
        let stmts = parse("for i = 0 to 20 step 2\n  print i\nnext");
        if let Stmt::ForLoop { var, step, .. } = &stmts[0] {
            assert_eq!(var, "i");
            assert!(step.is_some());
        } else { panic!("Expected ForLoop"); }
    }

    #[test]
    fn while_loop() {
        let stmts = parse("while x < 10\n  x = x + 1\nend");
        assert!(matches!(&stmts[0], Stmt::WhileLoop(_, _)));
    }

    #[test]
    fn break_stmt() {
        let stmts = parse("loop\n  break\nend");
        assert!(stmts.len() == 1);
    }

    // ── Subroutines ──────────────────────────────────────────────────────

    #[test]
    fn sub_def() {
        let stmts = parse("sub test()\n  print \"hi\"\nend");
        assert!(matches!(&stmts[0], Stmt::SubDef(name, _, _) if name == "test"));
    }

    #[test]
    fn sub_def_with_params() {
        let stmts = parse("sub draw(x, y)\n  print x\nend");
        if let Stmt::SubDef(name, params, _) = &stmts[0] {
            assert_eq!(name, "draw");
            assert_eq!(params, &vec!["x".to_string(), "y".to_string()]);
        } else { panic!("Expected SubDef"); }
    }

    #[test]
    fn call_stmt() {
        let stmts = parse("test()");
        assert!(matches!(&stmts[0], Stmt::Call(name, _) if name == "test"));
    }

    #[test]
    fn call_with_args() {
        let stmts = parse("draw(10, 20)");
        if let Stmt::Call(name, args) = &stmts[0] {
            assert_eq!(name, "draw");
            assert_eq!(args.len(), 2);
        } else { panic!("Expected Call"); }
    }

    #[test]
    fn return_stmt() {
        let stmts = parse("return");
        assert!(matches!(&stmts[0], Stmt::Return));
    }

    // ── Graphics / Screen ────────────────────────────────────────────────

    #[test]
    fn cls() {
        let stmts = parse("cls");
        assert!(matches!(&stmts[0], Stmt::Cls { fast: false }));
    }
    #[test] fn cls_fast() {
        let stmts = parse("cls fast");
        assert!(matches!(&stmts[0], Stmt::Cls { fast: true }));
    }
    #[test] fn graphics_on() {
        let stmts = parse("graphics on");
        assert!(matches!(&stmts[0], Stmt::Graphics { on: true, multi: false }));
    }
    #[test] fn graphics_off() {
        let stmts = parse("graphics off");
        assert!(matches!(&stmts[0], Stmt::Graphics { on: false, multi: false }));
    }
    #[test] fn graphics_on_multi() {
        let stmts = parse("graphics on multi");
        assert!(matches!(&stmts[0], Stmt::Graphics { on: true, multi: true }));
    }

    // ── Colors ───────────────────────────────────────────────────────────

    #[test] fn color_num() {
        let stmts = parse("color 7");
        assert!(matches!(&stmts[0], Stmt::Color { target: ColorTarget::Text, .. }));
    }
    #[test] fn color_text() {
        let stmts = parse("color text 7");
        assert!(matches!(&stmts[0], Stmt::Color { target: ColorTarget::Text, .. }));
    }
    #[test] fn color_border() {
        let stmts = parse("color border 2");
        assert!(matches!(&stmts[0], Stmt::Color { target: ColorTarget::Border, .. }));
    }
    #[test] fn color_bg() {
        let stmts = parse("color bg 0");
        assert!(matches!(&stmts[0], Stmt::Color { target: ColorTarget::Bg, .. }));
    }

    // ── Sys / Asm ────────────────────────────────────────────────────────

    #[test] fn sys_stmt() {
        let stmts = parse("sys $FFD2");
        assert!(matches!(&stmts[0], Stmt::Sys(0xFFD2)));
    }
    #[test] fn asm_inline() {
        let stmts = parse("asm $EA, $EA");
        assert!(matches!(&stmts[0], Stmt::AsmBytes(b) if b.len() == 2));
    }
    #[test] fn asm_block() {
        let stmts = parse("asm { $A9 $07 }");
        assert!(matches!(&stmts[0], Stmt::AsmBytes(b) if b.len() == 2));
    }

    // ── IntToStr (numstr) ────────────────────────────────────────────────

    #[test] fn numstr_stmt() {
        let stmts = parse("numstr score, $0340");
        assert!(matches!(&stmts[0], Stmt::IntToStr { var, addr } if var == "score" && *addr == 0x0340));
    }

    // ── New features: const, label, goto, poke, peek, rnd, abs, min, max, sgn ──

    #[test] fn const_stmt() {
        let stmts = parse("const SCREEN = $0400");
        assert!(matches!(&stmts[0], Stmt::Const(name, Expr::Number(0x0400)) if name == "SCREEN"));
    }

    #[test] fn const_stmt_decimal() {
        let stmts = parse("const SIZE = 100");
        assert!(matches!(&stmts[0], Stmt::Const(name, Expr::Number(100)) if name == "SIZE"));
    }

    #[test] fn label_stmt() {
        let stmts = parse("label main_loop");
        assert!(matches!(&stmts[0], Stmt::Label(name) if name == "main_loop"));
    }

    #[test] fn goto_stmt() {
        let stmts = parse("goto main_loop");
        assert!(matches!(&stmts[0], Stmt::Goto(name) if name == "main_loop"));
    }

    #[test] fn poke_stmt() {
        let stmts = parse("poke $D020, 2");
        assert!(matches!(&stmts[0], Stmt::Poke(Expr::Number(addr), Expr::Number(2)) if *addr == 0xD020u16 as i16));
    }

    #[test] fn poke_stmt_with_expr() {
        let stmts = parse("poke $0400 + 10, x");
        assert!(matches!(&stmts[0], Stmt::Poke(_, Expr::Var(name)) if name == "x"));
    }

    #[test] fn expr_peek() {
        let e = first_expr("peek($D012)");
        assert!(matches!(e, Expr::Peek(boxed) if matches!(*boxed, Expr::Number(addr) if addr == 0xD012u16 as i16)));
    }

    #[test] fn expr_peek_with_expr() {
        let e = first_expr("peek(addr)");
        assert!(matches!(e, Expr::Peek(boxed) if matches!(*boxed, Expr::Var(_))));
    }

    #[test] fn expr_rnd() {
        let e = first_expr("rnd()");
        assert!(matches!(e, Expr::Rnd));
    }

    #[test] fn expr_rnd_no_parens() {
        let e = first_expr("rnd");
        assert!(matches!(e, Expr::Rnd));
    }

    #[test] fn expr_abs() {
        let e = first_expr("abs(x)");
        assert!(matches!(e, Expr::Abs(boxed) if matches!(*boxed, Expr::Var(_))));
    }

    #[test] fn expr_abs_with_expr() {
        let e = first_expr("abs(x - 20)");
        assert!(matches!(e, Expr::Abs(boxed) if matches!(*boxed, Expr::BinOp(_, BinOp::Sub, _))));
    }

    #[test] fn expr_min() {
        let e = first_expr("min(x, 39)");
        assert!(matches!(e, Expr::Min(a, b) if matches!(*a, Expr::Var(_)) && matches!(*b, Expr::Number(39))));
    }

    #[test] fn expr_max() {
        let e = first_expr("max(y, 0)");
        assert!(matches!(e, Expr::Max(a, b) if matches!(*a, Expr::Var(_)) && matches!(*b, Expr::Number(0))));
    }

    #[test] fn expr_sgn() {
        let e = first_expr("sgn(x - 20)");
        assert!(matches!(e, Expr::Sgn(boxed) if matches!(*boxed, Expr::BinOp(_, BinOp::Sub, _))));
    }

    #[test] fn expr_sgn_simple() {
        let e = first_expr("sgn(dx)");
        assert!(matches!(e, Expr::Sgn(boxed) if matches!(*boxed, Expr::Var(_))));
    }

    // ── Arrays ───────────────────────────────────────────────────────────

    #[test]
    fn array_decl() {
        let stmts = parse("var scores = array(10)");
        if let Stmt::VarDecl { name, vtype, expr: Expr::Number(10) } = &stmts[0] {
            assert_eq!(name, "scores");
            assert!(matches!(vtype, Some(VarType::Array)));
        } else { panic!("Expected array VarDecl"); }
    }

    #[test]
    fn array_set() {
        let stmts = parse("scores[0] = 100");
        assert!(matches!(&stmts[0], Stmt::ArraySet(name, Expr::Number(0), Expr::Number(100)) if name == "scores"));
    }

    #[test]
    fn array_get() {
        let e = first_expr("scores[i]");
        assert!(matches!(e, Expr::ArrayGet(name, _) if name == "scores"));
    }

    // ── Word (16-bit) vars ───────────────────────────────────────────────

    #[test]
    fn word_var_decl() {
        let stmts = parse("var ptr: word = $0400");
        if let Stmt::VarDecl { name, vtype, .. } = &stmts[0] {
            assert_eq!(name, "ptr");
            assert!(matches!(vtype, Some(VarType::Word)));
        } else { panic!("Expected VarDecl"); }
    }

    #[test] fn const_substitution() {
        // const should substitute in expressions
        let stmts = parse("const X = 10\nvar y = X + 5");
        assert!(matches!(&stmts[1], Stmt::VarDecl { name, expr: Expr::BinOp(a, BinOp::Add, b), .. }
            if name == "y" && matches!(**a, Expr::Number(10)) && matches!(**b, Expr::Number(5))));
    }

    #[test] fn chr_str_expr() {
        let e = first_expr("chr$(65)");
        assert!(matches!(e, Expr::ChrStr(boxed) if matches!(*boxed, Expr::Number(65))));
    }

    #[test] fn plot_stmt() {
        let stmts = parse("plot 10, 20");
        assert!(matches!(&stmts[0], Stmt::Plot(Expr::Number(10), Expr::Number(20))));
    }

    #[test] fn plot_stmt_vars() {
        let stmts = parse("plot x, y");
        assert!(matches!(&stmts[0], Stmt::Plot(Expr::Var(a), Expr::Var(b)) if a == "x" && b == "y"));
    }

    #[test] fn gcls_stmt() {
        let stmts = parse("gcls");
        assert!(matches!(&stmts[0], Stmt::Gcls));
    }

    #[test] fn label_goto_sequence() {
        let stmts = parse("label start\nx = x + 1\ngoto start");
        assert_eq!(stmts.len(), 3);
        assert!(matches!(&stmts[0], Stmt::Label(_)));
        assert!(matches!(&stmts[1], Stmt::Assign(_, _)));
        assert!(matches!(&stmts[2], Stmt::Goto(_)));
    }
}
