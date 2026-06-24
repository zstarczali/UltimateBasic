# Ultimate Basic

A modern BASIC-like language that compiles directly to 6502 machine code for the
**Commodore 64** and **Commodore 64 Ultimate**. It produces `.prg` files that run in
VICE or on real hardware, and can also build `.d64` disk images.

Ultimate Basic looks like classic BASIC but compiles ahead of time — no interpreter,
no line numbers required. It adds typed variables (`int`/`word`/`float`/`string`/arrays),
subroutines and functions, structured control flow, and direct, high-level access to the
C64's hardware: bitmap and block graphics, sprites, SID sound and music, raster/CIA/NMI
interrupts, REU transfers, disk I/O, and inline 6502 assembly.

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
ub build demo.ub --d64 disk.d64       # also build a .d64 disk image
ub build demo.ub --d64 disk.d64 --add music.prg   # embed extra files in the .d64
```

| Option | Description |
|---|---|
| `-o, --output <file>` | Output `.prg` (default: `<input>.prg`) |
| `-v, --verbose` | Print full zero-page layout and a hex dump |
| `--no-stub` | Skip the BASIC `SYS` stub (code loads at `$0801`) |
| `--d64 [file]` | Also produce a `.d64` (default: `<output>.d64`) |
| `--add <file>` | Add an extra file to the `.d64` (repeatable) |

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
