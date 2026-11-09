// `palette` (1.6.7): the Ultimate 64 / C64U palette through the Ultimate Command Interface.
// The programs run on the 6502 emulator against a model of the interface ($DF1C/$DF1D).

mod common;
use common::cpu6502::{Cpu, Uci};
use common::cpu6502::{code_of, compile_src};
use ultimate_basic::compiler::{compile, compile_with_path, CompileOptions, CompileResult};

const OPTS: CompileOptions = CompileOptions { basic_stub: false, explicit: false, basic_print: false };

fn run_with_uci(res: &CompileResult, state: u8) -> Cpu {
    let mut cpu = Cpu::new(&res.prg);
    cpu.uci = Some(Uci { state, ..Uci::default() });
    cpu.run(2_000_000);
    cpu
}

fn commands(src: &str) -> Vec<Vec<u8>> {
    run_with_uci(&compile_src(src), 0).uci.unwrap().commands
}

fn errors(src: &str) -> Vec<String> {
    compile(src, &OPTS).errors
}

fn vpl() -> String {
    (0..16).map(|i| format!("# colour {i}\n{:02X} {:02X} {:02X} 0\n", i * 16, 255 - i, i)).collect()
}

#[test]
fn reset_and_a_constant_colour() {
    assert_eq!(commands("palette reset"), vec![vec![0x04, 0x54]]);
    assert_eq!(commands("palette 6, $3050C0"), vec![vec![0x04, 0x53, 6, 0x30, 0x50, 0xC0]]);
    assert_eq!(commands("palette 0, $FF00"), vec![vec![0x04, 0x53, 0, 0x00, 0xFF, 0x00]]);
    assert_eq!(commands("palette 15, 0"), vec![vec![0x04, 0x53, 15, 0, 0, 0]]);
    assert_eq!(commands("palette 1, 200, 100, 50"), vec![vec![0x04, 0x53, 1, 200, 100, 50]]);
}

#[test]
fn run_time_index_and_components() {
    let src = "var level = 0\nvar i = 0\nfor i = 0 to 15\n  level = i * 16\n  palette i, level, 255 - level, 7\nnext\n";
    let cmds = commands(src);
    assert_eq!(cmds.len(), 16);
    for (i, c) in cmds.iter().enumerate() {
        let level = (i * 16) as u8;
        assert_eq!(c, &vec![0x04, 0x53, i as u8, level, 255 - level, 7], "colour {i}");
    }
    // the index is masked to 0-15
    assert_eq!(commands("var k = 18\npalette k, $102030"), vec![vec![0x04, 0x53, 2, 0x10, 0x20, 0x30]]);
}

#[test]
fn a_palette_file() {
    let dir = std::env::temp_dir().join(format!("ub-u64-palette-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sunset.vpl"), vpl()).unwrap();
    let res = compile_with_path("palette \"sunset.vpl\"\n", &OPTS, Some(&dir.join("main.ub")));
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let cmds = run_with_uci(&res, 0).uci.unwrap().commands;
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].len(), 50);
    assert_eq!(&cmds[0][..2], &[0x04, 0x52]);
    assert_eq!(&cmds[0][2 + 3 * 5..2 + 3 * 6], &[0x50, 250, 5]);

    std::fs::write(dir.join("short.vpl"), "00 00 00\n").unwrap();
    let bad = compile_with_path("palette \"short.vpl\"\n", &OPTS, Some(&dir.join("main.ub")));
    assert!(bad.errors.iter().any(|e| e.contains("no 16 colours")), "{:?}", bad.errors);
    let missing = compile_with_path("palette \"none.vpl\"\n", &OPTS, Some(&dir.join("main.ub")));
    assert!(missing.errors.iter().any(|e| e.contains("cannot read")), "{:?}", missing.errors);
}

#[test]
fn leftover_data_is_accepted_and_no_interface_is_skipped() {
    // an earlier command left its answer unread: the routine accepts it first
    let res = compile_src("palette reset\nvar done = 1");
    let cpu = run_with_uci(&res, 0x20);
    assert_eq!(cpu.uci.unwrap().commands, vec![vec![0x04, 0x54]]);

    // a plain C64 (or VICE): $DF1D is not $C9, the routine skips itself
    let res = compile_src("palette 3, $FFFFFF\nvar done = 7");
    let mut cpu = Cpu::new(&res.prg);
    cpu.run(100_000);
    let zp = res.map.variables.iter().find(|v| v.name == "done").unwrap().addr();
    assert_eq!(cpu.mem[zp as usize], 7);
}

#[test]
fn the_routine_is_that_of_visualassembler() {
    // VA `.u64_palette_reset`: 61 bytes, the last ones BNE +2 and the command $04 $54
    let prg = compile_src("palette reset").prg;
    let load = u16::from_le_bytes([prg[0], prg[1]]);
    let code = &prg[2..];
    assert!(code.len() >= 61);
    let start = code.windows(3).position(|w| w == [0xAD, 0x1D, 0xDF]).unwrap();
    let routine = &code[start..start + 61];
    assert_eq!(routine, &ultimate_basic::compiler::u64pal::uci_bytes(load + start as u16, &[0x04, 0x54])[..]);
    // a constant colour needs no float runtime and no setup code
    let one = code_of("palette 1, $FFFFFF");
    assert!(one.len() < 75, "{} bytes", one.len());
}

#[test]
fn errors_are_reported() {
    assert!(errors("palette 16, $FFFFFF").iter().any(|e| e.contains("index 16")));
    assert!(errors("palette 1, 300, 0, 0").iter().any(|e| e.contains("red 300")));
    assert!(errors("palette 1, $1000000").iter().any(|e| e.contains("not $000000-$FFFFFF")));
    assert!(errors("palette 1").iter().any(|e| e.contains("palette reset")));
}

/// examples/u64_palette_demo.ub end to end: three keys, then one to end the fade.
#[test]
fn the_example_runs() {
    let path = std::path::Path::new("examples/u64_palette_demo.ub");
    let src = std::fs::read_to_string(path).unwrap();
    let res = compile_with_path(&src, &OPTS, Some(path));
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let mut cpu = Cpu::new(&res.prg);
    cpu.uci = Some(Uci::default());
    cpu.keys.extend([b' '; 4]);
    cpu.run(40_000_000);
    let cmds = cpu.uci.unwrap().commands;
    assert_eq!(cmds.len(), 3 + 128 + 1);
    assert_eq!(&cmds[0][..2], &[0x04, 0x52]);
    assert_eq!(&cmds[1][..2], &[0x04, 0x52]);
    assert_ne!(cmds[0], cmds[1]);
    assert_eq!(cmds[2], vec![0x04, 0x54]);
    assert_eq!(cmds[3 + 63], vec![0x04, 0x53, 0, 0, 63, 252]);
    assert_eq!(cmds[3 + 127], vec![0x04, 0x53, 0, 0, 0, 0]);
    assert_eq!(cmds[131], vec![0x04, 0x54]);
    // the bars: colour RAM of column 4 + 2c is colour c
    assert_eq!(cpu.mem[0xD800 + 5 * 40 + 4 + 2 * 9] & 15, 9);
}
