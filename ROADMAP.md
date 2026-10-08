# Ultimate Basic — Roadmap (deferred work)

Larger changes that were deliberately **not** done in 1.6.3, with what is known
about each so the work can start without re-research. Smaller known limitations
are listed in `CLAUDE.md` → *Known Limitations*.

---

## 1. Real floating point via the C64 BASIC ROM

**Today:** `float` / `single` / `double` are Q8.8 fixed point — 0 to 255.99, no
negative values, two decimals printed.

**Goal:** real 5-byte Microsoft floats (±1.7·10³⁸, ~9 digits), as in C64 BASIC,
by calling the floating-point routines that are already in the BASIC ROM
(`$A000-$BFFF`, banked in while the program runs from BASIC, `$01 = $37`).

**ROM entry points** (C64 BASIC V2; verify each in VICE before relying on it):

| Routine | Address | Effect |
|---|---|---|
| MOVFM  | `$BBA2` | FAC1 ← 5-byte float at (A=lo, Y=hi) |
| MOVMF  | `$BBD4` | 5-byte float at (X=lo, Y=hi) ← FAC1 |
| FADD   | `$B867` | FAC1 ← mem(A/Y) + FAC1 |
| FSUB   | `$B850` | FAC1 ← mem(A/Y) − FAC1 |
| FMULT  | `$BA28` | FAC1 ← mem(A/Y) × FAC1 |
| FDIV   | `$BB0F` | FAC1 ← mem(A/Y) ÷ FAC1 |
| FCOMP  | `$BC5B` | compare FAC1 with mem(A/Y) → A = 0 / 1 / $FF |
| GIVAYF | `$B391` | FAC1 ← signed 16-bit integer (A = hi, Y = lo) |
| FACINX / AYINT | `$B1AA` / `$B1BF` | FAC1 → 16-bit integer |
| FOUT   | `$BDDD` | FAC1 → decimal string at `$0100` (pointer in A/Y) |
| FIN    | `$BCF3` | parse number at TXTPTR (`$7A/$7B`) → FAC1 |

**The hard part — zero page.** The ROM float code uses zero page that the
compiler currently allocates for variables and scratch:

- FAC1 `$61-$66`, FAC2 `$69-$6E`, `$70` (rounding), `$22-$2A` (INDEX /
  temporaries used by FMULT/FDIV), `$7A/$7B` (TXTPTR, for FIN), `$FF`/`$0100+`
  (FOUT output).
- Compiler: permanent variables `$02-$4F`, scratch `$50-$7F`.

So the ZP map must be split around those bytes (or the variables must move to
RAM, see item 2) before any ROM call is safe. Every float operation must also
leave `$01` with BASIC ROM visible.

**Plan:** new `VarType::Real` (5 bytes, stored in RAM, not ZP) used for
`single`/`double`; expression evaluation as a small stack of 5-byte temps in
RAM; `print` via FOUT; `input` via a buffer + FIN; conversions with GIVAYF /
FACINX. Keep Q8.8 available as `float` (fast) or `fixed`.

**Why deferred:** the test emulator (`tests/common/cpu6502.rs`) only traps
KERNAL calls and has no BASIC ROM image, so this cannot be verified in
`cargo test`; it needs VICE (or a ROM image added to the test harness, which
needs a legally usable ROM).

---

## 2. Variables in normal RAM when zero page runs out

**Today:** every variable, sub/fn parameter and `for` limit lives in permanent
zero page `$02-$4F` (78 bytes, 2 bytes per variable) → about 35-39 variables,
then the build fails with "out of zero page".

**Why it is big:** the code generator emits zero-page instructions directly
(`LDA zp` = `$A5`, `STA zp` = `$85`, `(zp),Y` pointer access, …) for variables
in thousands of places; `vars: HashMap<String, u8>` stores a 1-byte address.

**Options, from cheapest:**

1. **Share parameter/local slots between subs that are never active at the
   same time.** The call graph now exists (`Parser::call_graph`, built for
   the recursion check in 1.6.3), so two subs that are not on a common call
   path can reuse the same ZP slots. Big programs are mostly subs, so this can
   free a lot without touching codegen addressing.
2. **Spill only cold variables to RAM:** keep pointers, loop counters and
   arithmetic operands in ZP; give plain value variables an absolute address
   and emit `LDA abs` / `STA abs` (same instruction set, 3-byte opcodes). Needs
   an `Addr { Zp(u8), Abs(u16) }` type in `vars` and a pass over every emit
   site that uses a variable address.
3. Move strings' pointers out of ZP into RAM and copy them to a scratch ZP
   pointer only when dereferenced.

---

## 3. More than 4 KB of array space

**Today:** arrays are allocated from `$C000` up to `$CFFF` (4096 bytes,
checked at compile time by `dim`).

**Options:**

1. **RAM under the BASIC ROM, `$A000-$BFFF` (+8 KB):** bank the BASIC ROM out
   (`$01 = $36`) at program start and back in at exit (`bye`, `end`, end of
   main) and around every BASIC ROM call (`chain` uses `$A659`/`$A7AE`, `bye`
   jumps to `$A659`; item 1 would add many more). KERNAL stays visible.
2. **Arrays right after the program code** (up to `$9FFF`): array addresses
   are assigned in `pre_scan`, before the code size is known — needs a
   two-pass layout (generate once to measure, then again with the final array
   base) or relocatable array references.
3. **User-chosen area:** e.g. `arrays $4000, $8000` directive, checked against
   code, charset, bitmap (`$2000`, double buffer `$4000-$7FFF`), `map view`
   buffers (`$3C00`) and SID / Koala payloads.

---

## Smaller open items

- **String ordering:** `<` / `>` between strings (only `=` / `<>` exist).
- **String size:** computed strings are capped at 80 characters per variable and 31 per
  string-array element; there is no heap / garbage collection.
- **PRINT spacing:** numbers are printed without the leading/trailing space QBasic and
  C64 BASIC add, and `,` does not tab to print zones.
- **`STR$` of a byte** keeps the old 3-digit form (`"007"`) for compatibility; 16-bit
  and signed values print without padding.
- **Byte arithmetic:** untyped `var` values are 8-bit and wrap silently
  (`var c = a + b` with 200 + 100 gives 44) — by design; `DIM … AS INTEGER` avoids it.
  A compile-time warning for likely overflow would help.
- **`mod` and `shr` on 16-bit values:** `mod` is 8-bit only (`-7 mod 3` gives 0) and `shr`
  is a logical shift; a 16-bit / signed `mod` and an arithmetic `shr` are missing.
- **`END` inside a SUB/FUNCTION** closes the routine (block syntax); use `bye` there to
  stop the program.
