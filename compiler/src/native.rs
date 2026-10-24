//! Compile Droide native functions to arm64-v8a and x86_64 ELF shared libraries.
//!
//! The language is i32-only. Each function is a C ABI symbol plus a JNI stub
//! `Java_<pkg>_Native_<name>` that shifts the JNI arguments and jumps to it.
//! Calls are PC-relative, so the library has no relocations and does not need libc.

use std::collections::{BTreeMap, HashMap};

use crate::ast::{
    NAssign, NBinary, NBlock, NCall, NExpr, NFun, NIdent, NIf, NLit, NReturn, NStmt, NUnary, NVar,
    NWhile,
};
use crate::error::CompileError;

const PAGE: u64 = 0x4000;
const DT_NULL: u64 = 0;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_SONAME: u64 = 14;
const DT_GNU_HASH: u64 = 0x6FFF_FEF5;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_GNU_STACK: u32 = 0x6474_E551;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
const SONAME: &str = "libdroide.so";

const SHSTR: &[u8] =
    b"\x00.dynstr\x00.dynsym\x00.hash\x00.gnu.hash\x00.dynamic\x00.text\x00.shstrtab\x00";

pub struct NativeLib {
    pub x86: Vec<u8>,
    pub arm: Vec<u8>,
    pub x86_symbols: BTreeMap<String, u64>,
    pub arm_symbols: BTreeMap<String, u64>,
    pub exports: Vec<String>,
}

pub fn compile_library(package: &str, funs: &[NFun]) -> Result<Option<NativeLib>, CompileError> {
    if funs.is_empty() {
        return Ok(None);
    }
    let mut names = HashMap::new();
    for fun in funs {
        names.insert(fun.name.to_lowercase(), fun.name.clone());
    }
    let (xtext, xsyms) = link(Arch::X86, funs, &names, package)?;
    let (atext, asyms) = link(Arch::Arm, funs, &names, package)?;
    let (xelf, xmap) = elf(EM_X86_64, &xtext, &xsyms);
    let (aelf, amap) = elf(EM_AARCH64, &atext, &asyms);
    Ok(Some(NativeLib {
        x86: xelf,
        arm: aelf,
        x86_symbols: xmap,
        arm_symbols: amap,
        exports: funs.iter().map(|fun| fun.name.clone()).collect(),
    }))
}

pub fn emulate_aarch64(
    blob: &[u8],
    symbols: &BTreeMap<String, u64>,
    name: &str,
    args: &[i32],
) -> Result<i32, CompileError> {
    let mut mem = vec![0u8; 0x40_0000];
    if blob.len() > mem.len() {
        return Err(CompileError::bare("arm image does not fit"));
    }
    mem[..blob.len()].copy_from_slice(blob);
    let mut x = [0u64; 31];
    let mut sp: u64 = 0x30_0000;
    if args.len() > x.len() {
        return Err(CompileError::bare("too many arguments"));
    }
    for (i, arg) in args.iter().enumerate() {
        x[i] = u64::from(*arg as u32);
    }
    let mut x30: u64 = 0;
    let mut pc = *symbols
        .get(name)
        .ok_or_else(|| CompileError::bare(format!("undefined symbol {name}")))?;
    let mut flag_n: u32 = 0;
    let mut flag_z: u32 = 0;
    let mut flag_v: u32 = 0;
    let mut steps: u32 = 0;

    while steps < 1_000_000 {
        steps += 1;
        let word = read_u32(&mem, pc)?;
        let mut nxt = pc.wrapping_add(4);
        if word == 0xA9BF_7BFD {
            sp = sp.wrapping_sub(16);
            write_u64(&mut mem, sp, x[29])?;
            write_u64(&mut mem, sp.wrapping_add(8), x30)?;
        } else if word == 0xA8C1_7BFD {
            x[29] = read_u64(&mem, sp)?;
            x30 = read_u64(&mem, sp.wrapping_add(8))?;
            sp = sp.wrapping_add(16);
        } else if word == 0xD65F_03C0 {
            if x30 == 0 {
                return Ok(x[0] as u32 as i32);
            }
            nxt = x30;
        } else if word & 0xFFC0_0000 == 0xB900_0000 {
            let imm = (word >> 10) & 0xFFF;
            let rn = (word >> 5) & 31;
            let rt = word & 31;
            let base = if rn == 31 { sp } else { x[rn as usize] };
            let addr = base.wrapping_add(u64::from(imm) * 4);
            write_u32(&mut mem, addr, getw(&x, rt))?;
        } else if word & 0xFFC0_0000 == 0xB940_0000 {
            let imm = (word >> 10) & 0xFFF;
            let rn = (word >> 5) & 31;
            let rt = word & 31;
            let base = if rn == 31 { sp } else { x[rn as usize] };
            let addr = base.wrapping_add(u64::from(imm) * 4);
            setw(&mut x, rt, read_u32(&mem, addr)?);
        } else if word & 0xFF80_0000 == 0x5280_0000 {
            let imm = (word >> 5) & 0xFFFF;
            let rd = word & 31;
            let hw = (word >> 21) & 3;
            setw(&mut x, rd, (u64::from(imm) << (hw * 16)) as u32);
        } else if word & 0xFF80_0000 == 0x7280_0000 {
            let imm = (word >> 5) & 0xFFFF;
            let rd = word & 31;
            let hw = (word >> 21) & 3;
            let cur = u64::from(getw(&x, rd));
            let shift = hw * 16;
            let value = (cur & !(0xFFFFu64 << shift)) | (u64::from(imm) << shift);
            setw(&mut x, rd, value as u32);
        } else if word & 0xFF20_0000 == 0x2A00_0000 && ((word >> 10) & 0x3F) == 0 {
            let rm = (word >> 16) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let value = getw(&x, rn) | getw(&x, rm);
            setw(&mut x, rd, value);
        } else if (word >> 24) == 0x0B && ((word >> 10) & 0x3F) == 0 {
            let rm = (word >> 16) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let left = if rn == 31 {
                sp
            } else {
                u64::from(getw(&x, rn))
            };
            let value = left.wrapping_add(u64::from(getw(&x, rm))) as u32;
            setw(&mut x, rd, value);
        } else if (word >> 24) == 0x4B && ((word >> 10) & 0x3F) == 0 {
            let rm = (word >> 16) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let left = if rn == 31 { 0 } else { u64::from(getw(&x, rn)) };
            let value = left.wrapping_sub(u64::from(getw(&x, rm))) as u32;
            setw(&mut x, rd, value);
        } else if (word >> 24) == 0x6B && ((word >> 10) & 0x3F) == 0 {
            let rm = (word >> 16) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let a = i64::from(getw(&x, rn) as i32);
            let b = i64::from(getw(&x, rm) as i32);
            let wide = a - b;
            let raw = wide as u32;
            flag_z = u32::from(raw == 0);
            flag_n = (raw >> 31) & 1;
            flag_v = u32::from(wide < -0x8000_0000 || wide > 0x7FFF_FFFF);
            if rd != 31 {
                setw(&mut x, rd, raw);
            }
        } else if word & 0xFFE0_8000 == 0x1B00_0000 {
            let rm = (word >> 16) & 31;
            let ra = (word >> 10) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let prod = i64::from(getw(&x, rn) as i32) * i64::from(getw(&x, rm) as i32);
            let sum = i64::from(getw(&x, ra)) + prod;
            setw(&mut x, rd, sum as u32);
        } else if word & 0xFFE0_8000 == 0x1B00_8000 {
            let rm = (word >> 16) & 31;
            let ra = (word >> 10) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let prod = i64::from(getw(&x, rn) as i32) * i64::from(getw(&x, rm) as i32);
            let diff = i64::from(getw(&x, ra)) - prod;
            setw(&mut x, rd, diff as u32);
        } else if word & 0xFFE0_FC00 == 0x1AC0_0C00 {
            let rm = (word >> 16) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let divisor = i64::from(getw(&x, rm) as i32);
            let dividend = i64::from(getw(&x, rn) as i32);
            // Signed division by zero yields 0. x86 idiv is left as a real idiv.
            let quot = if divisor == 0 { 0 } else { dividend / divisor };
            setw(&mut x, rd, quot as u32);
        } else if word & 0xFFE0_0C00 == 0x1A80_0400 {
            let cond_code = (word >> 12) & 0xF;
            let rm = (word >> 16) & 31;
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let value = if cond_holds(cond_code, flag_n, flag_z, flag_v)? {
                getw(&x, rn)
            } else {
                getw(&x, rm).wrapping_add(1)
            };
            setw(&mut x, rd, value);
        } else if (word >> 24) == 0x34 {
            let mut imm = ((word >> 5) & 0x7_FFFF) as i32;
            if imm & 0x4_0000 != 0 {
                imm -= 0x8_0000;
            }
            let rt = word & 31;
            if getw(&x, rt) == 0 {
                nxt = pc.wrapping_add((i64::from(imm) * 4) as u64);
            }
        } else if (word >> 26) == 0x5 {
            let mut imm = (word & 0x3FF_FFFF) as i32;
            if imm & 0x200_0000 != 0 {
                imm -= 0x400_0000;
            }
            nxt = pc.wrapping_add((i64::from(imm) * 4) as u64);
        } else if (word >> 26) == 0x25 {
            let mut imm = (word & 0x3FF_FFFF) as i32;
            if imm & 0x200_0000 != 0 {
                imm -= 0x400_0000;
            }
            x30 = pc.wrapping_add(4);
            nxt = pc.wrapping_add((i64::from(imm) * 4) as u64);
        } else if word & 0xFF80_0000 == 0x9100_0000 || word & 0xFF80_0000 == 0xD100_0000 {
            let mut imm = u64::from((word >> 10) & 0xFFF);
            if (word >> 22) & 1 == 1 {
                imm <<= 12;
            }
            let rn = (word >> 5) & 31;
            let rd = word & 31;
            let base = if rn == 31 { sp } else { x[rn as usize] };
            let value = if word & 0xFF80_0000 == 0x9100_0000 {
                base.wrapping_add(imm)
            } else {
                base.wrapping_sub(imm)
            };
            if rd == 31 {
                sp = value;
            } else {
                x[rd as usize] = value;
            }
        } else {
            return Err(CompileError::bare(format!(
                "arm emulator hit an unknown instruction {word:#x} at {pc:#x}"
            )));
        }
        pc = nxt;
    }
    Err(CompileError::bare("arm emulator ran too long"))
}

fn cond_holds(op: u32, n: u32, z: u32, v: u32) -> Result<bool, CompileError> {
    Ok(match op {
        0 => z == 1,
        1 => z == 0,
        10 => n == v,
        11 => n != v,
        12 => z == 0 && n == v,
        13 => z == 1 || n != v,
        _ => return Err(CompileError::bare(format!("bad condition {op}"))),
    })
}

fn getw(x: &[u64; 31], reg: u32) -> u32 {
    if reg == 31 {
        0
    } else {
        x[reg as usize] as u32
    }
}

fn setw(x: &mut [u64; 31], reg: u32, value: u32) {
    if reg != 31 {
        x[reg as usize] = u64::from(value);
    }
}

fn read_u32(mem: &[u8], addr: u64) -> Result<u32, CompileError> {
    let start = addr as usize;
    let end = start
        .checked_add(4)
        .ok_or_else(|| CompileError::bare("arm emulator address out of range"))?;
    if end > mem.len() {
        return Err(CompileError::bare(format!(
            "arm emulator address {start:#x}"
        )));
    }
    Ok(u32::from_le_bytes(mem[start..end].try_into().unwrap()))
}

fn read_u64(mem: &[u8], addr: u64) -> Result<u64, CompileError> {
    let start = addr as usize;
    let end = start
        .checked_add(8)
        .ok_or_else(|| CompileError::bare("arm emulator address out of range"))?;
    if end > mem.len() {
        return Err(CompileError::bare(format!(
            "arm emulator address {start:#x}"
        )));
    }
    Ok(u64::from_le_bytes(mem[start..end].try_into().unwrap()))
}

fn write_u32(mem: &mut [u8], addr: u64, value: u32) -> Result<(), CompileError> {
    let start = addr as usize;
    let end = start
        .checked_add(4)
        .ok_or_else(|| CompileError::bare("arm emulator address out of range"))?;
    if end > mem.len() {
        return Err(CompileError::bare(format!(
            "arm emulator address {start:#x}"
        )));
    }
    mem[start..end].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u64(mem: &mut [u8], addr: u64, value: u64) -> Result<(), CompileError> {
    let start = addr as usize;
    let end = start
        .checked_add(8)
        .ok_or_else(|| CompileError::bare("arm emulator address out of range"))?;
    if end > mem.len() {
        return Err(CompileError::bare(format!(
            "arm emulator address {start:#x}"
        )));
    }
    mem[start..end].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

pub(crate) fn jni_escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '_' => out.push_str("_1"),
            '.' => out.push('_'),
            ';' => out.push_str("_2"),
            '[' => out.push_str("_3"),
            ch if ch.is_alphanumeric() => out.push(ch),
            ch => out.push_str(&format!("_0{:04x}", u32::from(ch))),
        }
    }
    out
}

pub(crate) fn jni_symbol(package: &str, name: &str) -> String {
    format!("Java_{}_Native_{}", jni_escape(package), jni_escape(name))
}

fn elf_hash(name: &str) -> u32 {
    // Python keeps bits above 32 between rounds; only the low 32 are returned.
    let mut limbs = vec![0u32];
    for &byte in name.as_bytes() {
        let mut carry = u32::from(byte);
        for limb in &mut limbs {
            let cur = (u64::from(*limb) << 4) + u64::from(carry);
            *limb = cur as u32;
            carry = (cur >> 32) as u32;
        }
        if carry != 0 {
            limbs.push(carry);
        }
        let high = limbs[0] & 0xF000_0000;
        if high != 0 {
            limbs[0] ^= high >> 24;
            limbs[0] &= !high;
        }
    }
    limbs[0]
}

fn gnu_hash(name: &str) -> u32 {
    let mut value = 5381u32;
    for &byte in name.as_bytes() {
        value = value.wrapping_mul(33).wrapping_add(u32::from(byte));
    }
    value
}

fn align_up(value: usize, boundary: usize) -> usize {
    (value + boundary - 1) & !(boundary - 1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arch {
    X86,
    Arm,
}

#[derive(Clone, Copy)]
enum FixKind {
    Rel32,
    B,
    Cbz,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchHow {
    Call,
    Stub,
}

struct Buf {
    buf: Vec<u8>,
    labels: HashMap<String, usize>,
    fixups: Vec<(usize, String, FixKind)>,
    calls: Vec<(usize, String)>,
}

impl Buf {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            labels: HashMap::new(),
            fixups: Vec::new(),
            calls: Vec::new(),
        }
    }

    fn emit(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    fn u32(&mut self, word: u32) {
        self.buf.extend_from_slice(&word.to_le_bytes());
    }

    fn label(&mut self, name: &str) {
        self.labels.insert(name.to_string(), self.buf.len());
    }

    fn rel32(&mut self, label: &str) {
        self.fixups
            .push((self.buf.len(), label.to_string(), FixKind::Rel32));
        self.emit(&[0, 0, 0, 0]);
    }

    fn fixup(&mut self, label: &str, kind: FixKind) {
        self.fixups.push((self.buf.len(), label.to_string(), kind));
        self.u32(0);
    }

    fn call_site(&mut self, name: &str) {
        self.calls.push((self.buf.len(), name.to_string()));
    }

    fn patch(&mut self) -> Result<(), CompileError> {
        let fixups = self.fixups.clone();
        for (at, label, kind) in fixups {
            let Some(&target) = self.labels.get(&label) else {
                return Err(CompileError::bare(format!("undefined label {label}")));
            };
            let encoded = match kind {
                FixKind::Rel32 => {
                    let rel = target as i64 - (at as i64 + 4);
                    i32::try_from(rel).map_err(|_| CompileError::bare("relocation out of range"))?
                        as u32
                }
                FixKind::B => {
                    let rel = (target as i64 - at as i64).div_euclid(4);
                    0x1400_0000 | (rel as u32 & 0x03FF_FFFF)
                }
                FixKind::Cbz => {
                    let rel = (target as i64 - at as i64).div_euclid(4);
                    0x3400_0000 | ((rel as u32 & 0x7_FFFF) << 5)
                }
            };
            self.buf[at..at + 4].copy_from_slice(&encoded.to_le_bytes());
        }
        Ok(())
    }
}

fn disp(slot: usize) -> [u8; 4] {
    (-4 * (slot as i32 + 1)).to_le_bytes()
}

trait Asm {
    fn buf(&mut self) -> &mut Buf;
    fn prologue(&mut self, nargs: usize) -> Result<(), CompileError>;
    fn store(&mut self, slot: usize) -> Result<(), CompileError>;
    fn load(&mut self, slot: usize) -> Result<(), CompileError>;
    fn imm(&mut self, value: i64);
    fn binop(&mut self, op: &str, slot: usize) -> Result<(), CompileError>;
    fn neg(&mut self);
    fn cmp_set(&mut self, op: &str, slot: usize) -> Result<(), CompileError>;
    fn jump_if_zero(&mut self, label: &str);
    fn jump(&mut self, label: &str);
    fn call(&mut self, name: &str);
    fn arg(&mut self, index: usize, slot: usize) -> Result<(), CompileError>;
    fn epilogue(&mut self);
    fn ret(&mut self);
    fn label(&mut self, name: &str) {
        self.buf().label(name);
    }
    fn patch(&mut self) -> Result<(), CompileError> {
        self.buf().patch()
    }
    fn finish(self) -> (Vec<u8>, Vec<(usize, String)>);
}

struct X86 {
    buf: Buf,
}

impl X86 {
    fn new() -> Self {
        Self { buf: Buf::new() }
    }

    fn load_ecx(&mut self, slot: usize) {
        self.buf.emit(&[0x8B, 0x8D]);
        self.buf.emit(&disp(slot));
    }
}

impl Asm for X86 {
    fn buf(&mut self) -> &mut Buf {
        &mut self.buf
    }

    fn prologue(&mut self, nargs: usize) -> Result<(), CompileError> {
        self.buf.emit(&[0x55, 0x48, 0x89, 0xE5, 0x48, 0x81, 0xEC]);
        self.buf.emit(&512u32.to_le_bytes());
        let moves: [&[u8]; 6] = [
            &[0x89, 0xF8],
            &[0x89, 0xF0],
            &[0x89, 0xD0],
            &[0x89, 0xC8],
            &[0x44, 0x89, 0xC0],
            &[0x44, 0x89, 0xC8],
        ];
        if nargs > moves.len() {
            return Err(CompileError::bare("too many arguments"));
        }
        for i in 0..nargs {
            self.buf.emit(moves[i]);
            self.store(i)?;
        }
        Ok(())
    }

    fn store(&mut self, slot: usize) -> Result<(), CompileError> {
        self.buf.emit(&[0x89, 0x85]);
        self.buf.emit(&disp(slot));
        Ok(())
    }

    fn load(&mut self, slot: usize) -> Result<(), CompileError> {
        self.buf.emit(&[0x8B, 0x85]);
        self.buf.emit(&disp(slot));
        Ok(())
    }

    fn imm(&mut self, value: i64) {
        self.buf.emit(&[0xB8]);
        self.buf.emit(&(value as u32).to_le_bytes());
    }

    fn binop(&mut self, op: &str, slot: usize) -> Result<(), CompileError> {
        self.load_ecx(slot);
        match op {
            "+" => self.buf.emit(&[0x01, 0xC8]),
            "-" => self.buf.emit(&[0x29, 0xC8]),
            "*" => self.buf.emit(&[0x0F, 0xAF, 0xC1]),
            "/" => self.buf.emit(&[0x99, 0xF7, 0xF9]),
            "%" => self.buf.emit(&[0x99, 0xF7, 0xF9, 0x89, 0xD0]),
            _ => return Err(CompileError::bare(format!("bad op {op}"))),
        }
        Ok(())
    }

    fn neg(&mut self) {
        self.buf.emit(&[0xF7, 0xD8]);
    }

    fn cmp_set(&mut self, op: &str, slot: usize) -> Result<(), CompileError> {
        self.load_ecx(slot);
        self.buf.emit(&[0x39, 0xC8]);
        let set: [u8; 3] = match op {
            "==" => [0x0F, 0x94, 0xC0],
            "!=" => [0x0F, 0x95, 0xC0],
            "<" => [0x0F, 0x9C, 0xC0],
            ">" => [0x0F, 0x9F, 0xC0],
            "<=" => [0x0F, 0x9E, 0xC0],
            ">=" => [0x0F, 0x9D, 0xC0],
            _ => return Err(CompileError::bare(format!("bad op {op}"))),
        };
        self.buf.emit(&set);
        self.buf.emit(&[0x0F, 0xB6, 0xC0]);
        Ok(())
    }

    fn jump_if_zero(&mut self, label: &str) {
        self.buf.emit(&[0x85, 0xC0, 0x0F, 0x84]);
        self.buf.rel32(label);
    }

    fn jump(&mut self, label: &str) {
        self.buf.emit(&[0xE9]);
        self.buf.rel32(label);
    }

    fn call(&mut self, name: &str) {
        self.buf.emit(&[0xE8]);
        self.buf.call_site(name);
        self.buf.emit(&[0, 0, 0, 0]);
    }

    fn arg(&mut self, index: usize, slot: usize) -> Result<(), CompileError> {
        self.load(slot)?;
        let mov: &[u8] = match index {
            0 => &[0x89, 0xC7],
            1 => &[0x89, 0xC6],
            2 => &[0x89, 0xC2],
            3 => &[0x89, 0xC1],
            _ => return Err(CompileError::bare("bad arg")),
        };
        self.buf.emit(mov);
        Ok(())
    }

    fn epilogue(&mut self) {
        self.buf.emit(&[0x48, 0x89, 0xEC, 0x5D]);
    }

    fn ret(&mut self) {
        self.buf.emit(&[0xC3]);
    }

    fn finish(self) -> (Vec<u8>, Vec<(usize, String)>) {
        (self.buf.buf, self.buf.calls)
    }
}

struct Arm {
    buf: Buf,
}

impl Arm {
    fn new() -> Self {
        Self { buf: Buf::new() }
    }

    fn mem(&mut self, op: u32, reg: u32, slot: usize) -> Result<(), CompileError> {
        if slot >= 64 {
            return Err(CompileError::bare("native frame slot is out of range"));
        }
        self.buf.u32(op | ((slot as u32) << 10) | (31 << 5) | reg);
        Ok(())
    }
}

impl Asm for Arm {
    fn buf(&mut self) -> &mut Buf {
        &mut self.buf
    }

    fn prologue(&mut self, nargs: usize) -> Result<(), CompileError> {
        self.buf.u32(0xA9BF_7BFD);
        self.buf.u32(0x9100_03FD);
        self.buf.u32(0xD104_03FF);
        for i in 0..nargs {
            let i = i as u32;
            self.buf.u32(0xB900_0000 | (i << 10) | (31 << 5) | i);
        }
        Ok(())
    }

    fn store(&mut self, slot: usize) -> Result<(), CompileError> {
        self.mem(0xB900_0000, 0, slot)
    }

    fn load(&mut self, slot: usize) -> Result<(), CompileError> {
        self.mem(0xB940_0000, 0, slot)
    }

    fn imm(&mut self, value: i64) {
        let value = value as u32;
        self.buf.u32(0x5280_0000 | ((value & 0xFFFF) << 5));
        if value > 0xFFFF {
            self.buf.u32(0x72A0_0000 | (((value >> 16) & 0xFFFF) << 5));
        }
    }

    fn binop(&mut self, op: &str, slot: usize) -> Result<(), CompileError> {
        self.mem(0xB940_0000, 8, slot)?;
        match op {
            "+" => self.buf.u32(0x0B08_0000),
            "-" => self.buf.u32(0x4B08_0000),
            "*" => self.buf.u32(0x1B00_0000 | (8 << 16) | (31 << 10)),
            "/" => self.buf.u32(0x1AC0_0C00 | (8 << 16)),
            "%" => {
                self.buf.u32(0x2A00_03E9);
                self.buf.u32(0x1AC0_0C00 | (8 << 16));
                self.buf.u32(0x1B00_8000 | (8 << 16) | (9 << 10));
            }
            _ => return Err(CompileError::bare(format!("bad op {op}"))),
        }
        Ok(())
    }

    fn neg(&mut self) {
        self.buf.u32(0x4B00_03E0);
    }

    fn cmp_set(&mut self, op: &str, slot: usize) -> Result<(), CompileError> {
        self.mem(0xB940_0000, 8, slot)?;
        self.buf.u32(0x6B08_001F);
        let invert: u32 = match op {
            "==" => 1,
            "!=" => 0,
            "<" => 10,
            ">" => 13,
            "<=" => 12,
            ">=" => 11,
            _ => return Err(CompileError::bare(format!("bad op {op}"))),
        };
        self.buf.u32(0x1A9F_07E0 | (invert << 12));
        Ok(())
    }

    fn jump_if_zero(&mut self, label: &str) {
        self.buf.fixup(label, FixKind::Cbz);
    }

    fn jump(&mut self, label: &str) {
        self.buf.fixup(label, FixKind::B);
    }

    fn call(&mut self, name: &str) {
        self.buf.call_site(name);
        self.buf.u32(0x9400_0000);
    }

    fn arg(&mut self, index: usize, slot: usize) -> Result<(), CompileError> {
        self.mem(0xB940_0000, index as u32, slot)
    }

    fn epilogue(&mut self) {
        self.buf.u32(0x9100_03BF);
        self.buf.u32(0xA8C1_7BFD);
    }

    fn ret(&mut self) {
        self.buf.u32(0xD65F_03C0);
    }

    fn finish(self) -> (Vec<u8>, Vec<(usize, String)>) {
        (self.buf.buf, self.buf.calls)
    }
}

fn slots(fun: &NFun) -> Result<HashMap<String, usize>, CompileError> {
    let mut mapping = HashMap::new();
    let mut index = 0usize;
    for name in &fun.params {
        mapping.insert(name.clone(), index);
        index += 1;
    }
    fn walk(stmts: &[NStmt], mapping: &mut HashMap<String, usize>, index: &mut usize) {
        for stmt in stmts {
            match stmt {
                NStmt::Var(var) => {
                    if !mapping.contains_key(&var.name) {
                        mapping.insert(var.name.clone(), *index);
                        *index += 1;
                    }
                }
                NStmt::If(stmt) => {
                    walk(&stmt.then, mapping, index);
                    walk(&stmt.else_, mapping, index);
                }
                NStmt::While(stmt) => walk(&stmt.body, mapping, index),
                NStmt::Block(stmt) => walk(&stmt.stmts, mapping, index),
                NStmt::Assign(_) | NStmt::Return(_) => {}
            }
        }
    }
    walk(&fun.body, &mut mapping, &mut index);
    if index > 40 {
        return Err(CompileError::bare(format!(
            "{} has too many locals",
            fun.name
        )));
    }
    Ok(mapping)
}

struct Lower<'a> {
    fun: &'a NFun,
    slots: HashMap<String, usize>,
    names: &'a HashMap<String, String>,
    tsp: usize,
    lab: u32,
}

impl<'a> Lower<'a> {
    fn new(fun: &'a NFun, names: &'a HashMap<String, String>) -> Result<Self, CompileError> {
        Ok(Self {
            fun,
            slots: slots(fun)?,
            names,
            tsp: 0,
            lab: 0,
        })
    }

    fn fresh(&mut self) -> String {
        self.lab += 1;
        format!("n{}", self.lab)
    }

    fn temp(&mut self) -> Result<usize, CompileError> {
        let slot = 40 + self.tsp;
        self.tsp += 1;
        if slot >= 64 {
            return Err(CompileError::bare("native expression is too complex"));
        }
        Ok(slot)
    }

    fn slot(&self, name: &str) -> Result<usize, CompileError> {
        self.slots
            .get(name)
            .copied()
            .ok_or_else(|| CompileError::bare(format!("undefined native name {name}")))
    }

    fn expr<A: Asm>(&mut self, expr: &NExpr, asm: &mut A) -> Result<(), CompileError> {
        match expr {
            NExpr::Lit(NLit { value, .. }) => asm.imm(*value),
            NExpr::Ident(NIdent { name, .. }) => {
                let slot = self.slot(name)?;
                asm.load(slot)?;
            }
            NExpr::Unary(NUnary { op, expr, .. }) => {
                self.expr(expr, asm)?;
                if op == "-" {
                    asm.neg();
                }
            }
            NExpr::Binary(NBinary {
                op, left, right, ..
            }) => {
                self.expr(right, asm)?;
                let slot = self.temp()?;
                asm.store(slot)?;
                self.expr(left, asm)?;
                if matches!(op.as_str(), "==" | "!=" | "<" | ">" | "<=" | ">=") {
                    asm.cmp_set(op, slot)?;
                } else {
                    asm.binop(op, slot)?;
                }
                self.tsp -= 1;
            }
            NExpr::Call(NCall { name, args, .. }) => {
                let mut saved = Vec::new();
                for arg in args {
                    self.expr(arg, asm)?;
                    let slot = self.temp()?;
                    asm.store(slot)?;
                    saved.push(slot);
                }
                for (index, slot) in saved.iter().copied().enumerate() {
                    asm.arg(index, slot)?;
                }
                let target = self.names.get(&name.to_lowercase()).ok_or_else(|| {
                    CompileError::bare(format!("undefined native function {name}"))
                })?;
                asm.call(target);
                self.tsp -= saved.len();
            }
        }
        Ok(())
    }

    fn stmts<A: Asm>(&mut self, stmts: &[NStmt], asm: &mut A) -> Result<(), CompileError> {
        for stmt in stmts {
            match stmt {
                NStmt::Var(NVar { name, expr, .. }) => {
                    self.expr(expr, asm)?;
                    let slot = self.slot(name)?;
                    asm.store(slot)?;
                }
                NStmt::Assign(NAssign { name, expr, .. }) => {
                    self.expr(expr, asm)?;
                    let slot = self.slot(name)?;
                    asm.store(slot)?;
                }
                NStmt::Return(NReturn { expr, .. }) => {
                    if let Some(expr) = expr {
                        self.expr(expr, asm)?;
                    } else {
                        asm.imm(0);
                    }
                    asm.epilogue();
                    asm.ret();
                }
                NStmt::If(NIf {
                    cond, then, else_, ..
                }) => {
                    self.expr(cond, asm)?;
                    let else_l = self.fresh();
                    let end_l = self.fresh();
                    asm.jump_if_zero(&else_l);
                    self.stmts(then, asm)?;
                    asm.jump(&end_l);
                    asm.label(&else_l);
                    self.stmts(else_, asm)?;
                    asm.label(&end_l);
                }
                NStmt::While(NWhile { cond, body, .. }) => {
                    let head = self.fresh();
                    let end = self.fresh();
                    asm.label(&head);
                    self.expr(cond, asm)?;
                    asm.jump_if_zero(&end);
                    self.stmts(body, asm)?;
                    asm.jump(&head);
                    asm.label(&end);
                }
                NStmt::Block(NBlock { stmts, .. }) => self.stmts(stmts, asm)?,
            }
        }
        Ok(())
    }

    fn run<A: Asm>(&mut self, asm: &mut A) -> Result<(), CompileError> {
        asm.prologue(self.fun.params.len())?;
        self.stmts(&self.fun.body, asm)?;
        asm.imm(0);
        asm.epilogue();
        asm.ret();
        asm.patch()?;
        Ok(())
    }
}

fn compile_one(
    fun: &NFun,
    names: &HashMap<String, String>,
    arch: Arch,
) -> Result<(Vec<u8>, Vec<(usize, String)>), CompileError> {
    let mut lower = Lower::new(fun, names)?;
    match arch {
        Arch::X86 => {
            let mut asm = X86::new();
            lower.run(&mut asm)?;
            Ok(asm.finish())
        }
        Arch::Arm => {
            let mut asm = Arm::new();
            lower.run(&mut asm)?;
            Ok(asm.finish())
        }
    }
}

struct SymTab {
    order: Vec<String>,
    map: HashMap<String, (usize, usize)>,
}

impl SymTab {
    fn new() -> Self {
        Self {
            order: Vec::new(),
            map: HashMap::new(),
        }
    }

    fn insert(&mut self, name: String, value: (usize, usize)) {
        if self.map.contains_key(&name) {
            self.map.insert(name, value);
        } else {
            self.map.insert(name.clone(), value);
            self.order.push(name);
        }
    }

    fn get(&self, name: &str) -> Option<(usize, usize)> {
        self.map.get(name).copied()
    }

    fn items(&self) -> Vec<(String, usize, usize)> {
        self.order
            .iter()
            .map(|name| {
                let (off, size) = self.map[name];
                (name.clone(), off, size)
            })
            .collect()
    }
}

fn jni_stub(arch: Arch) -> (Vec<u8>, usize) {
    match arch {
        Arch::X86 => {
            // mov edi, edx; mov esi, ecx; mov edx, r8d; mov ecx, r9d; jmp rel32
            (
                vec![
                    0x89, 0xD7, 0x89, 0xCE, 0x44, 0x89, 0xC2, 0x44, 0x89, 0xC1, 0xE9,
                ],
                11,
            )
        }
        Arch::Arm => {
            let words = [
                0x2A02_03E0u32,
                0x2A03_03E1,
                0x2A04_03E2,
                0x2A05_03E3,
                0x1400_0000,
            ];
            let mut buf = Vec::with_capacity(20);
            for word in words {
                buf.extend_from_slice(&word.to_le_bytes());
            }
            (buf, 16)
        }
    }
}

fn poke(text: &mut Vec<u8>, at: usize, bytes: &[u8]) -> Result<(), CompileError> {
    if at > text.len() {
        return Err(CompileError::bare("bad patch"));
    }
    let end = at + bytes.len();
    if end > text.len() {
        text.resize(end, 0);
    }
    text[at..end].copy_from_slice(bytes);
    Ok(())
}

fn link(
    arch: Arch,
    funs: &[NFun],
    names: &HashMap<String, String>,
    package: &str,
) -> Result<(Vec<u8>, Vec<(String, usize, usize)>), CompileError> {
    let mut compiled = Vec::new();
    for fun in funs {
        compiled.push((fun, compile_one(fun, names, arch)?));
    }
    let mut text = Vec::new();
    let mut symbols = SymTab::new();
    let mut patches = Vec::new();
    for (fun, (buf, calls)) in &compiled {
        let start = text.len();
        text.extend_from_slice(buf);
        symbols.insert(fun.name.clone(), (start, buf.len()));
        for (at, name) in calls {
            patches.push((start + at, name.clone(), PatchHow::Call));
        }
    }
    for (fun, _) in &compiled {
        let (stub, jmp_at) = jni_stub(arch);
        let start = text.len();
        text.extend_from_slice(&stub);
        symbols.insert(jni_symbol(package, &fun.name), (start, stub.len()));
        patches.push((start + jmp_at, fun.name.clone(), PatchHow::Stub));
    }
    for (at, name, how) in patches {
        let (target, _) = symbols
            .get(&name)
            .ok_or_else(|| CompileError::bare(format!("undefined native function {name}")))?;
        match arch {
            Arch::X86 => {
                let rel = target as i64 - (at as i64 + 4);
                let rel = i32::try_from(rel)
                    .map_err(|_| CompileError::bare("relocation out of range"))?;
                poke(&mut text, at, &rel.to_le_bytes())?;
            }
            Arch::Arm => {
                let rel = (target as i64 - at as i64).div_euclid(4);
                let opcode = if how == PatchHow::Stub {
                    0x1400_0000
                } else {
                    0x9400_0000
                };
                let word = opcode | (rel as u32 & 0x03FF_FFFF);
                poke(&mut text, at, &word.to_le_bytes())?;
            }
        }
    }
    Ok((text, symbols.items()))
}

struct Defined {
    name: String,
    off: u64,
    size: u64,
    hash: u32,
    name_off: u32,
}

fn pack_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn pack_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn pack_u64(buf: &mut Vec<u8>, value: u64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn pack_u32s(words: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 4);
    for word in words {
        pack_u32(&mut out, *word);
    }
    out
}

fn sh_name(name: &str) -> u32 {
    match name {
        ".dynstr" => 1,
        ".dynsym" => 9,
        ".hash" => 17,
        ".gnu.hash" => 23,
        ".dynamic" => 33,
        ".text" => 42,
        ".shstrtab" => 48,
        _ => 0,
    }
}

fn shdr(
    name: &str,
    typ: u32,
    flags: u64,
    addr: u64,
    off: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entsize: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    pack_u32(&mut out, sh_name(name));
    pack_u32(&mut out, typ);
    pack_u64(&mut out, flags);
    pack_u64(&mut out, addr);
    pack_u64(&mut out, off);
    pack_u64(&mut out, size);
    pack_u32(&mut out, link);
    pack_u32(&mut out, info);
    pack_u64(&mut out, align);
    pack_u64(&mut out, entsize);
    out
}

fn phdr(
    typ: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(56);
    pack_u32(&mut out, typ);
    pack_u32(&mut out, flags);
    pack_u64(&mut out, offset);
    pack_u64(&mut out, vaddr);
    pack_u64(&mut out, paddr);
    pack_u64(&mut out, filesz);
    pack_u64(&mut out, memsz);
    pack_u64(&mut out, align);
    out
}

fn put(image: &mut [u8], at: usize, data: &[u8]) {
    image[at..at + data.len()].copy_from_slice(data);
}

fn elf(
    machine: u16,
    text: &[u8],
    symbols: &[(String, usize, usize)],
) -> (Vec<u8>, BTreeMap<String, u64>) {
    let mut defined = Vec::new();
    for (name, off, size) in symbols {
        defined.push(Defined {
            name: name.clone(),
            off: *off as u64,
            size: *size as u64,
            hash: gnu_hash(name),
            name_off: 0,
        });
    }
    let nbuckets = defined.len().max(1);
    let mut groups: Vec<Vec<Defined>> = (0..nbuckets).map(|_| Vec::new()).collect();
    for sym in defined {
        let bucket = (sym.hash as usize) % nbuckets;
        groups[bucket].push(sym);
    }
    let mut ordered = Vec::new();
    let mut buckets = vec![0u32; nbuckets];
    let mut chain = Vec::new();
    for (index, group) in groups.into_iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        buckets[index] = 1 + ordered.len() as u32;
        let last = group.len() - 1;
        for (pos, sym) in group.into_iter().enumerate() {
            let mut bits = sym.hash & 0xFFFF_FFFE;
            if pos == last {
                bits |= 1;
            }
            chain.push(bits);
            ordered.push(sym);
        }
    }
    let mut dynstr = vec![0u8];
    let mut add_str = |text_value: &str| -> u32 {
        let at = dynstr.len() as u32;
        dynstr.extend_from_slice(text_value.as_bytes());
        dynstr.push(0);
        at
    };
    let soname_off = add_str(SONAME);
    for sym in &mut ordered {
        sym.name_off = add_str(&sym.name);
    }
    let nsyms = 1 + ordered.len();
    let mut sys_buckets = vec![0u32; nsyms];
    let mut sys_chains = vec![0u32; nsyms];
    for (index, sym) in ordered.iter().enumerate() {
        let index = index + 1;
        let slot = (elf_hash(&sym.name) as usize) % nsyms;
        sys_chains[index] = sys_buckets[slot];
        sys_buckets[slot] = index as u32;
    }
    let mut sysv = Vec::new();
    pack_u32(&mut sysv, nsyms as u32);
    pack_u32(&mut sysv, nsyms as u32);
    sysv.extend_from_slice(&pack_u32s(&sys_buckets));
    sysv.extend_from_slice(&pack_u32s(&sys_chains));
    let mut gnu = Vec::new();
    pack_u32(&mut gnu, nbuckets as u32);
    pack_u32(&mut gnu, 1);
    pack_u32(&mut gnu, 1);
    pack_u32(&mut gnu, 6);
    pack_u64(&mut gnu, u64::MAX);
    gnu.extend_from_slice(&pack_u32s(&buckets));
    if !chain.is_empty() {
        gnu.extend_from_slice(&pack_u32s(&chain));
    }

    let ehdr = 64usize;
    let phnum = 3usize;
    let phdr_size = phnum * 56;
    let mut cursor = align_up(ehdr + phdr_size, 8);
    let dynstr_off = cursor;
    cursor = align_up(cursor + dynstr.len(), 8);
    let dynsym_off = cursor;
    let dynsym_size = nsyms * 24;
    cursor = align_up(cursor + dynsym_size, 4);
    let hash_off = cursor;
    cursor = align_up(cursor + sysv.len(), 8);
    let gnu_off = cursor;
    cursor = align_up(cursor + gnu.len(), 8);
    let dyn_off = cursor;
    let dyn_size = 8 * 16;
    cursor = align_up(cursor + dyn_size, 16);
    let text_off = cursor;
    cursor = align_up(cursor + text.len(), 8);
    let shstr_off = cursor;
    cursor = align_up(cursor + SHSTR.len(), 8);
    let shoff = cursor;
    let shnum = 8usize;
    let file_size = shoff + shnum * 64;

    let mut sym_bytes = vec![0u8; 24];
    for sym in &ordered {
        let info = (1u8 << 4) | 2;
        pack_u32(&mut sym_bytes, sym.name_off);
        sym_bytes.push(info);
        sym_bytes.push(0);
        pack_u16(&mut sym_bytes, 6);
        pack_u64(&mut sym_bytes, text_off as u64 + sym.off);
        pack_u64(&mut sym_bytes, sym.size);
    }

    let mut dynamic = Vec::new();
    for (tag, val) in [
        (DT_HASH, hash_off as u64),
        (DT_GNU_HASH, gnu_off as u64),
        (DT_STRTAB, dynstr_off as u64),
        (DT_SYMTAB, dynsym_off as u64),
        (DT_STRSZ, dynstr.len() as u64),
        (DT_SYMENT, 24u64),
        (DT_SONAME, u64::from(soname_off)),
        (DT_NULL, 0u64),
    ] {
        pack_u64(&mut dynamic, tag);
        pack_u64(&mut dynamic, val);
    }

    let mut image = vec![0u8; file_size];
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[6] = 1;
    let mut ehdr_rest = Vec::with_capacity(48);
    pack_u16(&mut ehdr_rest, 3);
    pack_u16(&mut ehdr_rest, machine);
    pack_u32(&mut ehdr_rest, 1);
    pack_u64(&mut ehdr_rest, 0);
    pack_u64(&mut ehdr_rest, ehdr as u64);
    pack_u64(&mut ehdr_rest, shoff as u64);
    pack_u32(&mut ehdr_rest, 0);
    pack_u16(&mut ehdr_rest, 64);
    pack_u16(&mut ehdr_rest, 56);
    pack_u16(&mut ehdr_rest, phnum as u16);
    pack_u16(&mut ehdr_rest, 64);
    pack_u16(&mut ehdr_rest, shnum as u16);
    pack_u16(&mut ehdr_rest, 7);
    put(&mut image, 16, &ehdr_rest);

    let mut ph = Vec::new();
    ph.extend(phdr(
        PT_LOAD,
        PF_R | PF_X,
        0,
        0,
        0,
        file_size as u64,
        file_size as u64,
        PAGE,
    ));
    ph.extend(phdr(
        PT_DYNAMIC,
        PF_R,
        dyn_off as u64,
        dyn_off as u64,
        dyn_off as u64,
        dyn_size as u64,
        dyn_size as u64,
        8,
    ));
    ph.extend(phdr(PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 0, 0x10));
    put(&mut image, ehdr, &ph);
    put(&mut image, dynstr_off, &dynstr);
    put(&mut image, dynsym_off, &sym_bytes);
    put(&mut image, hash_off, &sysv);
    put(&mut image, gnu_off, &gnu);
    put(&mut image, dyn_off, &dynamic);
    put(&mut image, text_off, text);
    put(&mut image, shstr_off, SHSTR);

    let sections = [
        shdr("", 0, 0, 0, 0, 0, 0, 0, 0, 0),
        shdr(
            ".dynstr",
            3,
            2,
            dynstr_off as u64,
            dynstr_off as u64,
            dynstr.len() as u64,
            0,
            0,
            1,
            0,
        ),
        shdr(
            ".dynsym",
            11,
            2,
            dynsym_off as u64,
            dynsym_off as u64,
            dynsym_size as u64,
            1,
            1,
            8,
            24,
        ),
        shdr(
            ".hash",
            5,
            2,
            hash_off as u64,
            hash_off as u64,
            sysv.len() as u64,
            2,
            0,
            4,
            4,
        ),
        shdr(
            ".gnu.hash",
            0x6FFF_FFF6,
            2,
            gnu_off as u64,
            gnu_off as u64,
            gnu.len() as u64,
            2,
            0,
            8,
            0,
        ),
        shdr(
            ".dynamic",
            6,
            2,
            dyn_off as u64,
            dyn_off as u64,
            dyn_size as u64,
            1,
            0,
            8,
            16,
        ),
        shdr(
            ".text",
            1,
            6,
            text_off as u64,
            text_off as u64,
            text.len() as u64,
            0,
            0,
            16,
            0,
        ),
        shdr(
            ".shstrtab",
            3,
            0,
            0,
            shstr_off as u64,
            SHSTR.len() as u64,
            0,
            0,
            1,
            0,
        ),
    ];
    let mut blob = Vec::new();
    for section in sections {
        blob.extend(section);
    }
    put(&mut image, shoff, &blob);

    let mut resolved = BTreeMap::new();
    for sym in &ordered {
        resolved.insert(sym.name.clone(), text_off as u64 + sym.off);
    }
    (image, resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{NBinary, NCall, NExpr, NFun, NIdent, NIf, NLit, NReturn, NStmt};

    fn lit(value: i64) -> NExpr {
        NExpr::Lit(NLit {
            line: 1,
            col: 1,
            value,
        })
    }

    fn ident(name: &str) -> NExpr {
        NExpr::Ident(NIdent {
            line: 1,
            col: 1,
            name: name.to_string(),
        })
    }

    fn binary(op: &str, left: NExpr, right: NExpr) -> NExpr {
        NExpr::Binary(NBinary {
            line: 1,
            col: 1,
            op: op.to_string(),
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn call(name: &str, args: Vec<NExpr>) -> NExpr {
        NExpr::Call(NCall {
            line: 1,
            col: 1,
            name: name.to_string(),
            args,
        })
    }

    fn ret(expr: NExpr) -> NStmt {
        NStmt::Return(NReturn {
            line: 1,
            col: 1,
            expr: Some(expr),
        })
    }

    #[test]
    fn fib_ten_is_55() {
        // fun fib(n: i32): i32 {
        //   if n < 2 { return n; }
        //   return fib(n - 1) + fib(n - 2);
        // }
        let fib = NFun {
            line: 1,
            col: 1,
            end_line: 5,
            name: "fib".into(),
            params: vec!["n".into()],
            ret: "i32".into(),
            body: vec![
                NStmt::If(NIf {
                    line: 2,
                    col: 3,
                    cond: binary("<", ident("n"), lit(2)),
                    then: vec![ret(ident("n"))],
                    else_: vec![],
                }),
                ret(binary(
                    "+",
                    call("fib", vec![binary("-", ident("n"), lit(1))]),
                    call("fib", vec![binary("-", ident("n"), lit(2))]),
                )),
            ],
        };
        let lib = compile_library("com.droide.hello", &[fib])
            .unwrap()
            .expect("library");
        let got = emulate_aarch64(&lib.arm, &lib.arm_symbols, "fib", &[10]).unwrap();
        assert_eq!(got, 55);
    }
}
