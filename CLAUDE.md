# NUltimate Basic

A custom BASIC-like language compiler targeting the Commodore 64 and Commodore 64 Ultimate. Produces `.prg` files runnable in VICE or on real hardware.

## Build & Run

```bash
cargo build --release
cargo test
ultimate-basic build demo.ub -o demo.prg
ultimate-basic build demo.ub --d64 disk.d64
```

## Project Structure

```
src/
  lib.rs               – public API: compile()
  main.rs              – CLI entry point
  compiler/
    mod.rs             – compile() + CompileOptions + CompileResult
    lexer.rs           – tokeniser  (Lexer → Vec<Token>)
    parser.rs          – AST builder (Parser → Vec<Stmt>)
    ast.rs             – Expr, Stmt, BinOp, ColorTarget, VarType enums
    codegen.rs         – 6502 code generator (Codegen)
examples/
  features.ub          – original feature demo
  new_features.ub      – arrays, word vars, sub params, string vars demo
```

## Architecture

```
.ub source
  → Lexer::tokenize()  → Vec<Token>
  → Parser::parse()    → Vec<Stmt>
  → Codegen::compile() → Vec<u8>  (raw machine code)
  → mod.rs             → PRG = BASIC stub + machine code
```

### Two-Pass Compilation

1. **Pass 1** – every statement except `SubDef` → main program body
2. `RTS` — end of main program  
3. **Pass 2** – only `SubDef` statements → subroutines appended after main

Sub bodies are never executed at startup. Forward references (`Call` to an
unknown name, `Goto` to an unknown label) are recorded and patched by
`patch_forward_refs()` at the end.

### Pre-Scan

Before either pass, `pre_scan()` walks the AST to:
- Allocate zero-page slots for every subroutine's parameters
- Register arrays and assign their base addresses (`$C000+`)

### Zero Page Layout

| Range | Purpose |
|---|---|
| `$00–$01` | CPU I/O port — never touch |
| `$02–$4F` | **Permanent**: variables, loop counters, for-limit/step, sub params (`perm_zp`) |
| `$50–$7F` | **Scratch**: expression evaluation, reset before each statement (`tmp_zp`) |
| `$FB` | RNG seed (LCG) |
| `$FC–$FE` | Free for future use |

`tmp_zp` is reset to `$50` at the start of every statement in `gen_stmts()`.
This prevents zero-page overflow into the KERNAL area (`$7A–$7B` = BASIC
current-line pointer).

### PRG Format

```
[01 08]            – load address header ($0801)
[0B 08][0A 00]     – BASIC line link ($080B), line number 10
[9E]               – SYS token
[32 30 36 31]      – "2061" (= $080D decimal)
[00][00 00]        – end of BASIC program
<machine code>     – loaded at $080D
```

### Array Storage

Arrays (`var a = array(N)`) are allocated from `$C000` upward — free RAM on
the C64 with no ROM overlay when no cartridge is present.

---

## Language Reference

### Variables and Constants

```basic
var x = 10               # 8-bit integer (default)
var ptr: word = $0400    # 16-bit (two ZP bytes, lo/hi)
var msg = "HELLO"        # string (inferred from literal)
var s: string = "TEXT"   # string (explicit type)
var scores = array(10)   # byte array, 10 elements at $C000+
const SCREEN = $0400     # compile-time constant (substituted inline)
```

**Types**

| Type | Width | Notes |
|---|---|---|
| `int` | 8-bit | default for numeric vars |
| `word` | 16-bit | two ZP bytes; usable as address in `poke`/`peek` |
| `string` | pointer | ZP pair → null-terminated PETSCII in code segment |
| `array(N)` | N bytes | lives at `$C000+`, not ZP |

### Arithmetic & Bitwise

```basic
x = x + 1
y = a * b - c / 2
z = x and 15             # bitwise AND  ($25 AND zp)
w = a or b               # bitwise OR   ($05 ORA zp)
```

`and` / `or` are **bitwise** (not logical), consistent with C64 BASIC convention.

### Comparison

```basic
if x == 10 then
if x != 0 then
if x < 5 then
if x >= 20 then
```

Comparisons return 1 (true) or 0 (false) in the accumulator.

### Print

```basic
print "HELLO"
print x
print x, y, "text"            # any mix of vars, numbers, strings
print "Score: ", score, "!"
print                          # blank line (newline only)

# String concatenation with +
print "Hello " + "World"      # compile-time fold → single literal
print s1 + s2                 # runtime: prints s1 then s2 (no alloc)
print "Name: " + name         # literal + string var
print "Score: " + n           # string literal + numeric var
print n, " items" + " left"   # mixed — works in any order
```

String `+` in a **print context**:
- `StringLit + StringLit` → folded at **compile time** to one literal (zero extra code)
- Any operand containing a string var → both sides printed **sequentially** at runtime
- `num + num` → still performs **numeric addition** (prints the sum)

### Branching

```basic
if x == 1 then
  print "YES"
else
  print "NO"
end
```

### Loops

```basic
loop 5               # counted loop (5 iterations)
  print "HI"
end

loop                 # infinite loop
  x = x + 1
  if x == 100 then break end
end

for i = 1 to 10      # for..next (preferred syntax)
  print i
next

for i = 0 to 20 step 2
  print i
next i               # variable name after 'next' is optional

loop i = 1 to 10     # legacy syntax — still works, identical code
  print i
end

while x < 100
  x = x + 1
end
```

### Labels and Goto

```basic
label main_loop
  x = x + 1
  if x < 10 then goto main_loop end
```

Forward `goto` (label defined later) is supported via patch-on-emit.

### Subroutines

```basic
sub greet()
  print "HELLO!"
end

sub set_color(col)
  color border col
  color text   col
end

sub add(a, b)
  var result = a + b
  print result
end

call greet           # call keyword
greet()              # or bare name with parens
set_color(6)
add(10, 20)
```

Parameters are passed via dedicated zero-page slots (allocated in pre-scan).
Recursion is **not** supported (ZP slots are static).

### Arrays

```basic
var scores = array(8)    # allocates 8 bytes at $C000

scores[0] = 100          # constant index → STA $C000
scores[i] = 99           # variable index → STA (ptr),Y

var v = scores[0]        # constant index → LDA $C000
var v = scores[i]        # variable index → LDA (ptr),Y

print scores[2]          # inline in print
```

### 16-bit Variables (word)

```basic
var ptr: word = $0400    # stored as two ZP bytes (lo, hi)
var reg: word = $D020

poke reg, 6              # STA (reg),Y — full 16-bit address
var v = peek(reg)        # LDA (reg),Y
```

### String Variables

```basic
var msg = "PLAYER ONE"   # JMP over PETSCII data; ZP pair → string
var sep = "=========="

print msg                # prints via LDA (ptr),Y loop
print msg + sep          # sequential print (no heap alloc)
```

String data is stored inline in the code segment, preceded by a `JMP` to
skip over it. The ZP pair points to the start of the PETSCII bytes.

### Screen

```basic
cls                      # KERNAL CLS ($E544)
cls manual               # manual: fill screen RAM + color RAM + HOME
color text 14            # text color: $0286
color border 6           # border:     $D020
color bg 0               # background: $D021
graphics on              # VIC-II bitmap mode (320×200)
graphics off             # back to text mode
```

### Keyboard

```basic
var key = getch          # busy-loop on $FFE4 until keypress; returns ASCII
```

### Memory

```basic
poke $D020, 2            # STA $D020 (absolute)
poke addr_var, 6         # STA (addr_var),Y  if addr_var is word type
var v = peek($D012)      # LDA $D012
var v = peek(addr_var)   # LDA (addr_var),Y  if addr_var is word type
```

### Math Functions

```basic
var a = abs(x - 20)      # two's-complement absolute value
var b = min(x, 39)       # 8-bit minimum
var c = max(x, 0)        # 8-bit maximum
var s = sgn(score)       # 0 = zero, 1 = positive, $FF = negative
var r = rnd()            # LCG pseudo-random 0–255; seed from raster line
```

### Inline Assembly

```basic
sys $FFD2                # JSR $FFD2
asm $EA, $EA             # inline bytes (NOP NOP)
asm {
  $A9 $07                # LDA #7
  $8D $86 $02            # STA $0286
}
```

### String ↔ Integer

```basic
int_to_str score, $0340  # writes "042\0" to $0340 (always 3 digits)
var n = str_to_int("42") # compile-time: Expr::Number(42)
```

---

## C64 Memory Map (key addresses)

| Address | Description |
|---|---|
| `$0286` | Cursor / text colour |
| `$0400–$07E7` | Screen RAM (1000 chars) |
| `$D800–$DBE7` | Colour RAM |
| `$D011` | VIC-II control (BMM bit = bitmap mode) |
| `$D012` | Raster line counter |
| `$D018` | VIC-II memory layout register |
| `$D020` | Border colour |
| `$D021` | Background colour |
| `$C000–$CFFF` | Free RAM — used for arrays |
| `$E544` | KERNAL CLS |
| `$E566` | KERNAL HOME (cursor reset) |
| `$FFD2` | KERNAL CHROUT (output character) |
| `$FFE4` | KERNAL GETIN (read key, no wait) |
| `$FFF3` | KERNAL PLOT (get/set cursor position) |

---

## CLI Reference

```
ultimate-basic build <input.ub> [OPTIONS]

  -o, --output <file>   Output .prg file (default: <input>.prg)
  --no-stub             Skip the BASIC SYS stub (code loads at $0801)
  --d64 <file>          Also produce a .d64 disk image
  -h, --help            Show help
```

---

## Adding a New Feature — Quick Guide

1. **Token** — `lexer.rs`: add variant to `Token` enum + match arm in `read_ident()` / `tokenize()`
2. **AST** — `ast.rs`: add variant to `Expr` or `Stmt`
3. **Parser** — `parser.rs`: handle the new token in `parse_stmt()` or `parse_primary()`
4. **Codegen** — `codegen.rs`: implement in `gen_stmt()` or `eval_expr()`
5. **Tests** — each file has a `#[cfg(test)]` section; add unit tests there and integration tests in `tests/integration_tests.rs`

---

## Known Limitations

| Feature | Limitation |
|---|---|
| Integer arithmetic | 8-bit unsigned (0–255); `word` vars hold 16-bit values but arithmetic is 8-bit |
| 16-bit arithmetic | No carry propagation for `word + word`; use `poke`/`peek` patterns instead |
| Arrays | Byte arrays only; max total size ~4 KB (`$C000–$CFFF`) |
| Subroutines | No recursion — ZP parameter slots are statically allocated |
| String vars | Read-only after init; assignment replaces the pointer, not the data |
| String concat runtime | `s1 + s2` prints sequentially — no heap allocation or length tracking |
| `rnd()` | Simple LCG, not cryptographic; period = 256 |
| `abs()` / `sgn()` / `min()` / `max()` | 8-bit values only |
| Error reporting | Compile-time only; no runtime error handling |
