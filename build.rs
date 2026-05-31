// build.rs – combines assets/ultimate_basic_*.ico → assets/icon.ico + sets Windows file properties
use std::fs;

fn main() {
    let sizes = [16u32, 32, 48, 64, 128, 256, 512];
    let paths: Vec<String> = sizes
        .iter()
        .map(|s| format!("assets/ultimate_basic_{s}.ico"))
        .collect();
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();

    let combined = combine_icos(&path_refs);
    assert!(!combined.is_empty(), "No icon files found in assets/");
    fs::write("assets/icon.ico", &combined).expect("Failed to write combined icon.ico");

    // Re-run if any source icon changes
    for p in &paths {
        println!("cargo:rerun-if-changed={p}");
    }

    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set(
            "FileDescription",
            "Ultimate Basic – C64/C64 Ultimate BASIC compiler",
        );
        res.set("ProductName", "Ultimate Basic");
        res.set("CompanyName", "Zsolt Tarczali");
        res.set("LegalCopyright", "Copyright \u{00A9} 2026 Zsolt Tarczali");
        res.set("FileVersion", env!("CARGO_PKG_VERSION"));
        res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        res.set_icon("assets/icon.ico");
        res.compile().expect("Failed to compile Windows resources");
    }
}

/// Reads multiple single-size .ico files and combines them into one multi-resolution .ico.
fn combine_icos(paths: &[&str]) -> Vec<u8> {
    struct Entry {
        width: u8,
        height: u8,
        color_count: u8,
        planes: u16,
        bit_count: u16,
        data: Vec<u8>,
    }

    let mut entries: Vec<Entry> = Vec::new();

    for path in paths {
        let raw = match fs::read(path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        if raw.len() < 22 {
            continue;
        }
        // ICO directory entry starts at byte 6
        let e = &raw[6..22];
        let data_size = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let data_offset = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        if data_offset + data_size > raw.len() {
            continue;
        }
        entries.push(Entry {
            width: e[0],
            height: e[1],
            color_count: e[2],
            planes: u16::from_le_bytes([e[4], e[5]]),
            bit_count: u16::from_le_bytes([e[6], e[7]]),
            data: raw[data_offset..data_offset + data_size].to_vec(),
        });
    }

    if entries.is_empty() {
        return vec![];
    }

    let n = entries.len();
    let dir_offset = 6 + n * 16; // header(6) + n × dir_entry(16)
    let mut ico: Vec<u8> = Vec::new();

    // ICO file header
    ico.extend_from_slice(&[0, 0, 1, 0]); // reserved, type=1
    ico.extend_from_slice(&(n as u16).to_le_bytes()); // image count

    // Directory entries
    let mut img_offset = dir_offset as u32;
    for e in &entries {
        ico.push(e.width);
        ico.push(e.height);
        ico.push(e.color_count);
        ico.push(0); // reserved
        ico.extend_from_slice(&e.planes.to_le_bytes());
        ico.extend_from_slice(&e.bit_count.to_le_bytes());
        ico.extend_from_slice(&(e.data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&img_offset.to_le_bytes());
        img_offset += e.data.len() as u32;
    }

    // Image data blocks
    for e in &entries {
        ico.extend_from_slice(&e.data);
    }

    ico
}
