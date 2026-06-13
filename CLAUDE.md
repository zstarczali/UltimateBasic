# NUltimate Basic

A custom BASIC-like language compiler targeting the Commodore 64 and Commodore 64 Ultimate. Produces `.prg` files runnable in VICE or on real hardware.

## Build & Run

```bash
cargo build --release
cargo test
ub build demo.ub -o demo.prg
ub build demo.ub --d64 disk.d64
ub build demo.ub --d64          # auto: demo.d64
ub build demo.ub --d64 disk.d64 --add music.prg --add loader.prg
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
  bitmap_demo.ub       – 320×200 bitmap, plot, circle, line
  block_demo.ub        – 80×50 block graphics, plot4, circle4, graphics on block
  joystick_demo.ub     – joystick reading, sprite movement
  mux_demo.ub          – raster sprite multiplexer (3 windows × 8 sprites)
  orbit_demo.ub        – 24-sprite orbit with pulsating radius
  plasma_demo.ub       – plasma-effect bitmap with raster bar animation
  sprite_data.ub       – sprdef shape data (included by other demos)
  sprite_mux_orbit.ub  – 24-sprite orbit with sprdef + precomputed positions
  sprite_orbit_demo.ub – 8 hardware sprites in circular orbit via sin/cos
  reu_bitmap_demo.ub   – REU stash/fetch with bitmap graphics
  tenprint.ub          – 5 TENPRINT maze implementations with menu; demos lowercase charset mode
  text_scroll_demo.ub – hardware horizontal fine scroll text scroller
  fn_demo.ub          – text scroller rewritten with fn + typed string params
  function_demo.ub    – fn return value demo (square, add, max, clamp)
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

1. **Pass 1** – every statement except `SubDef` / `FnDef` → main program body
2. `RTS` — end of main program  
3. **Pass 2** – only `SubDef` / `FnDef` statements → subroutines/functions appended after main

Sub bodies are never executed at startup. Forward references (`Call` to an
unknown name, `Goto` to an unknown label) are recorded and patched by
`patch_forward_refs()` at the end.

### Pre-Scan

Before either pass, `pre_scan()` walks the AST to:
- Allocate zero-page slots for every subroutine's and function's parameters
- Register arrays and assign their base addresses (`$C000+`)
- Allocate a 2-byte ZP return-value slot (`fn_ret_zp`) if any `fn` has a `: word` or `: float` return type

### Zero Page Layout

| Range | Purpose |
|---|---|
| `$00–$01` | CPU I/O port — never touch |
| `$02–$4F` | **Permanent**: variables, loop counters, for-limit/step, sub params (`perm_zp`) |
| `$50–$7F` | **Scratch**: expression evaluation, reset before each statement (`tmp_zp`) |
| `$FB` | RNG seed (LCG) |
| `$FC–$FD` | `fn_ret_zp` (2 bytes, if word/float fn present), else free |
| `$FE` | free |

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
var f: float = 3.5       # Q8.8 fixed-point (hi=integer, lo=fraction)
var msg = "HELLO"        # string (inferred from literal)
var s: string = "TEXT"   # string (explicit type)
var scores = array(10)   # byte array, 10 elements at $C000+
var times  = array_word(8) # word array, 8 word elements at $C000+
const SCRADDR = $0400    # compile-time constant (substituted inline)
```

All keywords and identifiers are **case-insensitive**. The lexer lowercases everything;
constant names that match a language keyword (e.g. `SCREEN`, `BORDER`) will be tokenised
as that keyword — use non-keyword names like `SCRADDR`, `BORDER_ADDR`.

**Types**

| Type | Width | Notes |
|---|---|---|
| `int` | 8-bit | default for numeric vars |
| `word` | 16-bit | two ZP bytes; usable as address in `poke`/`peek` |
| `float` | 16-bit Q8.8 | hi byte = integer part (0–255), lo byte = fraction |
| `string` | pointer | ZP pair → null-terminated PETSCII in code segment |
| `array(N)` | N bytes | byte elements; lives at `$C000+`, not ZP |
| `array_word(N)` | N×2 bytes | word (16-bit) elements; lives at `$C000+`, not ZP |

### Arithmetic & Bitwise

```basic
x = x + 1
y = a * b - c / 2
r = x mod 3              # 8-bit modulo (remainder); result 0–(divisor-1)
z = x and 15             # bitwise AND  ($25 AND zp)
w = a or b               # bitwise OR   ($05 ORA zp)
v = a xor b              # bitwise XOR  ($45 EOR zp)
m = x shl 3              # shift left 3 bits (unrolled ASL loop)
n = x shr 2              # shift right 2 bits (unrolled LSR loop)
```

`and` / `or` / `xor` / `shl` / `shr` are **bitwise**, consistent with C64 BASIC convention.
`not x` is **logical NOT** (0 → 1, non-zero → 0) — for bitwise complement use `x xor 255`.
`mod` implements 8-bit unsigned remainder via an SEC/SBC/BCS loop followed by CLC/ADC restore.

### Increment / Decrement

```basic
inc x                    # x = x + 1  (INC zp — single 6502 instruction)
dec x                    # x = x - 1  (DEC zp — single 6502 instruction)
```

For `word` variables, 16-bit carry is handled automatically:
- `inc`: `INC lo; BNE skip; INC hi` — wraps correctly through 0→1 in the high byte
- `dec`: `LDA lo; BNE skip; DEC hi; DEC lo` — borrows correctly from the high byte

### Compound Assignments

```basic
x += 5                   # x = x + 5
x -= 3                   # x = x - 3
x *= 2                   # x = x * 2
x /= 4                   # x = x / 4
x and= 15                # x = x and 15   (bitwise AND)
x or= 64                 # x = x or 64    (bitwise OR)
x xor= 255               # x = x xor 255  (bitwise XOR)
x shl= 2                 # x = x shl 2    (shift left)
x shr= 1                 # x = x shr 1    (shift right)
```

All compound assignments generate the same code as the expanded form. They are syntax sugar only; any valid expression is accepted on the right side.

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

# Spacing / cursor control in print
print spc(5)                  # print 5 space characters
print tab(20), "VALUE"        # move cursor to column 20, then print
print "A", spc(3), "B"        # mix freely with other print args

# String concatenation with +
print "Hello " + "World"      # compile-time fold → single literal
print s1 + s2                 # runtime: prints s1 then s2 (no alloc)
print "Name: " + name         # literal + string var
print "Score: " + n           # string literal + numeric var
print n, " items" + " left"   # mixed — works in any order
```

`spc(n)` emits n space characters (`$20`) via CHROUT. If n = 0, nothing is printed.
`tab(n)` moves the cursor to absolute column n using KERNAL PLOT (`$FFF0`); column n is 0-based (0–39).

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

### Select / Case

```basic
select x
  case 1:
    print "ONE"
  case 2:
    print "TWO"
  else:
    print "OTHER"
end
```

`select expr` evaluates the expression once and compares it against each `case` value in order. The first matching case body is executed and control jumps to after `end` (subsequent cases are skipped). The optional `else:` body runs if no case matches. Any number of `case` arms is supported; `else:` must come last. All values must be 8-bit (0–255).

### Loops

```basic
loop 5               # counted loop (5 iterations)
  print "HI"
end

loop                 # infinite loop
  x = x + 1
  if x == 5 then continue end  # skip to next iteration
  if x == 100 then break end
end

for i = 1 to 10      # for..next (preferred syntax)
  if i == 5 then continue end  # skip to increment step
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

repeat               # do-while: body executes at least once
  x = x + 1
until x == 100       # loop back if condition is false, exit when true
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

Typed parameters are preserved end-to-end:
```basic
sub copy(src:string)    # src gets 2-byte ZP pointer pair
  var c = src[i]        # string indexing → LDA (ptr),Y
end
```

### Functions (fn)

```basic
fn square(x)
  return x * x
end

fn add(a, b)
  return a + b
end

fn clamp(val, lo, hi)
  if val < lo then return lo end
  if val > hi then return hi end
  return val
end

var s = square(9)         # fn call as expression, result in A
print add(10, 20)         # usable inline in print
var c: word = get_ptr()   # word return type supported
```

Functions support optional `: word` or `: float` return type annotation. 16-bit
return values are stored in a dedicated ZP pair (`fn_ret_zp`, allocated in pre-scan)
before RTS. The caller reads from that pair after JSR.

`fn` is emitted in pass 2 (same as `sub`), so function bodies are never executed
at startup. Forward references are fully supported.

### Arrays

```basic
var scores = array(8)    # allocates 8 bytes at $C000

scores[0] = 100          # constant index → STA $C000
scores[i] = 99           # variable index → STA (ptr),Y

var v = scores[0]        # constant index → LDA $C000
var v = scores[i]        # variable index → LDA (ptr),Y

print scores[2]          # inline in print

var times = array_word(8)  # allocates 16 bytes (8×2) at $C000+

times[0] = $1234         # constant index → STA $C000 (lo), STA $C001 (hi)
times[i] = $1234         # variable index → ASL A for stride; (ptr),Y × 2

var t: word = times[0]   # constant index → LDA $C000, LDA $C001
var t: word = times[i]   # variable index → ASL A; LDA (ptr),Y × 2
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

lowercase                # CHR$(14) → switch VIC-II to lowercase/uppercase charset
uppercase                # CHR$(142) → switch VIC-II back to uppercase/graphics charset

scroll x 3               # horizontal fine scroll: $D016 bits 0-2 = 3 (range 0-7)
scroll y 2               # vertical fine scroll:   $D011 bits 0-2 = 2 (range 0-7)
scroll x n               # value can be a variable or expression (masked to bits 0-2)
scroll x 7 narrow        # set fine scroll and force 38-column mode (hide edge column)
scroll x 0 wide          # set fine scroll and restore 40-column mode
scroll row 12 left       # shift screen RAM row 12 left by one character
```

`lowercase` emits `LDA #$0E; JSR $FFD2` (KERNAL CHROUT) at runtime to activate the
lowercase/uppercase charset. String literals after `lowercase` are automatically
encoded with swapped case so that source `"Hello World"` displays as **Hello World**
on screen: uppercase source chars are stored as `$41+0x20` (→ PETSCII lowercase slot)
and lowercase source chars as `$61−0x20` (→ PETSCII uppercase slot).
`uppercase` emits `LDA #$8E; JSR $FFD2` and restores normal uppercase/graphics charset
(source case-encoding reverts to direct mapping). `cls` does **not** reset the charset mode.

`scroll x n` computes `(n AND 7)` and writes it into bits 0-2 of `$D016` (preserving bits 3-7).
`scroll x n narrow` writes the fine-scroll bits and clears `$D016` bit 3 (38-column mode).
`scroll x n wide` writes the fine-scroll bits and sets `$D016` bit 3 (40-column mode).
`scroll y n` computes `(n AND 7)` and writes it into bits 0-2 of `$D011` (preserving bits 3-7).
`scroll row R left` shifts one constant screen row left; write the new rightmost character with `screen 39, R, ch`.
Useful for smooth hardware scrolling: decrement each frame from 7 to 0, then shift screen RAM and reset to 7.

### Ultimate 64 — CPU Speed

```basic
speed 4              # set U64 CPU to 4 MHz (RMW $D031 bits 0-3)
speed 48             # 48 MHz  (max U64; 64 MHz on Elite-II)
speed max            # alias: index 15 (fastest available)
speed off            # alias: index 0  (1 MHz — back to normal)

badlines on          # enable badline timing  ($D031 bit 7 = 0)
badlines off         # disable badline timing ($D031 bit 7 = 1)

var t = turbo()      # 1 if turbo active (bits 0-3 of $D031 != 0), 0 if at 1 MHz
```

Register `$D031` (U64 Turbo Control):
- bits 0-3: speed index (0=1MHz … 15=48MHz on U64, 15=64MHz on Elite-II)
- bit 7: badlines timing (0=enabled / 1=disabled)
- bits 4-6: unused — preserved by RMW

Available MHz values and their indices:
`1(0), 2(1), 3(2), 4(3), 5(4), 6(5), 8(6), 10(7), 12(8), 14(9), 16(10), 20(11), 24(12), 32(13), 40(14), 48(15)`

For constant MHz values: compile-time floor-lookup (`speed 7` → index 5 = 6 MHz).
For variable values: raw index (0–15) OR'd into bits 0-3 via read-modify-write.

Requires U64 Turbo Control mode set to `U64 Turbo Registers` or `Turbo Enable Bit` in the U64 config menu.
On a plain C64, writes to `$D031` are harmless (open bus / ignored).

`graphics on` and `graphics on multi` leave the display **blanked** (DEN=0). Call `display on`
screen 10, 5, ch         # col 10, row 5 — col/row can be variables
screen 5, 3, 42, 7       # char 42 at col 5, row 3, color 7 (also writes to color RAM $D800)
screen x, y, ch, col     # all four arguments as variables
```

`screen col, row, char [, color]` writes directly to screen RAM (`$0400 + row*40 + col`) and
optionally to color RAM (`$D800 + row*40 + col`). For constant col/row the address is computed
at compile time (a single `STA abs`); for variable col/row the address is computed at runtime.

`graphics on` and `graphics on multi` leave the display **blanked** (DEN=0). Call `display on`
after `gcls` and drawing to show the result without the initial bitmap-RAM flash.

### Keyboard

```basic
var key = getch()        # busy-loop on $FFE4 until keypress; returns PETSCII code
var k   = inkey()        # non-blocking $FFE4: returns PETSCII code, or 0 if no key pressed
var j = joy(2)           # read joystick port 2 (CIA1 $DC00); returns inverted bits 0-4
var j = joy(1)           # read joystick port 1 (CIA1 $DC01)
                         # bit0=up(1), bit1=down(2), bit2=left(4), bit3=right(8), bit4=fire(16)
var mx = mouse_x()       # 1351 mouse X position (SID POT X, $D419); 0-255
var my = mouse_y()       # 1351 mouse Y position (SID POT Y, $D41A); 0-255
var mb = mouse_btn()     # mouse buttons: bit0=left (fire, $DC01 bit4), bit1=right (up-pin, $DC01 bit0)
```

`getch()` busy-loops until a key is pressed. `inkey()` returns immediately with 0 if no key is available — use it in game loops.

### Timing

```basic
wait 50                  # wait 50 raster-line transitions (~3.2 ms at 1 MHz)
wait raster 100          # spin until $D012 == 100 (raster line 100)
delay 1                  # wait 1 PAL frame (1/50 s); delay 20 ≈ 0.4 s
delay n                  # n can be a variable or expression (8-bit, 0–255)
```

`wait N` counts N changes in `$D012`. Each change ≈ 1 raster line ≈ 64 cycles.
`wait raster N` busy-polls until `$D012 == N`; useful for raster-split effects.
`delay N` waits exactly N full PAL frames by watching raster line 200 as the frame boundary — each frame is ≈ 20 ms (50 Hz PAL). Uses 1 scratch ZP byte and a 14-byte inline loop.

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

sid volume 15            # master volume full ($D418 = $0F); range 0-15
sid volume 0             # silence (master volume = 0)
sid stop                 # zero all 25 SID registers ($D400-$D418) — complete silence
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

`sid volume N` writes N directly to `$D418`. Bits 0-3 = volume (0-15), bits 4-7 = filter mode.
`sid stop` emits a 10-byte zero-fill loop (`LDX #24; LDA #0; STA $D400,X; DEX; BPL`) — faster than 25 individual pokes.

### Music Playback

High-level `music play/stop/pause/resume` commands built on top of `load sid` and CIA1 timer A.

```basic
load sid "tune.sid"           # embed SID file; defines sid_init / sid_play

music play                    # init sub-tune 0, start CIA1 timer A IRQ at 50 Hz PAL
music play 1                  # start from sub-tune 1 (0-based song index)
music stop                    # disable CIA1 IRQ + zero all 25 SID registers
music pause                   # disable CIA1 timer A IRQ (SID output frozen, not zeroed)
music resume                  # re-enable CIA1 timer A IRQ (resume from pause point)
```

| Statement | Code emitted |
|---|---|
| `music play [n]` | `LDA #n; JSR sid_init`; CIA1 timer A setup at 19 656 cycles; `$0314`/`$0315` → wrapper |
| `music stop` | `SEI; LDA #$7F; STA $DC0D; CLI` + SID zero-fill loop (`$D400–$D418`) |
| `music pause` | `SEI; LDA #$7F; STA $DC0D; CLI` |
| `music resume` | `SEI; LDA #$81; STA $DC0D; CLI` |

`music play` generates a shared IRQ **wrapper** (emitted once in post-code):
```asm
  LDA #$01       ; ACK CIA1 timer A IRQ ($DC0D)
  STA $DC0D
  JSR sid_play   ; advance one frame of music
  JMP $EA81      ; irq_exit: restore A/X/Y + RTI
```

Requires a prior `load sid` statement (to define `sid_init` / `sid_play`). Calling `music play` without `load sid` emits a JSR to address 0.

`bye` uses `JSR $E544` (direct KERNAL clear-screen) then `SEI; LDA #$FF; STA $91; CLI; RTS`.
Clearing `$91` prevents BASIC from printing "BREAK IN 10" if the user pressed RUN/STOP during
the program.

### Sprites

```basic
sprite 0, x, y, $2000    # sprite 0: set X, Y position and data pointer
sprite 0, x, y           # without data pointer (keeps existing)
sprite on  0             # enable sprite 0 ($D015 |= bit0)
sprite off 0             # disable sprite 0 ($D015 &= ~bit0)
sprite color 0, 7        # sprite 0 color = yellow ($D027)
sprite multicolor 0, on  # enable multicolor mode for sprite 0 ($D01C |= bit0)
sprite multicolor 0, off # disable multicolor mode ($D01C &= ~bit0)
sprite expand x 0, on    # double width for sprite 0 ($D01D |= bit0)
sprite expand x 0, off   # normal width ($D01D &= ~bit0)
sprite expand y 0, on    # double height for sprite 0 ($D017 |= bit0)
sprite expand y 0, off   # normal height ($D017 &= ~bit0)
sprite priority 0, on    # sprite behind background ($D01B |= bit0)
sprite priority 0, off   # sprite in front of background ($D01B &= ~bit0)
var h = sprite_hit()     # sprite–sprite collision ($D01E, cleared on read)
var b = sprite_bg_hit()  # sprite–background collision ($D01F, cleared on read)

sprdef 0                 # inline sprite data: 63 bytes, 64-byte aligned, sets $07F8+id
  %00111100,0            # 3 bytes per row × 21 rows = 63 bytes total
  ...
end
```

`sprdef id ... end` embeds 63 sprite bytes at the next 64-byte-aligned address in the code segment (preceded by a `JMP` to skip over it), then writes `addr>>6` to `$07F8+id` at runtime. Fewer than 63 bytes are zero-padded. Values must be compile-time constants; use `%` prefix for binary literals.

| Concept | Notes |
|---|---|
| Sprite ID | compile-time constant 0–7 |
| X | 8-bit const or `word` var (9-bit: values 256–319 set $D010 MSB bit at runtime) |
| Y | 8-bit expression, 0–255 |
| `data_addr` | 64-byte-aligned address; stored as `addr>>6` at `$07F8+id` |
| Multicolor | shared colors in `$D025` / `$D026`; set individually via `poke` |
| Expand | doubles sprite size in X (`$D01D`) or Y (`$D017`) direction |
| Priority | `on` = sprite behind background, `off` = sprite in front |

**VIC-II sprite registers:**

| Register | Description |
|---|---|
| `$D000+id×2` | Sprite X low byte |
| `$D001+id×2` | Sprite Y |
| `$D010` | Sprite X MSB (bit per sprite) |
| `$D015` | Sprite enable (bit per sprite) |
| `$D017` | Sprite expand Y (bit per sprite) |
| `$D01B` | Sprite priority / bg collision (bit per sprite) |
| `$D01C` | Sprite multicolor enable |
| `$D01D` | Sprite expand X (bit per sprite) |
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

var w: word = peek16($C000)   # read 16-bit little-endian: lo=$C000, hi=$C001
poke16 $0314, $EA81           # write 16-bit little-endian: lo→$0314, hi→$0315
poke16 ptr, w                 # word var as address; word var as value
```

`peek16(addr)` reads two consecutive bytes (lo, hi) and returns them as a 16-bit `word`. In an 8-bit context (e.g. assigned to an `int` var) only the low byte is returned.
`poke16 addr, val` writes two bytes: lo(val) → addr, hi(val) → addr+1. Both `addr` and `val` may be constants, `word` variables, or expressions.

### String Functions

```basic
var n = len(msg)         # length of null-terminated string var (0–255); byte-count loop
var c = asc(msg)         # PETSCII code of first character (0 if empty string)
var c = asc("A")         # compile-time: returns constant PETSCII code
var n = val(s)           # runtime: parse decimal PETSCII string → 8-bit int ("042" → 42)
var c = msg[i]           # string index: PETSCII code of character at index i (LDA (ptr),Y)
```

`len(s)` walks the string until it finds a `$00` byte and returns the count in A.
`asc(s)` loads the first byte of the string via `(ptr),Y` with Y=0. Both accept string literals (compile-time constant) or string variables (runtime).
`val(s)` iterates the null-terminated string accumulating `result = result*10 + digit` for each `'0'`–`'9'` PETSCII byte; stops at null or non-digit. Returns 8-bit result.
`s[i]` for a string variable loads the byte at index `i`: constant index emits `LDY #i; LDA (ptr),Y`; variable index evaluates `i` into A then `TAY; LDA (ptr),Y`.

### Float / Fixed-Point (Q8.8)

```basic
var f: float = 3.5       # hi=3, lo=128 (= 0x0380); prints as "3.50"
var g: float = 0         # integer literal promoted to 0.0 (hi=0, lo=0)

f = 1.5                  # literal Q8.8 assignment
f = f + 1.5              # 16-bit Q8.8 addition

var n = int(f)           # extract integer part (hi byte) → 8-bit int
print f                  # prints as "N.DD" (always 2 fractional digits)
```

- Q8.8 format: `hi_byte = integer_part`, `lo_byte = frac * 256` (rounded)
- Integer assignment (`f = 5`) stores `hi=5, lo=0` (= 5.0) automatically
- `int(f)` emits `LDA zp+1` (hi byte)
- `print f` calls `print_fixed(zp)`: prints hi via `print_decimal`, then `.`, then `(lo*100)>>8` as 2-digit zero-padded decimal via Russian Peasant multiply
- Arithmetic uses the same 16-bit path as `word` vars (`eval_expr_word` / `gen_word_assign`)
- No float multiplication or division between two float vars (not implemented)

### Math Functions

```basic
var a = abs(x - 20)      # two's-complement absolute value
var b = min(x, 39)       # 8-bit minimum
var c = max(x, 0)        # 8-bit maximum
var s = sgn(score)       # 0 = zero, 1 = positive (1–127), $FF = negative (128–255)
var r = rnd()            # LCG pseudo-random 0–255; seed from raster line
var r = rnd(10)          # LCG pseudo-random 0–9 (rnd() mod n; result 0..n-1)
var s = sin(angle)       # sine lookup: angle 0-255 (full circle), returns 0-255 (center=128)
var c = cos(angle)       # cosine = sin(angle+64); same scale
```

### Number Formatting

```basic
print hex(n)             # print n as 2 uppercase hex digits (e.g. 255 → "FF")
print bin(n)             # print n as 8-bit binary string  (e.g.  10 → "00001010")
print "val: ", hex(x)   # works in mixed print lists
print dec(n, 4)          # right-justified decimal in a field of 4 chars (e.g. 42 → "  42")
print dec(n, width)      # width can be a variable or expression
```

`hex(n)` and `bin(n)` are print-context functions; in expression context they evaluate to `n` unchanged.
`dec(n, width)` pads with spaces on the left to fill `width` characters. If the number has more digits than `width`, it prints without padding (no truncation). In expression context it evaluates to `n` unchanged.

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
var x = 5  # inline comment
poke $D020, 6 : color border 6  # colon separates two statements on one line
```

`rem` and `#` are comments — everything to end of line is ignored.
`:` separates multiple statements on one line (like C64 BASIC).

### Compile-time File Embedding

```basic
incbin "sprites.bin"     # embed raw binary bytes at current code position
include "defs.ub"        # inline another .ub source file (lexed+parsed in place)
```

### Disk I/O (runtime)

```basic
load "PROGRAM"           # KERNAL LOAD from device 8, to file's own load address
load "DATA", $C000       # load to specific address (secondary address = 1)
load "DATA", ptr         # address from word variable

save "DATA", $C000, 4096 # KERNAL SAVE from $C000, 4096 bytes → device 8
save "PROG", start, len  # addr and len from word/int variables
```

Calls `SETNAM` ($FFBD) + `SETLFS` ($FFBA, device 8) + `LOAD` ($FFD5) or `SAVE` ($FFD8).
`save` requires both `addr` and `len`. The `addr` is stored in a scratch ZP pair; KERNAL SAVE receives that ZP address in A, end address (addr+len) in X/Y.

### SID Music

```basic
load sid "tune.sid"            # embed SID music at its native load address
load sid "tune.sid", $2000     # override: embed at $2000 regardless of SID header
```

`load sid` reads a PSID or RSID file at **compile time**, strips the header, and appends the raw music bytes to the output `.prg` at the specified load address (padded with zeros if necessary). After `load sid`, two compile-time constants are automatically defined:

| Constant   | Value | Description |
|---|---|---|
| `sid_init` | init address from SID header | Call once to initialise the tune (A = song number, 0-based) |
| `sid_play` | play address from SID header | Call every frame (50 Hz PAL) to advance playback |

Both constants can be used anywhere a constant address is accepted: `sys`, `irq`, `poke`, expressions, etc.

**Typical usage with a raster IRQ:**

```basic
load sid "music.sid"          # embeds tune, defines sid_init / sid_play

sub music_irq()
  poke $D019, $FF             # ACK VIC raster IRQ
  sys sid_play                # advance one frame of playback
  irq_exit                    # JMP $EA81: restore A/X/Y + RTI (proper IRQ exit)
end

sys sid_init, 0               # initialise SID chip: A=0 → first sub-tune
irq music_irq, $C0            # fire the IRQ at raster line $C0 (50 Hz)

poke $D418, $0F               # master volume on
```

**Notes:**
- The load address from the SID header is used by default; the optional `, addr` overrides it.
- `sid_init` / `sid_play` are substituted at parse time, so they work in `sys`, `irq`, `asm { LDA #<sid_play }`, etc.
- The SID data is placed **after** all generated code, padded with zeros from the code end to the SID load address. The compiler aborts (with a warning) if the SID load address would overlap generated code.
- PSID v1 (`data_offset = $76`) and PSID v2 (`data_offset = $7C`) are both supported. If the SID header's load address field is 0, the load address is taken from the first two bytes of the data section (PRG-style, little-endian).
- Only one `load sid` per program is meaningful (the last one wins if multiple are present).



```basic
open 1, 8, 2, "MYFILE"  # open logical file 1, device 8, secondary 2, name "MYFILE"
open 2, 4, 7             # open printer (device 4), no filename
open ch, dev, sec        # channel, device, secondary from variables

print# 1, "HELLO"        # send "HELLO"+CR to logical file 1
print# ch, x, "text"     # any mix of vars, strings — same as print but to file

close 1                  # close logical file 1
close ch                 # channel from variable
```

`open` calls `SETNAM` ($FFBD) + `SETLFS` ($FFBA) + `OPEN` ($FFC0). Without a filename, SETNAM is called with length 0.
`print#` routes output to the given logical file via `CHKOUT` ($FFC9), then calls `CHROUT` for each character (and a trailing CR), then restores output via `CLRCHN` ($FFCC).
`close` puts the channel number in A and calls `CLOSE` ($FFC3).

| KERNAL | Address | Description |
|---|---|---|
| `SETNAM` | `$FFBD` | Set filename (A=len, X/Y=ptr) |
| `SETLFS` | `$FFBA` | Set logical/physical/secondary (A/X/Y) |
| `OPEN`   | `$FFC0` | Open logical file |
| `CLOSE`  | `$FFC3` | Close logical file (A=channel) |
| `CHKOUT` | `$FFC9` | Direct output to channel (X=channel) |
| `CLRCHN` | `$FFCC` | Restore default I/O channels |

### Cursor Positioning

```basic
cursor 20, 10            # move cursor to column 20, row 10 (KERNAL PLOT $FFF0)
cursor x, y              # column from variable x (0-39), row from y (0-24)
```

Calls KERNAL PLOT (`$FFF0`) with carry **clear** (C=0 = SET position; C=1 = READ position).
PLOT expects X = row, Y = column. `cursor col, row` maps: col → Y register, row → X register.

`print at col, row` combines cursor positioning and printing in one statement:

```basic
print at 20, 10, "HELLO"          # move to col 20, row 10, then print
print at x, y, "Score:", score    # any mix of exprs — same as print, but positioned
print at 0, 0                     # move cursor only (no text, no newline)
```

`print at` does **not** emit a trailing newline — the cursor is left at the end of the printed text.
This avoids accidental screen scroll when printing at row 24 (the last visible row).
`print col, row, "text"` without `at` still prints col and row **as values** (existing behaviour).

### Input

```basic
input score              # read up to 3 digits from keyboard → 8-bit int var
input "Name: ", name     # optional prompt string, then read line → string var
input "Score: ", score   # prompt + int input
```

`input` uses KERNAL BASIN (`$FFCF`) for blocking, echoed line input with DEL support.
- **Int var**: accepts only `0`–`9`, max 3 chars; converts digit string → 8-bit value on CR.
- **String var**: accepts up to 30 chars; stores as null-terminated string in inline buffer; pointer stored in the string var's ZP pair.

### Memory Utilities

```basic
fill $0400, 1000, 32     # fill 1000 bytes starting at $0400 with value 32
fill addr, 256, 0        # addr can be word var; len 256 = exactly one full page
fill ptr, len_word, val  # all operands can be expressions / word vars

memcopy $C000, $0400, 256   # copy 256 bytes from $C000 → $0400
memcopy src_ptr, dst_ptr, 40 # word vars for source and destination

drawmem $C000, $0400, 8, 10, 40 # blit 8×10 rect from $C000 → screen at $0400, stride 40
drawmem src_ptr, dst_ptr, w, h, 40 # word vars for src/dst
```

Both `fill` and `memcopy` support 16-bit lengths (0–65535). When `len` is a numeric literal,
its high byte = page count, low byte = partial count. For 8-bit expressions, only up to 255
bytes are copied per call (high byte = 0). Use `word` variables for lengths > 255.

`drawmem src, dst, width, height, stride` copies a 2-D rectangular block. `src` is read
linearly (packed rows); `dst` advances by `stride` bytes between rows — use `40` ($28) for
the C64 screen or color RAM (40 columns). Width, height and stride are all 8-bit values.
`src` and `dst` may be constants, `word` variables, or 8-bit expressions.

### Raster IRQ

```basic
irq my_handler           # set raster IRQ at line 0, handler = sub or address
irq my_handler, 100      # set raster IRQ at raster line 100
irq $C800, 200           # handler at fixed address $C800, line 200
irq addr_word            # handler address from a word variable
```

Sets up a raster IRQ via the BASIC soft vector (`$0314`/`$0315`):
1. SEI — disable interrupts during setup
2. Disable CIA1 timer IRQ (`$DC0D = $7F`) — prevents CIA1 from competing
3. ACK pending VIC IRQ (`$D019`)
4. Clear raster bit 8 (`$D011 &= $7F`) — restricts trigger lines to 0–255
5. Write raster line → `$D012`
6. Enable VIC raster IRQ (`$D01A = $01`)
7. Write handler lo/hi → `$0314`/`$0315`
8. CLI — re-enable interrupts

**Handler requirements:** The routine pointed to by `$0314` must end with `JMP $EA81`
(KERNAL end-of-IRQ — restores A/X/Y and executes RTI). Using plain `RTI` will corrupt
the stack because the KERNAL's own IRQ entry code already pushed A/X/Y before calling
the `$0314` vector. The handler should also ACK the VIC IRQ before any other work:
```basic
sub my_handler()
  poke $D019, $FF      # ACK VIC IRQ (write 1s to clear flags)
  # ... your IRQ work here ...
  irq_exit             # JMP $EA81: restore A/X/Y + RTI (proper IRQ handler exit)
end
```

Forward references are supported: `irq my_handler` works even when `my_handler` is defined
after the `irq` statement (same forward-ref mechanism as `call`).
Paths are relative to the current working directory.

`include` inlines the parsed statements of another source file. Constants defined in the
included file are visible after the include point.

### NMI Handler

```basic
nmi my_nmi               # set NMI vector $0318/$0319 to handler sub or address
nmi $C800                # fixed address

sub my_nmi()
  # ... NMI work here ...
  nmi_exit               # JMP $FE47 — proper NMI exit (restores A/X/Y + RTI)
end
```

`nmi handler` writes the handler address to the NMI soft vector (`$0318`/`$0319`) with SEI/CLI. The hardware NMI vector `$FFFA` points to the KERNAL NMI routine which branches through `$0318`. The handler **must** end with `nmi_exit` (emits `JMP $FE47`). Forward references supported.

### CIA1 Timer IRQ

```basic
cia_timer 19656, my_handler   # CIA1 timer A: period 19656 cycles (~50 Hz PAL)
cia_timer period, handler      # period can be a word variable or expression
```

Sets up CIA1 timer A as a periodic IRQ via the BASIC soft vector (`$0314`/`$0315`):
1. SEI
2. `$DC0D = $7F` — disable all CIA1 IRQs
3. Load 16-bit period lo→`$DC04`, hi→`$DC05`
4. Write handler lo/hi → `$0314`/`$0315`
5. `$DC0D = $81` — enable CIA1 timer A IRQ
6. `$DC0E = $01` — start timer A continuous
7. CLI

The handler must end with `irq_exit` (or `sys $EA81`) and should ACK the CIA1 IRQ:
```basic
sub my_handler()
  poke $DC0D, $01      # ACK CIA1 timer A IRQ
  # ... work ...
  irq_exit             # JMP $EA81: restore A/X/Y + RTI
end
```

PAL timing: clock = 985 248 Hz. Period for 50 Hz ≈ 19 705 cycles (`$4CC9`). Forward references supported.

### Error Handling

```basic
onerr goto err_handler   # set KERNAL I/O error vector ($0300/$0301) to a label
```

`onerr goto label` writes the label address (lo, hi) to `$0300` / `$0301`. When a KERNAL I/O
error occurs the KERNAL executes `JMP ($0300)` → the label. Forward references (label defined
after `onerr goto`) are fully supported. Unresolved labels are reported as compile-time errors.

```basic
onerr goto disk_err
load "MISSING", $C000    # if this fails, KERNAL jumps to disk_err
...
label disk_err
  print "DISK ERROR"
  bye
```

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
plot erase x, y          # clear pixel at (x, y) — AND ~mask into byte
plot xor x, y            # toggle (XOR) pixel at (x, y) — EOR mask into byte
circle x, y, r           # midpoint circle centered at (x, y) with radius r; clips off-screen points
line x1, y1, x2, y2      # Bresenham line from (x1,y1) to (x2,y2); x: 0-255, y: 0-199
paint x, y               # 4-connected flood fill from (x, y); fills clear pixels bounded by set ones
mplot x, y, color        # set multicolor pixel at (x: 0-159, y: 0-199), color 0-3 (requires graphics on multi)
```

All `graphics on` variants blank the VIC display during setup ($D011 DEN bit) to prevent
mode-switch glitches. Call `display on` after `gcls` and drawing to unblank.

**Hires (standard) bitmap**: each pixel is 0 or 1; foreground/background per 8×8 cell from color RAM.
**Multicolor bitmap**: each pixel is 2 bits → 4 colors per 8×8 cell (effective 160×200 resolution).

`gcls` in hires/multicolor mode clears bitmap $2000-$3FFF + fills video matrix.
`plot` emits a compact helper subroutine once per program (all `plot` calls share it via `JSR`).
`plot erase` and `plot xor` each emit their own helper (only if used); all three share the same ZP block.
`paint` emits a ~200-byte flood-fill helper + allocates 512 bytes of stack at `$C000+` (same pool as arrays).
`mplot` emits a shared ~115-byte helper (emitted once in post-code). The helper:
1. Computes byte address: `$2000 + (y>>3)*320 + (x>>2)*8 + (y&7)` (same row formula as hires)
2. Computes pair index `x&3` and shift count `(3-pair)*2` (6, 4, 2, or 0)
3. Builds `and_mask = ~($03 << shift_count)` and `set_bits = (color&3) << shift_count`
4. Read-Modify-Write: `LDA (ptr),Y; AND and_mask; ORA set_bits; STA (ptr),Y`

X supports the full 320-pixel width. For x ≤ 255 the high byte is 0; for x = 256–319 it is 1, which the helper adds as an extra +256 to the byte address. `word` variables work directly as x.

Pixel byte formula: `$2000 + (y>>3)*320 + (x and $1F8) + (y and 7)`,  bit: `$80 >> (x and 7)`

### Block Graphics (80×50)

```basic
graphics on block        # 80×50 block-pixel mode (text mode + custom 4-pixel charset @ $2800)
graphics off             # back to text mode
gcls                     # clear block playfield: screen RAM $0400-$07FF + color RAM $D800-$DBFF

plot4 x, y               # set block pixel at (x, y);  x: 0-79, y: 0-49
plot4 erase x, y         # clear block pixel at (x, y)
circle4 x, y, r          # draw midpoint circle in block pixels; clips to 80×50
```

Block mode is a chunky low-res mode layered on standard 40×25 text mode. A 16-character custom
charset is built and copied to `$2800`; each character encodes a 2×2 quadrant grid
(bit3=top-left, bit2=top-right, bit1=bottom-left, bit0=bottom-right). Each text cell therefore
holds 2×2 block pixels → an effective 80×50 grid. No bitmap RAM is used (`$2000-$3FFF` stays
free), making it faster than hires bitmap.

`emit_graphics_on_block()`:
1. Blanks display (`$D011` DEN), disables all sprites and clears sprite MCM/expand/priority
2. Forces 40-column mode (`$D016`), VIC bank 0 (`$DD00`), and clears MCM/ECM/BMM
3. Builds the 16-char charset inline (`JMP` over 128 bytes) and copies it to `$2800`
4. Sets `$D018 = $1A` (screen `$0400`, charset `$2800`), `$D011 = $0B`

`plot4 x, y` computes the cell address `$0400 + (y>>1)*40 + (x>>1)`, derives the quadrant bit
from `(x&1, y&1)`, and OR's it into the cell so overlapping block pixels accumulate.
`plot4 erase` AND's the inverse mask. Both share a helper using ZP `$FB/$FC/$FD`.
`circle4 x, y, r` uses an 8-bit midpoint circle helper and plots through the same `plot4` helper.
It clips generated points outside the 80×50 block-pixel playfield.

`gcls` in block mode clears screen RAM (`$0400-$07FF`) and color RAM (`$D800-$DBFF`) with
forward `INX/BNE` page loops. (Earlier versions used a descending `LDX #231 / DEX / BPL` loop
that only ran once — bit 7 of `$E7` is set, so `BPL` exits immediately — leaving the bottom
~6 rows holding KERNAL `$20` spaces that rendered as black/garbage blocks. Always use forward
`INX/BNE` page loops for ≥128-byte fills.)

See `examples/block_demo.ub`.

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
sys $FFD2, 7             # LDA #7 ; JSR $FFD2  (pass byte value in A register)
irq_exit                 # JMP $EA81 — proper IRQ handler exit (restore A/X/Y + RTI)
asm $EA, $EA             # inline raw bytes (NOP NOP) — legacy form
asm {
  ; Full 6502 mnemonics and addressing modes
  LDA #$07               ; immediate
  STA $0286              ; absolute
  LDA $50                ; zero-page  ($50 ≤ $FF → ZP auto-selected)
  STA $0400,X            ; absolute,X indexed
  LDA ($50),Y            ; (indirect),Y
  LDA ($50,X)            ; (indirect,X)
  JSR $FFD2              ; subroutine call
  JMP $C000              ; absolute jump
  JMP ($FFFC)            ; indirect jump

  CLC
  ADC #1                 ; 16-bit carry: ADC lo then ADC #0
  SEC
  SBC #1

  TAX                    ; implied / transfer
  ASL A                  ; accumulator (also just: ASL)
  LSR A
  ROL
  ROR

  ; Branches — operand is an absolute address; offset is computed automatically
  BNE loop               ; forward or backward branch to local label
  BEQ done

loop:
  NOP
done:
  RTS

  ; #<label / #>label — lo / hi byte of a label address
  LDA #<handler
  STA $0314
  LDA #>handler
  STA $0315

  ; * — current assembly location
  JMP *

  ; Raw hex bytes (backward-compatible with old asm { $xx ... } syntax)
  $EA $EA                ; two NOP bytes
}
```

**Addressing modes supported:**

| Syntax | Mode | Bytes | Example |
|---|---|---|---|
| (no operand) | Implied | 1 | `NOP`, `RTS` |
| `A` | Accumulator | 1 | `ASL A`, `LSR` |
| `#value` | Immediate | 2 | `LDA #$07` |
| `$zz` (0–255) | Zero-page | 2 | `LDA $50` |
| `$zz,X` | ZP,X | 2 | `LDA $50,X` |
| `$zz,Y` | ZP,Y | 2 | `LDX $50,Y` |
| `$xxxx` | Absolute | 3 | `LDA $0400` |
| `$xxxx,X` | Absolute,X | 3 | `LDA $0400,X` |
| `$xxxx,Y` | Absolute,Y | 3 | `LDA $0400,Y` |
| `($xxxx)` | Indirect | 3 | `JMP ($FFFC)` |
| `($zz,X)` | (Indirect,X) | 2 | `LDA ($50,X)` |
| `($zz),Y` | (Indirect),Y | 2 | `LDA ($50),Y` |
| `label` | Relative | 2 | `BNE label` (branches only) |

**Notes:**
- `$zz` (1–2 hex digits, value ≤ 255) selects zero-page if the instruction supports it; otherwise auto-upgrades to absolute. Use `$00xx` (4 digits) to force absolute.
- Branch operands are absolute addresses; the relative byte offset is computed by the assembler.
- Local labels (`name:`) are scoped to the `asm { }` block. Forward branches are resolved in pass 2.
- `#<label` / `#>label` yield the lo / hi byte of a label's address.
- `*` yields the current instruction address, so `JMP *` assembles as a self-loop.
- Lines starting with `$`, `%`, or a digit are emitted as raw bytes (backward-compatible with the old `asm { $A9 $07 }` form).
- Inside `asm { }`, comments are `;` or `//` to end of line. (`#` is the immediate prefix, not a comment.)
- The `asm $EA, $EA` single-line raw-byte form is unchanged.

**Mixing `asm { }` with subroutine parameters**

Parameter names are **not accessible** inside `asm { }` blocks — only the compiler knows their
zero-page addresses. Use UltimateBasic statements to move parameter values into known
locations *before* the `asm { }` block:

```basic
sub set_colors(border_col, bg_col)
  poke $D020, border_col   # UltimateBasic resolves the ZP address
  poke $D021, bg_col
  asm {
    ; values are already in $D020 / $D021
    LDA $D020
    ; ...
  }
end
```

For routines whose entire body is assembly — especially IRQ handlers that cross-reference
each other — put **all** handlers in a single top-level `asm { }` block in the main
program.  Labels defined in the same block are all in scope, so `irq1` and `irq2` can
reference each other freely.  See `examples/raster_irq_demo.ub`.

### String ↔ Integer

```basic
numstr score, $0340      # writes "042\0" to $0340 (always 3 digits, zero-padded)
var n = str_to_int("42") # compile-time: Expr::Number(42)

print str$(score)                # print 8-bit int as 3-digit decimal string ("000"–"255")
print "Score: " + str$(score)   # usable in string concat print context
var s: string = str$(n)          # assign to string var (shared static buffer)
```

`numstr` converts an 8-bit variable to a 3-character decimal ASCII string (always 3 digits, e.g. `5` → `"005"`) stored at the given absolute address, followed by a null terminator. The keyword is `numstr` (not `int_to_str`).

`str$(n)` is the expression form: converts an 8-bit value to a 3-digit null-terminated decimal string and returns a pointer to it (stored in a permanent ZP pair). Always 3 digits with leading zeros (`"000"`–`"255"`). Uses a single shared 4-byte static buffer — calling `str$(n)` again overwrites the previous result.

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
| `$FFF0` | KERNAL PLOT (get/set cursor position; C=0=SET, C=1=READ) |
| `$FFF3` | KERNAL IOBASE (returns CIA1 base address $DC00 in X/Y) |

---

## CLI Reference

```
ub build <input.ub> [OPTIONS]

  -o, --output <file>   Output .prg file (default: <input>.prg)
  -v, --verbose         Show full ZP layout + code hex dump after build
  --no-stub             Skip the BASIC SYS stub (code loads at $0801)
  --d64 [file]          Also produce a .d64 disk image;
                          without a filename defaults to <output>.d64
  --add <file>          Add an extra file to the .d64 disk image;
                          may be repeated for multiple files
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
| Integer arithmetic | 8-bit unsigned (0–255); `word` vars hold 16-bit values |
| Subroutines | No recursion — ZP parameter slots are statically allocated |
| String vars | Read-only after init; assignment replaces the pointer, not the data |
| String concat runtime | `s1 + s2` prints sequentially — no heap allocation or length tracking |
| `rnd()` | Simple LCG, not cryptographic; period = 256 |
| `abs()` / `sgn()` / `min()` / `max()` | 8-bit values only; `abs`/`sgn` treat values as signed (bit 7 = negative → `abs` two's-complements, `sgn` returns `$FF`); `min`/`max` are unsigned (0–255) |
| `plot` | Out-of-range pixels are silently clipped (CheckPlot: Y ≥ 200 or X ≥ 320 → skip) |
| `mplot` | No bounds checking — x must be 0–159, y must be 0–199 |
| `plot4` | No bounds checking — x must be 0–79, y must be 0–49 (block mode) |
| `circle4` | Clips off-screen block pixels; useful radius is roughly 0–49 in 80×50 block mode |
| `chr$` | No PETSCII↔ASCII mapping — n is passed as-is to CHROUT |
| `music play` | Requires `load sid`; emits one shared wrapper (last `music play` wins if called multiple times) |
| Error reporting | Compile-time only; `onerr goto` handles KERNAL I/O errors at runtime |
| `poke`/`peek` with offset | `poke ptr + i, val` truncates `ptr+i` to 8 bits when `i` is a variable; use `msg[i]` for 16-bit-safe indexed access |
| `fn` return values | 8-bit return works in all expression contexts; `: word` return works for `var w: word = fn()` but 16-bit fn calls in 8-bit contexts read only the lo byte |
| `fn` bodies inside `sub` | Not scanned recursively by pre_scan helpers (has_plot_stmt, etc.) — any required ZP helpers must be detected at the top level |
