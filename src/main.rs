// Ultimate Basic – C64 BASIC compiler (CLI)
// Compiles .ub files to .prg / .d64
//
// Usage:
//   ub build <input.ub> [--output <out.prg>] [--no-stub] [--d64 <disk.d64>]
//   ub --help

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use ultimate_basic::compiler::{CompileOptions, MemoryMap, compile_with_path};

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
    println!("Commodore Ultimate Basic – C64 BASIC compiler");
    println!();
    println!("Usage:");
    println!("  ub build <input.ub> [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -o, --output <file>   Output .prg file (default: <input>.prg)");
    println!("  -v, --verbose         Show full ZP layout and code hex dump");
    println!("  --no-stub              Omit BASIC SYS stub (raw machine code at $0801)");
    println!("  --d64 [file]           Also produce a .d64 disk image (default: <output>.d64)");
    println!("  --add <file>           Add extra file(s) to the .d64 image (repeatable)");
    println!("  -h, --help             Show this help");
    println!();
    println!("Examples:");
    println!("  ub build demo.ub -o demo.prg --d64 disk.d64");
    println!("  ub build game.ub --d64 --add music.prg --add levels.prg");
}

fn cmd_build(args: &[String]) {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut basic_stub = true;
    let mut verbose = false;
    let mut d64_out: Option<PathBuf> = None;
    let mut extra_files: Vec<PathBuf> = Vec::new();

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

    let opts = CompileOptions { basic_stub };
    let result = compile_with_path(&source, &opts, Some(&input));

    if !result.errors.is_empty() {
        eprintln!("Compilation errors:");
        for e in &result.errors {
            eprintln!("  {e}");
        }
        process::exit(1);
    }

    fs::write(&output_path, &result.prg).unwrap_or_else(|e| {
        eprintln!("Error writing {}: {e}", output_path.display());
        process::exit(1);
    });

    println!(
        "  {} -> {} ({} bytes, BASIC stub: {})",
        input.file_name().unwrap_or_default().to_string_lossy(),
        output_path.display(),
        result.prg.len(),
        if basic_stub { "yes" } else { "no" }
    );

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
fn make_d64(path: &PathBuf, disk_name: &str, files: &[(&str, &[u8])]) {
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

    // Standard 35-track 1541: 683 sectors, 174 848 bytes total
    let mut disk = vec![0u8; 683 * 256];

    // === Allocate data sectors for each file (track 17 down to 1) ===
    let mut alloc_track = 17usize;
    let mut alloc_sec = 0usize;
    let mut file_sectors: Vec<Vec<(usize, usize)>> = Vec::new();
    for (_name, data) in files.iter() {
        let n_secs = ((data.len() + 253) / 254).max(1);
        let mut dsec: Vec<(usize, usize)> = Vec::with_capacity(n_secs);
        for _ in 0..n_secs {
            dsec.push((alloc_track, alloc_sec));
            alloc_sec += 1;
            if alloc_sec >= sectors_for_track(alloc_track) {
                alloc_sec = 0;
                if alloc_track > 1 {
                    alloc_track -= 1;
                } else {
                    eprintln!("D64 error: disk full");
                    break;
                }
            }
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
                disk[o + 1] = (n + 1) as u8; // 1-based offset of last data byte
                disk[o + 2..o + 2 + n].copy_from_slice(&data[po..]);
            }
        }
    }

    // === BAM block: track 18, sector 0 ===
    // [0]=dir track, [1]=dir sector, [2]=DOS ver,
    // [4..8F]=BAM entries (4 bytes each for tracks 1-35),
    // [90..9F]=disk name, [A0..A4]=disk ID + DOS type
    let bam = sec_off(18, 0);
    disk[bam] = 18; // first dir track
    disk[bam + 1] = 1; // first dir sector
    disk[bam + 2] = 0x41; // DOS version 'A'

    // BAM entries — all-free initially
    for t in 1usize..=35 {
        let nsec = sectors_for_track(t) as u8;
        let (b1, b2, b3): (u8, u8, u8) = match nsec {
            21 => (0xFF, 0xFF, 0x1F),
            19 => (0xFF, 0xFF, 0x07),
            18 => (0xFF, 0xFF, 0x03),
            _ => (0xFF, 0xFF, 0x01), // 17
        };
        let p = bam + 4 + (t - 1) * 4;
        disk[p] = nsec;
        disk[p + 1] = b1;
        disk[p + 2] = b2;
        disk[p + 3] = b3;
    }

    // Number of directory sectors needed on track 18 (8 entries per sector)
    let n_dir_secs = ((files.len() + 7) / 8).max(1);

    // Mark track 18: sector 0 (BAM) + sectors 1..=n_dir_secs (directory) used
    {
        let used = 1 + n_dir_secs;
        let p = bam + 4 + 17 * 4; // track 18 BAM entry
        disk[p] -= used as u8;
        for s in 0..used {
            disk[p + 1 + s / 8] &= !(1u8 << (s % 8));
        }
    }
    // Mark all file data sectors as used
    for dsec in &file_sectors {
        for &(t, s) in dsec {
            let p = bam + 4 + (t - 1) * 4;
            disk[p] -= 1;
            disk[p + 1 + s / 8] &= !(1u8 << (s % 8));
        }
    }

    // Disk name (16 bytes, padded 0xA0) at bam+0x90
    let dn: Vec<u8> = disk_name
        .bytes()
        .take(16)
        .map(|b| b.to_ascii_uppercase())
        .collect();
    for i in 0..16 {
        disk[bam + 0x90 + i] = dn.get(i).copied().unwrap_or(0xA0);
    }
    disk[bam + 0xA0] = b'U'; // disk ID
    disk[bam + 0xA1] = b'B';
    disk[bam + 0xA2] = 0xA0;
    disk[bam + 0xA3] = 0x32; // '2'
    disk[bam + 0xA4] = 0x41; // 'A'
    for i in 5..=10usize {
        disk[bam + 0xA0 + i] = 0xA0;
    }

    // === Directory sectors: track 18, sectors 1 … n_dir_secs ===
    // Each sector: bytes 0-1 = chain link, then 8 x 30-byte entries
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
            let de = dir + 2 + ei * 30;
            disk[de] = 0x82; // PRG, closed
            disk[de + 1] = dsec[0].0 as u8; // first data track
            disk[de + 2] = dsec[0].1 as u8; // first data sector
            let pn: Vec<u8> = name
                .bytes()
                .take(16)
                .map(|b| b.to_ascii_uppercase())
                .collect();
            for i in 0..16 {
                disk[de + 3 + i] = pn.get(i).copied().unwrap_or(0xA0);
            }
            disk[de + 28] = n_secs as u8;
            disk[de + 29] = (n_secs >> 8) as u8;
        }
    }

    fs::write(path, &disk).unwrap_or_else(|e| eprintln!("D64 write error: {e}"));
    println!(
        "  D64  -> {} ({} file{})",
        path.display(),
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );
}
