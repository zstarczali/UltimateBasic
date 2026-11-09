//! `palette`: the Ultimate 64 / C64U video palette through the Ultimate Command Interface
//! ($DF1C status / control, $DF1D command / identification `$C9`; firmware 3.15+).
//! Control target $04: $52 + 16 x R,G,B sets all colours, $53 idx,R,G,B one, $54 the menu palette.
//! The routine and the palette-file reader are byte for byte those of VisualAssembler / VMA
//! (`_u64UciBytes`, `_u64PalColors`; VMA `src/u64pal.rs`).

/// Size of the routine before its command bytes.
pub const UCI_CODE: usize = 59;

/// The routine at `addr` followed by `data`: skip without an interface ($DF1D != $C9), wait for
/// idle (accepting data left by an earlier command), write the command, push it, wait while
/// busy, accept the answer. A and X are destroyed.
pub fn uci_bytes(addr: u16, data: &[u8]) -> Vec<u8> {
    let len = data.len() as u8;
    let at = addr.wrapping_add(UCI_CODE as u16);
    let mut out = vec![
        0xAD, 0x1D, 0xDF, 0xC9, 0xC9, 0xD0, 52 + len,
        0xAD, 0x1C, 0xDF, 0x29, 0x30, 0xF0, 0x0B, 0x29, 0x20, 0xF0, 0xF5,
        0xA9, 0x02, 0x8D, 0x1C, 0xDF, 0xD0, 0xEE,
        0xA2, 0x00, 0xBD, at as u8, (at >> 8) as u8, 0x8D, 0x1D, 0xDF, 0xE8, 0xE0, len, 0xD0, 0xF5,
        0xA9, 0x01, 0x8D, 0x1C, 0xDF,
        0xAD, 0x1C, 0xDF, 0x29, 0x30, 0xC9, 0x10, 0xF0, 0xF7,
        0xA9, 0x02, 0x8D, 0x1C, 0xDF, 0xD0, len,
    ];
    out.extend_from_slice(data);
    out
}

/// Command bytes of `palette "f"`.
pub fn palette_data(colors: &[u32]) -> Vec<u8> {
    let mut data = vec![0x04, 0x52];
    for c in colors {
        data.extend([(c >> 16) as u8, (c >> 8) as u8, *c as u8]);
    }
    data
}

/// The 16 colours of a palette file: VICE .vpl, GIMP .gpl, text with 16 `#RRGGBB` / `$RRGGBB` /
/// `0xRRGGBB` values, or a Photoshop .act (binary, 768 bytes or more).
pub fn palette_colors(bytes: &[u8]) -> Option<Vec<u32>> {
    if bytes.is_empty() {
        return None;
    }
    let text: Vec<char> = bytes.iter().map(|&b| b as char).collect();
    parse_text(&text).or_else(|| if bytes.len() >= 768 { parse_act(bytes) } else { None })
}

// JS `\s` over Latin-1 text
fn js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}')
}

fn hex_val(c: char) -> Option<u32> {
    c.to_digit(16).filter(|_| c.is_ascii())
}

/// `^\s*(\d{1,3})\s+(\d{1,3})\s+(\d{1,3})(?:\s|$)` (GIMP) or, after `#…` is cut and the line trimmed,
/// `^([0-9a-f]{1,2})\s+…(?:\s|$)` (VICE); `max` = digits per field, `radix` 10 / 16.
fn triple(line: &[char], radix: u32, max: usize) -> Option<[u32; 3]> {
    let mut i = 0;
    let mut out = [0u32; 3];
    for (k, v) in out.iter_mut().enumerate() {
        if k > 0 {
            let s = i;
            while i < line.len() && js_space(line[i]) {
                i += 1;
            }
            if i == s {
                return None;
            }
        }
        let s = i;
        while i < line.len() && i - s < max && line[i].is_ascii() && line[i].to_digit(radix).is_some() {
            i += 1;
        }
        if i == s {
            return None;
        }
        // a longer run of digits does not match `{1,max}` followed by \s or the end
        *v = line[s..i].iter().fold(0, |a, c| a * radix + c.to_digit(radix).unwrap());
    }
    (i == line.len() || js_space(line[i])).then_some(out)
}

fn parse_text(text: &[char]) -> Option<Vec<u32>> {
    let joined: String = text.iter().filter(|&&c| c != '\r').collect();
    let lines: Vec<Vec<char>> = joined.split('\n').map(|l| l.chars().collect()).collect();
    let mut out = Vec::new();
    let first: Vec<char> = lines[0].iter().copied().skip_while(|&c| js_space(c)).collect();
    if first.len() >= 12 && first[..12].iter().zip("GIMP Palette".chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b)) {
        for line in &lines {
            let start = line.iter().position(|&c| !js_space(c)).unwrap_or(line.len());
            if let Some([r, g, b]) = triple(&line[start..], 10, 3) {
                out.push((r.min(255) << 16) | (g.min(255) << 8) | b.min(255));
            }
        }
        return (out.len() >= 16).then(|| out[..16].to_vec());
    }
    // (?:#|\$|0x)[0-9a-f]{6}\b, case-insensitive, over the whole text
    let mut hex6 = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let pre = if text[i] == '#' || text[i] == '$' {
            1
        } else if text[i] == '0' && i + 1 < text.len() && (text[i + 1] == 'x' || text[i + 1] == 'X') {
            2
        } else {
            0
        };
        if pre > 0 && i + pre + 6 <= text.len() && text[i + pre..i + pre + 6].iter().all(|&c| hex_val(c).is_some()) {
            let end = i + pre + 6;
            if end == text.len() || !(text[end].is_ascii_alphanumeric() || text[end] == '_') {
                hex6.push(text[i + pre..end].iter().fold(0, |a, &c| a * 16 + hex_val(c).unwrap()));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    if hex6.len() >= 16 {
        return Some(hex6[..16].to_vec());
    }
    for line in &lines {
        let cut = line.iter().position(|&c| c == '#').unwrap_or(line.len());
        let body = &line[..cut];
        let s = body.iter().position(|&c| !js_space(c)).unwrap_or(body.len());
        let e = body.iter().rposition(|&c| !js_space(c)).map_or(s, |p| p + 1);
        if let Some([r, g, b]) = triple(&body[s..e.max(s)], 16, 2) {
            out.push((r << 16) | (g << 8) | b);
        }
    }
    (out.len() >= 16).then(|| out[..16].to_vec())
}

fn parse_act(bytes: &[u8]) -> Option<Vec<u32>> {
    (bytes.len() >= 48).then(|| (0..16).map(|i| ((bytes[i * 3] as u32) << 16) | ((bytes[i * 3 + 1] as u32) << 8) | bytes[i * 3 + 2] as u32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(text: &str) -> Option<Vec<u32>> {
        palette_colors(text.as_bytes())
    }

    #[test]
    fn vice_gimp_hex_and_act_palettes() {
        let vpl: String = (0..16).map(|i| format!("# c{i}\n{:02X} {:02x} {:X} 4\n", i, 255 - i, i)).collect();
        assert_eq!(colors(&vpl).unwrap()[15], 0x0FF00F);
        let gpl: String = "GIMP Palette\nName: x\n#\n".to_string() + &(0..16).map(|i| format!("{} 300 {}\tname\n", i, i * 2)).collect::<String>();
        assert_eq!(colors(&gpl).unwrap()[3], 0x03FF06);
        let hex: String = (0..16).map(|i| format!("$1020{:02X}, ", i)).collect();
        assert_eq!(colors(&hex).unwrap()[9], 0x102009);
        assert_eq!(colors("$1234567 not a colour"), None);
        assert_eq!(colors("AB CD"), None);
        let mut act = vec![0u8; 772];
        act[3..6].copy_from_slice(&[1, 2, 3]);
        assert_eq!(palette_colors(&act).unwrap()[1], 0x010203);
    }

    #[test]
    fn the_routine_matches_visualassembler() {
        let b = uci_bytes(0x1000, &[0x04, 0x54]);
        assert_eq!(b.len(), UCI_CODE + 2);
        assert_eq!(&b[27..30], &[0xBD, 0x3B, 0x10]);
        assert_eq!(b[6], 54);
        assert_eq!(&b[57..], &[0xD0, 2, 0x04, 0x54]);
        assert_eq!(palette_data(&[0x123456; 16]).len(), 50);
    }
}
