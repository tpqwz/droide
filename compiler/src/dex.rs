//! DEX assembler and file writer. DEX 035, no external toolchain.

use std::collections::{BTreeMap, HashMap, HashSet};

use sha1::{Digest, Sha1};

use crate::error::CompileError;

pub const ACC_PUBLIC: u32 = 0x1;
pub const ACC_PRIVATE: u32 = 0x2;
pub const ACC_STATIC: u32 = 0x8;
pub const ACC_FINAL: u32 = 0x10;
pub const ACC_SUPER: u32 = 0x20;
pub const ACC_NATIVE: u32 = 0x100;
pub const ACC_INTERFACE: u32 = 0x200;
pub const ACC_ABSTRACT: u32 = 0x400;
pub const ACC_CONSTRUCTOR: u32 = 0x10000;

const NO_INDEX: u32 = 0xFFFFFFFF;
const MAGIC: &[u8; 8] = b"dex\n035\0";

fn uleb(mut value: u32) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            return out;
        }
    }
}

fn mutf8(text: &str) -> Result<Vec<u8>, CompileError> {
    let mut out = Vec::new();
    for ch in text.chars() {
        let code = ch as u32;
        if code == 0 {
            out.extend_from_slice(&[0xC0, 0x80]);
        } else if code < 0x80 {
            out.push(code as u8);
        } else if code < 0x800 {
            out.push((0xC0 | (code >> 6)) as u8);
            out.push((0x80 | (code & 0x3F)) as u8);
        } else if code < 0x10000 {
            out.push((0xE0 | (code >> 12)) as u8);
            out.push((0x80 | ((code >> 6) & 0x3F)) as u8);
            out.push((0x80 | (code & 0x3F)) as u8);
        } else {
            return Err(CompileError::bare("string is outside the BMP"));
        }
    }
    Ok(out)
}

fn shorty_char(desc: &str) -> &str {
    match desc {
        "V" | "I" | "Z" | "B" | "S" | "C" | "F" | "J" | "D" => desc,
        _ => "L",
    }
}

fn shorty(ret: &str, params: &[String]) -> String {
    let mut out = String::from(shorty_char(ret));
    for param in params {
        out.push_str(shorty_char(param));
    }
    out
}

/// zlib.adler32: s1 starts at 1, modulus 65521.
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn align_up(value: usize, n: usize) -> usize {
    (value + n - 1) & !(n - 1)
}

fn py_repr(s: &str) -> String {
    let prefer_double = s.contains('\'') && !s.contains('"');
    let quote = if prefer_double { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' if !prefer_double => out.push_str("\\'"),
            '"' if prefer_double => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

fn py_tuple(items: &[String]) -> String {
    match items {
        [] => "()".to_string(),
        [one] => format!("({},)", py_repr(one)),
        many => {
            let parts: Vec<String> = many.iter().map(|s| py_repr(s)).collect();
            format!("({})", parts.join(", "))
        }
    }
}

fn put_u16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn as_u16(value: usize) -> Result<u16, CompileError> {
    u16::try_from(value).map_err(|_| CompileError::bare("dex index overflow"))
}

fn as_u32(value: usize) -> Result<u32, CompileError> {
    u32::try_from(value).map_err(|_| CompileError::bare("dex index overflow"))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MethodRef {
    pub owner: String,
    pub name: String,
    pub ret: String,
    pub params: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldRef {
    pub owner: String,
    pub name: String,
    pub ty: String,
}

enum Part {
    Word(u16),
    Str(String),
    Type(String),
    Method(MethodRef),
    Field(FieldRef),
}

enum Item {
    Op {
        parts: Vec<Part>,
        text: String,
    },
    Label(String),
    Branch {
        kind: String,
        regs: Vec<u16>,
        label: String,
        text: String,
    },
}

pub struct Asm {
    items: Vec<Item>,
    error: Option<String>,
}

struct Resolver<'a> {
    strings: &'a HashMap<String, usize>,
    types: &'a HashMap<String, usize>,
    fields: &'a HashMap<FieldRef, usize>,
    methods: &'a HashMap<MethodRef, usize>,
}

impl<'a> Resolver<'a> {
    fn string(&self, value: &str) -> u16 {
        (self.strings[value] & 0xFFFF) as u16
    }

    fn ty(&self, desc: &str) -> u16 {
        (self.types[desc] & 0xFFFF) as u16
    }

    fn field(&self, key: &FieldRef) -> u16 {
        (self.fields[key] & 0xFFFF) as u16
    }

    fn method(&self, key: &MethodRef) -> u16 {
        (self.methods[key] & 0xFFFF) as u16
    }
}

impl Asm {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            error: None,
        }
    }

    fn fail(&mut self, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(message.into());
        }
    }

    fn op(&mut self, parts: Vec<Part>, text: String) {
        self.items.push(Item::Op { parts, text });
    }

    pub fn label(&mut self, name: &str) {
        self.items.push(Item::Label(name.to_string()));
    }

    fn branch(&mut self, kind: &str, regs: Vec<u16>, label: &str, text: String) {
        self.items.push(Item::Branch {
            kind: kind.to_string(),
            regs,
            label: label.to_string(),
            text,
        });
    }

    pub fn const_(&mut self, reg: u16, value: i64) {
        if (-8..=7).contains(&value) {
            let nibble = (value as u16) & 0xF;
            let word = 0x12u16 | ((reg & 0xF) << 8) | (nibble << 12);
            self.op(vec![Part::Word(word)], format!("const/4 v{reg}, #{value}"));
        } else if (-32768..=32767).contains(&value) {
            let word0 = 0x13u16 | ((reg & 0xFF) << 8);
            let word1 = value as u16;
            self.op(
                vec![Part::Word(word0), Part::Word(word1)],
                format!("const/16 v{reg}, #{value}"),
            );
        } else {
            let raw = value as u32;
            let word0 = 0x14u16 | ((reg & 0xFF) << 8);
            let lo = raw as u16;
            let hi = (raw >> 16) as u16;
            self.op(
                vec![Part::Word(word0), Part::Word(lo), Part::Word(hi)],
                format!("const v{reg}, #{value}"),
            );
        }
    }

    /// `const-wide/16`. The value occupies `reg` and `reg + 1`, so `reg` is even.
    pub fn const_wide(&mut self, reg: u16, value: i64) {
        if reg % 2 != 0 {
            self.fail(format!("const-wide v{reg} is odd"));
            return;
        }
        if !(-32768..=32767).contains(&value) {
            self.fail("const-wide is too wide");
            return;
        }
        let word0 = 0x16u16 | ((reg & 0xFF) << 8);
        self.op(
            vec![Part::Word(word0), Part::Word(value as u16)],
            format!("const-wide/16 v{reg}, #{value}"),
        );
    }

    /// `sget`. The field index is 16 bits and the destination register is 8 bits.
    pub fn sget(&mut self, dest: u16, key: FieldRef) {
        let name = key.name.clone();
        let word = 0x60u16 | ((dest & 0xFF) << 8);
        self.op(
            vec![Part::Word(word), Part::Field(key)],
            format!("sget v{dest}, {name}"),
        );
    }

    pub fn const_string(&mut self, reg: u16, text: &str) {
        let mut shown = String::new();
        for c in text.chars() {
            match c {
                '\\' => shown.push_str("\\\\"),
                '"' => shown.push_str("\\\""),
                c => shown.push(c),
            }
        }
        let word = 0x1Au16 | ((reg & 0xFF) << 8);
        self.op(
            vec![Part::Word(word), Part::Str(text.to_string())],
            format!("const-string v{reg}, \"{shown}\""),
        );
    }

    pub fn const_class(&mut self, reg: u16, desc: &str) {
        let word = 0x1Cu16 | ((reg & 0xFF) << 8);
        self.op(
            vec![Part::Word(word), Part::Type(desc.to_string())],
            format!("const-class v{reg}, {desc}"),
        );
    }

    pub fn move_(&mut self, dest: u16, src: u16, obj: bool) {
        if dest == src {
            return;
        }
        let op = if obj { 0x07u16 } else { 0x01u16 };
        let word = op | ((dest & 0xF) << 8) | ((src & 0xF) << 12);
        let name = if obj { "move-object" } else { "move" };
        self.op(vec![Part::Word(word)], format!("{name} v{dest}, v{src}"));
    }

    pub fn move_result(&mut self, reg: u16, obj: bool) {
        let op = if obj { 0x0Cu16 } else { 0x0Au16 };
        let word = op | ((reg & 0xFF) << 8);
        let name = if obj {
            "move-result-object"
        } else {
            "move-result"
        };
        self.op(vec![Part::Word(word)], format!("{name} v{reg}"));
    }

    /// Wide results occupy `reg` and `reg + 1`. The register must be even.
    pub fn move_result_wide(&mut self, reg: u16) {
        let word = 0x0Bu16 | ((reg & 0xFF) << 8);
        self.op(vec![Part::Word(word)], format!("move-result-wide v{reg}"));
    }

    pub fn check_cast(&mut self, reg: u16, desc: &str) {
        let word = 0x1Fu16 | ((reg & 0xFF) << 8);
        self.op(
            vec![Part::Word(word), Part::Type(desc.to_string())],
            format!("check-cast v{reg}, {desc}"),
        );
    }

    /// `new-array` is format 22c: destination and size are both 4-bit registers.
    pub fn new_array(&mut self, dest: u16, size: u16, desc: &str) {
        let word = 0x23u16 | ((dest & 0xF) << 8) | ((size & 0xF) << 12);
        self.op(
            vec![Part::Word(word), Part::Type(desc.to_string())],
            format!("new-array v{dest}, v{size}, {desc}"),
        );
    }

    /// `aput-object vAA, vBB, vCC`: value, array, index.
    pub fn aput_object(&mut self, value: u16, array: u16, index: u16) {
        let word0 = 0x4Du16 | ((value & 0xFF) << 8);
        let word1 = (array & 0xFF) | ((index & 0xFF) << 8);
        self.op(
            vec![Part::Word(word0), Part::Word(word1)],
            format!("aput-object v{value}, v{array}, v{index}"),
        );
    }

    pub fn ret(&mut self, reg: u16, kind: &str) {
        if kind == "void" {
            self.op(vec![Part::Word(0x0E)], "return-void".to_string());
        } else if kind == "obj" {
            let word = 0x11u16 | ((reg & 0xFF) << 8);
            self.op(vec![Part::Word(word)], format!("return-object v{reg}"));
        } else {
            let word = 0x0Fu16 | ((reg & 0xFF) << 8);
            self.op(vec![Part::Word(word)], format!("return v{reg}"));
        }
    }

    pub fn new_instance(&mut self, reg: u16, desc: &str) {
        let word = 0x22u16 | ((reg & 0xFF) << 8);
        self.op(
            vec![Part::Word(word), Part::Type(desc.to_string())],
            format!("new-instance v{reg}, {desc}"),
        );
    }

    pub fn invoke(&mut self, kind: &str, key: MethodRef, regs: &[u16]) {
        if regs.len() > 5 {
            self.fail("invoke has more than 5 registers");
            return;
        }
        let opcode = match kind {
            "virtual" => 0x6Eu16,
            "super" => 0x6F,
            "direct" => 0x70,
            "static" => 0x71,
            "interface" => 0x72,
            other => {
                self.fail(format!("unknown invoke kind {other}"));
                return;
            }
        };
        let mut padded = [0u16; 5];
        for (i, reg) in regs.iter().enumerate() {
            padded[i] = reg & 0xF;
        }
        let (c, d, e, f, g) = (padded[0], padded[1], padded[2], padded[3], padded[4]);
        // 35c: count is the high nibble, fifth register is bits 8-11.
        let word0 = opcode | (g << 8) | (((regs.len() as u16) & 0xF) << 12);
        let word2 = c | (d << 4) | (e << 8) | (f << 12);
        let args = regs
            .iter()
            .map(|r| format!("v{r}"))
            .collect::<Vec<_>>()
            .join(", ");
        let text = format!("invoke-{kind} {{{args}}}, {}->{}", key.owner, key.name);
        self.op(
            vec![Part::Word(word0), Part::Method(key), Part::Word(word2)],
            text,
        );
    }

    fn field_op(&mut self, op: u16, a: u16, b: u16, key: FieldRef, text: String) {
        let word = op | ((a & 0xF) << 8) | ((b & 0xF) << 12);
        self.op(vec![Part::Word(word), Part::Field(key)], text);
    }

    pub fn iget(&mut self, dest: u16, obj: u16, key: FieldRef) {
        let name = key.name.clone();
        self.field_op(
            0x52,
            dest,
            obj,
            key,
            format!("iget v{dest}, v{obj}, {name}"),
        );
    }

    pub fn iget_obj(&mut self, dest: u16, obj: u16, key: FieldRef) {
        let name = key.name.clone();
        self.field_op(
            0x54,
            dest,
            obj,
            key,
            format!("iget-object v{dest}, v{obj}, {name}"),
        );
    }

    pub fn iput(&mut self, src: u16, obj: u16, key: FieldRef) {
        let name = key.name.clone();
        self.field_op(0x59, src, obj, key, format!("iput v{src}, v{obj}, {name}"));
    }

    pub fn iput_obj(&mut self, src: u16, obj: u16, key: FieldRef) {
        let name = key.name.clone();
        self.field_op(
            0x5B,
            src,
            obj,
            key,
            format!("iput-object v{src}, v{obj}, {name}"),
        );
    }

    pub fn binop(&mut self, op: u8, dest: u16, a: u16, b: u16, name: &str) {
        let word0 = (op as u16) | ((dest & 0xFF) << 8);
        let word1 = (a & 0xFF) | ((b & 0xFF) << 8);
        self.op(
            vec![Part::Word(word0), Part::Word(word1)],
            format!("{name} v{dest}, v{a}, v{b}"),
        );
    }

    pub fn unop(&mut self, op: u8, dest: u16, src: u16, name: &str) {
        let word = (op as u16) | ((dest & 0xF) << 8) | ((src & 0xF) << 12);
        self.op(vec![Part::Word(word)], format!("{name} v{dest}, v{src}"));
    }

    pub fn if_zero(&mut self, kind: &str, reg: u16, label: &str) {
        if kind != "eqz" && kind != "nez" {
            self.fail(format!("unknown if kind {kind}"));
            return;
        }
        self.branch(kind, vec![reg], label, format!("if-{kind} v{reg}"));
    }

    pub fn if_cmp(&mut self, kind: &str, a: u16, b: u16, label: &str) {
        self.branch(kind, vec![a, b], label, format!("if-{kind} v{a}, v{b}"));
    }

    pub fn goto(&mut self, label: &str) {
        self.branch("goto", Vec::new(), label, "goto".to_string());
    }

    fn encode(&self, resolver: &Resolver) -> Result<(Vec<u16>, Vec<String>), CompileError> {
        if let Some(message) = &self.error {
            return Err(CompileError::bare(message.clone()));
        }
        let mut labels: HashMap<String, usize> = HashMap::new();
        let mut pos = 0usize;
        for item in &self.items {
            match item {
                Item::Label(name) => {
                    if labels.contains_key(name) {
                        return Err(CompileError::bare(format!("duplicate label {name}")));
                    }
                    labels.insert(name.clone(), pos);
                }
                Item::Op { parts, .. } => pos += parts.len(),
                Item::Branch { .. } => pos += 2,
            }
        }
        let mut words = Vec::new();
        let mut listing = Vec::new();
        pos = 0;
        for item in &self.items {
            match item {
                Item::Label(name) => {
                    listing.push(format!("{pos:04x}  :{name}"));
                }
                Item::Op { parts, text } => {
                    let mut encoded = Vec::with_capacity(parts.len());
                    for part in parts {
                        let word = match part {
                            Part::Word(value) => *value,
                            Part::Str(value) => resolver.string(value),
                            Part::Type(desc) => resolver.ty(desc),
                            Part::Method(key) => resolver.method(key),
                            Part::Field(key) => resolver.field(key),
                        };
                        encoded.push(word);
                    }
                    listing.push(format!("{pos:04x}  {text}"));
                    pos += encoded.len();
                    words.extend(encoded);
                }
                Item::Branch {
                    kind,
                    regs,
                    label,
                    text,
                } => {
                    let Some(&target) = labels.get(label) else {
                        return Err(CompileError::bare(format!("unknown label {label}")));
                    };
                    let offset = target as i64 - pos as i64;
                    if !(-32768..=32767).contains(&offset) {
                        return Err(CompileError::bare("branch is too far"));
                    }
                    let off = offset as u16;
                    let pair = if kind == "goto" {
                        [0x29u16, off]
                    } else if kind == "eqz" {
                        [0x38u16 | ((regs[0] & 0xFF) << 8), off]
                    } else if kind == "nez" {
                        [0x39u16 | ((regs[0] & 0xFF) << 8), off]
                    } else {
                        let opcode = match kind.as_str() {
                            "eq" => 0x32u16,
                            "ne" => 0x33,
                            "lt" => 0x34,
                            "ge" => 0x35,
                            "gt" => 0x36,
                            "le" => 0x37,
                            _ => return Err(CompileError::bare(format!("unknown branch {kind}"))),
                        };
                        let (a, b) = (regs[0], regs[1]);
                        [opcode | ((a & 0xF) << 8) | ((b & 0xF) << 12), off]
                    };
                    listing.push(format!("{pos:04x}  {text} -> {label}"));
                    words.extend(pair);
                    pos += 2;
                }
            }
        }
        // Trailing nop so insns_size is even (a real nop, not a padding ushort outside the count).
        if words.len() % 2 == 1 {
            listing.push(format!("{pos:04x}  nop"));
            words.push(0x0000);
        }
        Ok((words, listing))
    }
}

pub struct MethodModel {
    pub name: String,
    pub ret: String,
    pub params: Vec<String>,
    pub access: u32,
    pub asm: Option<Asm>,
    pub registers: u16,
    pub ins: u16,
    pub outs: u16,
    pub words: Vec<u16>,
    pub listing: Vec<String>,
}

impl MethodModel {
    pub fn new(
        name: &str,
        ret: &str,
        params: Vec<String>,
        access: u32,
        asm: Option<Asm>,
        registers: u16,
        ins: u16,
        outs: u16,
    ) -> Self {
        Self {
            name: name.to_string(),
            ret: ret.to_string(),
            params,
            access,
            asm,
            registers,
            ins,
            outs,
            words: Vec::new(),
            listing: Vec::new(),
        }
    }

    fn is_direct(&self) -> bool {
        self.access & (ACC_STATIC | ACC_PRIVATE | ACC_CONSTRUCTOR) != 0
    }

    fn key(&self, owner: &str) -> MethodRef {
        MethodRef {
            owner: owner.to_string(),
            name: self.name.clone(),
            ret: self.ret.clone(),
            params: self.params.clone(),
        }
    }
}

pub struct ClassModel {
    pub desc: String,
    pub super_desc: String,
    pub access: u32,
    pub source: String,
    pub interfaces: Vec<String>,
    fields: Vec<(String, String, u32)>,
    methods: Vec<MethodModel>,
}

impl ClassModel {
    pub fn new(desc: &str, super_desc: &str, access: u32, source: &str) -> Self {
        Self {
            desc: desc.to_string(),
            super_desc: super_desc.to_string(),
            access: access | ACC_SUPER,
            source: source.to_string(),
            interfaces: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
        }
    }

    pub fn field(&mut self, name: &str, desc: &str, access: u32) {
        self.fields
            .push((name.to_string(), desc.to_string(), access));
    }

    pub fn method(&mut self, method: MethodModel) {
        self.methods.push(method);
    }

    pub fn interfaces(&mut self) -> &mut Vec<String> {
        &mut self.interfaces
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodImage {
    pub words: Vec<u16>,
    pub registers: u16,
    pub ins: u16,
    pub outs: u16,
    pub is_static: bool,
    pub is_native: bool,
    pub listing: Vec<String>,
    pub access: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassImage {
    pub methods: BTreeMap<(String, String, Vec<String>), MethodImage>,
    pub fields: Vec<(String, String, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DexImage {
    pub strings: Vec<String>,
    pub types: Vec<String>,
    pub methods: Vec<MethodRef>,
    pub fields: Vec<FieldRef>,
    pub classes: BTreeMap<String, ClassImage>,
}

struct Pool {
    strings: HashSet<String>,
    types: HashSet<String>,
    protos: HashMap<(String, Vec<String>), String>,
    fields: HashSet<FieldRef>,
    methods: HashSet<MethodRef>,
}

impl Pool {
    fn new() -> Self {
        Self {
            strings: HashSet::new(),
            types: HashSet::new(),
            protos: HashMap::new(),
            fields: HashSet::new(),
            methods: HashSet::new(),
        }
    }

    fn add_type(&mut self, desc: &str) {
        self.types.insert(desc.to_string());
        self.strings.insert(desc.to_string());
    }

    fn add_proto(&mut self, ret: &str, params: &[String]) {
        let key = (ret.to_string(), params.to_vec());
        if self.protos.contains_key(&key) {
            return;
        }
        let shy = shorty(ret, params);
        self.strings.insert(shy.clone());
        self.add_type(ret);
        for param in params {
            self.add_type(param);
        }
        self.protos.insert(key, shy);
    }

    fn add_field(&mut self, key: FieldRef) {
        if self.fields.contains(&key) {
            return;
        }
        self.add_type(&key.owner);
        self.add_type(&key.ty);
        self.strings.insert(key.name.clone());
        self.fields.insert(key);
    }

    fn add_method(&mut self, key: MethodRef) {
        if self.methods.contains(&key) {
            return;
        }
        self.add_type(&key.owner);
        self.strings.insert(key.name.clone());
        self.add_proto(&key.ret, &key.params);
        self.methods.insert(key);
    }
}

struct TypeLists {
    order: Vec<Vec<String>>,
    blobs: HashMap<Vec<String>, Vec<u8>>,
}

impl TypeLists {
    fn new() -> Self {
        Self {
            order: Vec::new(),
            blobs: HashMap::new(),
        }
    }

    fn intern(
        &mut self,
        descs: &[String],
        type_index: &HashMap<String, usize>,
    ) -> Result<Vec<String>, CompileError> {
        if descs.is_empty() {
            return Ok(Vec::new());
        }
        let key = descs.to_vec();
        if self.blobs.contains_key(&key) {
            return Ok(key);
        }
        let mut blob = Vec::new();
        blob.extend_from_slice(&(key.len() as u32).to_le_bytes());
        for desc in &key {
            blob.extend_from_slice(&as_u16(type_index[desc])?.to_le_bytes());
        }
        if key.len() % 2 == 1 {
            blob.extend_from_slice(&0u16.to_le_bytes());
        }
        self.order.push(key.clone());
        self.blobs.insert(key.clone(), blob);
        Ok(key)
    }
}

pub struct DexFile {
    pub listing: Vec<String>,
    pub image: Option<DexImage>,
    classes: Vec<ClassModel>,
}

impl DexFile {
    pub fn new() -> Self {
        Self {
            listing: Vec::new(),
            image: None,
            classes: Vec::new(),
        }
    }

    pub fn add(&mut self, class: ClassModel) {
        self.classes.push(class);
    }

    pub fn build(&mut self) -> Result<Vec<u8>, CompileError> {
        let mut pool = Pool::new();
        for cls in &self.classes {
            pool.add_type(&cls.desc);
            pool.add_type(&cls.super_desc);
            pool.strings.insert(cls.desc.clone());
            if !cls.source.is_empty() {
                pool.strings.insert(cls.source.clone());
            }
            for iface in &cls.interfaces {
                pool.add_type(iface);
            }
            for (name, ty, _access) in &cls.fields {
                pool.add_field(FieldRef {
                    owner: cls.desc.clone(),
                    name: name.clone(),
                    ty: ty.clone(),
                });
            }
            for method in &cls.methods {
                pool.add_method(method.key(&cls.desc));
                let Some(asm) = &method.asm else { continue };
                for item in &asm.items {
                    let Item::Op { parts, .. } = item else {
                        continue;
                    };
                    for part in parts {
                        match part {
                            Part::Str(text) => {
                                pool.strings.insert(text.clone());
                            }
                            Part::Type(desc) => pool.add_type(desc),
                            Part::Field(key) => pool.add_field(key.clone()),
                            Part::Method(key) => pool.add_method(key.clone()),
                            Part::Word(_) => {}
                        }
                    }
                }
            }
        }

        let mut string_list: Vec<String> = pool.strings.into_iter().collect();
        string_list.sort();
        let string_index: HashMap<String, usize> = string_list
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i))
            .collect();

        let mut type_list: Vec<String> = pool.types.into_iter().collect();
        type_list.sort_by_key(|desc| string_index[desc]);
        let type_index: HashMap<String, usize> = type_list
            .iter()
            .enumerate()
            .map(|(i, d)| (d.clone(), i))
            .collect();

        let mut proto_items: Vec<((String, Vec<String>), String)> =
            pool.protos.into_iter().collect();
        proto_items.sort_by_key(|(key, _)| {
            let params: Vec<usize> = key.1.iter().map(|p| type_index[p]).collect();
            (type_index[&key.0], params)
        });
        let proto_index: HashMap<(String, Vec<String>), usize> = proto_items
            .iter()
            .enumerate()
            .map(|(i, (key, _))| (key.clone(), i))
            .collect();

        let mut field_list: Vec<FieldRef> = pool.fields.into_iter().collect();
        field_list.sort_by(|a, b| {
            (
                type_index[&a.owner],
                string_index[&a.name],
                type_index[&a.ty],
            )
                .cmp(&(
                    type_index[&b.owner],
                    string_index[&b.name],
                    type_index[&b.ty],
                ))
        });
        let field_index: HashMap<FieldRef, usize> = field_list
            .iter()
            .enumerate()
            .map(|(i, key)| (key.clone(), i))
            .collect();

        let mut method_list: Vec<MethodRef> = pool.methods.into_iter().collect();
        method_list.sort_by(|a, b| {
            let ka = (
                type_index[&a.owner],
                string_index[&a.name],
                proto_index[&(a.ret.clone(), a.params.clone())],
            );
            let kb = (
                type_index[&b.owner],
                string_index[&b.name],
                proto_index[&(b.ret.clone(), b.params.clone())],
            );
            ka.cmp(&kb)
        });
        let method_index: HashMap<MethodRef, usize> = method_list
            .iter()
            .enumerate()
            .map(|(i, key)| (key.clone(), i))
            .collect();

        let resolver = Resolver {
            strings: &string_index,
            types: &type_index,
            fields: &field_index,
            methods: &method_index,
        };
        for cls in &mut self.classes {
            for method in &mut cls.methods {
                if let Some(asm) = &method.asm {
                    let (words, listing) = asm.encode(&resolver)?;
                    method.words = words;
                    method.listing = listing;
                }
            }
        }

        let mut string_blobs = Vec::with_capacity(string_list.len());
        for text in &string_list {
            let raw = mutf8(text)?;
            let mut blob = uleb(text.chars().count() as u32);
            blob.extend(raw);
            blob.push(0);
            string_blobs.push(blob);
        }

        let mut type_lists = TypeLists::new();
        let mut iface_keys = Vec::with_capacity(self.classes.len());
        for cls in &self.classes {
            iface_keys.push(type_lists.intern(&cls.interfaces, &type_index)?);
        }
        let mut proto_param_keys: HashMap<(String, Vec<String>), Vec<String>> = HashMap::new();
        for (key, _shy) in &proto_items {
            let interned = type_lists.intern(&key.1, &type_index)?;
            proto_param_keys.insert(key.clone(), interned);
        }

        let mut code_blobs = Vec::new();
        let mut code_owners: Vec<(usize, usize)> = Vec::new();
        for (ci, cls) in self.classes.iter().enumerate() {
            for (mi, method) in cls.methods.iter().enumerate() {
                if method.asm.is_none() {
                    continue;
                }
                let mut blob = Vec::new();
                blob.extend_from_slice(&method.registers.to_le_bytes());
                blob.extend_from_slice(&method.ins.to_le_bytes());
                blob.extend_from_slice(&method.outs.to_le_bytes());
                blob.extend_from_slice(&0u16.to_le_bytes());
                blob.extend_from_slice(&0u32.to_le_bytes());
                blob.extend_from_slice(&(method.words.len() as u32).to_le_bytes());
                for word in &method.words {
                    blob.extend_from_slice(&word.to_le_bytes());
                }
                code_blobs.push(blob);
                code_owners.push((ci, mi));
            }
        }

        let header_size = 0x70usize;
        let string_ids_size = string_list.len();
        let type_ids_size = type_list.len();
        let proto_ids_size = proto_items.len();
        let field_ids_size = field_list.len();
        let method_ids_size = method_list.len();
        let class_defs_size = self.classes.len();

        let mut offset = header_size;
        let string_ids_off = if string_ids_size == 0 { 0 } else { offset };
        offset += 4 * string_ids_size;
        let type_ids_off = if type_ids_size == 0 { 0 } else { offset };
        offset += 4 * type_ids_size;
        let proto_ids_off = if proto_ids_size == 0 { 0 } else { offset };
        offset += 12 * proto_ids_size;
        let field_ids_off = if field_ids_size == 0 { 0 } else { offset };
        offset += 8 * field_ids_size;
        let method_ids_off = if method_ids_size == 0 { 0 } else { offset };
        offset += 8 * method_ids_size;

        let mut classes_sorted: Vec<usize> = (0..self.classes.len()).collect();
        classes_sorted.sort_by_key(|&index| type_index[&self.classes[index].desc]);
        let class_defs_off = if class_defs_size == 0 { 0 } else { offset };
        offset += 32 * class_defs_size;
        let data_off = align_up(offset, 4);

        let mut cursor = data_off;
        let mut string_offs = Vec::with_capacity(string_blobs.len());
        for blob in &string_blobs {
            string_offs.push(cursor);
            cursor += blob.len();
        }
        cursor = align_up(cursor, 4);
        let mut type_list_off: HashMap<Vec<String>, usize> = HashMap::new();
        for key in &type_lists.order {
            cursor = align_up(cursor, 4);
            type_list_off.insert(key.clone(), cursor);
            cursor += type_lists.blobs[key].len();
        }
        cursor = align_up(cursor, 4);
        let mut code_off: HashMap<(usize, usize), usize> = HashMap::new();
        for (owner, blob) in code_owners.iter().zip(code_blobs.iter()) {
            cursor = align_up(cursor, 4);
            code_off.insert(*owner, cursor);
            cursor += blob.len();
        }

        let mut class_data_blobs = Vec::with_capacity(classes_sorted.len());
        for &ci in &classes_sorted {
            let cls = &self.classes[ci];
            let mut field_items = Vec::new();
            for (name, ty, access) in &cls.fields {
                let key = FieldRef {
                    owner: cls.desc.clone(),
                    name: name.clone(),
                    ty: ty.clone(),
                };
                field_items.push((field_index[&key], *access));
            }
            field_items.sort();
            let mut direct = Vec::new();
            let mut virtual_methods = Vec::new();
            for (mi, method) in cls.methods.iter().enumerate() {
                let idx = method_index[&method.key(&cls.desc)];
                let coff = if method.asm.is_none() {
                    0
                } else {
                    code_off[&(ci, mi)]
                };
                if method.is_direct() {
                    direct.push((idx, method.access, coff));
                } else {
                    virtual_methods.push((idx, method.access, coff));
                }
            }
            direct.sort();
            virtual_methods.sort();
            let mut blob = Vec::new();
            // Every field is written in the instance section, including static ones.
            blob.extend(uleb(0));
            blob.extend(uleb(field_items.len() as u32));
            blob.extend(uleb(direct.len() as u32));
            blob.extend(uleb(virtual_methods.len() as u32));
            let mut prev = 0usize;
            for (idx, access) in field_items {
                blob.extend(uleb((idx - prev) as u32));
                blob.extend(uleb(access));
                prev = idx;
            }
            for group in [direct, virtual_methods] {
                prev = 0;
                for (idx, access, coff) in group {
                    blob.extend(uleb((idx - prev) as u32));
                    blob.extend(uleb(access));
                    blob.extend(uleb(as_u32(coff)?));
                    prev = idx;
                }
            }
            class_data_blobs.push(blob);
        }

        let mut class_data_off = Vec::with_capacity(class_data_blobs.len());
        for blob in &class_data_blobs {
            class_data_off.push(cursor);
            cursor += blob.len();
        }
        cursor = align_up(cursor, 4);
        let map_off = cursor;

        let mut map_items: Vec<(u16, u32, u32)> = vec![(0x0000, 1, 0)];
        if string_ids_size != 0 {
            map_items.push((0x0001, as_u32(string_ids_size)?, as_u32(string_ids_off)?));
        }
        if type_ids_size != 0 {
            map_items.push((0x0002, as_u32(type_ids_size)?, as_u32(type_ids_off)?));
        }
        if proto_ids_size != 0 {
            map_items.push((0x0003, as_u32(proto_ids_size)?, as_u32(proto_ids_off)?));
        }
        if field_ids_size != 0 {
            map_items.push((0x0004, as_u32(field_ids_size)?, as_u32(field_ids_off)?));
        }
        if method_ids_size != 0 {
            map_items.push((0x0005, as_u32(method_ids_size)?, as_u32(method_ids_off)?));
        }
        if class_defs_size != 0 {
            map_items.push((0x0006, as_u32(class_defs_size)?, as_u32(class_defs_off)?));
        }
        if !code_owners.is_empty() {
            map_items.push((
                0x2001,
                as_u32(code_owners.len())?,
                as_u32(code_off[&code_owners[0]])?,
            ));
        }
        if !string_list.is_empty() {
            map_items.push((0x2002, as_u32(string_list.len())?, as_u32(string_offs[0])?));
        }
        if !type_lists.order.is_empty() {
            let first = type_list_off.values().copied().min().unwrap();
            map_items.push((0x1001, as_u32(type_lists.order.len())?, as_u32(first)?));
        }
        if !class_data_blobs.is_empty() {
            map_items.push((
                0x2000,
                as_u32(class_data_blobs.len())?,
                as_u32(class_data_off[0])?,
            ));
        }
        map_items.push((0x1000, 1, as_u32(map_off)?));
        map_items.sort_by_key(|item| item.2);

        let map_size = 4 + 12 * map_items.len();
        let file_size = map_off + map_size;
        let data_size = file_size - data_off;
        let mut out = vec![0u8; file_size];
        out[..8].copy_from_slice(MAGIC);
        put_u32(&mut out, 32, as_u32(file_size)?);
        put_u32(&mut out, 36, header_size as u32);
        put_u32(&mut out, 40, 0x12345678);
        put_u32(&mut out, 44, 0);
        put_u32(&mut out, 48, 0);
        put_u32(&mut out, 52, as_u32(map_off)?);
        put_u32(&mut out, 56, as_u32(string_ids_size)?);
        put_u32(&mut out, 60, as_u32(string_ids_off)?);
        put_u32(&mut out, 64, as_u32(type_ids_size)?);
        put_u32(&mut out, 68, as_u32(type_ids_off)?);
        put_u32(&mut out, 72, as_u32(proto_ids_size)?);
        put_u32(&mut out, 76, as_u32(proto_ids_off)?);
        put_u32(&mut out, 80, as_u32(field_ids_size)?);
        put_u32(&mut out, 84, as_u32(field_ids_off)?);
        put_u32(&mut out, 88, as_u32(method_ids_size)?);
        put_u32(&mut out, 92, as_u32(method_ids_off)?);
        put_u32(&mut out, 96, as_u32(class_defs_size)?);
        put_u32(&mut out, 100, as_u32(class_defs_off)?);
        put_u32(&mut out, 104, as_u32(data_size)?);
        put_u32(&mut out, 108, as_u32(data_off)?);

        if string_ids_size != 0 {
            let mut cursor = string_ids_off;
            for off in &string_offs {
                put_u32(&mut out, cursor, as_u32(*off)?);
                cursor += 4;
            }
        }
        if type_ids_size != 0 {
            let mut cursor = type_ids_off;
            for desc in &type_list {
                put_u32(&mut out, cursor, as_u32(string_index[desc])?);
                cursor += 4;
            }
        }
        if proto_ids_size != 0 {
            let mut cursor = proto_ids_off;
            for (key, shy) in &proto_items {
                let param_key = &proto_param_keys[key];
                let param_off = if param_key.is_empty() {
                    0
                } else {
                    type_list_off[param_key]
                };
                put_u32(&mut out, cursor, as_u32(string_index[shy])?);
                put_u32(&mut out, cursor + 4, as_u32(type_index[&key.0])?);
                put_u32(&mut out, cursor + 8, as_u32(param_off)?);
                cursor += 12;
            }
        }
        if field_ids_size != 0 {
            let mut cursor = field_ids_off;
            for key in &field_list {
                put_u16(&mut out, cursor, as_u16(type_index[&key.owner])?);
                put_u16(&mut out, cursor + 2, as_u16(type_index[&key.ty])?);
                put_u32(&mut out, cursor + 4, as_u32(string_index[&key.name])?);
                cursor += 8;
            }
        }
        if method_ids_size != 0 {
            let mut cursor = method_ids_off;
            for key in &method_list {
                put_u16(&mut out, cursor, as_u16(type_index[&key.owner])?);
                put_u16(
                    &mut out,
                    cursor + 2,
                    as_u16(proto_index[&(key.ret.clone(), key.params.clone())])?,
                );
                put_u32(&mut out, cursor + 4, as_u32(string_index[&key.name])?);
                cursor += 8;
            }
        }
        if class_defs_size != 0 {
            let mut cursor = class_defs_off;
            for (slot, &ci) in classes_sorted.iter().enumerate() {
                let cls = &self.classes[ci];
                let iface_key = &iface_keys[ci];
                let iface_off = if iface_key.is_empty() {
                    0
                } else {
                    type_list_off[iface_key]
                };
                let source_idx = if cls.source.is_empty() {
                    NO_INDEX
                } else {
                    as_u32(string_index[&cls.source])?
                };
                put_u32(&mut out, cursor, as_u32(type_index[&cls.desc])?);
                put_u32(&mut out, cursor + 4, cls.access);
                put_u32(&mut out, cursor + 8, as_u32(type_index[&cls.super_desc])?);
                put_u32(&mut out, cursor + 12, as_u32(iface_off)?);
                put_u32(&mut out, cursor + 16, source_idx);
                put_u32(&mut out, cursor + 20, 0);
                put_u32(&mut out, cursor + 24, as_u32(class_data_off[slot])?);
                put_u32(&mut out, cursor + 28, 0);
                cursor += 32;
            }
        }

        for (off, blob) in string_offs.iter().zip(string_blobs.iter()) {
            out[*off..*off + blob.len()].copy_from_slice(blob);
        }
        for key in &type_lists.order {
            let off = type_list_off[key];
            let blob = &type_lists.blobs[key];
            out[off..off + blob.len()].copy_from_slice(blob);
        }
        for (owner, blob) in code_owners.iter().zip(code_blobs.iter()) {
            let off = code_off[owner];
            out[off..off + blob.len()].copy_from_slice(blob);
        }
        for (off, blob) in class_data_off.iter().zip(class_data_blobs.iter()) {
            out[*off..*off + blob.len()].copy_from_slice(blob);
        }

        put_u32(&mut out, map_off, as_u32(map_items.len())?);
        let mut at = map_off + 4;
        for (typ, size, item_off) in map_items {
            put_u16(&mut out, at, typ);
            put_u16(&mut out, at + 2, 0);
            put_u32(&mut out, at + 4, size);
            put_u32(&mut out, at + 8, item_off);
            at += 12;
        }

        let digest = Sha1::digest(&out[32..]);
        out[12..32].copy_from_slice(&digest);
        let checksum = adler32(&out[12..]);
        put_u32(&mut out, 8, checksum);

        let mut classes = BTreeMap::new();
        for cls in &self.classes {
            let mut methods = BTreeMap::new();
            for method in &cls.methods {
                methods.insert(
                    (
                        method.name.clone(),
                        method.ret.clone(),
                        method.params.clone(),
                    ),
                    MethodImage {
                        words: method.words.clone(),
                        registers: method.registers,
                        ins: method.ins,
                        outs: method.outs,
                        is_static: method.access & ACC_STATIC != 0,
                        is_native: method.asm.is_none() && method.access & ACC_NATIVE != 0,
                        listing: method.listing.clone(),
                        access: method.access,
                    },
                );
            }
            classes.insert(
                cls.desc.clone(),
                ClassImage {
                    methods,
                    fields: cls.fields.clone(),
                },
            );
        }
        self.image = Some(DexImage {
            strings: string_list,
            types: type_list,
            methods: method_list,
            fields: field_list,
            classes,
        });

        self.listing.clear();
        for &ci in &classes_sorted {
            let cls = &self.classes[ci];
            self.listing.push(format!("class {}", cls.desc));
            for method in &cls.methods {
                self.listing.push(format!(
                    "  {}{}{}",
                    method.name,
                    py_tuple(&method.params),
                    method.ret
                ));
                for line in &method.listing {
                    self.listing.push(format!("    {line}"));
                }
            }
        }
        Ok(out)
    }
}

pub fn check_dex(data: &[u8]) -> Result<(), CompileError> {
    if data.len() < 8 || &data[..8] != MAGIC {
        return Err(CompileError::bare("bad dex magic"));
    }
    if data.len() < 32 {
        return Err(CompileError::bare("dex checksum mismatch"));
    }
    let checksum = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if checksum != adler32(&data[12..]) {
        return Err(CompileError::bare("dex checksum mismatch"));
    }
    let digest = Sha1::digest(&data[32..]);
    if data[12..32] != digest[..] {
        return Err(CompileError::bare("dex signature mismatch"));
    }
    if data.len() < 44 {
        return Err(CompileError::bare("dex size mismatch"));
    }
    let file_size = u32::from_le_bytes(data[32..36].try_into().unwrap()) as usize;
    if file_size != data.len() {
        return Err(CompileError::bare("dex size mismatch"));
    }
    let endian = u32::from_le_bytes(data[40..44].try_into().unwrap());
    if endian != 0x12345678 {
        return Err(CompileError::bare("dex endian mismatch"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_init() -> MethodRef {
        MethodRef {
            owner: "Ljava/lang/Object;".into(),
            name: "<init>".into(),
            ret: "V".into(),
            params: Vec::new(),
        }
    }

    #[test]
    fn invoke_direct_count_is_high_nibble() {
        let mut one = Asm::new();
        one.invoke("direct", object_init(), &[0]);
        one.ret(0, "void");
        let mut four = Asm::new();
        four.invoke("direct", object_init(), &[1, 2, 3, 4]);
        four.ret(0, "void");

        let mut cls = ClassModel::new("LTiny;", "Ljava/lang/Object;", ACC_PUBLIC, "Tiny");
        cls.method(MethodModel::new(
            "<init>",
            "V",
            vec![],
            ACC_PUBLIC | ACC_CONSTRUCTOR,
            Some(one),
            1,
            1,
            1,
        ));
        cls.method(MethodModel::new(
            "four",
            "V",
            vec![],
            ACC_PUBLIC | ACC_STATIC,
            Some(four),
            5,
            0,
            4,
        ));
        let mut dex = DexFile::new();
        dex.add(cls);
        let bytes = dex.build().expect("build");
        check_dex(&bytes).expect("check");
        let image = dex.image.expect("image");
        let class = image.classes.get("LTiny;").expect("class");
        let init = class
            .methods
            .get(&("<init>".into(), "V".into(), vec![]))
            .expect("init");
        let four_m = class
            .methods
            .get(&("four".into(), "V".into(), vec![]))
            .expect("four");
        assert_eq!((init.words[0] >> 12) & 0xF, 1);
        assert_eq!(init.words[0] & 0xFF, 0x70);
        assert_eq!((four_m.words[0] >> 12) & 0xF, 4);
        assert_eq!(four_m.words[2], 1 | (2 << 4) | (3 << 8) | (4 << 12));
    }
}
