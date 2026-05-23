#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(i16),
    StringLit(String),
    Ident(String),

    // Keywords
    Var,
    Sub,
    End,
    If,
    Then,
    Else,
    Loop,
    While,
    To,
    Step,
    Break,
    Print,
    Return,
    Call,
    Cls,
    Graphics,
    Display,  // display on / display off — controls VIC DEN bit
    On,
    Off,
    Fast,
    Sys,
    Asm,
    StrToInt,
    IntToStr,
    Color,
    Text,
    Border,
    Bg,
    Getch,
    ReuPresent,  // reu_present() — inline REU detection, returns 0 or 1
    And,
    Or,
    Not,
    Xor,
    Shl,
    Shr,
    Wait,
    Raster,
    Sound,
    Sprite,          // sprite id, x, y [, data_addr]
    SpriteOn,        // sprite_on id
    SpriteOff,       // sprite_off id
    SpriteColor,     // sprite_color id, color
    SpriteMulticolor,// sprite_multicolor id, on/off
    SpriteHit,       // sprite_hit()  — $D01E sprite-sprite collision
    SpriteBgHit,     // sprite_bg_hit() — $D01F sprite-background collision
    SpriteDef,       // sprite_def id, b0..b62 — align+embed sprite data, init $07F8+id
    Int,
    Str,
    Float,
    Const,
    Label,
    Goto,
    Poke,
    Peek,
    Rnd,
    Abs,
    Min,
    Max,
    Sgn,
    Word,
    Array,
    For,
    Next,
    Chr,
    Plot,
    Gcls,
    Bye,
    Joy,
    Line,
    Sin,
    Cos,
    Hex,
    Bin,
    Reu,
    Stash,
    Fetch,
    Multi,
    Incbin,
    Include,
    Data,
    Read,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Assign,

    // Address / hex literal
    Addr(u16),

    // Punctuation
    Colon,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    LBrace,
    RBrace,
    Newline,
    Eof,
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
}

impl Lexer {
    pub fn new(src: &str) -> Self {
        Self { input: src.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.input.get(self.pos).copied();
        self.pos += 1;
        c
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(' ') | Some('\t') | Some('\r')) {
            self.advance();
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = vec![];
        loop {
            self.skip_spaces();
            match self.peek() {
                None => { tokens.push(Token::Eof); break; }
                Some('\n') => { self.advance(); tokens.push(Token::Newline); }
                Some('#') | Some(';') => { while !matches!(self.peek(), None | Some('\n')) { self.advance(); } }
                Some('"') => tokens.push(self.read_string()),
                Some('$') => tokens.push(self.read_hex()),
                Some('{') => { self.advance(); tokens.push(Token::LBrace); }
                Some('}') => { self.advance(); tokens.push(Token::RBrace); }
                Some(c) if c.is_ascii_digit() => tokens.push(self.read_number()),
                Some(c) if c.is_alphabetic() || c == '_' => tokens.push(self.read_ident()),
                Some('+') => { self.advance(); tokens.push(Token::Plus); }
                Some('-') => { self.advance(); tokens.push(Token::Minus); }
                Some('*') => { self.advance(); tokens.push(Token::Star); }
                Some('/') => { self.advance(); tokens.push(Token::Slash); }
                Some('=') => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); tokens.push(Token::Eq); }
                    else { tokens.push(Token::Assign); }
                }
                Some('!') => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); tokens.push(Token::NotEq); }
                    else { tokens.push(Token::Not); }
                }
                Some('&') => {
                    self.advance();
                    if self.peek() == Some('&') { self.advance(); tokens.push(Token::And); }
                }
                Some('|') => {
                    self.advance();
                    if self.peek() == Some('|') { self.advance(); tokens.push(Token::Or); }
                }
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); tokens.push(Token::LtEq); }
                    else { tokens.push(Token::Lt); }
                }
                Some('>') => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); tokens.push(Token::GtEq); }
                    else { tokens.push(Token::Gt); }
                }
                Some('[') => { self.advance(); tokens.push(Token::LBracket); }
                Some(']') => { self.advance(); tokens.push(Token::RBracket); }
                Some('(') => { self.advance(); tokens.push(Token::LParen); }
                Some(')') => { self.advance(); tokens.push(Token::RParen); }
                Some(',') => { self.advance(); tokens.push(Token::Comma); }
                Some(':') => { self.advance(); tokens.push(Token::Colon); }
                Some(c) => { self.advance(); eprintln!("Unknown char: {c}"); }
            }
        }
        tokens
    }

    fn read_string(&mut self) -> Token {
        self.advance(); // skip "
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c == '"' { self.advance(); break; }
            s.push(c);
            self.advance();
        }
        Token::StringLit(s)
    }

    fn read_number(&mut self) -> Token {
        let mut s = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            s.push(self.advance().unwrap());
        }
        let val: u32 = s.parse().unwrap_or(0);
        if val > 0x7FFF { Token::Addr(val as u16) } else { Token::Number(val as i16) }
    }

    fn read_hex(&mut self) -> Token {
        self.advance(); // skip '$'
        let mut s = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_hexdigit()) {
            s.push(self.advance().unwrap());
        }
        let val = u16::from_str_radix(&s, 16).unwrap_or(0);
        if val > 0x7FFF { Token::Addr(val) } else { Token::Number(val as i16) }
    }

    fn read_ident(&mut self) -> Token {
        let mut s = String::new();
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
            s.push(self.advance().unwrap());
        }
        // Special: "chr$" — BASIC-style char-by-code function
        if s == "chr" && self.peek() == Some('$') {
            self.advance();
            return Token::Chr;
        }
        match s.as_str() {
            "var"      => Token::Var,
            "sub"      => Token::Sub,
            "end"      => Token::End,
            "if"       => Token::If,
            "then"     => Token::Then,
            "else"     => Token::Else,
            "loop"     => Token::Loop,
            "while"    => Token::While,
            "to"       => Token::To,
            "step"     => Token::Step,
            "break"    => Token::Break,
            "print"    => Token::Print,
            "return"   => Token::Return,
            "call"     => Token::Call,
            "cls"        => Token::Cls,
            "graphics"   => Token::Graphics,
            "display"    => Token::Display,
            "on"         => Token::On,
            "off"        => Token::Off,
            "fast"       => Token::Fast,
            "sys"        => Token::Sys,
            "asm"        => Token::Asm,
            "str_to_int" => Token::StrToInt,
            "int_to_str" => Token::IntToStr,
            "color"      => Token::Color,
            "text"       => Token::Text,
            "border"     => Token::Border,
            "bg"         => Token::Bg,
            "getch"      => Token::Getch,
            "reu_present" => Token::ReuPresent,
            "and"        => Token::And,
            "or"         => Token::Or,
            "not"        => Token::Not,
            "xor"        => Token::Xor,
            "shl"        => Token::Shl,
            "shr"        => Token::Shr,
            "wait"       => Token::Wait,
            "raster"     => Token::Raster,
            "sound"      => Token::Sound,
            "int"        => Token::Int,
            "string"     => Token::Str,
            "float"      => Token::Float,
            "const"      => Token::Const,
            "label"      => Token::Label,
            "goto"       => Token::Goto,
            "poke"       => Token::Poke,
            "peek"       => Token::Peek,
            "rnd"        => Token::Rnd,
            "abs"        => Token::Abs,
            "min"        => Token::Min,
            "max"        => Token::Max,
            "sgn"        => Token::Sgn,
            "word"       => Token::Word,
            "array"      => Token::Array,
            "for"        => Token::For,
            "next"       => Token::Next,
            "plot"       => Token::Plot,
            "gcls"       => Token::Gcls,
            "bye"        => Token::Bye,
            "joy"        => Token::Joy,
            "line"       => Token::Line,
            "sin"        => Token::Sin,
            "cos"        => Token::Cos,
            "hex"        => Token::Hex,
            "bin"        => Token::Bin,
            "reu"        => Token::Reu,
            "stash"      => Token::Stash,
            "fetch"      => Token::Fetch,
            "multi"      => Token::Multi,
            "exit"       => Token::Bye,
            "incbin"     => Token::Incbin,
            "include"    => Token::Include,
            "data"       => Token::Data,
            "read"       => Token::Read,
            "sprite"            => Token::Sprite,
            "sprite_on"         => Token::SpriteOn,
            "sprite_off"        => Token::SpriteOff,
            "sprite_color"      => Token::SpriteColor,
            "sprite_multicolor" => Token::SpriteMulticolor,
            "sprite_hit"        => Token::SpriteHit,
            "sprite_bg_hit"     => Token::SpriteBgHit,
            "sprite_def"       => Token::SpriteDef,
            "rem"        => {
                while !matches!(self.peek(), None | Some('\n')) { self.advance(); }
                if self.peek() == Some('\n') { self.advance(); }
                Token::Newline
            }
            _            => Token::Ident(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(src: &str) -> Vec<Token> {
        Lexer::new(src).tokenize()
    }

    // ── Operators ────────────────────────────────────────────────────────

    #[test] fn plus()  { assert_eq!(tokenize("+"), vec![Token::Plus, Token::Eof]); }
    #[test] fn minus() { assert_eq!(tokenize("-"), vec![Token::Minus, Token::Eof]); }
    #[test] fn star()  { assert_eq!(tokenize("*"), vec![Token::Star, Token::Eof]); }
    #[test] fn slash() { assert_eq!(tokenize("/"), vec![Token::Slash, Token::Eof]); }
    #[test] fn eq()    { assert_eq!(tokenize("=="), vec![Token::Eq, Token::Eof]); }
    #[test] fn not_eq(){ assert_eq!(tokenize("!="), vec![Token::NotEq, Token::Eof]); }
    #[test] fn lt()    { assert_eq!(tokenize("<"), vec![Token::Lt, Token::Eof]); }
    #[test] fn gt()    { assert_eq!(tokenize(">"), vec![Token::Gt, Token::Eof]); }
    #[test] fn lt_eq() { assert_eq!(tokenize("<="), vec![Token::LtEq, Token::Eof]); }
    #[test] fn gt_eq() { assert_eq!(tokenize(">="), vec![Token::GtEq, Token::Eof]); }
    #[test] fn assign(){ assert_eq!(tokenize("="), vec![Token::Assign, Token::Eof]); }
    #[test] fn and_op(){ assert_eq!(tokenize("&&"), vec![Token::And, Token::Eof]); }
    #[test] fn or_op() { assert_eq!(tokenize("||"), vec![Token::Or, Token::Eof]); }
    #[test] fn not_op(){ assert_eq!(tokenize("!"), vec![Token::Not, Token::Eof]); }

    // ── Punctuation ──────────────────────────────────────────────────────

    #[test] fn colon() { assert_eq!(tokenize(":"), vec![Token::Colon, Token::Eof]); }
    #[test] fn lparen(){ assert_eq!(tokenize("("), vec![Token::LParen, Token::Eof]); }
    #[test] fn rparen(){ assert_eq!(tokenize(")"), vec![Token::RParen, Token::Eof]); }
    #[test] fn comma() { assert_eq!(tokenize(","), vec![Token::Comma, Token::Eof]); }
    #[test] fn lbrace(){ assert_eq!(tokenize("{"), vec![Token::LBrace, Token::Eof]); }
    #[test] fn rbrace(){ assert_eq!(tokenize("}"), vec![Token::RBrace, Token::Eof]); }

    // ── Keywords ─────────────────────────────────────────────────────────

    #[test] fn kw_var()  { assert_eq!(tokenize("var")[0],  Token::Var); }
    #[test] fn kw_sub()  { assert_eq!(tokenize("sub")[0],  Token::Sub); }
    #[test] fn kw_end()  { assert_eq!(tokenize("end")[0],  Token::End); }
    #[test] fn kw_if()   { assert_eq!(tokenize("if")[0],   Token::If); }
    #[test] fn kw_then() { assert_eq!(tokenize("then")[0], Token::Then); }
    #[test] fn kw_else() { assert_eq!(tokenize("else")[0], Token::Else); }
    #[test] fn kw_loop() { assert_eq!(tokenize("loop")[0], Token::Loop); }
    #[test] fn kw_while(){ assert_eq!(tokenize("while")[0],Token::While); }
    #[test] fn kw_to()   { assert_eq!(tokenize("to")[0],   Token::To); }
    #[test] fn kw_step() { assert_eq!(tokenize("step")[0], Token::Step); }
    #[test] fn kw_break(){ assert_eq!(tokenize("break")[0],Token::Break); }
    #[test] fn kw_print(){ assert_eq!(tokenize("print")[0],Token::Print); }
    #[test] fn kw_return(){assert_eq!(tokenize("return")[0],Token::Return);}
    #[test] fn kw_call() { assert_eq!(tokenize("call")[0], Token::Call); }
    #[test] fn kw_cls()  { assert_eq!(tokenize("cls")[0],  Token::Cls); }
    #[test] fn kw_sys()  { assert_eq!(tokenize("sys")[0],  Token::Sys); }
    #[test] fn kw_asm()  { assert_eq!(tokenize("asm")[0],  Token::Asm); }
    #[test] fn kw_color(){ assert_eq!(tokenize("color")[0],Token::Color); }
    #[test] fn kw_text() { assert_eq!(tokenize("text")[0], Token::Text); }
    #[test] fn kw_border(){assert_eq!(tokenize("border")[0],Token::Border);}
    #[test] fn kw_bg()   { assert_eq!(tokenize("bg")[0],   Token::Bg); }
    #[test] fn kw_getch(){ assert_eq!(tokenize("getch")[0],Token::Getch); }
    #[test] fn kw_and()  { assert_eq!(tokenize("and")[0],  Token::And); }
    #[test] fn kw_or()   { assert_eq!(tokenize("or")[0],   Token::Or); }
    #[test] fn kw_not()  { assert_eq!(tokenize("not")[0],  Token::Not); }
    #[test] fn kw_int()  { assert_eq!(tokenize("int")[0],  Token::Int); }
    #[test] fn kw_string(){assert_eq!(tokenize("string")[0],Token::Str);}
    #[test] fn kw_float(){ assert_eq!(tokenize("float")[0],Token::Float);}
    #[test] fn kw_graphics(){assert_eq!(tokenize("graphics")[0],Token::Graphics);}
    #[test] fn kw_on()   { assert_eq!(tokenize("on")[0],   Token::On); }
    #[test] fn kw_off()  { assert_eq!(tokenize("off")[0],  Token::Off); }
    #[test] fn kw_fast()  {assert_eq!(tokenize("fast")[0],  Token::Fast);}
    #[test] fn kw_str_to_int(){assert_eq!(tokenize("str_to_int")[0],Token::StrToInt);}
    #[test] fn kw_int_to_str(){assert_eq!(tokenize("int_to_str")[0],Token::IntToStr);}
    #[test] fn kw_const()  { assert_eq!(tokenize("const")[0], Token::Const); }
    #[test] fn kw_label()  { assert_eq!(tokenize("label")[0], Token::Label); }
    #[test] fn kw_goto()   { assert_eq!(tokenize("goto")[0],  Token::Goto); }
    #[test] fn kw_poke()   { assert_eq!(tokenize("poke")[0],  Token::Poke); }
    #[test] fn kw_peek()   { assert_eq!(tokenize("peek")[0],  Token::Peek); }
    #[test] fn kw_rnd()    { assert_eq!(tokenize("rnd")[0],   Token::Rnd); }
    #[test] fn kw_abs()    { assert_eq!(tokenize("abs")[0],   Token::Abs); }
    #[test] fn kw_min()    { assert_eq!(tokenize("min")[0],   Token::Min); }
    #[test] fn kw_max()    { assert_eq!(tokenize("max")[0],   Token::Max); }
    #[test] fn kw_sgn()    { assert_eq!(tokenize("sgn")[0],   Token::Sgn); }
    #[test] fn kw_word()   { assert_eq!(tokenize("word")[0],  Token::Word); }
    #[test] fn kw_array()  { assert_eq!(tokenize("array")[0], Token::Array); }
    #[test] fn kw_for()    { assert_eq!(tokenize("for")[0],   Token::For); }
    #[test] fn kw_next()   { assert_eq!(tokenize("next")[0],  Token::Next); }
    #[test] fn kw_plot()   { assert_eq!(tokenize("plot")[0],  Token::Plot); }
    #[test] fn kw_joy()    { assert_eq!(tokenize("joy")[0],   Token::Joy); }
    #[test] fn kw_line()   { assert_eq!(tokenize("line")[0],  Token::Line); }
    #[test] fn kw_sin()    { assert_eq!(tokenize("sin")[0],   Token::Sin); }
    #[test] fn kw_cos()    { assert_eq!(tokenize("cos")[0],   Token::Cos); }
    #[test] fn kw_hex()    { assert_eq!(tokenize("hex")[0],   Token::Hex); }
    #[test] fn kw_bin()    { assert_eq!(tokenize("bin")[0],   Token::Bin); }
    #[test] fn kw_reu()    { assert_eq!(tokenize("reu")[0],   Token::Reu); }
    #[test] fn kw_reu_present() { assert_eq!(tokenize("reu_present")[0], Token::ReuPresent); }
    #[test] fn kw_gcls()   { assert_eq!(tokenize("gcls")[0],  Token::Gcls); }
    #[test] fn kw_bye()    { assert_eq!(tokenize("bye")[0],   Token::Bye); }
    #[test] fn kw_exit()   { assert_eq!(tokenize("exit")[0],  Token::Bye); }
    #[test] fn kw_incbin() { assert_eq!(tokenize("incbin")[0], Token::Incbin); }
    #[test] fn kw_include(){ assert_eq!(tokenize("include")[0], Token::Include); }
    #[test] fn kw_data()   { assert_eq!(tokenize("data")[0],  Token::Data); }
    #[test] fn kw_read()   { assert_eq!(tokenize("read")[0],  Token::Read); }
    #[test]
    fn rem_skips_to_eol() {
        let toks = tokenize("rem ignored text\nvar x = 1");
        // rem consumes line and emits Newline; next tokens are var x = 1
        assert!(toks.iter().any(|t| t == &Token::Var), "var should follow rem line");
    }
    #[test]
    fn semicolon_skips_to_eol() {
        let toks = tokenize("42 ; this is ignored\n99");
        assert_eq!(toks[0], Token::Number(42));
        assert!(toks.iter().any(|t| t == &Token::Number(99)));
    }
    #[test] fn kw_chr_dollar() {
        assert_eq!(tokenize("chr$")[0], Token::Chr);
    }
    #[test] fn chr_dollar_no_ident_after() {
        // "chr$" followed by '(' should tokenize as Chr + LParen
        let toks = tokenize("chr$(65)");
        assert_eq!(toks[0], Token::Chr);
        assert_eq!(toks[1], Token::LParen);
    }
    #[test] fn lbracket()  { assert_eq!(tokenize("["), vec![Token::LBracket, Token::Eof]); }
    #[test] fn rbracket()  { assert_eq!(tokenize("]"), vec![Token::RBracket, Token::Eof]); }

    // ── Numbers ──────────────────────────────────────────────────────────

    #[test] fn number_simple() {
        assert_eq!(tokenize("42")[0], Token::Number(42));
    }
    #[test] fn number_zero() {
        assert_eq!(tokenize("0")[0], Token::Number(0));
    }
    #[test] fn number_negative() {
        // minus is a separate token
        assert_eq!(tokenize("-5"), vec![Token::Minus, Token::Number(5), Token::Eof]);
    }

    // ── Hex numbers ──────────────────────────────────────────────────────

    #[test] fn hex_small() {
        assert_eq!(tokenize("$FF")[0], Token::Number(0xFF));
    }
    #[test] fn hex_addr() {
        // Values > 0x7FFF produce Addr token
        // $D020 = 53280, which is > 0x7FFF.
        assert_eq!(tokenize("$D020")[0], Token::Addr(0xD020));
    }
    #[test] fn hex_border() {
        // $0286 = 646, small number
        assert_eq!(tokenize("$0286")[0], Token::Number(0x0286));
    }

    // ── Identifiers ──────────────────────────────────────────────────────

    #[test] fn ident_simple() {
        assert_eq!(tokenize("hello")[0], Token::Ident("hello".into()));
    }
    #[test] fn ident_with_underscore() {
        assert_eq!(tokenize("my_var")[0], Token::Ident("my_var".into()));
    }

    // ── String literals ─────────────────────────────────────────────────

    #[test] fn string_lit() {
        assert_eq!(tokenize("\"hello\""), vec![Token::StringLit("hello".into()), Token::Eof]);
    }
    #[test] fn empty_string() {
        assert_eq!(tokenize("\"\""), vec![Token::StringLit("".into()), Token::Eof]);
    }

    // ── Comments ─────────────────────────────────────────────────────────
    #[test]
    fn comment_ignored() {
        assert_eq!(tokenize("# comment\n42"), vec![Token::Newline, Token::Number(42), Token::Eof]);
    }
    #[test]
    fn comment_at_end() {
        assert_eq!(tokenize("42# comment"), vec![Token::Number(42), Token::Eof]);
    }
    // ── Multi-token ──────────────────────────────────────────────────────

    #[test] fn multi_token_statement() {
        let tokens = tokenize("var x = 5");
        assert_eq!(tokens, vec![
            Token::Var,
            Token::Ident("x".into()),
            Token::Assign,
            Token::Number(5),
            Token::Eof,
        ]);
    }

    #[test] fn print_with_args() {
        let tokens = tokenize("print \"Hi\", 42");
        assert_eq!(tokens, vec![
            Token::Print,
            Token::StringLit("Hi".into()),
            Token::Comma,
            Token::Number(42),
            Token::Eof,
        ]);
    }

    #[test] fn for_loop_tokens() {
        let tokens = tokenize("loop i = 1 to 5 step 2");
        assert_eq!(tokens, vec![
            Token::Loop,
            Token::Ident("i".into()),
            Token::Assign,
            Token::Number(1),
            Token::To,
            Token::Number(5),
            Token::Step,
            Token::Number(2),
            Token::Eof,
        ]);
    }

    #[test] fn color_border_tokens() {
        let tokens = tokenize("color border 2");
        assert_eq!(tokens, vec![
            Token::Color,
            Token::Border,
            Token::Number(2),
            Token::Eof,
        ]);
    }

    #[test] fn typed_var_tokens() {
        let tokens = tokenize("var s: string = \"hi\"");
        assert_eq!(tokens, vec![
            Token::Var,
            Token::Ident("s".into()),
            Token::Colon,
            Token::Str,
            Token::Assign,
            Token::StringLit("hi".into()),
            Token::Eof,
        ]);
    }

    #[test] fn asm_block_tokens() {
        let tokens = tokenize("asm { $A9 $07 }");
        assert_eq!(tokens, vec![
            Token::Asm,
            Token::LBrace,
            Token::Number(0xA9),
            Token::Number(0x07),
            Token::RBrace,
            Token::Eof,
        ]);
    }

    #[test] fn newline_separates() {
        let tokens = tokenize("var x = 1\nvar y = 2");
        assert_eq!(tokens, vec![
            Token::Var, Token::Ident("x".into()), Token::Assign, Token::Number(1),
            Token::Newline,
            Token::Var, Token::Ident("y".into()), Token::Assign, Token::Number(2),
            Token::Eof,
        ]);
    }
}
