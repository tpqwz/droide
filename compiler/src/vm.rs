//! Type verifier and integer interpreter for the opcodes Droide emits.
//! const/4 0 is the zero type, which merges with both ints and references.

use std::collections::HashMap;

use crate::dex::{DexImage, MethodImage};
use crate::error::CompileError;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Bottom,
    Zero,
    Int,
    Ref,
    Float,
    Conflict,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::Bottom => "bottom",
            Ty::Zero => "zero",
            Ty::Int => "int",
            Ty::Ref => "ref",
            Ty::Float => "float",
            Ty::Conflict => "conflict",
        }
    }

    fn ok_int(self) -> bool {
        matches!(self, Ty::Int | Ty::Zero)
    }

    fn ok_ref(self) -> bool {
        matches!(self, Ty::Ref | Ty::Zero)
    }
}

fn s32(value: i64) -> i64 {
    let bits = value as u32;
    bits as i32 as i64
}

fn s16(value: u16) -> i32 {
    if value >= 0x8000 {
        value as i32 - 0x10000
    } else {
        value as i32
    }
}

fn py_tuple(items: &[Ty]) -> String {
    let parts: Vec<String> = items.iter().map(|ty| format!("'{}'", ty.name())).collect();
    match parts.len() {
        0 => "()".to_string(),
        1 => format!("({},)", parts[0]),
        _ => format!("({})", parts.join(", ")),
    }
}

fn len_at(words: &[u16], pc: usize) -> Result<usize, CompileError> {
    if pc >= words.len() {
        return Err(CompileError::bare(format!("unknown opcode at {pc:#x}")));
    }
    let op = words[pc] & 0xFF;
    if matches!(
        op,
        0x00 | 0x0E | 0x0F | 0x11 | 0x12 | 0x01 | 0x07 | 0x0A | 0x0B | 0x0C | 0x7B | 0x82 | 0x87
    ) {
        return Ok(1);
    }
    if matches!(
        op,
        0x13 | 0x16
            | 0x1A
            | 0x1C
            | 0x1F
            | 0x22
            | 0x23
            | 0x29
            | 0x32
            | 0x33
            | 0x34
            | 0x35
            | 0x36
            | 0x37
            | 0x38
            | 0x39
            | 0x52
            | 0x54
            | 0x59
            | 0x5B
            | 0x60
            | 0x90
            | 0x91
            | 0x92
            | 0x93
            | 0x94
            | 0x4D
            | 0xA6
            | 0xA7
            | 0xA8
            | 0xA9
    ) {
        return Ok(2);
    }
    if matches!(op, 0x14 | 0x6E | 0x6F | 0x70 | 0x71 | 0x72) {
        return Ok(3);
    }
    Err(CompileError::bare(format!(
        "unknown opcode {op:#x} at {pc:#x}"
    )))
}

fn merge(a: Ty, b: Ty) -> Ty {
    if a == b || b == Ty::Bottom {
        return a;
    }
    if a == Ty::Bottom {
        return b;
    }
    if a == Ty::Zero && matches!(b, Ty::Int | Ty::Ref) {
        return b;
    }
    if b == Ty::Zero && matches!(a, Ty::Int | Ty::Ref) {
        return a;
    }
    if a == Ty::Conflict || b == Ty::Conflict {
        return Ty::Conflict;
    }
    Ty::Conflict
}

fn invoke_regs(words: &[u16], pc: usize) -> Vec<usize> {
    let count = ((words[pc] >> 12) & 0xF) as usize;
    let word2 = words[pc + 2];
    let regs = [
        (word2 & 0xF) as usize,
        ((word2 >> 4) & 0xF) as usize,
        ((word2 >> 8) & 0xF) as usize,
        ((word2 >> 12) & 0xF) as usize,
        ((words[pc] >> 8) & 0xF) as usize,
    ];
    regs.into_iter().take(count).collect()
}

fn desc_ty(desc: &str) -> Ty {
    if desc == "F" {
        Ty::Float
    } else if desc.starts_with('L') || desc.starts_with('[') {
        Ty::Ref
    } else {
        Ty::Int
    }
}

fn field_type(image: &DexImage, index: usize) -> Result<Ty, CompileError> {
    let field = image
        .fields
        .get(index)
        .ok_or_else(|| CompileError::bare(format!("bad field index {index}")))?;
    Ok(desc_ty(&field.ty))
}

fn method_ret(image: &DexImage, index: usize) -> Result<Option<Ty>, CompileError> {
    let method = image
        .methods
        .get(index)
        .ok_or_else(|| CompileError::bare(format!("bad method index {index}")))?;
    if method.ret == "V" {
        Ok(None)
    } else {
        Ok(Some(desc_ty(&method.ret)))
    }
}

fn need(regs: &[Ty], reg: usize, kind: &str, pc: usize) -> Result<(), CompileError> {
    let ty = *regs
        .get(reg)
        .ok_or_else(|| CompileError::bare(format!("v{reg} out of range at {pc:#x}")))?;
    let ok = match kind {
        "int" => ty.ok_int(),
        "ref" => ty.ok_ref(),
        "float" => ty == Ty::Float,
        _ => false,
    };
    if !ok {
        return Err(CompileError::bare(format!(
            "v{reg} is {}, expected {kind} at {pc:#x}",
            ty.name()
        )));
    }
    Ok(())
}

fn put(regs: &mut [Ty], reg: usize, ty: Ty, pc: usize) -> Result<(), CompileError> {
    let slot = regs
        .get_mut(reg)
        .ok_or_else(|| CompileError::bare(format!("v{reg} out of range at {pc:#x}")))?;
    *slot = ty;
    Ok(())
}

fn jump(
    state: &mut HashMap<usize, Vec<Ty>>,
    work: &mut Vec<usize>,
    pc: usize,
    regs: &[Ty],
) -> Result<(), CompileError> {
    let merged = regs.to_vec();
    let Some(old) = state.get(&pc).cloned() else {
        state.insert(pc, merged);
        work.push(pc);
        return Ok(());
    };
    if old == merged {
        return Ok(());
    }
    let nxt: Vec<Ty> = old
        .iter()
        .zip(merged.iter())
        .map(|(a, b)| merge(*a, *b))
        .collect();
    if nxt.iter().any(|ty| *ty == Ty::Conflict) {
        return Err(CompileError::bare(format!(
            "register type conflict at {pc:#x}: {} vs {}",
            py_tuple(&old),
            py_tuple(&merged)
        )));
    }
    if nxt != old {
        state.insert(pc, nxt);
        work.push(pc);
    }
    Ok(())
}

fn branch_target(pc: usize, off: u16) -> Result<usize, CompileError> {
    let target = pc as i64 + s16(off) as i64;
    if target < 0 {
        return Err(CompileError::bare(format!("bad branch target {target:#x}")));
    }
    Ok(target as usize)
}

pub fn verify_image(image: &DexImage) -> Result<(), CompileError> {
    let mut errors = Vec::new();
    for (desc, cls) in &image.classes {
        for (key, method) in &cls.methods {
            if method.is_native || method.words.is_empty() {
                continue;
            }
            if let Err(err) = verify_method(image, method, &key.2) {
                errors.push(format!("{desc}->{}: {err}", key.0));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CompileError::bare(errors.join("; ")))
    }
}

fn verify_method(
    image: &DexImage,
    method: &MethodImage,
    params: &[String],
) -> Result<(), CompileError> {
    let words = &method.words;
    let count = method.registers as usize;
    if method.ins as usize > count {
        return Err(CompileError::bare("ins exceeds registers"));
    }
    let mut entry = vec![Ty::Bottom; count];
    let mut slot = count - method.ins as usize;
    if !method.is_static {
        put(&mut entry, slot, Ty::Ref, 0)?;
        slot += 1;
    }
    for desc in params {
        let ty = desc_ty(desc);
        put(&mut entry, slot, ty, 0)?;
        slot += 1;
    }
    let mut state: HashMap<usize, Vec<Ty>> = HashMap::new();
    state.insert(0, entry);
    let mut work = vec![0usize];
    let mut seen = 0usize;
    while let Some(start) = work.pop() {
        seen += 1;
        if seen > 100000 {
            return Err(CompileError::bare("verifier did not converge"));
        }
        let mut regs = state[&start].clone();
        let mut pc = start;
        let mut pending: Option<Ty> = None;
        loop {
            if pc >= words.len() {
                return Err(CompileError::bare(format!("unknown opcode at {pc:#x}")));
            }
            let op = words[pc] & 0xFF;
            let size = len_at(words, pc)?;
            if pc + size > words.len() {
                return Err(CompileError::bare(format!(
                    "truncated instruction at {pc:#x}"
                )));
            }
            let mut end = false;
            let mut nxt = Some(pc + size);
            match op {
                0x00 => {}
                0x12 => {
                    let reg = ((words[pc] >> 8) & 0xF) as usize;
                    let nibble = (words[pc] >> 12) & 0xF;
                    let value = if nibble < 8 {
                        nibble as i32
                    } else {
                        nibble as i32 - 16
                    };
                    put(
                        &mut regs,
                        reg,
                        if value == 0 { Ty::Zero } else { Ty::Int },
                        pc,
                    )?;
                }
                0x13 => {
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    let value = s16(words[pc + 1]);
                    put(
                        &mut regs,
                        reg,
                        if value == 0 { Ty::Zero } else { Ty::Int },
                        pc,
                    )?;
                }
                0x14 => {
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    let raw = (words[pc + 1] as u32) | ((words[pc + 2] as u32) << 16);
                    put(
                        &mut regs,
                        reg,
                        if raw == 0 { Ty::Zero } else { Ty::Int },
                        pc,
                    )?;
                }
                0x16 => {
                    // A long is two register words. Search's latch timeout is the only user.
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    if reg % 2 != 0 {
                        return Err(CompileError::bare(format!(
                            "const-wide v{reg} is odd"
                        )));
                    }
                    put(&mut regs, reg, Ty::Int, pc)?;
                    put(&mut regs, reg + 1, Ty::Int, pc)?;
                }
                0x60 => {
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    put(
                        &mut regs,
                        reg,
                        field_type(image, words[pc + 1] as usize)?,
                        pc,
                    )?;
                }
                0x1A | 0x1C | 0x22 => {
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    put(&mut regs, reg, Ty::Ref, pc)?;
                }
                0x01 | 0x07 => {
                    let dest = ((words[pc] >> 8) & 0xF) as usize;
                    let src = ((words[pc] >> 12) & 0xF) as usize;
                    if op == 0x07 {
                        need(&regs, src, "ref", pc)?;
                        let ty = regs[src];
                        put(
                            &mut regs,
                            dest,
                            if ty != Ty::Zero { Ty::Ref } else { Ty::Zero },
                            pc,
                        )?;
                    } else {
                        let ty = *regs.get(src).ok_or_else(|| {
                            CompileError::bare(format!("v{src} out of range at {pc:#x}"))
                        })?;
                        put(&mut regs, dest, ty, pc)?;
                        if regs[dest] == Ty::Bottom {
                            return Err(CompileError::bare(format!("move of undefined v{src}")));
                        }
                    }
                }
                0x0A | 0x0B | 0x0C => {
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    let Some(value) = pending else {
                        return Err(CompileError::bare(format!(
                            "move-result without invoke at {pc:#x}"
                        )));
                    };
                    if op == 0x0B {
                        // A double is two register words. The dialect never sees the wide value.
                        if value == Ty::Ref {
                            return Err(CompileError::bare("move-result-wide of a reference"));
                        }
                        if reg % 2 != 0 {
                            return Err(CompileError::bare(format!(
                                "move-result-wide v{reg} is odd"
                            )));
                        }
                        put(&mut regs, reg, Ty::Int, pc)?;
                        put(&mut regs, reg + 1, Ty::Int, pc)?;
                    } else {
                        if op == 0x0C && value != Ty::Ref {
                            return Err(CompileError::bare("move-result-object of a non-reference"));
                        }
                        if op == 0x0A && value == Ty::Ref {
                            return Err(CompileError::bare("move-result of a reference"));
                        }
                        put(&mut regs, reg, value, pc)?;
                    }
                    pending = None;
                }
                0x7B => {
                    let dest = ((words[pc] >> 8) & 0xF) as usize;
                    let src = ((words[pc] >> 12) & 0xF) as usize;
                    need(&regs, src, "int", pc)?;
                    put(&mut regs, dest, Ty::Int, pc)?;
                }
                0x82 => {
                    let dest = ((words[pc] >> 8) & 0xF) as usize;
                    let src = ((words[pc] >> 12) & 0xF) as usize;
                    need(&regs, src, "int", pc)?;
                    put(&mut regs, dest, Ty::Float, pc)?;
                }
                0x87 => {
                    let dest = ((words[pc] >> 8) & 0xF) as usize;
                    let src = ((words[pc] >> 12) & 0xF) as usize;
                    need(&regs, src, "float", pc)?;
                    put(&mut regs, dest, Ty::Int, pc)?;
                }
                0x90 | 0x91 | 0x92 | 0x93 | 0x94 | 0xA6 | 0xA7 | 0xA8 | 0xA9 => {
                    let dest = ((words[pc] >> 8) & 0xFF) as usize;
                    let a = (words[pc + 1] & 0xFF) as usize;
                    let b = ((words[pc + 1] >> 8) & 0xFF) as usize;
                    let kind = if op >= 0xA6 { "float" } else { "int" };
                    need(&regs, a, kind, pc)?;
                    need(&regs, b, kind, pc)?;
                    let produced = if op >= 0xA6 { Ty::Float } else { Ty::Int };
                    put(&mut regs, dest, produced, pc)?;
                }
                0x52 | 0x54 => {
                    let dest = ((words[pc] >> 8) & 0xF) as usize;
                    let obj = ((words[pc] >> 12) & 0xF) as usize;
                    need(&regs, obj, "ref", pc)?;
                    put(
                        &mut regs,
                        dest,
                        field_type(image, words[pc + 1] as usize)?,
                        pc,
                    )?;
                }
                0x59 | 0x5B => {
                    let src = ((words[pc] >> 8) & 0xF) as usize;
                    let obj = ((words[pc] >> 12) & 0xF) as usize;
                    need(&regs, obj, "ref", pc)?;
                    let fty = field_type(image, words[pc + 1] as usize)?;
                    let kind = match fty {
                        Ty::Ref => "ref",
                        Ty::Float => "float",
                        _ => "int",
                    };
                    need(&regs, src, kind, pc)?;
                }
                0x6E | 0x6F | 0x70 | 0x71 | 0x72 => {
                    let method_index = words[pc + 1] as usize;
                    let args = invoke_regs(words, pc);
                    let ret = method_ret(image, method_index)?;
                    let called = image.methods.get(method_index).ok_or_else(|| {
                        CompileError::bare(format!("bad method index {method_index}"))
                    })?;
                    for reg in &args {
                        let ty = regs.get(*reg).copied().ok_or_else(|| {
                            CompileError::bare(format!("v{reg} out of range at {pc:#x}"))
                        })?;
                        if ty == Ty::Bottom {
                            return Err(CompileError::bare(format!(
                                "invoke uses undefined v{reg}"
                            )));
                        }
                    }
                    if called.name == "<init>" && !args.is_empty() {
                        put(&mut regs, args[0], Ty::Ref, pc)?;
                    }
                    pending = ret;
                }
                0x0E => {
                    end = true;
                    nxt = None;
                }
                0x0F => {
                    need(&regs, ((words[pc] >> 8) & 0xFF) as usize, "int", pc)?;
                    end = true;
                    nxt = None;
                }
                0x11 => {
                    need(&regs, ((words[pc] >> 8) & 0xFF) as usize, "ref", pc)?;
                    end = true;
                    nxt = None;
                }
                0x29 => {
                    end = true;
                    nxt = Some(branch_target(pc, words[pc + 1])?);
                }
                0x1F => {
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    need(&regs, reg, "ref", pc)?;
                    put(&mut regs, reg, Ty::Ref, pc)?;
                }
                0x23 => {
                    let dest = ((words[pc] >> 8) & 0xF) as usize;
                    let size = ((words[pc] >> 12) & 0xF) as usize;
                    need(&regs, size, "int", pc)?;
                    put(&mut regs, dest, Ty::Ref, pc)?;
                }
                0x4D => {
                    let value = ((words[pc] >> 8) & 0xFF) as usize;
                    let array = (words[pc + 1] & 0xFF) as usize;
                    let index = ((words[pc + 1] >> 8) & 0xFF) as usize;
                    need(&regs, value, "ref", pc)?;
                    need(&regs, array, "ref", pc)?;
                    need(&regs, index, "int", pc)?;
                }
                0x38 | 0x39 => {
                    // ART allows if-eqz and if-nez on an int or a reference. A null location uses that.
                    let reg = ((words[pc] >> 8) & 0xFF) as usize;
                    let ty = *regs.get(reg).ok_or_else(|| {
                        CompileError::bare(format!("v{reg} out of range at {pc:#x}"))
                    })?;
                    if !(ty.ok_int() || ty.ok_ref()) {
                        return Err(CompileError::bare(format!(
                            "v{reg} is {}, expected int or ref at {pc:#x}",
                            ty.name()
                        )));
                    }
                    jump(
                        &mut state,
                        &mut work,
                        branch_target(pc, words[pc + 1])?,
                        &regs,
                    )?;
                    end = true;
                    nxt = Some(pc + 2);
                }
                0x32 | 0x33 | 0x34 | 0x35 | 0x36 | 0x37 => {
                    let a = ((words[pc] >> 8) & 0xF) as usize;
                    let b = ((words[pc] >> 12) & 0xF) as usize;
                    need(&regs, a, "int", pc)?;
                    need(&regs, b, "int", pc)?;
                    jump(
                        &mut state,
                        &mut work,
                        branch_target(pc, words[pc + 1])?,
                        &regs,
                    )?;
                    end = true;
                    nxt = Some(pc + 2);
                }
                _ => {
                    return Err(CompileError::bare(format!(
                        "verifier does not know opcode {op:#x}"
                    )))
                }
            }
            if pending.is_some() && !matches!(op, 0x6E | 0x6F | 0x70 | 0x71 | 0x72) {
                return Err(CompileError::bare("invoke result was not moved"));
            }
            if !end {
                pc = nxt.expect("straight-line successor");
                continue;
            }
            if let Some(next) = nxt {
                jump(&mut state, &mut work, next, &regs)?;
            }
            break;
        }
    }
    Ok(())
}

pub fn run_static(
    image: &DexImage,
    desc: &str,
    name: &str,
    args: &[i32],
) -> Result<i32, CompileError> {
    let class = image
        .classes
        .get(desc)
        .ok_or_else(|| CompileError::bare(format!("no static method {name}")))?;
    let mut found: Option<&MethodImage> = None;
    for ((mname, _ret, _params), method) in &class.methods {
        if mname == name && method.is_static && !method.is_native {
            found = Some(method);
            break;
        }
    }
    let Some(method) = found else {
        return Err(CompileError::bare(format!("no static method {name}")));
    };
    let widened: Vec<i64> = args.iter().map(|arg| *arg as i64).collect();
    match run_method(image, method, &widened, 0)? {
        Some(value) => Ok(value as i32),
        None => Err(CompileError::bare("void return")),
    }
}

fn run_method(
    image: &DexImage,
    method: &MethodImage,
    args: &[i64],
    depth: u32,
) -> Result<Option<i64>, CompileError> {
    if depth > 40 {
        return Err(CompileError::bare("interpreter recursion limit"));
    }
    if method.ins > method.registers {
        return Err(CompileError::bare("ins exceeds registers"));
    }
    let mut regs = vec![0i64; method.registers as usize];
    let base = method.registers as usize - method.ins as usize;
    for (index, arg) in args.iter().enumerate() {
        let slot = base + index;
        if slot >= regs.len() {
            return Err(CompileError::bare("too many arguments"));
        }
        regs[slot] = s32(*arg);
    }
    let words = &method.words;
    let mut pc = 0usize;
    let mut pending: Option<i64> = None;
    let mut steps = 0usize;
    while steps < 100000 {
        steps += 1;
        if pc >= words.len() {
            return Err(CompileError::bare(format!(
                "interpreter cannot execute opcode at {pc:#x}"
            )));
        }
        let op = words[pc] & 0xFF;
        let size = len_at(words, pc).map_err(|_| {
            CompileError::bare(format!("interpreter cannot execute opcode {op:#x}"))
        })?;
        if pc + size > words.len() {
            return Err(CompileError::bare(format!(
                "interpreter cannot execute opcode {op:#x}"
            )));
        }
        match op {
            0x12 => {
                let reg = ((words[pc] >> 8) & 0xF) as usize;
                let nibble = (words[pc] >> 12) & 0xF;
                let value = if nibble < 8 {
                    nibble as i64
                } else {
                    nibble as i64 - 16
                };
                *regs
                    .get_mut(reg)
                    .ok_or_else(|| CompileError::bare(format!("v{reg} out of range")))? = value;
                pc += 1;
            }
            0x13 => {
                let reg = ((words[pc] >> 8) & 0xFF) as usize;
                *regs
                    .get_mut(reg)
                    .ok_or_else(|| CompileError::bare(format!("v{reg} out of range")))? =
                    s16(words[pc + 1]) as i64;
                pc += 2;
            }
            0x14 => {
                let reg = ((words[pc] >> 8) & 0xFF) as usize;
                let raw = (words[pc + 1] as u32) | ((words[pc + 2] as u32) << 16);
                *regs
                    .get_mut(reg)
                    .ok_or_else(|| CompileError::bare(format!("v{reg} out of range")))? =
                    s32(raw as i64);
                pc += 3;
            }
            0x01 => {
                let dest = ((words[pc] >> 8) & 0xF) as usize;
                let src = ((words[pc] >> 12) & 0xF) as usize;
                let value = *regs
                    .get(src)
                    .ok_or_else(|| CompileError::bare(format!("v{src} out of range")))?;
                *regs
                    .get_mut(dest)
                    .ok_or_else(|| CompileError::bare(format!("v{dest} out of range")))? = value;
                pc += 1;
            }
            0x0A => {
                let reg = ((words[pc] >> 8) & 0xFF) as usize;
                let Some(value) = pending else {
                    return Err(CompileError::bare("move-result without a value"));
                };
                *regs
                    .get_mut(reg)
                    .ok_or_else(|| CompileError::bare(format!("v{reg} out of range")))? = value;
                pending = None;
                pc += 1;
            }
            0x7B => {
                let dest = ((words[pc] >> 8) & 0xF) as usize;
                let src = ((words[pc] >> 12) & 0xF) as usize;
                let value = *regs
                    .get(src)
                    .ok_or_else(|| CompileError::bare(format!("v{src} out of range")))?;
                *regs
                    .get_mut(dest)
                    .ok_or_else(|| CompileError::bare(format!("v{dest} out of range")))? =
                    s32(-value);
                pc += 1;
            }
            0x90 | 0x91 | 0x92 | 0x93 | 0x94 => {
                let dest = ((words[pc] >> 8) & 0xFF) as usize;
                let a = (words[pc + 1] & 0xFF) as usize;
                let b = ((words[pc + 1] >> 8) & 0xFF) as usize;
                let left = *regs
                    .get(a)
                    .ok_or_else(|| CompileError::bare(format!("v{a} out of range")))?;
                let right = *regs
                    .get(b)
                    .ok_or_else(|| CompileError::bare(format!("v{b} out of range")))?;
                let value = match op {
                    0x90 => s32(left + right),
                    0x91 => s32(left - right),
                    0x92 => s32(left * right),
                    0x93 | 0x94 => {
                        if right == 0 {
                            return Err(CompileError::bare("division by zero"));
                        }
                        // Python `int(left / right)` is truncating float division.
                        let quot = ((left as f64) / (right as f64)) as i64;
                        if op == 0x93 {
                            quot
                        } else {
                            left - quot * right
                        }
                    }
                    _ => unreachable!(),
                };
                *regs
                    .get_mut(dest)
                    .ok_or_else(|| CompileError::bare(format!("v{dest} out of range")))? = value;
                pc += 2;
            }
            0x0F => {
                let reg = ((words[pc] >> 8) & 0xFF) as usize;
                let value = *regs
                    .get(reg)
                    .ok_or_else(|| CompileError::bare(format!("v{reg} out of range")))?;
                return Ok(Some(s32(value)));
            }
            0x0E => return Ok(None),
            0x29 => {
                pc = branch_target(pc, words[pc + 1])?;
            }
            0x38 | 0x39 => {
                let reg = ((words[pc] >> 8) & 0xFF) as usize;
                let value = *regs
                    .get(reg)
                    .ok_or_else(|| CompileError::bare(format!("v{reg} out of range")))?;
                let take = if op == 0x38 { value == 0 } else { value != 0 };
                pc = if take {
                    branch_target(pc, words[pc + 1])?
                } else {
                    pc + 2
                };
            }
            0x32 | 0x33 | 0x34 | 0x35 | 0x36 | 0x37 => {
                let a = ((words[pc] >> 8) & 0xF) as usize;
                let b = ((words[pc] >> 12) & 0xF) as usize;
                let left = *regs
                    .get(a)
                    .ok_or_else(|| CompileError::bare(format!("v{a} out of range")))?;
                let right = *regs
                    .get(b)
                    .ok_or_else(|| CompileError::bare(format!("v{b} out of range")))?;
                let take = match op {
                    0x32 => left == right,
                    0x33 => left != right,
                    0x34 => left < right,
                    0x35 => left >= right,
                    0x36 => left > right,
                    0x37 => left <= right,
                    _ => unreachable!(),
                };
                pc = if take {
                    branch_target(pc, words[pc + 1])?
                } else {
                    pc + 2
                };
            }
            0x71 => {
                let arg_regs = invoke_regs(words, pc);
                let called = image.methods.get(words[pc + 1] as usize).ok_or_else(|| {
                    CompileError::bare(format!("bad method index {}", words[pc + 1]))
                })?;
                let target = image.classes.get(&called.owner).and_then(|cls| {
                    cls.methods.get(&(
                        called.name.clone(),
                        called.ret.clone(),
                        called.params.clone(),
                    ))
                });
                let Some(target) = target else {
                    return Err(CompileError::bare(format!(
                        "interpreter cannot call {}->{}",
                        called.owner, called.name
                    )));
                };
                if target.is_native {
                    return Err(CompileError::bare(format!(
                        "interpreter cannot call {}->{}",
                        called.owner, called.name
                    )));
                }
                let mut call_args = Vec::with_capacity(arg_regs.len());
                for reg in arg_regs {
                    call_args.push(
                        *regs
                            .get(reg)
                            .ok_or_else(|| CompileError::bare(format!("v{reg} out of range")))?,
                    );
                }
                pending = run_method(image, target, &call_args, depth + 1)?;
                pc += 3;
            }
            _ => {
                return Err(CompileError::bare(format!(
                    "interpreter cannot execute opcode {op:#x}"
                )))
            }
        }
    }
    Err(CompileError::bare("interpreter ran too long"))
}
