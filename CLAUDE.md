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
v = a xor b              # bitwise XOR  ($45 EOR zp)
m = x shl 3              # shift left 3 bits (unrolled ASL loop)
n = x shr 2              # shift right 2 bits (unrolled LSR loop)
```

`and` / `or` / `xor` / `shl` / `shr` are **bitwise**, consistent with C64 BASIC convention.
`not x` is **logical NOT** (0 → 1, non-zero → 0) — for bitwise complement use `x xor 255`.

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
cls fast                 # fast fill: screen RAM + color RAM + HOME
color text 14            # text color: $0286
color border 6           # border:     $D020
color bg 0               # background: $D021
graphics on              # VIC-II hires bitmap mode (320×200)
graphics on multi        # VIC-II multicolor bitmap mode (160×200, 4 colors/cell)
graphics off             # back to text mode
display on               # re-enable VIC display ($D011 bit4 = DEN → 1)
display off              # blank display  ($D011 bit4 = DEN → 0)
```

`graphics on` leaves display **blanked** (DEN=0). Call `display on` after `gcls` and drawing
to show the result without the initial bitmap-RAM flash.

### Keyboard

```basic
var key = getch()        # busy-loop on $FFE4 until keypress; returns ASCII
var j = joy(2)           # read joystick port 2 (CIA1 $DC00); returns inverted bits 0-4
var j = joy(1)           # read joystick port 1 (CIA1 $DC01)
                         # bit0=up(1), bit1=down(2), bit2=left(4), bit3=right(8), bit4=fire(16)
```

### Timing

```basic
wait 50                  # wait 50 raster-line transitions (~3.2 ms at 1 MHz)
wait raster 100          # spin until $D012 == 100 (raster line 100)
```

`wait N` counts N changes in `$D012`. Each change ≈ 1 raster line ≈ 64 cycles.
`wait raster N` busy-polls until `$D012 == N`; useful for raster-split effects.

### Exit

```basic
bye                      # JSR $E544 (KERNAL CLS), clear stop-key flag, RTS — clean return to BASIC
exit                     # alias for bye
```

### SID Sound

```basic
sound 0, $1CAD, 25       # voice 0, freq $1CAD (≈ middle C on PAL), 25 PAL frames
sound 1, freq_word, 50   # voice 1, freq from word var, 50 frames (1 s)
sound 2, 0, 0            # voice 2, silence (gate on/off immediately)
```

Syntax: `sound <channel>, <freq>, <duration>`

| Parameter  | Type   | Notes |
|---|---|---|
| `channel`  | const  | 0, 1, or 2 (compile-time only) |
| `freq`     | 16-bit | constant, `word` var, or 8-bit expr (hi=0) |
| `duration` | 8-bit  | PAL frames (1/50 s each); 0 = immediate gate off |

SID note frequencies (PAL, 985 248 Hz): freq = note_hz × 16.78. E.g. middle C (261.63 Hz) ≈ $1CAD.
Fixed ADSR: attack/decay = `$09`, sustain/release = `$F0`, waveform = sawtooth.
Master volume (`$D418`) is always set to `$0F`.

`bye` uses `JSR $E544` (direct KERNAL clear-screen) then `SEI; LDA #$FF; STA $91; CLI; RTS`.
Clearing `$91` prevents BASIC from printing "BREAK IN 10" if the user pressed RUN/STOP during
the program.

### Sprites

```basic
sprite 0, x, y, $2000    # sprite 0: set X, Y position and data pointer
sprite 0, x, y           # without data pointer (keeps existing)
sprite_on  0             # enable sprite 0 ($D015 |= bit0)
sprite_off 0             # disable sprite 0 ($D015 &= ~bit0)
sprite_color 0, 7        # sprite 0 color = yellow ($D027)
sprite_multicolor 0, on  # enable multicolor mode for sprite 0 ($D01C |= bit0)
sprite_multicolor 0, off # disable multicolor mode ($D01C &= ~bit0)
var h = sprite_hit()     # sprite–sprite collision ($D01E, cleared on read)
var b = sprite_bg_hit()  # sprite–background collision ($D01F, cleared on read)
```

| Concept | Notes |
|---|---|
| Sprite ID | compile-time constant 0–7 |
| X | 8-bit const or `word` var (9-bit: values 256–319 set $D010 MSB bit at runtime) |
| Y | 8-bit expression, 0–255 |
| `data_addr` | 64-byte-aligned address; stored as `addr>>6` at `$07F8+id` |
| Multicolor | shared colors in `$D025` / `$D026`; set individually via `poke` |

**VIC-II sprite registers:**

| Register | Description |
|---|---|
| `$D000+id×2` | Sprite X low byte |
| `$D001+id×2` | Sprite Y |
| `$D010` | Sprite X MSB (bit per sprite) |
| `$D015` | Sprite enable (bit per sprite) |
| `$D01C` | Sprite multicolor enable |
| `$D01E` | Sprite–sprite collision (read-clears) |
| `$D01F` | Sprite–background collision (read-clears) |
| `$D027+id` | Sprite color (0–15) |
| `$07F8+id` | Sprite data pointer (value = addr >> 6) |

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
var s = sin(angle)       # sine lookup: angle 0-255 (full circle), returns 0-255 (center=128)
var c = cos(angle)       # cosine = sin(angle+64); same scale
```

### Number Formatting

```basic
print hex(n)             # print n as 2 uppercase hex digits (e.g. 255 → "FF")
print bin(n)             # print n as 8-bit binary string  (e.g.  10 → "00001010")
print "val: ", hex(x)   # works in mixed print lists
```

`hex(n)` and `bin(n)` are print-context functions; in expression context they evaluate to `n` unchanged.

### REU (RAM Expansion Unit)

```basic
var ok = reu_present()               # 1 if REU detected, 0 if not

reu stash $4000, 0, $0000, 16384  # copy 16 KB from C64:$4000  → REU bank 0:$0000
reu fetch $4000, 0, $0000, 16384  # copy 16 KB from REU bank 0:$0000 → C64:$4000
reu swap  $4000, 0, $0000, 256    # swap 256 bytes between C64 and REU
```

`reu_present()` is a built-in expression (like `getch()`). It performs a write/read-back test
on `$DF04` (REU C64 base-address-lo register) using `$55` then `$AA`. Returns 1 if both
read-backs match (REU present), 0 otherwise. No ZP scratch, 31 bytes inline 6502.

Syntax: `reu <op> c64_addr, reu_bank, reu_offset, length`

| Parameter | Width | Description |
|---|---|---|
| `c64_addr` | 16-bit | C64 RAM start (constant, `word` var, or 8-bit expr) |
| `reu_bank` | 8-bit | REU bank number (0–7 for 512 KB unit) |
| `reu_offset` | 16-bit | Offset within REU bank |
| `length` | 16-bit | Bytes to transfer (0 = 65536 in REU) |

REU register mapping: $DF02/$DF03 = C64 addr, $DF04/$DF05 = REU offset, $DF06 = bank,
$DF07/$DF08 = length, $DF01 = command ($B0=stash, $B1=fetch, $B2=swap).

Requires a real REU or emulated REU (VICE: enable Georam/REU). The transfer is synchronous
(CPU halted during DMA).

### Comments

```basic
# hash comment (existing)
rem this is also a comment
var x = 5  ; inline comment
```

`rem` and `;` are treated identically to `#` — everything to end of line is ignored.

### Compile-time File Embedding

```basic
incbin "sprites.bin"     # embed raw binary bytes at current code position
include "defs.ub"        # inline another .ub source file (lexed+parsed in place)
```

`incbin` embeds the file's bytes verbatim. Useful for sprite data, character sets, music.
Paths are relative to the current working directory.

`include` inlines the parsed statements of another source file. Constants defined in the
included file are visible after the include point.

### Data / Read

```basic
data 1, 2, 3, 255        # constant byte table (compiled inline, after all code)
read varname             # load next byte from table into varname (auto-declares if needed)
```

All `data` values are collected at compile time into a single block. A 2-byte ZP data
pointer is automatically allocated and initialized at program start. Each `read` advances
the pointer. Values must be byte-sized constants (0–255).

### Bitmap Graphics

```basic
graphics on              # VIC-II hires bitmap mode (320×200), bitmap at $2000
graphics on multi        # VIC-II multicolor bitmap mode (160×200, 4 colors per 8×8 cell)
graphics off             # back to text mode
gcls                     # clear bitmap (zero-fill $2000-$3FFF)
plot x, y                # set pixel at (x, y);  x: 0-319, y: 0-199
circle x, y, r           # midpoint circle centered at (x, y) with radius r; clips off-screen points
line x1, y1, x2, y2      # Bresenham line from (x1,y1) to (x2,y2); x: 0-255, y: 0-199
```

Both `graphics on` variants blank the VIC display during setup ($D011 DEN bit) to prevent
mode-switch glitches, then re-enable it in the new mode.

**Hires (standard) bitmap**: each pixel is 0 or 1; foreground/background per 8×8 cell from color RAM.
**Multicolor bitmap**: each pixel is 2 bits → 4 colors per 8×8 cell (effective 160×200 resolution).

`gcls` should be called after `graphics on` to start with a blank screen.
`plot` emits a compact helper subroutine once per program (all `plot` calls share it via `JSR`).

X supports the full 320-pixel width. For x ≤ 255 the high byte is 0; for x = 256–319 it is 1, which the helper adds as an extra +256 to the byte address. `word` variables work directly as x.

Pixel byte formula: `$2000 + (y>>3)*320 + (x and $1F8) + (y and 7)`,  bit: `$80 >> (x and 7)`

### chr$

```basic
print chr$(65)           # output PETSCII character 65 = 'A'
print chr$(13)           # output carriage return ($0D)
var c = chr$(n)          # store PETSCII code n in variable c (same as n)
print ">" + chr$(42)     # works in string concat context
```

`chr$(n)` is the character-by-code function (like C64 BASIC `CHR$`). In print context it calls CHROUT directly. In other contexts it evaluates to the raw byte value of n.

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
  -v, --verbose         Show full ZP layout + code hex dump after build
  --no-stub             Skip the BASIC SYS stub (code loads at $0801)
  --d64 <file>          Also produce a .d64 disk image
  -h, --help            Show help
```

A memory map is always printed on successful build (variables, subroutines, arrays,
load address). `--verbose` additionally shows the internal ZP allocation (plot helper,
data pointer) and a full hex dump of the generated machine code.

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
| `plot` | No erase/XOR mode — only pixel-set (OR); no bounds checking |
| `plot` | Plotting outside 0–319 × 0–199 corrupts adjacent memory |
| `chr$` | No PETSCII↔ASCII mapping — n is passed as-is to CHROUT |
| Error reporting | Compile-time only; no runtime error handling |
