"""Compile Droide native functions to arm64-v8a and x86_64 ELF shared libraries.

The language is i32-only. Each function is a C ABI symbol plus a JNI stub
`Java_<pkg>_Native_<name>` that shifts the JNI arguments and jumps to it.
Calls are PC-relative, so the library has no relocations and does not need libc.
"""

import struct

from . import ast
from .errors import CompileError

PAGE = 0x4000
DT_NULL = 0
DT_HASH = 4
DT_STRTAB = 5
DT_SYMTAB = 6
DT_STRSZ = 10
DT_SYMENT = 11
DT_SONAME = 14
DT_GNU_HASH = 0x6FFFFEF5
PT_LOAD = 1
PT_DYNAMIC = 2
PT_GNU_STACK = 0x6474E551
PF_X, PF_W, PF_R = 1, 2, 4
EM_X86_64 = 62
EM_AARCH64 = 183
SONAME = "libdroide.so"


def jni_symbol(package, name):
    def esc(text):
        out = []
        for ch in text:
            if ch == "_":
                out.append("_1")
            elif ch == ".":
                out.append("_")
            elif ch == ";":
                out.append("_2")
            elif ch == "[":
                out.append("_3")
            elif ch.isalnum():
                out.append(ch)
            else:
                out.append("_0%04x" % ord(ch))
        return "".join(out)

    return "Java_" + esc(package) + "_Native_" + esc(name)


def _s32(value):
    value &= 0xFFFFFFFF
    if value >= 0x80000000:
        return value - 0x100000000
    return value


class _Buf:
    def __init__(self):
        self.buf = bytearray()
        self.labels = {}
        self.fixups = []
        self.calls = []

    def emit(self, data):
        self.buf += data

    def u32(self, word):
        self.buf += struct.pack("<I", word & 0xFFFFFFFF)

    def label(self, name):
        self.labels[name] = len(self.buf)

    def rel32(self, label):
        self.fixups.append((len(self.buf), label, "rel32"))
        self.emit(b"\x00\x00\x00\x00")

    def patch(self):
        for at, label, kind in self.fixups:
            if label not in self.labels:
                raise CompileError(f"undefined label {label}")
            if kind == "rel32":
                rel = self.labels[label] - (at + 4)
                self.buf[at:at + 4] = struct.pack("<i", rel)
            elif kind == "b":
                rel = (self.labels[label] - at) // 4
                self.buf[at:at + 4] = struct.pack("<I", 0x14000000 | (rel & 0x3FFFFFF))
            elif kind == "cbz":
                rel = (self.labels[label] - at) // 4
                self.buf[at:at + 4] = struct.pack("<I", 0x34000000 | ((rel & 0x7FFFF) << 5))
            else:
                raise CompileError(f"bad fixup {kind}")


def _disp(slot):
    return struct.pack("<i", -4 * (slot + 1))


class X86(_Buf):
    def prologue(self, nargs):
        self.emit(b"\x55\x48\x89\xe5\x48\x81\xec")
        self.emit(struct.pack("<I", 512))
        moves = [b"\x89\xf8", b"\x89\xf0", b"\x89\xd0", b"\x89\xc8", b"\x44\x89\xc0", b"\x44\x89\xc8"]
        for i in range(nargs):
            self.emit(moves[i])
            self.store(i)

    def store(self, slot):
        self.emit(b"\x89\x85" + _disp(slot))

    def load(self, slot):
        self.emit(b"\x8b\x85" + _disp(slot))

    def imm(self, value):
        self.emit(b"\xb8" + struct.pack("<I", value & 0xFFFFFFFF))

    def _ecx(self, slot):
        self.emit(b"\x8b\x8d" + _disp(slot))

    def binop(self, op, slot):
        self._ecx(slot)
        if op == "+":
            self.emit(b"\x01\xc8")
        elif op == "-":
            self.emit(b"\x29\xc8")
        elif op == "*":
            self.emit(b"\x0f\xaf\xc1")
        elif op == "/":
            self.emit(b"\x99\xf7\xf9")
        elif op == "%":
            self.emit(b"\x99\xf7\xf9\x89\xd0")
        else:
            raise CompileError(f"bad op {op}")

    def neg(self):
        self.emit(b"\xf7\xd8")

    def cmp_set(self, op, slot):
        self._ecx(slot)
        self.emit(b"\x39\xc8")
        self.emit({
            "==": b"\x0f\x94\xc0",
            "!=": b"\x0f\x95\xc0",
            "<": b"\x0f\x9c\xc0",
            ">": b"\x0f\x9f\xc0",
            "<=": b"\x0f\x9e\xc0",
            ">=": b"\x0f\x9d\xc0",
        }[op])
        self.emit(b"\x0f\xb6\xc0")

    def jump_if_zero(self, label):
        self.emit(b"\x85\xc0\x0f\x84")
        self.rel32(label)

    def jump(self, label):
        self.emit(b"\xe9")
        self.rel32(label)

    def call(self, name):
        self.emit(b"\xe8")
        self.calls.append((len(self.buf), name, "x86"))
        self.emit(b"\x00\x00\x00\x00")

    def arg(self, index, slot):
        self.load(slot)
        self.emit([b"\x89\xc7", b"\x89\xc6", b"\x89\xc2", b"\x89\xc1"][index])

    def epilogue(self):
        self.emit(b"\x48\x89\xec\x5d")

    def ret(self):
        self.emit(b"\xc3")


class Arm(_Buf):
    def prologue(self, nargs):
        self.u32(0xA9BF7BFD)  # stp x29, x30, [sp, #-16]!
        self.u32(0x910003FD)  # mov x29, sp
        self.u32(0xD10403FF)  # sub sp, sp, #256
        for i in range(nargs):
            self.u32(0xB9000000 | (i << 10) | (31 << 5) | i)

    def _mem(self, op, reg, slot):
        if not 0 <= slot < 64:
            raise CompileError("native frame slot is out of range")
        self.u32(op | (slot << 10) | (31 << 5) | reg)

    def store(self, slot):
        self._mem(0xB9000000, 0, slot)

    def load(self, slot):
        self._mem(0xB9400000, 0, slot)

    def imm(self, value):
        value &= 0xFFFFFFFF
        self.u32(0x52800000 | ((value & 0xFFFF) << 5))
        if value > 0xFFFF:
            self.u32(0x72A00000 | (((value >> 16) & 0xFFFF) << 5))

    def _w8(self, slot):
        self._mem(0xB9400000, 8, slot)

    def binop(self, op, slot):
        self._w8(slot)
        if op == "+":
            self.u32(0x0B080000)
        elif op == "-":
            self.u32(0x4B080000)
        elif op == "*":
            self.u32(0x1B000000 | (8 << 16) | (31 << 10))
        elif op == "/":
            self.u32(0x1AC00C00 | (8 << 16))
        elif op == "%":
            self.u32(0x2A0003E9)  # mov w9, w0
            self.u32(0x1AC00C00 | (8 << 16))
            self.u32(0x1B008000 | (8 << 16) | (9 << 10))
        else:
            raise CompileError(f"bad op {op}")

    def neg(self):
        self.u32(0x4B0003E0)

    def cmp_set(self, op, slot):
        self._w8(slot)
        self.u32(0x6B08001F)  # cmp w0, w8
        invert = {"==": 1, "!=": 0, "<": 10, ">": 13, "<=": 12, ">=": 11}[op]
        self.u32(0x1A9F07E0 | (invert << 12))

    def jump_if_zero(self, label):
        self.fixups.append((len(self.buf), label, "cbz"))
        self.u32(0)

    def jump(self, label):
        self.fixups.append((len(self.buf), label, "b"))
        self.u32(0)

    def call(self, name):
        self.calls.append((len(self.buf), name, "arm"))
        self.u32(0x94000000)

    def arg(self, index, slot):
        self._mem(0xB9400000, index, slot)

    def epilogue(self):
        self.u32(0x910003BF)  # mov sp, x29
        self.u32(0xA8C17BFD)  # ldp x29, x30, [sp], #16

    def ret(self):
        self.u32(0xD65F03C0)


def _slots(fun):
    mapping = {}
    index = 0
    for name in fun.params:
        mapping[name] = index
        index += 1

    def walk(stmts):
        nonlocal index
        for stmt in stmts:
            if isinstance(stmt, ast.NVar):
                if stmt.name not in mapping:
                    mapping[stmt.name] = index
                    index += 1
            elif isinstance(stmt, ast.NIf):
                walk(stmt.then)
                walk(stmt.else_)
            elif isinstance(stmt, ast.NWhile):
                walk(stmt.body)

    walk(fun.body)
    if index > 40:
        raise CompileError(f"{fun.name} has too many locals")
    return mapping


class _Lower:
    def __init__(self, fun, names):
        self.fun = fun
        self.slots = _slots(fun)
        self.names = names
        self.tsp = 0
        self.lab = 0

    def fresh(self):
        self.lab += 1
        return f"n{self.lab}"

    def temp(self):
        slot = 40 + self.tsp
        self.tsp += 1
        if slot >= 64:
            raise CompileError("native expression is too complex")
        return slot

    def expr(self, expr, asm):
        if isinstance(expr, ast.NLit):
            asm.imm(int(expr.value))
            return
        if isinstance(expr, ast.NIdent):
            asm.load(self.slots[expr.name])
            return
        if isinstance(expr, ast.NUnary):
            self.expr(expr.expr, asm)
            if expr.op == "-":
                asm.neg()
            return
        if isinstance(expr, ast.NBinary):
            self.expr(expr.right, asm)
            slot = self.temp()
            asm.store(slot)
            self.expr(expr.left, asm)
            if expr.op in ("==", "!=", "<", ">", "<=", ">="):
                asm.cmp_set(expr.op, slot)
            else:
                asm.binop(expr.op, slot)
            self.tsp -= 1
            return
        if isinstance(expr, ast.NCall):
            saved = []
            for arg in expr.args:
                self.expr(arg, asm)
                slot = self.temp()
                asm.store(slot)
                saved.append(slot)
            for index, slot in enumerate(saved):
                asm.arg(index, slot)
            asm.call(self.names[expr.name.lower()])
            self.tsp -= len(saved)
            return
        raise CompileError("cannot compile native expression")

    def stmts(self, stmts, asm):
        for stmt in stmts:
            if isinstance(stmt, ast.NVar):
                if stmt.expr is None:
                    asm.imm(0)
                else:
                    self.expr(stmt.expr, asm)
                asm.store(self.slots[stmt.name])
            elif isinstance(stmt, ast.NAssign):
                self.expr(stmt.expr, asm)
                asm.store(self.slots[stmt.name])
            elif isinstance(stmt, ast.NReturn):
                self.expr(stmt.expr, asm)
                asm.epilogue()
                asm.ret()
            elif isinstance(stmt, ast.NIf):
                self.expr(stmt.cond, asm)
                else_l = self.fresh()
                end_l = self.fresh()
                asm.jump_if_zero(else_l)
                self.stmts(stmt.then, asm)
                asm.jump(end_l)
                asm.label(else_l)
                self.stmts(stmt.else_, asm)
                asm.label(end_l)
            elif isinstance(stmt, ast.NWhile):
                head = self.fresh()
                end = self.fresh()
                asm.label(head)
                self.expr(stmt.cond, asm)
                asm.jump_if_zero(end)
                self.stmts(stmt.body, asm)
                asm.jump(head)
                asm.label(end)
            elif isinstance(stmt, ast.NCall):
                self.expr(stmt, asm)
            else:
                raise CompileError("cannot compile native statement")

    def run(self, asm):
        asm.prologue(len(self.fun.params))
        self.stmts(self.fun.body, asm)
        asm.imm(0)
        asm.epilogue()
        asm.ret()
        asm.patch()


def _compile_one(fun, names, kind):
    asm = X86() if kind == "x86" else Arm()
    _Lower(fun, names).run(asm)
    return asm


def _jni_stub(kind):
    if kind == "x86":
        # mov edi, edx; mov esi, ecx; mov edx, r8d; mov ecx, r9d; jmp rel32
        return b"\x89\xd7\x89\xce\x44\x89\xc2\x44\x89\xc1\xe9", 11
    words = [0x2A0203E0, 0x2A0303E1, 0x2A0403E2, 0x2A0503E3, 0x14000000]
    return b"".join(struct.pack("<I", w) for w in words), 16


def _link(kind, funs, names, package):
    compiled = []
    for fun in funs:
        asm = _compile_one(fun, names, kind)
        compiled.append((fun, asm))
    text = bytearray()
    symbols = {}  # name -> (offset, size)
    patches = []
    for fun, asm in compiled:
        start = len(text)
        text += asm.buf
        symbols[fun.name] = (start, len(asm.buf))
        for at, name, _how in asm.calls:
            patches.append((start + at, name, kind))
    for fun, _asm in compiled:
        stub, jmp_at = _jni_stub(kind)
        start = len(text)
        text += stub
        symbols[jni_symbol(package, fun.name)] = (start, len(stub))
        patches.append((start + jmp_at, fun.name, "stub-" + kind))
    for at, name, how in patches:
        target = symbols[name][0]
        if how == "x86":
            rel = target - (at + 4)
            text[at:at + 4] = struct.pack("<i", rel)
        elif how == "stub-x86":
            rel = target - (at + 4)
            text[at:at + 4] = struct.pack("<i", rel)
        else:
            rel = (target - at) // 4
            text[at:at + 4] = struct.pack("<I", (0x14000000 if how.startswith("stub") else 0x94000000) | (rel & 0x3FFFFFF))
    ordered = [(name, off, size) for name, (off, size) in symbols.items()]
    return bytes(text), ordered


def _elf_hash(name):
    value = 0
    for byte in name.encode():
        value = (value << 4) + byte
        high = value & 0xF0000000
        if high:
            value ^= high >> 24
        value &= ~high
    return value & 0xFFFFFFFF


def _gnu_hash(name):
    value = 5381
    for byte in name.encode():
        value = ((value * 33) + byte) & 0xFFFFFFFF
    return value


def _align(value, boundary):
    return (value + boundary - 1) & ~(boundary - 1)


def _elf(machine, text, symbols):
    defined = []
    for name, off, size in symbols:
        defined.append({"name": name, "off": off, "size": size, "hash": _gnu_hash(name)})
    nbuckets = max(1, len(defined))
    groups = [[] for _ in range(nbuckets)]
    for sym in defined:
        groups[sym["hash"] % nbuckets].append(sym)
    ordered = []
    buckets = [0] * nbuckets
    chain = []
    for index, group in enumerate(groups):
        if not group:
            continue
        buckets[index] = 1 + len(ordered)
        for pos, sym in enumerate(group):
            ordered.append(sym)
            bits = sym["hash"] & 0xFFFFFFFE
            if pos == len(group) - 1:
                bits |= 1
            chain.append(bits)
    dynstr = bytearray(b"\x00")

    def add_str(text_value):
        at = len(dynstr)
        dynstr.extend(text_value.encode() + b"\x00")
        return at

    soname_off = add_str(SONAME)
    for sym in ordered:
        sym["name_off"] = add_str(sym["name"])
    nsyms = 1 + len(ordered)
    sys_buckets = [0] * nsyms
    sys_chains = [0] * nsyms
    for index, sym in enumerate(ordered, start=1):
        slot = _elf_hash(sym["name"]) % nsyms
        sys_chains[index] = sys_buckets[slot]
        sys_buckets[slot] = index
    sysv = struct.pack("<II", nsyms, nsyms)
    sysv += struct.pack("<" + "I" * nsyms, *sys_buckets)
    sysv += struct.pack("<" + "I" * nsyms, *sys_chains)
    gnu = struct.pack("<IIII", nbuckets, 1, 1, 6)
    gnu += struct.pack("<Q", 0xFFFFFFFFFFFFFFFF)
    gnu += struct.pack("<" + "I" * nbuckets, *buckets)
    if chain:
        gnu += struct.pack("<" + "I" * len(chain), *chain)
    shstr = b"\x00.dynstr\x00.dynsym\x00.hash\x00.gnu.hash\x00.dynamic\x00.text\x00.shstrtab\x00"
    sh_names = {
        ".dynstr": 1,
        ".dynsym": 9,
        ".hash": 17,
        ".gnu.hash": 23,
        ".dynamic": 33,
        ".text": 42,
        ".shstrtab": 48,
    }
    ehdr = 64
    phnum = 3
    phdr_size = phnum * 56
    cursor = _align(ehdr + phdr_size, 8)
    dynstr_off = cursor
    cursor = _align(cursor + len(dynstr), 8)
    dynsym_off = cursor
    dynsym_size = nsyms * 24
    cursor = _align(cursor + dynsym_size, 4)
    hash_off = cursor
    cursor = _align(cursor + len(sysv), 8)
    gnu_off = cursor
    cursor = _align(cursor + len(gnu), 8)
    dyn_off = cursor
    dyn_size = 8 * 16
    cursor = _align(cursor + dyn_size, 16)
    text_off = cursor
    cursor = _align(cursor + len(text), 8)
    shstr_off = cursor
    cursor = _align(cursor + len(shstr), 8)
    shoff = cursor
    shnum = 8
    file_size = shoff + shnum * 64

    def sym_bytes():
        out = bytearray(24)
        for sym in ordered:
            info = (1 << 4) | 2
            out += struct.pack("<IBBHQQ", sym["name_off"], info, 0, 6, text_off + sym["off"], sym["size"])
        return bytes(out)

    dyn = b"".join(struct.pack("<QQ", tag, val) for tag, val in (
        (DT_HASH, hash_off),
        (DT_GNU_HASH, gnu_off),
        (DT_STRTAB, dynstr_off),
        (DT_SYMTAB, dynsym_off),
        (DT_STRSZ, len(dynstr)),
        (DT_SYMENT, 24),
        (DT_SONAME, soname_off),
        (DT_NULL, 0),
    ))
    image = bytearray(file_size)
    image[0:4] = b"\x7fELF"
    image[4] = 2
    image[5] = 1
    image[6] = 1
    struct.pack_into("<HHIQQQIHHHHHH", image, 16, 3, machine, 1, 0, ehdr, shoff, 0, 64, 56, phnum, 64, shnum, 7)
    ph = struct.pack("<IIQQQQQQ", PT_LOAD, PF_R | PF_X, 0, 0, 0, file_size, file_size, PAGE)
    ph += struct.pack("<IIQQQQQQ", PT_DYNAMIC, PF_R, dyn_off, dyn_off, dyn_off, dyn_size, dyn_size, 8)
    ph += struct.pack("<IIQQQQQQ", PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 0, 0x10)
    image[ehdr:ehdr + len(ph)] = ph
    image[dynstr_off:dynstr_off + len(dynstr)] = dynstr
    image[dynsym_off:dynsym_off + dynsym_size] = sym_bytes()
    image[hash_off:hash_off + len(sysv)] = sysv
    image[gnu_off:gnu_off + len(gnu)] = gnu
    image[dyn_off:dyn_off + len(dyn)] = dyn
    image[text_off:text_off + len(text)] = text
    image[shstr_off:shstr_off + len(shstr)] = shstr

    def shdr(name, typ, flags, addr, off, size, link, info, align, entsize):
        return struct.pack("<IIQQQQIIQQ", sh_names.get(name, 0), typ, flags, addr, off, size, link, info, align, entsize)

    sections = [
        shdr("", 0, 0, 0, 0, 0, 0, 0, 0, 0),
        shdr(".dynstr", 3, 2, dynstr_off, dynstr_off, len(dynstr), 0, 0, 1, 0),
        shdr(".dynsym", 11, 2, dynsym_off, dynsym_off, dynsym_size, 1, 1, 8, 24),
        shdr(".hash", 5, 2, hash_off, hash_off, len(sysv), 2, 0, 4, 4),
        shdr(".gnu.hash", 0x6FFFFFF6, 2, gnu_off, gnu_off, len(gnu), 2, 0, 8, 0),
        shdr(".dynamic", 6, 2, dyn_off, dyn_off, dyn_size, 1, 0, 8, 16),
        shdr(".text", 1, 6, text_off, text_off, len(text), 0, 0, 16, 0),
        shdr(".shstrtab", 3, 0, 0, shstr_off, len(shstr), 0, 0, 1, 0),
    ]
    blob = b"".join(sections)
    image[shoff:shoff + len(blob)] = blob
    resolved = {sym["name"]: text_off + sym["off"] for sym in ordered}
    return bytes(image), resolved


def compile_library(package, funs):
    if not funs:
        return None
    names = {}
    for fun in funs:
        names[fun.name.lower()] = fun.name
    xtext, xsyms = _link("x86", funs, names, package)
    atext, asyms = _link("arm", funs, names, package)
    xelf, xmap = _elf(EM_X86_64, xtext, xsyms)
    aelf, amap = _elf(EM_AARCH64, atext, asyms)
    return {
        "x86": xelf,
        "arm": aelf,
        "x86_symbols": xmap,
        "arm_symbols": amap,
        "exports": [fun.name for fun in funs],
    }


def emulate_aarch64(blob, symbols, name, args):
    """Run one exported function inside the arm64 library. Used as an oracle."""
    mem = bytearray(0x400000)
    mem[:len(blob)] = blob
    x = [0] * 31
    sp = 0x300000
    for i, arg in enumerate(args):
        x[i] = arg & 0xFFFFFFFF
    x30 = 0
    pc = symbols[name]
    flags = {"N": 0, "Z": 0, "V": 0}
    steps = 0

    def getw(reg):
        if reg == 31:
            return 0
        return x[reg] & 0xFFFFFFFF

    def setw(reg, value):
        if reg != 31:
            x[reg] = value & 0xFFFFFFFF

    def cond(op):
        n, z, v = flags["N"], flags["Z"], flags["V"]
        table = {
            0: z == 1,
            1: z == 0,
            10: n == v,
            11: n != v,
            12: z == 0 and n == v,
            13: z == 1 or n != v,
        }
        return table[op]

    while steps < 1000000:
        steps += 1
        word = struct.unpack_from("<I", mem, pc)[0]
        nxt = pc + 4
        if word == 0xA9BF7BFD:
            sp -= 16
            struct.pack_into("<Q", mem, sp, x[29])
            struct.pack_into("<Q", mem, sp + 8, x30)
        elif word == 0xA8C17BFD:
            x[29] = struct.unpack_from("<Q", mem, sp)[0]
            x30 = struct.unpack_from("<Q", mem, sp + 8)[0]
            sp += 16
        elif word == 0xD65F03C0:
            if x30 == 0:
                return _s32(x[0]), steps
            nxt = x30
        elif (word & 0xFFC00000) == 0xB9000000:
            imm = (word >> 10) & 0xFFF
            rn = (word >> 5) & 31
            rt = word & 31
            base = sp if rn == 31 else x[rn]
            struct.pack_into("<I", mem, base + imm * 4, getw(rt))
        elif (word & 0xFFC00000) == 0xB9400000:
            imm = (word >> 10) & 0xFFF
            rn = (word >> 5) & 31
            rt = word & 31
            base = sp if rn == 31 else x[rn]
            setw(rt, struct.unpack_from("<I", mem, base + imm * 4)[0])
        elif (word & 0xFF800000) == 0x52800000:
            imm = (word >> 5) & 0xFFFF
            rd = word & 31
            hw = (word >> 21) & 3
            setw(rd, imm << (hw * 16))
        elif (word & 0xFF800000) == 0x72800000:
            imm = (word >> 5) & 0xFFFF
            rd = word & 31
            hw = (word >> 21) & 3
            cur = getw(rd)
            shift = hw * 16
            setw(rd, (cur & ~(0xFFFF << shift)) | (imm << shift))
        elif (word & 0xFF200000) == 0x2A000000 and ((word >> 10) & 0x3F) == 0:
            rm = (word >> 16) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            setw(rd, getw(rn) | getw(rm))
        elif (word >> 24) == 0x0B and ((word >> 10) & 0x3F) == 0:
            rm = (word >> 16) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            left = sp if rn == 31 else getw(rn)
            setw(rd, left + getw(rm))
        elif (word >> 24) == 0x4B and ((word >> 10) & 0x3F) == 0:
            rm = (word >> 16) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            left = 0 if rn == 31 else getw(rn)
            setw(rd, left - getw(rm))
        elif (word >> 24) == 0x6B and ((word >> 10) & 0x3F) == 0:
            rm = (word >> 16) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            a = _s32(getw(rn))
            b = _s32(getw(rm))
            wide = a - b
            raw = wide & 0xFFFFFFFF
            flags["Z"] = 1 if raw == 0 else 0
            flags["N"] = (raw >> 31) & 1
            flags["V"] = 1 if wide < -0x80000000 or wide > 0x7FFFFFFF else 0
            if rd != 31:
                setw(rd, raw)
        elif (word & 0xFFE08000) == 0x1B000000:
            rm = (word >> 16) & 31
            ra = (word >> 10) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            setw(rd, getw(ra) + _s32(getw(rn)) * _s32(getw(rm)))
        elif (word & 0xFFE08000) == 0x1B008000:
            rm = (word >> 16) & 31
            ra = (word >> 10) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            setw(rd, getw(ra) - _s32(getw(rn)) * _s32(getw(rm)))
        elif (word & 0xFFE0FC00) == 0x1AC00C00:
            rm = (word >> 16) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            divisor = _s32(getw(rm))
            setw(rd, 0 if divisor == 0 else int(_s32(getw(rn)) / divisor))
        elif (word & 0xFFE00C00) == 0x1A800400:
            # csinc
            cond_code = (word >> 12) & 0xF
            rm = (word >> 16) & 31
            rn = (word >> 5) & 31
            rd = word & 31
            if cond(cond_code):
                setw(rd, getw(rn))
            else:
                setw(rd, (getw(rm) + 1) & 0xFFFFFFFF)
        elif (word >> 24) == 0x34:
            imm = (word >> 5) & 0x7FFFF
            if imm & 0x40000:
                imm -= 0x80000
            rt = word & 31
            if getw(rt) == 0:
                nxt = pc + imm * 4
        elif (word >> 26) == 0x5:
            imm = word & 0x3FFFFFF
            if imm & 0x2000000:
                imm -= 0x4000000
            nxt = pc + imm * 4
        elif (word >> 26) == 0x25:
            imm = word & 0x3FFFFFF
            if imm & 0x2000000:
                imm -= 0x4000000
            x30 = pc + 4
            nxt = pc + imm * 4
        elif (word & 0xFF800000) == 0x91000000 or (word & 0xFF800000) == 0xD1000000:
            imm = (word >> 10) & 0xFFF
            if (word >> 22) & 1:
                imm <<= 12
            rn = (word >> 5) & 31
            rd = word & 31
            base = sp if rn == 31 else x[rn]
            value = base + imm if (word & 0xFF800000) == 0x91000000 else base - imm
            if rd == 31:
                sp = value
            else:
                x[rd] = value & 0xFFFFFFFFFFFFFFFF
        else:
            raise CompileError(f"arm emulator hit an unknown instruction {word:#x} at {pc:#x}")
        pc = nxt
    raise CompileError("arm emulator ran too long")
