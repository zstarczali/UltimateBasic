// The source lines of the debugger's line map (.dbg): every `poke` below writes to an address made
// from its own line number, so the bytes of a span tell which line they really came from.

use ultimate_basic::compiler::{compile_with_path, CompileOptions, CompileResult};

const OPTS: CompileOptions = CompileOptions { basic_stub: false, explicit: false, basic_print: false };
const BASE: usize = 0xC000;

/// Replaces each `@` with a `poke` to BASE + line number.
fn build(lines: &[&str]) -> (String, CompileResult) {
    let source: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| l.replace('@', &format!("poke {}, 1", BASE + i + 1)))
        .collect();
    let source = source.join("\n") + "\n";
    let result = compile_with_path(&source, &OPTS, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    (source, result)
}

/// Checks each poke: the innermost span holding its STA abs must name its own line.
fn assert_lines(lines: &[&str]) {
    let (_, result) = build(lines);
    let map = &result.map;
    let code = &map.code_bytes;
    let mut checked = 0;
    for (i, line) in lines.iter().enumerate() {
        if !line.contains('@') {
            continue;
        }
        let number = i + 1;
        let target = BASE + number;
        let bytes = [0x8D, target as u8, (target >> 8) as u8];
        let at = code
            .windows(3)
            .position(|w| w == bytes)
            .unwrap_or_else(|| panic!("line {number}: no STA ${target:04X} in the code"));
        let span = map
            .listing_spans
            .iter()
            .filter(|s| s.start <= at && at < s.end)
            .min_by_key(|s| s.end - s.start)
            .unwrap_or_else(|| panic!("line {number}: no span at offset {at}"));
        assert_eq!(span.source_line, number, "`{}`: mapped to line {}", line.trim(), span.source_line);
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn sub_before_the_main_program() {
    assert_lines(&[
        "var n = 0",
        "# comment",
        "sub flash()",
        "  @",
        "  @",
        "end sub",
        "",
        "for n = 1 to 3",
        "  flash()",
        "  @",
        "next",
        "while 1",
        "  @",
        "wend",
    ]);
}

#[test]
fn sub_after_the_main_program() {
    assert_lines(&["var n = 0", "for n = 1 to 3", "  flash()", "  @", "next", "", "sub flash()", "  @", "end sub"]);
}

#[test]
fn subs_and_functions_between_main_statements() {
    assert_lines(&[
        "@",
        "sub a()",
        "  @",
        "end sub",
        "@",
        "function f(x as byte) as byte",
        "  @",
        "  return x + 1",
        "end function",
        "var v = f(1)",
        "a()",
        "@",
        "sub b()",
        "  if v = 2 then",
        "    @",
        "  else",
        "    @",
        "  end if",
        "end sub",
        "b()",
        "@",
    ]);
}

#[test]
fn nested_blocks_and_select() {
    assert_lines(&[
        "sub s(k as byte)",
        "  select case k",
        "    case 1",
        "      @",
        "    case 2, 3",
        "      @",
        "    case else",
        "      @",
        "  end select",
        "end sub",
        "var i = 0",
        "s(i)",
        "do",
        "  @",
        "  i = i + 1",
        "  if i = 2 then",
        "    @",
        "  elseif i = 3 then",
        "    @",
        "  end if",
        "loop until i = 4",
        "@",
    ]);
}

#[test]
fn statements_that_print_several_items_stay_in_step() {
    assert_lines(&[
        "sub p()",
        "  print \"A\"; \"B\"; \"C\"",
        "  @",
        "end sub",
        "print \"X\"; \"Y\"",
        "@",
        "p()",
        "print using \"##.#\"; 1.5",
        "@",
    ]);
}
