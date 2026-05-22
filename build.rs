// build.rs – generates assets/icon.ico
use std::fs;

fn main() {
    let _ = fs::create_dir_all("assets");
    let ico = make_ico();
    fs::write("assets/icon.ico", &ico).expect("Failed to write icon.ico");
}

fn make_ico() -> Vec<u8> {
    let w: u32 = 32;
    let h: u32 = 32;
    let bpp: u16 = 32;
    // Image data: BITMAPINFOHEADER(40) + pixels(w*h*4)
    let img_size: u32 = 40 + w * h * 4;
    let total_size: u32 = 6 + 16 + img_size;

    let mut b = Vec::with_capacity(total_size as usize);

    // ICO header
    b.extend_from_slice(&[0, 0, 1, 0, 1, 0]);

    // Entry
    b.push(w as u8);
    b.push(h as u8);
    b.push(0); b.push(0); // colors, reserved
    b.extend_from_slice(&1u16.to_le_bytes());  // planes
    b.extend_from_slice(&bpp.to_le_bytes());   // bpp
    b.extend_from_slice(&img_size.to_le_bytes());
    b.extend_from_slice(&22u32.to_le_bytes()); // offset

    // BITMAPINFOHEADER
    b.extend_from_slice(&40u32.to_le_bytes());    // hdr size
    b.extend_from_slice(&(w as i32).to_le_bytes()); // width
    b.extend_from_slice(&((h * 2) as i32).to_le_bytes()); // height (2x for ICO)
    b.extend_from_slice(&1u16.to_le_bytes());     // planes
    b.extend_from_slice(&bpp.to_le_bytes());      // bpp
    b.extend_from_slice(&0u32.to_le_bytes());     // compression
    b.extend_from_slice(&(w * h * 4).to_le_bytes()); // image size
    b.extend_from_slice(&0i32.to_le_bytes());     // x ppm
    b.extend_from_slice(&0i32.to_le_bytes());     // y ppm
    b.extend_from_slice(&0u32.to_le_bytes());     // colors used
    b.extend_from_slice(&0u32.to_le_bytes());     // colors important

    // Pixels: dark bg (#1a1a2e) with cyan "UB" in top-left
    let bg: [u8; 4] = [0x2e, 0x1a, 0x1a, 0xff]; // BGRA
    let fg: [u8; 4] = [0xa0, 0xc8, 0x00, 0xff]; // BGRA cyan

    // Simple "U" shape bitmap (32x32)
    let letter = [
        // U (columns 2-6, rows 3-12)
        (2,3,2,12), (3,3,3,11), (4,3,4,11), (5,3,5,11), (6,3,6,12),
        // B (columns 8-13, rows 3-12)
        (8,3,8,12), (9,3,9,12), (10,3,10,12), (11,3,11,7), (12,3,12,7), (13,3,13,6),
        (11,8,11,12), (12,8,12,12), (13,7,13,12),
    ];

    let mut pixels = vec![bg; (w * h) as usize];
    for &(x1, y1, x2, y2) in &letter {
        for x in x1..=x2 {
            for y in y1..=y2 {
                let idx = y as usize * w as usize + x as usize;
                if idx < pixels.len() {
                    pixels[idx] = fg;
                }
            }
        }
    }

    // Write top-down (ICO stores bottom-up for BMP DIB)
    for y in (0..h).rev() {
        for x in 0..w {
            let px = pixels[(y * w + x) as usize];
            b.extend_from_slice(&px);
        }
    }

    b
}