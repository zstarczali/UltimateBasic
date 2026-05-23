# Ultimate Basic — C64 compiler

A modern BASIC-like language that compiles directly to 6502 machine code for the
Commodore 64. Output: `.prg` files (VICE or real hardware) and `.d64` disk images.

## Quick start

```bash
cargo build --release

# Compile to .prg (memory map printed on success)
ultimate-basic build demo.ub -o demo.prg

# Verbose: also prints zero-page layout and hex dump
ultimate-basic build demo.ub -v

# Compile + create .d64 disk image
ultimate-basic build demo.ub --d64 disk.d64
```

## Language reference

### Variables and constants

```basic
var x = 10               # 8-bit integer (default)
var ptr: word = $0400    # 16-bit — two zero-page bytes, usable as 16-bit address
var msg = "HELLO"        # string variable (pointer to inline PETSCII data)
var s: string = "TEXT"   # string with explicit type
var scores = array(10)   # byte array, 10 elements stored at $C000+
const BORDER = $D020     # compile-time constant (substituted inline, no ZP slot)
```

| Type | Width | Notes |
|---|---|---|
| `int` | 8-bit | default for numeric literals |
| `word` | 16-bit | two ZP bytes; can be used as address in `poke`/`peek` |
| `string` | pointer | ZP pair → null-terminated PETSCII in code segment |
| `array(N)` | N bytes | lives at `$C000+`, not in ZP |

### Comments

```basic
# hash comment
rem this is also a comment
var x = 5  ; inline semicolon comment
var x = 5  : var y = 6  # colon separates statements on one line
```

### Operators

```basic
x = x + 1
y = a * b - c / 2
z = x and 15             # bitwise AND
w = a or b               # bitwise OR
v = a xor b              # bitwise XOR
m = x shl 3              # shift left
n = x shr 2              # shift right
```

Comparisons: `==`  `!=`  `<`  `>`  `<=`  `>=`  (return 1/0)

### Print

```basic
print "HELLO"
print x
print x, y, "text"
print                     # blank line

print "A=" + a           # string + numeric var
print s1 + s2            # two string vars → sequential print
print "Hello " + "World" # two literals → compile-time fold
print chr$(13)           # print by PETSCII code
```

### chr$

```basic
print chr$(65)           # output character with PETSCII code 65 ('A')
var c = chr$(n)          # store byte value n in variable c
print ">" + chr$(42)     # usable in string concat
```

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

for i = 1 to 10      # for..next (preferred)
  print i
next

for i = 0 to 20 step 2
  print i
next i               # variable name after 'next' is optional

loop i = 1 to 10     # legacy loop..end syntax — identical code
  print i
end

while x < 100
  x = x + 1
end
```

### Labels and goto

```basic
label main_loop
  x = x + 1
  if x < 10 then goto main_loop end
```

Forward `goto` (label defined later) is fully supported.

### Subroutines

```basic
sub greet()
  print "HELLO!"
end

sub set_color(col)
  color border col
  color text   col
end

greet()              # call with parens
call greet           # call keyword (no parens)
set_color(6)
```

Parameters are passed via dedicated zero-page slots. No recursion (slots are static).

### Arrays

```basic
var scores = array(8)    # 8 bytes at $C000

scores[0] = 100          # constant index → STA $C000
scores[i] = 99           # variable index → STA (ptr),Y
var v = scores[i]        # LDA (ptr),Y
print scores[2]          # usable inline in print
```

### 16-bit (word) variables

```basic
var ptr: word = $0400    # two ZP bytes: lo=$00 hi=$04
poke ptr, 6              # STA (ptr),Y
var v = peek(ptr)        # LDA (ptr),Y
```

### Bitmap graphics

```basic
graphics on              # VIC-II hires bitmap mode (320×200, 1bpp); bitmap at $2000
graphics on multi        # VIC-II multicolor bitmap mode (160×200, 2bpp, 4 colours/cell)
graphics off             # return to text mode

gcls                     # clear bitmap (fills $2000-$3FFF) + set video matrix colors

plot x, y                # set pixel at (x, y);  x: 0-319,  y: 0-199
circle x, y, r           # midpoint circle centered at (x, y) with radius r; clips off-screen points
line x1, y1, x2, y2      # Bresenham line from (x1,y1) to (x2,y2); x: 0-255, y: 0-199
```

Both `graphics on` variants blank the display (`LDA $D011 / AND #$EF / STA $D011`) while
switching VIC registers, then re-enable it in the target mode — prevents mode-switch glitches.

### Screen and color

```basic
cls                      # clear screen (KERNAL $E544)
cls fast                 # fast fill: screen RAM + color RAM + HOME

color text 14            # text color register $0286
color border 6           # $D020
color bg 0               # $D021

display on               # re-enable VIC display ($D011 DEN bit)
display off              # blank display
```

### Keyboard

```basic
var key = getch()        # busy-wait on $FFE4 until key; returns PETSCII code
var k   = inkey()        # non-blocking: returns PETSCII code, or 0 if no key pressed
var j = joy(2)           # read joystick port 2; returns inverted bits 0-4
var j = joy(1)           # read joystick port 1
                         # bit0=up(1) bit1=down(2) bit2=left(4) bit3=right(8) bit4=fire(16)
```

### Exit

```basic
bye                      # JSR $E544 (clear screen), clear STOP flag, RTS to BASIC
exit                     # alias for bye
```

### Timing

```basic
wait 50                  # wait 50 raster-line transitions (~3.2 ms)
wait raster 100          # spin until $D012 == 100 (raster-split effects)
```

### SID Sound

```basic
sound 0, $1CAD, 25       # voice 0, freq $1CAD (≈ middle C PAL), 25 frames duration
sound 1, freq_word, 50   # voice 1, freq from word var, 50 frames (1 s at 50 Hz)
sound 2, 0, 0            # voice 2, silence
```

`sound <channel>, <freq>, <duration>` — duration in PAL frames (1/50 s each).
Fixed ADSR: attack/decay `$09`, sustain/release `$F0`, sawtooth waveform.
Master volume `$D418` always set to `$0F`.

### Sprites

```basic
sprite 0, x, y, $2000    # sprite 0: set X, Y position and data pointer
sprite 0, x, y           # without data pointer (keeps existing)
sprite on  0             # enable sprite 0 ($D015 |= bit0)
sprite off 0             # disable sprite 0 ($D015 &= ~bit0)
sprite color 0, 7        # sprite 0 color = yellow ($D027)
sprite multicolor 0, on  # enable multicolor mode for sprite 0 ($D01C |= bit0)
sprite multicolor 0, off # disable multicolor mode ($D01C &= ~bit0)
var h = sprite_hit()     # sprite–sprite collision ($D01E, cleared on read)
var b = sprite_bg_hit()  # sprite–background collision ($D01F, cleared on read)
```

X supports full 9-bit range (0–319): use a `word` variable for runtime values > 255.
Sprite data pointer: `data_addr` must be 64-byte aligned; stored as `addr >> 6` at `$07F8+id`.

### Sprite definition

```basic
sprdef 0
  $00,$3C,$00,  $00,$FF,$00,  $03,$FF,$C0,  $07,$FF,$E0,
  $0F,$FF,$F0,  $0F,$FF,$F0,  $1F,$FF,$F8,  $1F,$FF,$F8,
  $1F,$FF,$F8,  $0F,$FF,$F0,  $0F,$FF,$F0,  $07,$FF,$E0,
  $03,$FF,$C0,  $00,$FF,$00,  $00,$3C,$00,  $00,$00,$00,
  $00,$00,$00,  $00,$00,$00,  $00,$00,$00,  $00,$00,$00,
  $00,$00,$00
end
```

`sprdef id ... end` embeds 63 sprite bytes at the next 64-byte-aligned address in the code
segment, emits a `JMP` over them, and automatically sets `$07F8+id = data_addr >> 6`.
To use the same shape for multiple sprites, read back the pointer:

```basic
var pg = peek($07F8)   # pointer set by sprdef 0
poke $07F9, pg         # copy to sprites 1–7
```

### Memory

```basic
poke $D020, 2            # STA $D020
poke addr_var, 6         # STA (addr_var),Y  — if addr_var is word type
var v = peek($D012)      # LDA $D012
var v = peek(addr_var)   # LDA (addr_var),Y  — if addr_var is word type
```

### Disk I/O

```basic
load "PROGRAM"           # KERNAL LOAD: loads file from device 8 to its native address
load "DATA", $C000       # loads file to a specific address
load "DATA", ptr         # addr from word variable
```

`load` calls KERNAL `SETNAM`+`SETLFS`+`LOAD` (`$FFBD`/`$FFBA`/`$FFD5`).
Without address: secondary address 0 (file's own 2-byte header used as load address).
With address: secondary address 1 (file loaded to specified location).

### Math functions

```basic
var a = abs(x - 20)      # two's-complement absolute value
var b = min(x, 39)       # 8-bit minimum
var c = max(x, 0)        # 8-bit maximum
var s = sgn(score)       # 0 = zero, 1 = positive, $FF = negative
var r = rnd()            # LCG pseudo-random 0-255; seed from raster linevar s = sin(angle)       # sine: angle 0-255 (full circle), returns 0-255 (center=128)
var c = cos(angle)       # cosine = sin(angle+64)

print hex(n)             # print as 2-digit uppercase hex
print bin(n)             # print as 8-bit binary string
```

### REU (RAM Expansion Unit)

```basic
var ok = reu_present()   # 1 if REU detected, 0 if not (write/read test on $DF04)

reu stash c64addr, bank, reu_addr, len  # copy C64 → REU
reu fetch c64addr, bank, reu_addr, len  # copy REU → C64
reu swap  c64addr, bank, reu_addr, len  # swap between C64 and REU
```

`reu_present()` performs a write/read-back test on REU register `$DF04`. Without an REU the
write is lost (open bus), so the read-back differs — reliably detects presence without
touching any side-effecting command register.

| Parameter | Width | Notes |
|---|---|---|
| `c64addr` | 16-bit | C64 RAM start — constant, `word` var, or 8-bit expr |
| `bank` | 8-bit | REU bank number (0–7 for a 512 KB unit) |
| `reu_addr` | 16-bit | Offset within the REU bank |
| `len` | 16-bit | Bytes to transfer (`0` = 65 536 in REU hardware) |

REU registers: `$DF01` command (`$B0` stash / `$B1` fetch / `$B2` swap),
`$DF02–$DF03` C64 addr, `$DF04–$DF05` REU offset, `$DF06` bank, `$DF07–$DF08` length.
Transfer is synchronous (CPU halted during DMA).
Requires a real REU or VICE: **Settings → Hardware → RAM Expansion Module**.

### Compile-time file embedding

```basic
incbin "sprites.bin"     # embed raw binary bytes at current code position
include "defs.ub"        # inline another .ub source file (lexed+parsed in place)
```

### Data / Read

```basic
data 1, 2, 3, 255        # constant byte table
read varname             # load next byte into varname (auto-declares if needed)
```

All `data` values are collected at compile time. A 2-byte ZP pointer is automatically
allocated and initialised at program start. Each `read` advances the pointer.

### Inline assembly

```basic
sys $FFD2                # JSR $FFD2
asm $EA, $EA             # inline bytes (NOP NOP)
asm {
  $A9 $07                # LDA #7
  $8D $86 $02            # STA $0286
}
```

### String ↔ integer

```basic
int_to_str score, $0340  # writes "042\0" at $0340 (always 3 digits)
var n = str_to_int("42") # compile-time: Expr::Number(42)
```

## Examples

| File | Description |
|---|---|
| `examples/features.ub` | const, label/goto, poke/peek, rnd, math functions |
| `examples/new_features.ub` | sub params, arrays, word vars, string vars |
| `examples/bitmap_demo.ub` | 320×200 bitmap, plot, graphics on/off |
| `examples/joystick_demo.ub` | joystick reading, sprite movement |
| `examples/mux_demo.ub` | raster sprite multiplexer (3 windows × 8 sprites = 24) |
| `examples/sprite_mux_orbit.ub` | 24-sprite orbit demo with sprdef + precomputed positions |
| `examples/reu_bitmap_demo.ub` | REU stash/fetch with bitmap graphics |

## Architecture

```
.ub source
  → Lexer  → Vec<Token>
  → Parser → Vec<Stmt>
  → Codegen → Vec<u8>   (raw 6502 machine code)
  → mod.rs → PRG = BASIC SYS stub + machine code
```

Two-pass compilation: Pass 1 = main code, Pass 2 = subroutine bodies (appended after
main `RTS`). Forward references (`call`, `goto`, `plot`) are patched at the end.

Zero-page layout: `$02-$4F` permanent (vars, loop counters, sub params), `$50-$7F`
scratch (reset per statement), `$FB` RNG seed.

## CLI reference

```
ultimate-basic build <input.ub> [OPTIONS]

  -o, --output <file>   Output .prg file (default: <input>.prg)
  -v, --verbose         Show zero-page layout and code hex dump
  --no-stub             Skip the BASIC SYS stub (code loads at $0801)
  --d64 [file]          Also produce a .d64 disk image;
                          without a filename defaults to <output>.d64
  -h, --help            Show help
```

After a successful build the compiler always prints a memory map:

```
demo.ub → demo.prg  (386 bytes)

  Load:    $080D – $0989

  Variables (zero page):
    score    ZP:$08   byte
    lives    ZP:$0A   byte
    msg      ZP:$0C   string
    ptr      ZP:$0E   word

  Subroutines:
    greet    $0900
    show     $0912

  Arrays ($C000+):
    sprites  $C000   8 bytes
```

With `-v` the output additionally shows the internal ZP allocations and a full hex dump.

## Building

```bash
cargo build --release    # binary: target/release/ultimate-basic
cargo test               # unit + integration tests
```

## Known limitations

| Feature | Limitation |
|---|---|
| Integer arithmetic | 8-bit unsigned (0-255) |
| word arithmetic | No carry propagation; use `poke`/`peek` patterns for 16-bit math |
| Arrays | Byte arrays only; max ~4 KB total (`$C000-$CFFF`) |
| Subroutines | No recursion — ZP param slots are statically allocated |
| String vars | Read-only after init; assignment replaces pointer, not data |
| `plot` | No erase/XOR mode — pixels can only be set, not cleared |
| `plot` | No bounds checking; x > 319 or y > 199 corrupts adjacent memory |
| `chr$` | n is passed as-is to CHROUT — no ASCII↔PETSCII conversion |
| `rnd()` | Simple LCG, period 256 |
| Error reporting | Compile-time only |
