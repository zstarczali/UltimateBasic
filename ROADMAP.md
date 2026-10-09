# Ultimate Basic — Roadmap (deferred work)

Larger changes that were deferred, with what is known about each so the work can
start without re-research. Smaller known limitations
are listed in `CLAUDE.md` → *Known Limitations*.

---

## Done in 1.6.4

The three big items of the 1.6.3 roadmap are implemented:

1. **Real floating point** — `single` / `double` are 5-byte floats in the C64 BASIC
   format. Instead of calling the BASIC ROM (its zero page `$22-$2A`, `$61-$70`, `$7A`
   collides with the compiler's, and the test emulator has no ROM image), the compiler
   assembles its own runtime from source: `src/compiler/codegen/float_lib.rs` (strict
   two-pass assembler `codegen/rtasm.rs`; FAC / ARG with 8 guard bits, 16-level float
   stack, + − × ÷, compare, int ↔ float, text ↔ float, `^`, SQR, SIN/COS/TAN/ATN,
   EXP/LOG via range reduction + polynomials). Code generation: `codegen/float.rs`.
   Only one 2-byte zero-page pointer is used. Tests: `tests/float_runtime.rs` (each
   routine against f64), `tests/real_numbers.rs` (compiled programs, random expressions).
2. **Variables in RAM** — `compiler/spill.rs` plans which variables move to RAM when
   zero page runs out; the code generator copies them into zero-page proxy slots around
   each statement (`spill_enter` / `spill_exit`). `compile()` retries with more spilled
   variables until it fits.
3. **12 KB of arrays** — `$A000-$CFFF` with the BASIC ROM banked out (`$01` bit 0) while
   the program runs, banked in at every exit.

---

## Next candidates

- **Float runtime size:** the whole ~4.4 KB library is appended as soon as a program uses
  floating point. Splitting it into parts (core / text / transcendental functions) that are
  included on demand would save 1-2 KB for simple programs.
- **Integer literals 32768-65535 in floating-point context** are read as 16-bit patterns
  (negative); the lexer/parser would need to keep "unsigned literal" information on
  `Expr::Number` (write `40000.0` meanwhile).
- **Floating point in IRQ handlers:** the runtime keeps its state in fixed RAM; it would
  need a save/restore (or a second state block) around handlers.
- **`RND` as a float** (0 ≤ x < 1, QBasic / C64 BASIC style) and `MOD` on floats.
- **`PRINT USING`** / fixed decimals for floats.

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
