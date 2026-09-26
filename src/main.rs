// Ultimate Basic – C64 BASIC compiler (CLI)
// Compiles .ub files to .prg / .crt / .d64
//
// Usage:
//   ub build <input.ub> [--output <out.prg>] [--no-stub] [--d64 <disk.d64>]
//   ub --help

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use ultimate_basic::compiler::{
    build_magic_desk_crt, compile_with_path, debug_output, CompileOptions, MemoryMap,
};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        print_help();
        return;
    }
    match args[1].as_str() {
        "build" => cmd_build(&args),
        _ => {
            eprintln!("Unknown: {}", args[1]);
            print_help();
            process::exit(1);
        }
    }
}

fn print_help() {
    println!(
        "Commodore Ultimate Basic – C64 BASIC compiler v{}",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Usage:");
    println!("  ub build <input.ub> [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -o, --output <file>   Output file (.prg or .crt; default: <input>.prg)");
    println!("  -v, --verbose         Show full ZP layout and code hex dump");
    println!("  --no-stub              Omit BASIC SYS stub (raw machine code at $0801)");
    println!("  --d64 [file]           Also produce a .d64 disk image (default: <output>.d64)");
    println!("  --debug                Produce .sym, .dbg and .vs debugger files");
    println!("  --asm                  Produce a readable codegen .asm listing");
    println!("  --add <file>           Add extra file(s) to the .d64 image (repeatable)");
    println!("  --explicit             Require :type on every var / sub-param / fn-param");
    println!("  -h, --help             Show this help");
    println!();
    println!("Examples:");
    println!("  ub build demo.ub -o demo.prg --d64 disk.d64");
    println!("  ub build game.ub --d64 --add music.prg --add levels.prg");
}

fn cmd_build(args: &[String]) {
    println!("Ultimate Basic v{}", env!("CARGO_PKG_VERSION"));
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut basic_stub = true;
    let mut verbose = false;
    let mut d64_out: Option<PathBuf> = None;
    let mut extra_files: Vec<PathBuf> = Vec::new();
    let mut debug_files = false;
    let mut asm_file = false;
    let mut explicit = false;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                if i < args.len() {
                    output = Some(args[i].clone().into());
                }
            }
            "--verbose" | "-v" => verbose = true,
            "--no-stub" => basic_stub = false,
            "--debug" => debug_files = true,
            "--asm" => asm_file = true,
            "--explicit" => explicit = true,
            "--d64" => {
                // --d64           → auto: <output>.d64  (empty PathBuf as sentinel)
                // --d64 <file>    → explicit path
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    i += 1;
                    d64_out = Some(PathBuf::from(&args[i]));
                } else {
                    d64_out = Some(PathBuf::new()); // sentinel: resolved below
                }
            }
            "--add" => {
                i += 1;
                if i < args.len() {
                    extra_files.push(PathBuf::from(&args[i]));
                } else {
                    eprintln!("Error: --add requires a file argument");
                    process::exit(1);
                }
            }
            a if !a.starts_with('-') && input.is_none() => input = Some(a.to_string().into()),
            _ => {
                eprintln!("Unknown option: {}", args[i]);
                process::exit(1);
            }
        }
        i += 1;
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("Error: no input file specified.");
        eprintln!("Usage: ub build <input.ub> [OPTIONS]");
        process::exit(1);
    });

    if !input.exists() {
        eprintln!("Error: file not found: {}", input.display());
        process::exit(1);
    }

    let source = fs::read_to_string(&input).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {e}", input.display());
        process::exit(1);
    });

    let output_path = output.unwrap_or_else(|| input.with_extension("prg"));
    let is_crt_output = output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("crt"));

    let opts = CompileOptions {
        basic_stub,
        explicit,
    };
    let result = compile_with_path(&source, &opts, Some(&input));

    if !result.errors.is_empty() {
        eprintln!("Compilation errors:");
        for e in &result.errors {
            eprintln!("  {e}");
        }
        process::exit(1);
    }

    let output_bytes = if is_crt_output {
        let cart_name = output_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        build_magic_desk_crt(&result.prg, &cart_name).unwrap_or_else(|e| {
            eprintln!("Error building {}: {e}", output_path.display());
            process::exit(1);
        })
    } else {
        result.prg.clone()
    };

    fs::write(&output_path, &output_bytes).unwrap_or_else(|e| {
        eprintln!("Error writing {}: {e}", output_path.display());
        process::exit(1);
    });

    let output_kind = if is_crt_output { "CRT" } else { "PRG" };

    println!(
        "  {} -> {} ({} bytes, {}, BASIC stub: {})",
        input.file_name().unwrap_or_default().to_string_lossy(),
        output_path.display(),
        output_bytes.len(),
        output_kind,
        if basic_stub { "yes" } else { "no" }
    );

    if debug_files {
        write_debug_file(
            &output_path.with_extension("sym"),
            &debug_output::sym(&result.map),
        );
        write_debug_file(
            &output_path.with_extension("dbg"),
            &debug_output::dbg(&result.map, &input),
        );
        write_debug_file(
            &output_path.with_extension("vs"),
            &debug_output::vice(&result.map),
        );
    }

    if asm_file {
        let asm_path = output_path.with_extension("asm");
        fs::write(&asm_path, &result.asm).unwrap_or_else(|e| {
            eprintln!("Error writing {}: {e}", asm_path.display());
            process::exit(1);
        });
        println!("  ASM:     {}", asm_path.display());
    }

    print_memory_map(&result.map, verbose);

    if let Some(d64_path) = d64_out {
        let d64_final = if d64_path.as_os_str().is_empty() {
            output_path.with_extension("d64")
        } else {
            d64_path
        };
        let prog_name = output_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_uppercase();
        let mut d64_files: Vec<(String, Vec<u8>)> = vec![(prog_name, result.prg.clone())];
        for f in &extra_files {
            match fs::read(f) {
                Ok(data) => {
                    let name = f
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_uppercase();
                    d64_files.push((name, data));
                }
                Err(e) => eprintln!("Warning: --add {}: {e}", f.display()),
            }
        }
        let refs: Vec<(&str, &[u8])> = d64_files
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        make_d64(&d64_final, "ULTIMATE BASIC", &refs);
    }
}

fn write_debug_file(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).unwrap_or_else(|e| {
        eprintln!("Error writing {}: {e}", path.display());
        process::exit(1);
    });
    println!("  Debug:   {}", path.display());
}

fn print_memory_map(map: &MemoryMap, verbose: bool) {
    let code_end = map
        .load_addr
        .wrapping_add(map.code_size.saturating_sub(1) as u16);

    println!();
    println!("  Load:    ${:04X} - ${:04X}", map.load_addr, code_end);
    println!("  Code:    {} bytes", map.code_size);

    println!();
    println!("  Variables (zero page):");
    if map.variables.is_empty() {
        println!("    (none)");
    } else {
        for var in &map.variables {
            println!(
                "    {:<16} ZP:${:02X}   {}",
                var.name, var.zp_addr, var.type_str
            );
        }
    }

    println!();
    println!("  Subroutines:");
    if map.subroutines.is_empty() {
        println!("    (none)");
    } else {
        for sub in &map.subroutines {
            println!("    {:<16} ${:04X}", sub.name, sub.addr);
        }
    }

    println!();
    println!("  Arrays ($C000+):");
    if map.arrays.is_empty() {
        println!("    (none)");
    } else {
        for arr in &map.arrays {
            println!(
                "    {:<16} ${:04X}   {} bytes",
                arr.name, arr.base_addr, arr.size
            );
        }
    }

    if !map.unused_vars.is_empty() {
        println!();
        println!("  Unused variables:");
        for name in &map.unused_vars {
            println!("    {}", name);
        }
    }

    if verbose {
        println!();
        println!("  Internal ZP:");
        match map.plot_zp {
            Some(zp) => println!("    plot helper  ZP:${:02X}-{:02X}", zp, zp.wrapping_add(5)),
            None => println!("    plot helper  (unused)"),
        }
        match map.line_zp {
            Some(zp) => println!(
                "    line helper  ZP:${:02X}-{:02X}",
                zp,
                zp.wrapping_add(11)
            ),
            None => println!("    line helper  (unused)"),
        }
        match map.sin_table_addr {
            Some(addr) => println!("    sin/cos table: ${:04X}-${:04X}", addr, addr + 255),
            None => println!("    sin/cos table (unused)"),
        }
        match map.data_zp {
            Some(zp) => println!("    data pointer ZP:${:02X}-{:02X}", zp, zp.wrapping_add(1)),
            None => println!("    data pointer (unused)"),
        }

        println!();
        println!("  Code Hex Dump:");
        print_hex_dump(map.load_addr, &map.code_bytes);
    }
}

fn print_hex_dump(start_addr: u16, bytes: &[u8]) {
    if bytes.is_empty() {
        println!("    (empty)");
        return;
    }

    for (i, chunk) in bytes.chunks(16).enumerate() {
        let addr = start_addr.wrapping_add((i * 16) as u16);
        print!("    ${:04X}: ", addr);
        for b in chunk {
            print!("{:02X} ", b);
        }
        println!();
    }
}

/// Build a D64 disk image containing one or more PRG files.
/// `files` is a list of (display_name, raw_bytes) pairs.
///
/// Layout as the 1541 DOS writes it: BAM at 18/0, directory from 18/1 (32-byte entries, 8 per
/// sector), file data on tracks 17..1 then 19..35 with the DOS interleave of 10 sectors.
fn make_d64(path: &PathBuf, disk_name: &str, files: &[(&str, &[u8])]) {
    match build_d64(disk_name, files) {
        Ok(disk) => {
            fs::write(path, &disk).unwrap_or_else(|e| {
                eprintln!("D64 write error: {e}");
                process::exit(1);
            });
        }
        Err(e) => {
            eprintln!("D64 error: {e}");
            process::exit(1);
        }
    }
    println!(
        "  D64  -> {} ({} file{})",
        path.display(),
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );
}

fn build_d64(disk_name: &str, files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    // Sectors per track for a standard 1541 disk
    fn sectors_for_track(t: usize) -> usize {
        match t {
            1..=17 => 21,
            18..=24 => 19,
            25..=30 => 18,
            _ => 17, // 31..=35
        }
    }
    // Byte offset of sector s (0-based) on track t (1-based) in the D64 image
    fn sec_off(t: usize, s: usize) -> usize {
        let mut off = 0usize;
        for i in 1..t {
            off += sectors_for_track(i);
        }
        (off + s) * 256
    }
    const INTERLEAVE: usize = 10; // 1541 DOS default for PRG files

    if files.len() > 144 {
        return Err(format!("too many files ({}), a D64 directory holds 144", files.len()));
    }

    // Standard 35-track 1541: 683 sectors, 174 848 bytes total
    let mut disk = vec![0u8; 683 * 256];
    let mut used = vec![vec![false; 21]; 36]; // [track][sector]

    // Track 18: BAM (sector 0) + directory sectors 1..=n_dir_secs
    let n_dir_secs = ((files.len() + 7) / 8).max(1);
    for s in 0..=n_dir_secs {
        used[18][s] = true;
    }

    // === Allocate data sectors: tracks 17 down to 1, then 19 up to 35, interleave 10 ===
    let track_order: Vec<usize> = (1..=17).rev().chain(19..=35).collect();
    let mut ti = 0usize; // index into track_order
    let mut next_sec = 0usize;
    let mut file_sectors: Vec<Vec<(usize, usize)>> = Vec::new();
    for (name, data) in files.iter() {
        let n_secs = ((data.len() + 253) / 254).max(1);
        let mut dsec: Vec<(usize, usize)> = Vec::with_capacity(n_secs);
        for _ in 0..n_secs {
            // find a free sector on the current track, from next_sec on; else the next track
            let found = loop {
                if ti >= track_order.len() {
                    break None;
                }
                let t = track_order[ti];
                let spt = sectors_for_track(t);
                if let Some(k) = (0..spt).find(|k| !used[t][(next_sec + k) % spt]) {
                    let s = (next_sec + k) % spt;
                    break Some((t, s));
                }
                ti += 1;
                next_sec = 0;
            };
            let Some((t, s)) = found else {
                return Err(format!("disk full while writing {name} (664 blocks max)"));
            };
            used[t][s] = true;
            dsec.push((t, s));
            next_sec = (s + INTERLEAVE) % sectors_for_track(t);
        }
        file_sectors.push(dsec);
    }

    // === Write file data into allocated sectors ===
    for (fi, (_name, data)) in files.iter().enumerate() {
        let dsec = &file_sectors[fi];
        let n_secs = dsec.len();
        let mut po = 0usize;
        for (i, &(t, s)) in dsec.iter().enumerate() {
            let o = sec_off(t, s);
            if i + 1 < n_secs {
                let (nt, ns) = dsec[i + 1];
                disk[o] = nt as u8;
                disk[o + 1] = ns as u8;
                disk[o + 2..o + 256].copy_from_slice(&data[po..po + 254]);
                po += 254;
            } else {
                let n = data.len() - po;
                disk[o] = 0;
                disk[o + 1] = (n + 1) as u8; // offset of the last data byte in the sector
                disk[o + 2..o + 2 + n].copy_from_slice(&data[po..]);
            }
        }
    }

    // === BAM block: track 18, sector 0 ===
    // [0]=dir track, [1]=dir sector, [2]=DOS ver 'A', [3]=0,
    // [4..8F]=BAM entries (4 bytes each for tracks 1-35),
    // [90..9F]=disk name, [A0..A1]=$A0, [A2..A3]=disk ID, [A4]=$A0, [A5..A6]="2A", [A7..AA]=$A0
    let bam = sec_off(18, 0);
    disk[bam] = 18; // first dir track
    disk[bam + 1] = 1; // first dir sector
    disk[bam + 2] = 0x41; // DOS version 'A'
    for t in 1usize..=35 {
        let spt = sectors_for_track(t);
        let p = bam + 4 + (t - 1) * 4;
        let mut free = 0u8;
        for s in 0..spt {
            if !used[t][s] {
                free += 1;
                disk[p + 1 + s / 8] |= 1u8 << (s % 8);
            }
        }
        disk[p] = free;
    }
    let dn: Vec<u8> = disk_name
        .bytes()
        .take(16)
        .map(|b| b.to_ascii_uppercase())
        .collect();
    for i in 0..16 {
        disk[bam + 0x90 + i] = dn.get(i).copied().unwrap_or(0xA0);
    }
    disk[bam + 0xA0] = 0xA0;
    disk[bam + 0xA1] = 0xA0;
    disk[bam + 0xA2] = b'U'; // disk ID "UB"
    disk[bam + 0xA3] = b'B';
    disk[bam + 0xA4] = 0xA0;
    disk[bam + 0xA5] = b'2'; // DOS type "2A"
    disk[bam + 0xA6] = b'A';
    for i in 0xA7..=0xAA {
        disk[bam + i] = 0xA0;
    }

    // === Directory sectors: track 18, sectors 1 … n_dir_secs ===
    // 8 entries of 32 bytes per sector; bytes 0-1 of the sector (= of the first entry) are the
    // link to the next directory sector.
    for ds in 0..n_dir_secs {
        let dir = sec_off(18, ds + 1);
        if ds + 1 < n_dir_secs {
            disk[dir] = 18;
            disk[dir + 1] = (ds + 2) as u8;
        } else {
            disk[dir] = 0;
            disk[dir + 1] = 0xFF;
        }
        for ei in 0..8usize {
            let fi = ds * 8 + ei;
            if fi >= files.len() {
                break;
            }
            let (name, _data) = &files[fi];
            let dsec = &file_sectors[fi];
            let n_secs = dsec.len();
            let de = dir + ei * 32;
            disk[de + 2] = 0x82; // PRG, closed
            disk[de + 3] = dsec[0].0 as u8; // first data track
            disk[de + 4] = dsec[0].1 as u8; // first data sector
            let pn: Vec<u8> = name
                .bytes()
                .take(16)
                .map(|b| b.to_ascii_uppercase())
                .collect();
            for i in 0..16 {
                disk[de + 5 + i] = pn.get(i).copied().unwrap_or(0xA0);
            }
            disk[de + 30] = n_secs as u8; // size in blocks, lo / hi
            disk[de + 31] = (n_secs >> 8) as u8;
        }
    }

    Ok(disk)
}
