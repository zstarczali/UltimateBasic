pub mod ast;
pub mod lexer;
pub mod parser;
pub mod codegen;

use lexer::Lexer;
use parser::Parser;
use codegen::Codegen;

pub struct CompileOptions {
    pub basic_stub: bool,
}

pub struct CompileResult {
    pub prg: Vec<u8>,
    pub errors: Vec<String>,
}

/// BASIC stub: 10 SYS 2061
/// Code must start at $080D (= 2061 decimal) for this to work.
const BASIC_STUB: &[u8] = &[
    0x01, 0x08, // PRG load address $0801
    0x0B, 0x08, // next line pointer -> $080B
    0x0A, 0x00, // line number 10
    0x9E,       // SYS token
    0x32, 0x30, 0x36, 0x31, // "2061"
    0x00,       // end of line
    0x00, 0x00, // end of BASIC program
]; // 14 bytes total, code starts at $080F

// $0801 + 14 = $080F = 2063 ... actually let me recalculate
// $0801 + 12 data bytes after header = $080D? Let me be precise.
// BASIC_STUB = [01 08] [0B 08] [0A 00] [9E] [32 30 36 31] [00] [00 00]
//               header  next   line#   SYS   "2061"       eol  end
// = 2 + 2 + 2 + 1 + 4 + 1 + 2 = 14 bytes
// Code starts at $0801 + 12 (excluding 2-byte header) = $080D = 2061 ✓

pub fn compile(source: &str, opts: &CompileOptions) -> CompileResult {
    let load_addr: u16 = if opts.basic_stub { 0x080D } else { 0x0801 };

    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let ast = parser.parse();
    let mut cg = Codegen::new(load_addr);
    let raw = cg.compile(&ast);
    let errors = cg.errors();

    let prg = if opts.basic_stub {
        let mut p = BASIC_STUB.to_vec();
        p.extend_from_slice(&raw);
        p
    } else {
        let mut p = vec![0x01, 0x08]; // PRG header
        p.extend_from_slice(&raw);
        p
    };

    CompileResult { prg, errors }
}
