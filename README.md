# Ultimate Basic

A modern BASIC-like language that compiles directly to 6502 machine code for the
**Commodore 64** and **Commodore 64 Ultimate**. It produces `.prg` files that run in
VICE or on real hardware, and can also build `.d64` disk images.

Ultimate Basic looks like classic BASIC but compiles ahead of time — no interpreter,
no line numbers required. It adds typed variables (`int`/`word`/`float`/`string`/arrays),
subroutines and functions, structured control flow, and direct, high-level access to the
C64's hardware: bitmap and block graphics, sprites, SID sound and music, raster/CIA/NMI
interrupts, REU transfers, disk I/O, and inline 6502 assembly.

Sprite support includes hardware collision registers, side-effect-free software AABB
tests with `box_hit()`, and frame selection from consecutive sprite animation data.
Character tile maps can be embedded from self-describing `.ubmap` files, rendered as
40×25 viewports, queried and modified at runtime, in normal or multicolor text mode.
Koala Painter images can also be validated, embedded, shown in multicolor bitmap
mode, and hidden again with `koala load`, `koala show`, and `koala hide`.

© 2026 Zsolt Tarczali

## Build

```bash
cargo build --release      # binary: target/release/ub
cargo test                 # unit + integration tests
```

## Usage

```bash
ub build demo.ub -o demo.prg          # compile to a .prg (prints a memory map)
ub build demo.ub -v                   # also print ZP layout + hex dump
ub build demo.ub --debug              # also write .sym, .dbg and .vs symbols
ub build demo.ub --d64 disk.d64       # also build a .d64 disk image
ub build demo.ub --d64 disk.d64 --add music.prg   # embed extra files in the .d64
```

| Option | Description |
|---|---|
| `-o, --output <file>` | Output `.prg` (default: `<input>.prg`) |
| `-v, --verbose` | Print full zero-page layout and a hex dump |
| `--no-stub` | Skip the BASIC `SYS` stub (code loads at `$0801`) |
| `--debug` | Also produce KickAssembler `.sym`, C64Debugger `.dbg`, and VICE `.vs` files |
| `--d64 [file]` | Also produce a `.d64` (default: `<output>.d64`) |
| `--add <file>` | Add an extra file to the `.d64` (repeatable) |

With `--debug`, the compiler writes three files beside the program: an importable
KickAssembler `.sym`, a C64Debugger/RetroDebugger `.dbg`, and a VICE monitor `.vs`.
They contain the program boundaries, variables, arrays, subroutines, and BASIC labels.
The `.dbg` export currently provides address symbols but not source-line stepping data.

## A taste

```basic
graphics on
gcls
for i = 0 to 199
  line 0, i, 319, 199 - i
next
display on
var k = getch()
graphics off
bye
```

## Documentation

The complete language and CLI reference lives in **[MANUAL.md](MANUAL.md)** — variables and
types, operators, control flow, subroutines/functions, graphics (bitmap, double-buffered,
block), sprites, sound and SID music, interrupts, REU, disk I/O, string/math functions, and
inline assembly.

Release history is in [whatnews.txt](whatnews.txt).

## Examples

Ready-to-build demos are in [`examples/`](examples/) — bitmap and block graphics, sprite
multiplexing, plasma and orbit effects, a flicker-free double-buffered 3D cube
(`cube_demo.ub`), REU stash/fetch, SID music playback, scrollers, and more.
