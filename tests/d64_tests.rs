// D64 image tests: build programs with `ub build ... --d64 ... --add ...` and read the image
// back like the 1541 DOS does (BAM, 32-byte directory entries, sector chains).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn spt(t: usize) -> usize {
    match t {
        1..=17 => 21,
        18..=24 => 19,
        25..=30 => 18,
        _ => 17,
    }
}

fn sector(d: &[u8], t: usize, s: usize) -> &[u8] {
    let off: usize = (1..t).map(spt).sum::<usize>() + s;
    &d[off * 256..off * 256 + 256]
}

struct Entry {
    name: String,
    blocks: usize,
    data: Vec<u8>,
    chain: Vec<(usize, usize)>,
}

fn read_dir(d: &[u8]) -> Vec<Entry> {
    assert_eq!(d.len(), 174_848, "35-track D64");
    let bam = sector(d, 18, 0);
    assert_eq!((bam[0], bam[1], bam[2]), (18, 1, 0x41), "BAM header");
    let mut out = vec![];
    let (mut t, mut s) = (18usize, 1usize);
    loop {
        let sec = sector(d, t, s);
        for i in 0..8 {
            let e = &sec[i * 32..i * 32 + 32];
            if e[2] == 0 {
                continue;
            }
            assert_eq!(e[2], 0x82, "closed PRG");
            let name: String = e[5..21]
                .iter()
                .take_while(|&&c| c != 0xA0)
                .map(|&c| c as char)
                .collect();
            let blocks = e[30] as usize | (e[31] as usize) << 8;
            let (mut ft, mut fs) = (e[3] as usize, e[4] as usize);
            let mut data = vec![];
            let mut chain = vec![];
            while ft != 0 {
                chain.push((ft, fs));
                let b = sector(d, ft, fs);
                if b[0] != 0 {
                    data.extend_from_slice(&b[2..]);
                } else {
                    data.extend_from_slice(&b[2..=b[1] as usize]);
                }
                ft = b[0] as usize;
                fs = b[1] as usize;
            }
            out.push(Entry { name, blocks, data, chain });
        }
        if sec[0] == 0 {
            break;
        }
        t = sec[0] as usize;
        s = sec[1] as usize;
    }
    out
}

/// every sector used by a file chain or the directory is allocated in the BAM, and nothing else
fn check_bam(d: &[u8], entries: &[Entry], dir_secs: usize) {
    let bam = sector(d, 18, 0);
    let mut used = std::collections::HashSet::new();
    used.insert((18usize, 0usize));
    for s in 1..=dir_secs {
        used.insert((18, s));
    }
    for e in entries {
        for &ts in &e.chain {
            assert!(used.insert(ts), "sector {ts:?} used twice");
        }
    }
    for t in 1..=35usize {
        let p = 4 + (t - 1) * 4;
        let bits = bam[p + 1] as u32 | (bam[p + 2] as u32) << 8 | (bam[p + 3] as u32) << 16;
        let mut free = 0;
        for s in 0..spt(t) {
            let is_free = bits >> s & 1 == 1;
            assert_eq!(is_free, !used.contains(&(t, s)), "BAM bit track {t} sector {s}");
            if is_free {
                free += 1;
            }
        }
        assert_eq!(bam[p], free, "BAM free count track {t}");
    }
}

fn workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ub_d64_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ub(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ub"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run ub")
}

fn pseudo(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed;
    let mut v = vec![0x01, 0x08];
    for _ in 0..n {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        v.push((x >> 16) as u8);
    }
    v
}

#[test]
fn d64_with_added_file_has_both_files_intact() {
    let dir = workdir("two");
    fs::write(dir.join("boot.ub"), "print \"HELLO\"\n").unwrap();
    let data = pseudo(37_000, 1);
    fs::write(dir.join("data.prg"), &data).unwrap();
    let out = ub(&dir, &["build", "boot.ub", "--d64", "disk.d64", "--add", "data.prg"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let img = fs::read(dir.join("disk.d64")).unwrap();
    let entries = read_dir(&img);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "BOOT");
    assert_eq!(entries[0].data, fs::read(dir.join("boot.prg")).unwrap());
    assert_eq!(entries[1].name, "DATA", "the 2nd entry used to be shifted by 2 bytes");
    assert_eq!(entries[1].data, data);
    for e in &entries {
        assert_eq!(e.blocks, e.chain.len(), "block count of {}", e.name);
    }
    check_bam(&img, &entries, 1);
    // disk ID "UB", DOS type "2A" at their standard places
    let bam = sector(&img, 18, 0);
    assert_eq!(&bam[0xA2..0xA7], &[b'U', b'B', 0xA0, b'2', b'A']);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn d64_with_many_files_uses_second_directory_sector() {
    let dir = workdir("many");
    fs::write(dir.join("main.ub"), "bye\n").unwrap();
    let mut args = vec!["build", "main.ub", "--d64", "disk.d64"];
    let names: Vec<String> = (1..=10).map(|i| format!("f{i}.prg")).collect();
    for (i, n) in names.iter().enumerate() {
        fs::write(dir.join(n), pseudo(i * 1000 + 1, i as u32)).unwrap();
    }
    for n in &names {
        args.push("--add");
        args.push(n);
    }
    let out = ub(&dir, &args);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let img = fs::read(dir.join("disk.d64")).unwrap();
    let entries = read_dir(&img);
    assert_eq!(entries.len(), 11);
    for (i, n) in names.iter().enumerate() {
        let e = &entries[i + 1];
        assert_eq!(e.name, format!("F{}", i + 1));
        assert_eq!(e.data, fs::read(dir.join(n)).unwrap(), "{n}");
    }
    check_bam(&img, &entries, 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn d64_uses_tracks_above_the_directory() {
    // 250 blocks + the program do not fit on tracks 1-17 (357 blocks) together with a
    // second 250-block file: the allocator must continue on track 19 and up
    let dir = workdir("big");
    fs::write(dir.join("main.ub"), "bye\n").unwrap();
    let a = pseudo(250 * 254, 7);
    let b = pseudo(250 * 254, 8);
    fs::write(dir.join("a.prg"), &a).unwrap();
    fs::write(dir.join("b.prg"), &b).unwrap();
    let out = ub(&dir, &["build", "main.ub", "--d64", "disk.d64", "--add", "a.prg", "--add", "b.prg"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let img = fs::read(dir.join("disk.d64")).unwrap();
    let entries = read_dir(&img);
    assert_eq!(entries[1].data, a);
    assert_eq!(entries[2].data, b);
    assert!(entries[2].chain.iter().any(|&(t, _)| t > 18));
    check_bam(&img, &entries, 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn d64_disk_full_is_an_error() {
    let dir = workdir("full");
    fs::write(dir.join("main.ub"), "bye\n").unwrap();
    fs::write(dir.join("huge.prg"), pseudo(700 * 254, 3)).unwrap();
    let out = ub(&dir, &["build", "main.ub", "--d64", "disk.d64", "--add", "huge.prg"]);
    assert!(!out.status.success(), "a disk-full image must fail the build");
    assert!(String::from_utf8_lossy(&out.stderr).contains("disk full"));
    assert!(!dir.join("disk.d64").exists());
    let _ = fs::remove_dir_all(&dir);
}
