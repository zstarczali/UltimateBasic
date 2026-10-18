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

## Done in 1.6.6

- **Float runtime size:** only the parts of the runtime a program reaches are linked
  (`float_lib::float_runtime_for` + `prune`: the source is split into chunks at every
  column-0 label, kept by reachability from the called `f_*` entries through operand
  references and fall-through). A program that only multiplies, adds and prints went from
  4603 to 2293 bytes; `examples/float_demo.ub` from 6319 to 5660.
- **Literals 32768-65535 in floating-point statements** keep their unsigned value
  (`d = 40000`, `a + 50000`, `-40000`, `$FFFF * 2`). The parser decides per statement
  (`Parser::stmt_is_real`): a statement that mentions a SINGLE / DOUBLE name
  (`pre_scan_real_names`: variables, arrays, parameters, `AS DOUBLE` functions), `^`, a
  literal only floating point can hold, or a floating-point builtin gets
  `Expr::FixedLit(_, v, true)` for them; every other statement keeps the 16-bit
  `Expr::Number`, so integer code is unchanged. Untyped `var x = 2.5` names are not in
  the set (a statement using only those still reads `40000` as 16 bits).
- **Floating point in IRQ / NMI handlers:** in a program with floating point, a sub named
  by `irq`, `nmi` or `cia_timer` gets `JSR save` at its start and `JSR restore` before
  each `irq_exit` / `nmi_exit` (`flt_handler_hook`). After the runtime is assembled,
  handlers that reach floating point (`flt_routines` + the AST call graph) get their own
  routines and buffer (`emit_flt_handler_state`): runtime RAM up to the float stack
  (`float_lib::state_range`), scratch zero page `$50-$7F` and the runtime pointer; `fsp`
  moves up one spare slot. Other handlers get `BIT $0000` instead (3 bytes, no effect).
  Programs without floating point are unchanged. Cost: about 2 × (state + 50) bytes
  copied per interrupt (~4-5k cycles). Not covered: a handler set by address or word
  variable, and a handler that leaves through a plain `RTS` / `RTI`.
- **`RND` as a float:** in a floating-point statement (`Parser::stmt_is_real`) `rnd` /
  `rnd()` is `FnCall("rnd")`, 0 <= x < 1 from `f_rnd` (xorshift32 13/17/5, the raster
  line mixed in on the first call; its state sits outside the IRQ-saved block).
  `rnd(n)` keeps its integer meaning (0 … n-1), and `rnd` in integer statements is still
  the byte generator — `INT(RND * 6)` into an INTEGER is not a floating-point statement,
  use `rnd(6)` there.
- **`MOD` on floats:** with a SINGLE / DOUBLE operand, `a mod b` = `a - b * FIX(a / b)`
  (sign of `a`, like VB / C `fmod`; `7.5 mod 2` = 1.5), `x mod 0` = x
  (`f_mod` / `f_modp` / `f_modarg`).
- **`PRINT USING`** (QBasic subset): `print using "fmt"; a, b; …` with a literal format.
  Numeric fields `#` / `.` / `,` (thousands) / leading `+` / trailing `-`, `%` prefix when
  the number does not fit (`-.50` drops a lone 0 first); string fields `&`, `!`,
  `\  \`; `_` escapes; values cycle through the fields, a trailing `;` / `,` keeps the
  line. The parser splits the format (`parse_using_format`) and lowers the statement to
  plain prints plus `Stmt::PrintUsingNum`, formatted by `f_using` (exact 32-bit → decimal
  digits, so up to ~9-10 significant digits; beyond 2^32 it prints `%` + the normal
  text). Not supported (compile errors): `**`, `$$`, `^^^^`, a non-literal format.
  A program with PRINT USING links the float runtime.
- **String ordering:** `<` `>` `<=` `>=` between strings, PETSCII byte order (a prefix
  sorts first), like C64 BASIC: `Helper::Order` returns $FF / 0 / 1, `emit_str_order`
  maps it to 1 / 0. Conditions take the value path (`is_byte_compare` excludes strings).
- **PRINT spacing (opt-in):** `--basic-print` / `option basic print` — numbers with the
  C64 leading (sign) and trailing space, `,` to the next 10-column zone, trailing `,`
  keeps the line. Default output is unchanged (existing screen layouts rely on it).
- **16-bit / signed `mod`, arithmetic `shr`:** a word or signed `mod` is a real 16-bit
  remainder with the sign of the dividend, like QBasic (`-7 mod 3` = -1, `7 mod -3` = 1,
  `(w + 5) mod 7` no longer drops to 8 bits; `x mod 0` = x) — `emit_word_mod`,
  `mod_is_signed`. `shr` of an INTEGER is arithmetic (`-8 shr 1` = -4, `emit_shr_hi`);
  WORD and byte `shr` stay logical. `gen_word_assign` sends these to `eval_expr_word`.

---

## Done after 1.6.6

- **Division:** `/` with a 16-bit, signed or negative divisor is a 16÷16 division (sign =
  xor of the operand signs, truncated: `-700 / 300` = -2) — `emit_word_divmod` (shared with
  `mod`), `div_needs_16`. It used the divisor's low byte (`-700 / -7` gave 0).
- **Byte overflow warning:** `CompileResult::warnings` (printed by the CLI, never fatal):
  a byte variable given a constant outside -128…255, or an expression with such a constant
  in its `+ - * shl` arithmetic (`b = a + 1000`) — `check_byte_overflow`. Masked results and
  byte + byte are not reported.
- **`END` in a QBasic routine:** in a SUB / FUNCTION closed by `END SUB` / `END FUNCTION`, an
  `END` alone on its line ends the program (also inside an `IF` there). Lexer markers
  `Token::EndRoutine` (`end sub|function`) and `Token::EndBlock` (`end if|select`, `wend`) look
  like `End` to the grammar (`Parser::token_at`); `qb_routine` turns a bare `end` into
  `EndProgramMark`. `END` / `bye` inside any routine now really leave the program: `$A659`
  returns to the current return address, so the entry stack pointer is saved
  (`exit_sp_zp`) and restored first.
- **`STR$` with BASIC spacing:** with `--basic-print` / `option basic print`, `STR$` gives
  the C64 text for every number type (`" 7"`, `"-42"`, `" 3.5"`) — `emit_basic_strn`. The
  default keeps the byte form `"007"`.
- **String length:** `option strings n[, m]` — n characters per computed string variable
  (1-255, 80 by default), m per string-array element (slot rounded up to 32/64/128/256
  bytes, 31 by default). `SourceOptions` carries all `option` settings to the code
  generator. There is still no heap: every variable keeps its own fixed buffer.

---

## Next candidates

All items of the 1.6.4 list are done (above); see *Smaller open items*.

---

## Smaller open items

- **String heap:** strings live in fixed per-variable buffers (`option strings` sets their
  size); a shared heap with garbage collection would save memory for many string
  variables, but is a redesign of `codegen/strings.rs`.
- **Mixed `END` styles:** inside a routine closed by `END SUB`, a UB-style block closed by
  a bare `end` on its own line (`if … ⏎ end`) now reads as program end — such routines
  must close their blocks QBasic-style (`END IF`) or close with a plain `end`.
