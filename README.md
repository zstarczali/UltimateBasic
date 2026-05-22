# Ultimate Basic – C64 BASIC compiler

Ultimate Basic is a compiler that translates a modern BASIC-like language into 6502 machine code for the Commodore 64. Output formats: raw `.prg` and `.d64` disk images.

## Quick start

```bash
# Compile a .ub file to .prg
ultimate-basic build demo.ub -o demo.prg

# Compile with .d64 disk image
ultimate-basic build demo.ub --d64 disk.d64

# Compile without BASIC SYS stub (raw code at $0801)
ultimate-basic build demo.ub -o raw.prg --no-stub
```

## Language reference

### Comments

```
# This is a comment
```

### Variables

```basic
var score = 0           # integer
var name = "hello"      # string (planned)
var pi = 3.14           # float (planned)
```

Variables are 8-bit integers (0-255). Strings and floats are planned.
All variables are stored in zero-page ($02-$4F), 2 bytes each.

### Operators

| Operator | Description | Precedence |
|----------|-------------|-----------|
| `*` `/` | Multiply, divide | Highest |
| `+` `-` | Add, subtract | |
| `==` `!=` `<` `>` `<=` `>=` | Comparison | |
| `not` / `!` | Logical NOT | |
| `and` / `&&` | Logical AND | |
| `or` / `\|\|` | Logical OR | Lowest |

### Assignment

```basic
x = 10
score = score + 1
name = "hello"
```

### Output

```basic
print "Hello World"
print "Score: ", score
print ""                  # blank line
```

### Input

```basic
var c = getch             # wait for keypress, return PETSCII code
```

### Constants

```basic
const SCREEN = 1024
const BORDER = $D020
const PI = 314
```

Constants are substituted at compile-time. Use them anywhere a number is expected.

### Labels and Goto

```basic
label start:
  print "looping..."
  goto start              # unconditional jump

label exit:
  print "done"
```

Forward references are supported (goto to a label defined later).

### Poke and Peek

```basic
poke $D020, 2             # set border to red
var v = peek($D012)       # read raster line
```

### Math functions

```basic
var r = rnd()             # random number 0-255
var a = abs(x - 100)      # absolute value
var m = min(a, b)         # minimum of two values
var x = max(a, b)         # maximum of two values
var s = sgn(x)            # sign: 0 if x==0, 1 if x!=0
```

### Control flow

#### If / Then / Else

```basic
if x == 5 then
  print "x is five"
end

if lives > 0 then
  print "alive"
else
  print "game over"
end

if x > 0 and y < 10 then
  print "in bounds"
end
```

#### Loops

| Loop type | Syntax |
|-----------|--------|
| Infinite | `loop ... end` |
| Counted | `loop 5 ... end` |
| For | `loop i = 1 to 10 ... end` |
| For + step | `loop i = 0 to 10 step 2 ... end` |
| While | `while x < 100 ... end` |

```basic
# For loop
loop i = 1 to 5
  print i
end

# For loop with step
loop j = 0 to 10 step 2
  print j
end

# While loop
var n = 0
while n < 10
  n = n + 1
  print n
end
```

#### Break

```basic
loop
  var c = getch
  if c == 81 then
    break                # exit innermost loop
  end
end
```

### Subroutines

```basic
sub hello()
  print "Hello!"
end

hello()                  # call subroutine
call hello               # alternative call syntax
```

Subroutines can be forward-referenced (called before they are defined).
Use `return` to exit a subroutine early.

### Colors

Controls text color, border color, and background color (C64 color codes 0-15):

| C64 color | Code |
|-----------|------|
| Black | 0 |
| White | 1 |
| Red | 2 |
| Cyan | 3 |
| Purple | 4 |
| Green | 5 |
| Blue | 6 |
| Yellow | 7 |
| Orange | 8 |
| Brown | 9 |
| Light red | 10 |
| Dark gray | 11 |
| Medium gray | 12 |
| Light green | 13 |
| Light blue | 14 |
| Light gray | 15 |

```basic
color 7                  # set text to yellow
color text 14            # set text to light blue
color border 2           # set border to red
color bg 0               # set background to black
```

### Screen

```basic
cls                      # clear screen (KERNAL)
cls manual               # full manual clear (screen + color RAM)
```

### Graphics

```basic
graphics on              # switch to bitmap mode (320x200)
graphics off             # return to text mode
```

### System / Assembly

```basic
sys $FFD2                # call KERNAL CHROUT

asm { $A9 $07 $8D $86 $02 }  # inline assembly block
asm $EA, $EA, $EA             # inline assembly bytes
```

### String conversion

```basic
int_to_str score, $0340  # convert integer to decimal string at address

var level = str_to_int("1")  # compile-time string-to-int conversion
```

## Examples

The `examples/` directory contains sample programs:

| File | Description |
|------|-------------|
| `examples/features.ub` | Demonstrates all new features: const, label/goto, poke/peek, rnd, abs, min, max, sgn |

```bash
# Compile the features demo
ultimate-basic build examples/features.ub -o features.prg --d64 features.d64
```

## File extensions

- `.ub` — Ultimate Basic source file
- `.prg` — C64 program file (with optional BASIC stub)
- `.d64` — C64 disk image

## Building from source

```bash
cargo build --release
```

The compiled binary is `target/release/ultimate-basic.exe` (or `ultimate-basic` on Linux/macOS).

## Architecture

```
Source (.ub)
  → Lexer (tokens)
    → Parser (AST)
      → Codegen (6502 machine code)
        → PRG file
```

- `src/compiler/lexer.rs` — Tokenizer (67 token types)
- `src/compiler/parser.rs` — Recursive-descent parser
- `src/compiler/ast.rs` — AST node types
- `src/compiler/codegen.rs` — 6502 code generator (direct byte emission)
- `src/compiler/mod.rs` — Compiler entry point, BASIC stub constant

## License

MIT