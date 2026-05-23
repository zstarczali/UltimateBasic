// Ultimate Basic – C64 BASIC compiler (CLI)
// Compiles .ub files to .prg / .d64
//
// Usage:
//   ultimate-basic build <input.ub> [--output <out.prg>] [--no-stub] [--d64 <disk.d64>]
//   ultimate-basic --help

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;

use ultimate_basic::compiler::{compile_with_path, CompileOptions, MemoryMap};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        print_help();
        return;
    }
    match args[1].as_str() {
        "build" => cmd_build(&args),
        _ => { eprintln!("Unknown: {}", args[1]); print_help(); process::exit(1); }
    }
}

fn print_help() {
    println!("Commodore Ultimate Basic – C64 BASIC compiler");
    println!();
    println!("Usage:");
    println!("  ultimate-basic build <input.ub> [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -o, --output <file>   Output .prg file (default: <input>.prg)");
    println!("  -v, --verbose         Show full ZP layout and code hex dump");
    println!("  --no-stub              Omit BASIC SYS stub (raw machine code at $0801)");
    println!("  --d64 <file>           Also produce a .d64 disk image");
    println!("  -h, --help             Show this help");
    println!();
    println!("Example:");
    println!("  ultimate-basic build demo.ub -o demo.prg --d64 disk.d64");
}

fn cmd_build(args: &[String]) {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut basic_stub = true;
    let mut verbose = false;
    let mut d64_out: Option<PathBuf> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => { i += 1; if i < args.len() { output = Some(args[i].clone().into()); } }
            "--verbose" | "-v" => verbose = true,
            "--no-stub" => basic_stub = false,
            "--d64" => { i += 1; if i < args.len() { d64_out = Some(args[i].clone().into()); } }
            a if !a.starts_with('-') && input.is_none() => input = Some(a.to_string().into()),
            _ => { eprintln!("Unknown option: {}", args[i]); process::exit(1); }
        }
        i += 1;
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("Error: no input file specified.");
        eprintln!("Usage: ultimate-basic build <input.ub> [OPTIONS]");
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

    println!("  {} -> {} ({} bytes, BASIC stub: {})",
        input.file_name().unwrap_or_default().to_string_lossy(),
        output_path.display(),
        result.prg.len(),
        if basic_stub { "yes" } else { "no" }
    );

    print_memory_map(&result.map, verbose);

    if let Some(d64_path) = d64_out {
        make_d64(&d64_path, "ULTIMATE BASIC", &result.prg);
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
            println!("    {:<16} ZP:${:02X}   {}", var.name, var.zp_addr, var.type_str);
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
            println!("    {:<16} ${:04X}   {} bytes", arr.name, arr.base_addr, arr.size);
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
            Some(zp) => println!("    line helper  ZP:${:02X}-{:02X}", zp, zp.wrapping_add(11)),
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

/// Minimal D64 disk image with a single PRG file.
fn make_d64(path: &PathBuf, _label: &str, prg: &[u8]) {
    let total_blocks = 683;
    let bytes_per_block = 256;
    let mut disk = vec![0u8; total_blocks * bytes_per_block];

    // BAM: track 18, sector 0
    let bam_off = (18 - 1) * 21 * bytes_per_block;
    disk[bam_off] = 18;     // directory track
    disk[bam_off + 1] = 1;  // directory sector
    disk[bam_off + 2] = 0x41; // 'A' DOS version

    // Fill BAM: track 1-17 (21 sectors), 18 (19 sectors with dir), 19+ free
    for t in 0..17 {
        let p = bam_off + 144 + t * 4;
        disk[p] = 21; disk[p+1] = 0xFF; disk[p+2] = 0xFF; disk[p+3] = 0x1F;
    }
    {
        let p = bam_off + 144 + 17 * 4;
        disk[p] = 19; disk[p+1] = 0xFC; disk[p+2] = 0xFF; disk[p+3] = 0x07;
    }
    for t in 18..35 {
        let p = bam_off + 144 + t * 4;
        let n = if t < 24 { 19 } else if t < 30 { 18 } else { 17 };
        let mask = if t < 24 { 0x07 } else if t < 30 { 0x03 } else { 0x01 };
        disk[p] = n; disk[p+1] = 0xFF; disk[p+2] = 0xFF; disk[p+3] = mask;
    }

    // Directory: track 18, sector 1
    let dir_off = (18 - 1) * 21 * bytes_per_block + bytes_per_block;
    let sectors = ((prg.len() + 253) / 254).max(1);

    disk[dir_off] = 0x82;      // PRG, closed
    disk[dir_off + 1] = 17;    // first track
    disk[dir_off + 2] = 0;     // first sector
    let name = b"DEMO";
    for (i, &b) in name.iter().enumerate() { disk[dir_off + 3 + i] = b; }
    for i in name.len()..16 { disk[dir_off + 3 + i] = 0xA0; }
    disk[dir_off + 28] = sectors as u8;
    disk[dir_off + 29] = (sectors >> 8) as u8;

    // Write PRG data starting at track 17, sector 0
    const SEC_PER_TRACK: usize = 21;
    let _data_off = (17 - 1) * SEC_PER_TRACK * bytes_per_block;
    let mut po = 0usize;
    for s in 0..sectors {
        let track: usize = if s < SEC_PER_TRACK { 17 } else { 16 };
        let sector: usize = if s < SEC_PER_TRACK { s } else { s - SEC_PER_TRACK };
        let so = (track - 1) * SEC_PER_TRACK * bytes_per_block + sector * bytes_per_block;

        disk[so] = if s + 1 < sectors { track as u8 } else { 0 };
        disk[so + 1] = if s + 1 < sectors { (sector + 1) as u8 } else { 0 };

        let n = (prg.len() - po).min(254);
        disk[so + 2..so + 2 + n].copy_from_slice(&prg[po..po + n]);
        po += n;
    }

    fs::write(path, &disk).unwrap_or_else(|e| eprintln!("D64: {e}"));
    println!("  D64  -> {}", path.display());
}