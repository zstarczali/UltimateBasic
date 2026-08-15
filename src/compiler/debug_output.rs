//! Debugger symbol-file exporters used by the command-line compiler.

use std::collections::BTreeMap;
use std::path::Path;

use super::MemoryMap;

fn clean_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    if result.is_empty() || result.as_bytes()[0].is_ascii_digit() {
        result.insert(0, '_');
    }
    result
}

fn symbols(map: &MemoryMap) -> BTreeMap<String, u16> {
    let mut result = BTreeMap::new();
    result.insert("program_start".into(), map.load_addr);
    result.insert(
        "program_end".into(),
        map.load_addr.wrapping_add(map.code_size as u16),
    );
    for var in &map.variables {
        result.insert(clean_name(&var.name), var.zp_addr as u16);
    }
    for array in &map.arrays {
        result.insert(clean_name(&array.name), array.base_addr);
    }
    for sub in &map.subroutines {
        result.insert(clean_name(&sub.name), sub.addr);
    }
    for label in &map.labels {
        result.insert(clean_name(&label.name), label.addr);
    }
    result
}

/// KickAssembler-compatible importable symbol source.
pub fn sym(map: &MemoryMap) -> String {
    symbols(map)
        .into_iter()
        .map(|(name, addr)| format!(".label {name} = ${addr:04x}\n"))
        .collect()
}

/// VICE monitor command file, loadable with `-moncommands` or `ll`.
pub fn vice(map: &MemoryMap) -> String {
    symbols(map)
        .into_iter()
        .map(|(name, addr)| format!("al C:{addr:04x} .{name}\n"))
        .collect()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// C64Debugger/RetroDebugger KickAssembler debug-dump format.
///
/// UltimateBasic does not yet retain instruction-to-source-line mappings, so
/// this export provides the code segment and all address labels.
pub fn dbg(map: &MemoryMap, source_path: &Path) -> String {
    let start = map.load_addr;
    let end = start.wrapping_add(map.code_size.saturating_sub(1) as u16);
    let source = xml_escape(&source_path.to_string_lossy());
    let mut out = format!(
        "<C64debugger version=\"1.0\">\n  <Sources values=\"INDEX,FILE\">\n    0,{source}\n  </Sources>\n  <Segment name=\"UltimateBasic\" values=\"START,END,FILE_IDX,LINE1,COL1,LINE2,COL2\">\n    {start:04x},{end:04x},0,1,1,1,1\n  </Segment>\n  <Labels values=\"SEGMENT,ADDRESS,NAME\">\n"
    );
    for (name, addr) in symbols(map) {
        out.push_str(&format!(
            "    UltimateBasic,{addr:04x},{}\n",
            xml_escape(&name)
        ));
    }
    out.push_str("  </Labels>\n  <Breakpoints values=\"SEGMENT,ADDRESS,ARGUMENT\">\n  </Breakpoints>\n  <Watches values=\"SEGMENT,ADDRESS,SIZE,FORMAT\">\n  </Watches>\n</C64debugger>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{ArrayEntry, LabelEntry, SubEntry, VarEntry};

    fn map() -> MemoryMap {
        MemoryMap {
            load_addr: 0x080d,
            code_size: 3,
            variables: vec![VarEntry {
                name: "score".into(),
                zp_addr: 0x02,
                type_str: "int".into(),
            }],
            subroutines: vec![SubEntry {
                name: "tick".into(),
                addr: 0x0810,
            }],
            labels: vec![LabelEntry {
                name: "main-loop".into(),
                addr: 0x0812,
            }],
            arrays: vec![ArrayEntry {
                name: "data".into(),
                base_addr: 0xc000,
                size: 8,
            }],
            plot_zp: None,
            line_zp: None,
            rect_zp: None,
            sin_table_addr: None,
            data_zp: None,
            code_bytes: vec![],
            unused_vars: vec![],
            listing_spans: vec![],
            listing_symbols: vec![],
            data_regions: vec![],
        }
    }

    #[test]
    fn exports_expected_symbol_syntaxes() {
        let map = map();
        assert!(sym(&map).contains(".label main_loop = $0812"));
        assert!(vice(&map).contains("al C:0810 .tick"));
        let debug = dbg(&map, Path::new("demo&test.ub"));
        assert!(debug.contains("0,demo&amp;test.ub"));
        assert!(debug.contains("UltimateBasic,0002,score"));
    }
}
