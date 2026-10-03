// Full (official-opcode) 6502 emulator used by semantic codegen tests.
//
// Programs are loaded from a PRG image and run until the main program's final
// RTS. KERNAL / BASIC ROM calls (JSR/JMP into $A000-$BFFF or $E000-$FFFF) are
// trapped: CHROUT ($FFD2) appends A to `output`, everything else returns
// immediately with A unchanged (GETIN returns 0).

#![allow(dead_code)]

use ultimate_basic::compiler::{compile, CompileOptions, CompileResult};

pub const CHROUT: u16 = 0xFFD2;
pub const GETIN: u16 = 0xFFE4;

pub struct Cpu {
    pub mem: Vec<u8>,
    pub pc: u16,
    pub sp: u8,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub c: bool,
    pub z: bool,
    pub i: bool,
    pub d: bool,
    pub v: bool,
    pub n: bool,
    pub cycles: u64,
    pub steps: u64,
    pub output: Vec<u8>,
    call_depth: usize,
}

impl Cpu {
    pub fn new(prg: &[u8]) -> Self {
        let mut mem = vec![0u8; 65536];
        let load = u16::from_le_bytes([prg[0], prg[1]]);
        let start = load as usize;
        mem[start..start + prg.len() - 2].copy_from_slice(&prg[2..]);
        // With a BASIC stub (`SYS 2061`) the machine code starts at $080D.
        let pc = if prg.len() > 7 && prg[6] == 0x9E { 0x080D } else { load };
        Cpu {
            mem,
            pc,
            sp: 0xFF,
            a: 0,
            x: 0,
            y: 0,
            c: false,
            z: false,
            i: false,
            d: false,
            v: false,
            n: false,
            cycles: 0,
            steps: 0,
            output: Vec::new(),
            call_depth: 0,
        }
    }

    /// Run until the top-level RTS. Panics if `max_steps` is exceeded.
    pub fn run(&mut self, max_steps: u64) {
        while self.step() {
            if self.steps > max_steps {
                panic!("CPU exceeded step budget at ${:04X}", self.pc);
            }
        }
    }

    /// Alias of `run` used by the older integration tests.
    pub fn run_until_main_rts(&mut self, max_steps: usize) {
        self.run(max_steps as u64)
    }

    fn is_rom(addr: u16) -> bool {
        (0xA000..0xC000).contains(&addr) || addr >= 0xE000
    }

    fn rom_call(&mut self, addr: u16) {
        match addr {
            CHROUT => self.output.push(self.a),
            GETIN => {
                self.a = 0;
                self.set_zn(0);
            }
            _ => {}
        }
        self.c = false;
    }

    /// PAL raster line derived from the cycle count (63 cycles x 312 lines).
    pub fn raster_line(&self) -> u16 {
        ((self.cycles / 63) % 312) as u16
    }
    fn rd(&self, addr: u16) -> u8 {
        match addr {
            0xD012 => self.raster_line() as u8,
            0xD011 => (self.mem[0xD011] & 0x7F) | if self.raster_line() > 255 { 0x80 } else { 0 },
            _ => self.mem[addr as usize],
        }
    }
    fn wr(&mut self, addr: u16, v: u8) {
        self.mem[addr as usize] = v;
    }
    fn fetch(&mut self) -> u8 {
        let b = self.rd(self.pc);
        self.pc = self.pc.wrapping_add(1);
        b
    }
    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch();
        let hi = self.fetch();
        u16::from_le_bytes([lo, hi])
    }
    fn rd16_zp(&self, zp: u8) -> u16 {
        u16::from_le_bytes([self.rd(zp as u16), self.rd(zp.wrapping_add(1) as u16)])
    }
    fn push(&mut self, v: u8) {
        self.wr(0x0100 | self.sp as u16, v);
        self.sp = self.sp.wrapping_sub(1);
    }
    fn pop(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.rd(0x0100 | self.sp as u16)
    }
    fn set_zn(&mut self, v: u8) {
        self.z = v == 0;
        self.n = v & 0x80 != 0;
    }
    fn status(&self, b: bool) -> u8 {
        (self.n as u8) << 7
            | (self.v as u8) << 6
            | 0x20
            | (b as u8) << 4
            | (self.d as u8) << 3
            | (self.i as u8) << 2
            | (self.z as u8) << 1
            | self.c as u8
    }
    fn set_status(&mut self, p: u8) {
        self.n = p & 0x80 != 0;
        self.v = p & 0x40 != 0;
        self.d = p & 0x08 != 0;
        self.i = p & 0x04 != 0;
        self.z = p & 0x02 != 0;
        self.c = p & 0x01 != 0;
    }

    fn adc(&mut self, m: u8) {
        let sum = self.a as u16 + m as u16 + self.c as u16;
        let r = sum as u8;
        self.v = (!(self.a ^ m) & (self.a ^ r) & 0x80) != 0;
        self.c = sum > 0xFF;
        self.a = r;
        self.set_zn(r);
    }
    fn sbc(&mut self, m: u8) {
        self.adc(!m);
    }
    fn cmp(&mut self, r: u8, m: u8) {
        self.c = r >= m;
        self.set_zn(r.wrapping_sub(m));
    }
    fn branch(&mut self, take: bool) {
        let off = self.fetch() as i8;
        if take {
            self.pc = self.pc.wrapping_add_signed(off as i16);
            self.cycles += 1;
        }
    }

    /// Effective address for the addressing-mode group of `op`.
    /// Returns None for immediate / accumulator / implied.
    fn ea(&mut self, mode: Mode) -> u16 {
        match mode {
            Mode::Zp => self.fetch() as u16,
            Mode::ZpX => self.fetch().wrapping_add(self.x) as u16,
            Mode::ZpY => self.fetch().wrapping_add(self.y) as u16,
            Mode::Abs => self.fetch16(),
            Mode::AbsX => self.fetch16().wrapping_add(self.x as u16),
            Mode::AbsY => self.fetch16().wrapping_add(self.y as u16),
            Mode::IndX => {
                let zp = self.fetch().wrapping_add(self.x);
                self.rd16_zp(zp)
            }
            Mode::IndY => {
                let zp = self.fetch();
                self.rd16_zp(zp).wrapping_add(self.y as u16)
            }
            Mode::Imm => {
                let a = self.pc;
                self.pc = self.pc.wrapping_add(1);
                a
            }
        }
    }

    /// Execute one instruction. Returns false on the top-level RTS.
    pub fn step(&mut self) -> bool {
        self.steps += 1;
        let op = self.fetch();
        use Mode::*;
        // Common ALU groups (cc=01): ORA AND EOR ADC STA LDA CMP SBC
        if op & 0x03 == 0x01 {
            let mode = match (op >> 2) & 7 {
                0 => IndX,
                1 => Zp,
                2 => Imm,
                3 => Abs,
                4 => IndY,
                5 => ZpX,
                6 => AbsY,
                _ => AbsX,
            };
            let addr = self.ea(mode);
            self.cycles += 3;
            match op >> 5 {
                0 => {
                    self.a |= self.rd(addr);
                    self.set_zn(self.a)
                }
                1 => {
                    self.a &= self.rd(addr);
                    self.set_zn(self.a)
                }
                2 => {
                    self.a ^= self.rd(addr);
                    self.set_zn(self.a)
                }
                3 => self.adc(self.rd(addr)),
                4 => {
                    if mode == Imm {
                        panic!("illegal opcode ${:02X}", op);
                    }
                    self.wr(addr, self.a)
                }
                5 => {
                    self.a = self.rd(addr);
                    self.set_zn(self.a)
                }
                6 => self.cmp(self.a, self.rd(addr)),
                _ => self.sbc(self.rd(addr)),
            }
            return true;
        }

        self.cycles += 2;
        match op {
            // ── shifts / rotates / inc / dec on memory ──
            0x06 | 0x16 | 0x0E | 0x1E | 0x26 | 0x36 | 0x2E | 0x3E | 0x46 | 0x56 | 0x4E
            | 0x5E | 0x66 | 0x76 | 0x6E | 0x7E | 0xC6 | 0xD6 | 0xCE | 0xDE | 0xE6 | 0xF6
            | 0xEE | 0xFE => {
                let mode = match (op >> 2) & 7 {
                    1 => Zp,
                    3 => Abs,
                    5 => ZpX,
                    _ => AbsX,
                };
                let addr = self.ea(mode);
                let m = self.rd(addr);
                let r = match op >> 5 {
                    0 => {
                        self.c = m & 0x80 != 0;
                        m << 1
                    }
                    1 => {
                        let r = (m << 1) | self.c as u8;
                        self.c = m & 0x80 != 0;
                        r
                    }
                    2 => {
                        self.c = m & 1 != 0;
                        m >> 1
                    }
                    3 => {
                        let r = (m >> 1) | (self.c as u8) << 7;
                        self.c = m & 1 != 0;
                        r
                    }
                    6 => m.wrapping_sub(1),
                    _ => m.wrapping_add(1),
                };
                self.wr(addr, r);
                self.set_zn(r);
                self.cycles += 3;
            }
            0x0A => {
                self.c = self.a & 0x80 != 0;
                self.a <<= 1;
                self.set_zn(self.a)
            }
            0x2A => {
                let r = (self.a << 1) | self.c as u8;
                self.c = self.a & 0x80 != 0;
                self.a = r;
                self.set_zn(r)
            }
            0x4A => {
                self.c = self.a & 1 != 0;
                self.a >>= 1;
                self.set_zn(self.a)
            }
            0x6A => {
                let r = (self.a >> 1) | (self.c as u8) << 7;
                self.c = self.a & 1 != 0;
                self.a = r;
                self.set_zn(r)
            }
            // ── loads / stores X, Y ──
            0xA2 | 0xA6 | 0xB6 | 0xAE | 0xBE => {
                let mode = match op {
                    0xA2 => Imm,
                    0xA6 => Zp,
                    0xB6 => ZpY,
                    0xAE => Abs,
                    _ => AbsY,
                };
                let a = self.ea(mode);
                self.x = self.rd(a);
                self.set_zn(self.x)
            }
            0xA0 | 0xA4 | 0xB4 | 0xAC | 0xBC => {
                let mode = match op {
                    0xA0 => Imm,
                    0xA4 => Zp,
                    0xB4 => ZpX,
                    0xAC => Abs,
                    _ => AbsX,
                };
                let a = self.ea(mode);
                self.y = self.rd(a);
                self.set_zn(self.y)
            }
            0x86 | 0x96 | 0x8E => {
                let mode = match op {
                    0x86 => Zp,
                    0x96 => ZpY,
                    _ => Abs,
                };
                let a = self.ea(mode);
                self.wr(a, self.x)
            }
            0x84 | 0x94 | 0x8C => {
                let mode = match op {
                    0x84 => Zp,
                    0x94 => ZpX,
                    _ => Abs,
                };
                let a = self.ea(mode);
                self.wr(a, self.y)
            }
            // ── compares X / Y, BIT ──
            0xE0 | 0xE4 | 0xEC | 0xC0 | 0xC4 | 0xCC => {
                let mode = match op & 0x0C {
                    0x00 => Imm,
                    0x04 => Zp,
                    _ => Abs,
                };
                let a = self.ea(mode);
                let m = self.rd(a);
                let r = if op >= 0xE0 { self.x } else { self.y };
                self.cmp(r, m)
            }
            0x24 | 0x2C => {
                let mode = if op == 0x24 { Zp } else { Abs };
                let a = self.ea(mode);
                let m = self.rd(a);
                self.z = self.a & m == 0;
                self.n = m & 0x80 != 0;
                self.v = m & 0x40 != 0;
            }
            // ── transfers / register inc-dec ──
            0xAA => {
                self.x = self.a;
                self.set_zn(self.x)
            }
            0x8A => {
                self.a = self.x;
                self.set_zn(self.a)
            }
            0xA8 => {
                self.y = self.a;
                self.set_zn(self.y)
            }
            0x98 => {
                self.a = self.y;
                self.set_zn(self.a)
            }
            0xBA => {
                self.x = self.sp;
                self.set_zn(self.x)
            }
            0x9A => self.sp = self.x,
            0xE8 => {
                self.x = self.x.wrapping_add(1);
                self.set_zn(self.x)
            }
            0xCA => {
                self.x = self.x.wrapping_sub(1);
                self.set_zn(self.x)
            }
            0xC8 => {
                self.y = self.y.wrapping_add(1);
                self.set_zn(self.y)
            }
            0x88 => {
                self.y = self.y.wrapping_sub(1);
                self.set_zn(self.y)
            }
            // ── flags ──
            0x18 => self.c = false,
            0x38 => self.c = true,
            0x58 => self.i = false,
            0x78 => self.i = true,
            0xB8 => self.v = false,
            0xD8 => self.d = false,
            0xF8 => self.d = true,
            0xEA => {}
            // ── stack ──
            0x48 => self.push(self.a),
            0x68 => {
                self.a = self.pop();
                self.set_zn(self.a)
            }
            0x08 => {
                let p = self.status(true);
                self.push(p)
            }
            0x28 => {
                let p = self.pop();
                self.set_status(p)
            }
            // ── branches ──
            0x10 => self.branch(!self.n),
            0x30 => self.branch(self.n),
            0x50 => self.branch(!self.v),
            0x70 => self.branch(self.v),
            0x90 => self.branch(!self.c),
            0xB0 => self.branch(self.c),
            0xD0 => self.branch(!self.z),
            0xF0 => self.branch(self.z),
            // ── jumps ──
            0x4C => {
                let t = self.fetch16();
                if Self::is_rom(t) {
                    // JMP into ROM behaves like a tail call: run the trap, then RTS.
                    self.rom_call(t);
                    return self.do_rts();
                }
                self.pc = t;
                self.cycles += 1;
            }
            0x6C => {
                let p = self.fetch16();
                // 6502 page-wrap bug
                let hi_addr = (p & 0xFF00) | (p.wrapping_add(1) & 0x00FF);
                self.pc = u16::from_le_bytes([self.rd(p), self.rd(hi_addr)]);
            }
            0x20 => {
                let t = self.fetch16();
                self.cycles += 4;
                if Self::is_rom(t) {
                    self.rom_call(t);
                } else {
                    let ret = self.pc.wrapping_sub(1);
                    self.push((ret >> 8) as u8);
                    self.push(ret as u8);
                    self.pc = t;
                    self.call_depth += 1;
                }
            }
            0x60 => return self.do_rts(),
            0x40 => {
                let p = self.pop();
                self.set_status(p);
                let lo = self.pop();
                let hi = self.pop();
                self.pc = u16::from_le_bytes([lo, hi]);
            }
            _ => panic!(
                "unsupported/illegal opcode ${:02X} at ${:04X}",
                op,
                self.pc.wrapping_sub(1)
            ),
        }
        true
    }

    fn do_rts(&mut self) -> bool {
        self.cycles += 4;
        if self.call_depth == 0 {
            return false;
        }
        let lo = self.pop();
        let hi = self.pop();
        self.pc = u16::from_le_bytes([lo, hi]).wrapping_add(1);
        self.call_depth -= 1;
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Imm,
    Zp,
    ZpX,
    ZpY,
    Abs,
    AbsX,
    AbsY,
    IndX,
    IndY,
}

/// A compiled + executed program, with helpers to read variables back.
pub struct Run {
    pub res: CompileResult,
    pub cpu: Cpu,
}

impl Run {
    pub fn var_zp(&self, name: &str) -> u8 {
        self.res
            .map
            .variables
            .iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| panic!("no variable '{}'", name))
            .zp_addr
    }
    pub fn byte(&self, name: &str) -> u8 {
        self.cpu.mem[self.var_zp(name) as usize]
    }
    pub fn word(&self, name: &str) -> u16 {
        let zp = self.var_zp(name) as usize;
        u16::from_le_bytes([self.cpu.mem[zp], self.cpu.mem[zp + 1]])
    }
    pub fn mem(&self, addr: u16) -> u8 {
        self.cpu.mem[addr as usize]
    }
    pub fn output(&self) -> String {
        self.cpu.output.iter().map(|&b| b as char).collect()
    }
    /// Size of the generated machine code in bytes (without PRG header).
    pub fn code_size(&self) -> usize {
        self.res.prg.len() - 2
    }
}

pub fn compile_src(src: &str) -> CompileResult {
    let res = compile(
        src,
        &CompileOptions {
            basic_stub: false,
            explicit: false,
        },
    );
    assert!(
        res.errors.is_empty(),
        "compile errors: {:?}\n--- source ---\n{}",
        res.errors,
        src
    );
    res
}

pub fn run_src(src: &str) -> Run {
    run_src_budget(src, 5_000_000)
}

pub fn run_src_budget(src: &str, max_steps: u64) -> Run {
    let res = compile_src(src);
    let mut cpu = Cpu::new(&res.prg);
    cpu.run(max_steps);
    Run { res, cpu }
}

/// Machine code (without PRG header) of a snippet.
pub fn code_of(src: &str) -> Vec<u8> {
    compile_src(src).prg[2..].to_vec()
}

pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}
