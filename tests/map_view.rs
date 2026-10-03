// `map view` (1.6.1): run programs on the 6502 emulator (raster from the cycle count)
// and check that the shown buffer, color RAM and fine scroll match the map.

mod common;
use common::cpu6502::Cpu;
use ultimate_basic::compiler::{compile_with_path, CompileOptions, CompileResult};

const OPTS: CompileOptions = CompileOptions { basic_stub: false, explicit: false };

/// A VisualAssembler Level Editor project: `screens` library screens of 2000 bytes
/// (1000 screen codes + 1000 colors), a w x h grid of slots and the VA metadata trailer.
fn level_file(w: usize, h: usize, cells: &[i32], screens: usize) -> Vec<u8> {
    let mut file = Vec::new();
    for s in 0..screens {
        file.extend((0..1000).map(|i| ((s * 37 + i * 7) & 0xFF) as u8));
        file.extend((0..1000).map(|i| ((s * 5 + i) % 16) as u8));
    }
    let cells: Vec<String> = cells.iter().map(|c| c.to_string()).collect();
    let manifest = format!(
        r#"{{"version":1,"kind":"lv-level","baseLength":{},"settings":{{"layout":"grid","w":{},"h":{},"cells":[{}],"multicolor":true,"bgColor":6,"mc1Color":5,"mc2Color":13,"charSource":"rom"}},"extras":[]}}"#,
        screens * 2000,
        w,
        h,
        cells.join(",")
    );
    file.extend_from_slice(manifest.as_bytes());
    file.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
    file.extend_from_slice(b"VA-BIN1!");
    file
}

fn compile_in_dir(tag: &str, src: &str, level: &[u8]) -> CompileResult {
    let dir = std::env::temp_dir().join(format!("ub-map-view-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("level.bin"), level).unwrap();
    compile_with_path(src, &OPTS, Some(&dir.join("main.ub")))
}

/// The composed map of `level_file(w, h, cells, ..)`: (chars, colors), row-major.
fn composed(w: usize, h: usize, cells: &[i32]) -> (Vec<u8>, Vec<u8>) {
    let (cols, rows) = (w * 40, h * 25);
    let mut chars = vec![0x20u8; cols * rows];
    let mut colors = vec![14u8; cols * rows];
    for (slot, &s) in cells.iter().enumerate() {
        if s < 0 {
            continue;
        }
        let s = s as usize;
        let (ox, oy) = ((slot % w) * 40, (slot / w) * 25);
        for r in 0..25 {
            for c in 0..40 {
                let i = r * 40 + c;
                chars[(oy + r) * cols + ox + c] = ((s * 37 + i * 7) & 0xFF) as u8;
                colors[(oy + r) * cols + ox + c] = ((s * 5 + i) % 16) as u8;
            }
        }
    }
    (chars, colors)
}

/// The shown view must be the map at pixel position (px, py).
fn assert_view(cpu: &Cpu, map: &(Vec<u8>, Vec<u8>), cols: usize, px: usize, py: usize) {
    let screen = match cpu.mem[0xD018] & 0xF0 {
        0x10 => 0x0400,
        0xF0 => 0x3C00,
        other => panic!("unexpected $D018 screen bits ${other:02X}"),
    };
    let (cx, cy) = (px / 8, py / 8);
    for r in 0..25 {
        for c in 0..40 {
            let m = (cy + r) * cols + cx + c;
            assert_eq!(cpu.mem[screen + r * 40 + c], map.0[m], "char row {r} col {c} at {px},{py}");
            assert_eq!(cpu.mem[0xD800 + r * 40 + c], map.1[m], "color row {r} col {c} at {px},{py}");
        }
    }
    assert_eq!(cpu.mem[0xD016] & 0x0F, 7 - (px & 7) as u8, "XSCROLL / 38 columns");
    assert_eq!(cpu.mem[0xD011] & 0x0F, 7 - (py & 7) as u8, "YSCROLL / 24 rows");
    assert_eq!(cpu.mem[0xD016] & 0x10, 0x10, "multicolor kept");
}

fn run_moves(tag: &str, dx: i32, dy: i32, frames: u32, start: (u32, u32)) -> (Cpu, usize, usize) {
    let (w, h) = (3, 3);
    let cells = [0, 1, 2, 3, 4, 5, 6, 7, 8];
    let level = level_file(w, h, &cells, 9);
    let step = |d: i32| if d < 0 { format!("- {}", -d) } else { format!("+ {d}") };
    let src = format!(
        "map load \"level.bin\", $4000\nvar px: word = {}\nvar py: word = {}\nvar n = 0\nfor n = 1 to {}\n  map view px, py\n  px = px {}\n  py = py {}\nnext\nfor n = 1 to 6\n  map view px, py\nnext\n",
        start.0,
        start.1,
        frames,
        step(dx),
        step(dy)
    );
    let res = compile_in_dir(tag, &src, &level);
    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let mut cpu = Cpu::new(&res.prg);
    cpu.run(40_000_000);
    let px = (start.0 as i32 + dx * frames as i32) as usize;
    let py = (start.1 as i32 + dy * frames as i32) as usize;
    (cpu, px, py)
}

#[test]
fn map_view_moving_right_and_down_shows_the_map() {
    let (cpu, px, py) = run_moves("rd", 3, 2, 60, (0, 0));
    assert_view(&cpu, &composed(3, 3, &[0, 1, 2, 3, 4, 5, 6, 7, 8]), 120, px, py);
}

#[test]
fn map_view_moving_left_and_up_past_256_pixels() {
    let (cpu, px, py) = run_moves("lu", -5, -3, 40, (300, 200));
    assert_view(&cpu, &composed(3, 3, &[0, 1, 2, 3, 4, 5, 6, 7, 8]), 120, px, py);
}

#[test]
fn map_view_one_pixel_steps_cross_char_cells() {
    let (cpu, px, py) = run_moves("one", 1, 1, 21, (5, 6));
    assert_view(&cpu, &composed(3, 3, &[0, 1, 2, 3, 4, 5, 6, 7, 8]), 120, px, py);
}

#[test]
fn map_view_clamps_to_the_map() {
    // 3x3 screens: x max (120 - 40) * 8 = 640, y max (75 - 25) * 8 = 400
    let (cpu, _, _) = run_moves("clamp", 50, 50, 20, (0, 0));
    assert_view(&cpu, &composed(3, 3, &[0, 1, 2, 3, 4, 5, 6, 7, 8]), 120, 640, 400);
}

#[test]
fn map_view_copies_sprite_pointers_to_the_second_buffer() {
    let (cpu, _, _) = run_moves("spr", 1, 0, 3, (0, 0));
    assert_eq!(&cpu.mem[0x3FF8..0x4000], &cpu.mem[0x07F8..0x0800]);
}

#[test]
fn map_load_composes_a_level_editor_project() {
    let cells = [1, -1, 0, 1];
    let level = level_file(2, 2, &cells, 2);
    let res = compile_in_dir("compose", "map load \"level.bin\"\nmap draw 40, 25\n", &level);
    assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
    let mut cpu = Cpu::new(&res.prg);
    cpu.run(2_000_000);
    let (chars, colors) = composed(2, 2, &cells);
    for r in 0..25 {
        for c in 0..40 {
            assert_eq!(cpu.mem[0x0400 + r * 40 + c], chars[(25 + r) * 80 + 40 + c]);
            assert_eq!(cpu.mem[0xD800 + r * 40 + c], colors[(25 + r) * 80 + 40 + c]);
        }
    }
    assert_eq!(cpu.mem[0xD021], 6);
    assert_eq!(cpu.mem[0xD016] & 0x10, 0x10);
}

#[test]
fn map_view_errors() {
    let small = level_file(1, 1, &[0], 1);
    let res = compile_in_dir("noload", "var x: word = 0\nmap view x, x\n", &small);
    assert!(res.errors.iter().any(|e| e.contains("map view requires a map load")), "{:?}", res.errors);
    // 2x5 screens inline = 20000 bytes of data from ~$0820: reaches $3C00
    let big = level_file(2, 5, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 1);
    let res = compile_in_dir("overlap", "map load \"level.bin\"\nmap view 0, 0\n", &big);
    assert!(res.errors.iter().any(|e| e.contains("$3C00-$3FFF")), "{:?}", res.errors);
    // the same map at $4000 is fine
    let res = compile_in_dir("placed", "map load \"level.bin\", $4000\nmap view 0, 0\n", &big);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    // a level larger than 255x255 cells
    let wide = level_file(7, 1, &[0; 7], 1);
    let res = compile_in_dir("wide", "map load \"level.bin\"\n", &wide);
    assert!(res.errors.iter().any(|e| e.contains("255x255")), "{:?}", res.errors);
}
