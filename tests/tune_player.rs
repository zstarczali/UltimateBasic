// The `tune` player against the Visual Assembler SID editor's engine: the
// fixture tune was exported by the editor's UltimateBasic exporter, the trace
// holds the SID registers ($D400-$D418) its engine (= the editor's exported
// player) writes after every frame. Regenerate both with
//   node <VisualAssembler>/scripts/sid-ub-fixture.js tests/fixtures

mod common;
use common::cpu6502::{compile_src, Cpu};
use ultimate_basic::compiler::{compile, CompileOptions};

const TUNE_BASE: u16 = 0x8000;

fn sid_trace(src: &str, frames: usize) -> Vec<[u8; 25]> {
    let res = compile_src(src);
    let mut cpu = Cpu::new(&res.prg);
    cpu.pc = TUNE_BASE; // sid_init
    let budget = cpu.steps + 100_000;
    cpu.run(budget);
    (0..frames)
        .map(|_| {
            cpu.pc = TUNE_BASE + 3; // sid_play, once per frame
            let budget = cpu.steps + 100_000;
            cpu.run(budget);
            let mut regs = [0u8; 25];
            regs.copy_from_slice(&cpu.mem[0xD400..0xD419]);
            regs
        })
        .collect()
}

#[test]
fn tune_player_matches_the_sid_editor_engine_frame_by_frame() {
    let src = include_str!("fixtures/sid_tune.ub");
    let expected: Vec<Vec<u8>> = include_str!("fixtures/sid_tune_trace.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect())
        .collect();
    let got = sid_trace(src, expected.len());
    for (frame, (g, e)) in got.iter().zip(&expected).enumerate() {
        assert_eq!(&g[..], &e[..], "SID registers differ at frame {}", frame);
    }
}

#[test]
fn instrument_modulations_move_the_registers() {
    let regs = sid_trace(include_str!("fixtures/sid_tune.ub"), 200);
    let distinct = |f: &dyn Fn(&[u8; 25]) -> u16, from: usize, to: usize| {
        let mut v: Vec<u16> = regs[from..to].iter().map(f).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    assert!(distinct(&|r| r[9] as u16 | (r[10] as u16) << 8, 10, 60) > 20, "PWM sweeps voice 2");
    assert!(distinct(&|r| (r[0x15] & 7) as u16 | (r[0x16] as u16) << 3, 5, 120) > 20, "filter cutoff sweep");
    assert!(regs.iter().any(|r| r[4] == 0x09), "first-frame waveform $09");
}

#[test]
fn tune_rejects_bad_table_steps_and_line_shapes() {
    let bad = "tune\n  inst 0, $41, $09, $F0, $0800, $0000, $00, $0F\n  itab 0, 1, 255, \"zz +1\"\n  order 0\n  pat 0, 0, \"C-4 00\"\nend\n";
    assert!(!compile(bad, &CompileOptions { basic_stub: false, explicit: false }).errors.is_empty());
    let bad = "tune\n  inst 0, $41, $09, $F0, $0800, $0000, $00, $0F\n  imod 0, 1, 2\n  order 0\n  pat 0, 0, \"C-4 00\"\nend\n";
    assert!(!compile(bad, &CompileOptions { basic_stub: false, explicit: false }).errors.is_empty());
}
