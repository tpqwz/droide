"""DEX assembler and file writer. DEX 035, no external toolchain."""

import hashlib
import struct
import zlib

from .errors import CompileError

ACC_PUBLIC = 0x1
ACC_PRIVATE = 0x2
ACC_STATIC = 0x8
ACC_FINAL = 0x10
ACC_SUPER = 0x20
ACC_NATIVE = 0x100
ACC_INTERFACE = 0x200
ACC_ABSTRACT = 0x400
ACC_CONSTRUCTOR = 0x10000

NO_INDEX = 0xFFFFFFFF


def uleb(value):
    if value < 0:
        raise ValueError("uleb of negative")
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def mutf8(text):
    out = bytearray()
    for ch in text:
        code = ord(ch)
        if code == 0:
            out.extend((0xC0, 0x80))
        elif code < 0x80:
            out.append(code)
        elif code < 0x800:
            out.extend((0xC0 | (code >> 6), 0x80 | (code & 0x3F)))
        elif code < 0x10000:
            out.extend((0xE0 | (code >> 12), 0x80 | ((code >> 6) & 0x3F), 0x80 | (code & 0x3F)))
        else:
            raise CompileError("string is outside the BMP")
    return bytes(out)


def shorty_char(desc):
    if desc in ("V", "I", "Z", "B", "S", "C", "F", "J", "D"):
        return desc[0] if desc != "V" else "V"
    return "L"


def shorty(ret, params):
    return shorty_char(ret) + "".join(shorty_char(p) for p in params)


class Asm:
    """Instruction stream with labels and pool references."""

    def __init__(self):
        self.items = []

    def op(self, parts, text):
        self.items.append(("op", parts, text))

    def label(self, name):
        self.items.append(("label", name))

    def branch(self, kind, regs, label, text):
        self.items.append(("branch", kind, tuple(regs), label, text))

    def const(self, reg, value):
        if -8 <= value <= 7:
            nibble = value & 0xF
            self.op([0x12 | ((reg & 0xF) << 8) | (nibble << 12)], f"const/4 v{reg}, #{value}")
        elif -32768 <= value <= 32767:
            self.op([0x13 | ((reg & 0xFF) << 8), value & 0xFFFF], f"const/16 v{reg}, #{value}")
        else:
            raw = value & 0xFFFFFFFF
            self.op(
                [0x14 | ((reg & 0xFF) << 8), raw & 0xFFFF, (raw >> 16) & 0xFFFF],
                f"const v{reg}, #{value}",
            )

    def const_wide(self, reg, value):
        """const-wide/16. The value occupies reg and reg + 1, so reg is even."""
        if reg % 2:
            raise CompileError(f"const-wide v{reg} is odd")
        if not -32768 <= value <= 32767:
            raise CompileError("const-wide is too wide")
        self.op([0x16 | ((reg & 0xFF) << 8), value & 0xFFFF], f"const-wide/16 v{reg}, #{value}")

    def sget(self, dest, key):
        """sget. The field index is 16 bits and the destination register is 8 bits."""
        self.op([0x60 | ((dest & 0xFF) << 8), ("f", key)], f"sget v{dest}, {key[1]}")

    def const_string(self, reg, text):
        shown = text.replace("\\", "\\\\").replace('"', '\\"')
        self.op([0x1A | ((reg & 0xFF) << 8), ("s", text)], f'const-string v{reg}, "{shown}"')

    def const_class(self, reg, desc):
        self.op([0x1C | ((reg & 0xFF) << 8), ("t", desc)], f"const-class v{reg}, {desc}")

    def move(self, dest, src, obj):
        if dest == src:
            return
        op = 0x07 if obj else 0x01
        self.op([op | ((dest & 0xF) << 8) | ((src & 0xF) << 12)], f"{'move-object' if obj else 'move'} v{dest}, v{src}")

    def move_result(self, reg, obj):
        op = 0x0C if obj else 0x0A
        self.op([op | ((reg & 0xFF) << 8)], f"{'move-result-object' if obj else 'move-result'} v{reg}")

    def move_result_wide(self, reg):
        self.op([0x0B | ((reg & 0xFF) << 8)], f"move-result-wide v{reg}")

    def check_cast(self, reg, desc):
        self.op([0x1F | ((reg & 0xFF) << 8), ("t", desc)], f"check-cast v{reg}, {desc}")

    def new_array(self, dest, size, desc):
        word = 0x23 | ((dest & 0xF) << 8) | ((size & 0xF) << 12)
        self.op([word, ("t", desc)], f"new-array v{dest}, v{size}, {desc}")

    def aput_object(self, value, array, index):
        word0 = 0x4D | ((value & 0xFF) << 8)
        word1 = (array & 0xFF) | ((index & 0xFF) << 8)
        self.op([word0, word1], f"aput-object v{value}, v{array}, v{index}")

    def ret(self, reg, kind):
        if kind == "void":
            self.op([0x0E], "return-void")
        elif kind == "obj":
            self.op([0x11 | ((reg & 0xFF) << 8)], f"return-object v{reg}")
        else:
            self.op([0x0F | ((reg & 0xFF) << 8)], f"return v{reg}")

    def new_instance(self, reg, desc):
        self.op([0x22 | ((reg & 0xFF) << 8), ("t", desc)], f"new-instance v{reg}, {desc}")

    def invoke(self, kind, key, regs):
        ops = {"virtual": 0x6E, "super": 0x6F, "direct": 0x70, "static": 0x71, "interface": 0x72}
        if len(regs) > 5:
            raise CompileError("invoke has more than 5 registers")
        padded = list(regs) + [0] * 5
        c, d, e, f, g = [r & 0xF for r in padded[:5]]
        # 35c is A|G|op: the count is the high nibble, the fifth register the next.
        word0 = ops[kind] | (g << 8) | ((len(regs) & 0xF) << 12)
        word2 = c | (d << 4) | (e << 8) | (f << 12)
        cls, name, ret, params = key
        text = f"invoke-{kind} {{{', '.join('v%d' % r for r in regs)}}}, {cls}->{name}"
        self.op([word0, ("m", key), word2], text)

    def field_op(self, op, a, b, key, text):
        self.op([op | ((a & 0xF) << 8) | ((b & 0xF) << 12), ("f", key)], text)

    def iget(self, dest, obj, key):
        self.field_op(0x52, dest, obj, key, f"iget v{dest}, v{obj}, {key[1]}")

    def iget_obj(self, dest, obj, key):
        self.field_op(0x54, dest, obj, key, f"iget-object v{dest}, v{obj}, {key[1]}")

    def iput(self, src, obj, key):
        self.field_op(0x59, src, obj, key, f"iput v{src}, v{obj}, {key[1]}")

    def iput_obj(self, src, obj, key):
        self.field_op(0x5B, src, obj, key, f"iput-object v{src}, v{obj}, {key[1]}")

    def binop(self, op, dest, a, b, name):
        self.op(
            [op | ((dest & 0xFF) << 8), (a & 0xFF) | ((b & 0xFF) << 8)],
            f"{name} v{dest}, v{a}, v{b}",
        )

    def unop(self, op, dest, src, name):
        self.op([op | ((dest & 0xF) << 8) | ((src & 0xF) << 12)], f"{name} v{dest}, v{src}")

    def if_zero(self, kind, reg, label):
        op = {"eqz": 0x38, "nez": 0x39}[kind]
        self.branch(kind, (reg,), label, f"if-{kind} v{reg}")

    def if_cmp(self, kind, a, b, label):
        self.branch(kind, (a, b), label, f"if-{kind} v{a}, v{b}")

    def goto(self, label):
        self.branch("goto", (), label, "goto")

    def encode(self, resolver):
        labels = {}
        pos = 0
        for item in self.items:
            if item[0] == "label":
                if item[1] in labels:
                    raise CompileError(f"duplicate label {item[1]}")
                labels[item[1]] = pos
            elif item[0] == "op":
                pos += len(item[1])
            elif item[0] == "branch":
                pos += 2
            else:
                raise CompileError("bad asm item")
        words = []
        listing = []
        pos = 0
        for item in self.items:
            if item[0] == "label":
                listing.append(f"{pos:04x}  :{item[1]}")
                continue
            if item[0] == "op":
                encoded = []
                for part in item[1]:
                    if isinstance(part, int):
                        encoded.append(part & 0xFFFF)
                    else:
                        kind, key = part
                        if kind == "s":
                            encoded.append(resolver.string(key))
                        elif kind == "t":
                            encoded.append(resolver.type(key))
                        elif kind == "m":
                            encoded.append(resolver.method(key))
                        elif kind == "f":
                            encoded.append(resolver.field(key))
                        else:
                            raise CompileError(f"bad ref {kind}")
                listing.append(f"{pos:04x}  {item[2]}")
                words.extend(encoded)
                pos += len(encoded)
                continue
            kind, regs, label, text = item[1], item[2], item[3], item[4]
            if label not in labels:
                raise CompileError(f"unknown label {label}")
            offset = labels[label] - pos
            if not -32768 <= offset <= 32767:
                raise CompileError("branch is too far")
            if kind == "goto":
                pair = [0x29, offset & 0xFFFF]
            elif kind == "eqz":
                pair = [0x38 | ((regs[0] & 0xFF) << 8), offset & 0xFFFF]
            elif kind == "nez":
                pair = [0x39 | ((regs[0] & 0xFF) << 8), offset & 0xFFFF]
            else:
                ops = {"eq": 0x32, "ne": 0x33, "lt": 0x34, "ge": 0x35, "gt": 0x36, "le": 0x37}
                a, b = regs
                pair = [ops[kind] | ((a & 0xF) << 8) | ((b & 0xF) << 12), offset & 0xFFFF]
            listing.append(f"{pos:04x}  {text} -> {label}")
            words.extend(pair)
            pos += 2
        if len(words) % 2 == 1:
            listing.append(f"{pos:04x}  nop")
            words.append(0x0000)
        return words, listing

    def refs(self):
        for item in self.items:
            if item[0] != "op":
                continue
            for part in item[1]:
                if not isinstance(part, int):
                    yield part


class MethodModel:
    def __init__(self, name, ret, params, access, asm, registers, ins, outs):
        self.name = name
        self.ret = ret
        self.params = tuple(params)
        self.access = access
        self.asm = asm
        self.registers = registers
        self.ins = ins
        self.outs = outs
        self.words = []
        self.listing = []

    @property
    def direct(self):
        return bool(self.access & (ACC_STATIC | ACC_PRIVATE | ACC_CONSTRUCTOR))

    def key(self, owner):
        return (owner, self.name, self.ret, self.params)


class ClassModel:
    def __init__(self, desc, super_desc, access, interfaces=None, source=""):
        self.desc = desc
        self.super = super_desc
        self.access = access | ACC_SUPER
        self.interfaces = list(interfaces or [])
        self.source = source
        self.fields = []  # (name, type_desc, access)
        self.methods = []

    def field(self, name, type_desc, access=ACC_PUBLIC):
        self.fields.append((name, type_desc, access))

    def method(self, method):
        self.methods.append(method)


class Pool:
    def __init__(self):
        self.items = []
        self.index = {}

    def add(self, key):
        if key not in self.index:
            self.index[key] = len(self.items)
            self.items.append(key)
        return self.index[key]


class DexFile:
    def __init__(self):
        self.classes = []

    def add(self, cls):
        self.classes.append(cls)

    def build(self):
        strings = set()
        types = set()
        protos = {}
        fields = {}
        methods = {}

        def add_type(desc):
            types.add(desc)
            strings.add(desc)

        def add_proto(ret, params):
            key = (ret, tuple(params))
            if key not in protos:
                protos[key] = shorty(ret, params)
                strings.add(protos[key])
                add_type(ret)
                for param in params:
                    add_type(param)
            return key

        def add_field(key):
            owner, name, type_desc = key
            fields[key] = True
            add_type(owner)
            add_type(type_desc)
            strings.add(name)

        def add_method(key):
            owner, name, ret, params = key
            methods[key] = True
            add_type(owner)
            strings.add(name)
            add_proto(ret, params)

        for cls in self.classes:
            add_type(cls.desc)
            add_type(cls.super)
            strings.add(cls.desc)
            if cls.source:
                strings.add(cls.source)
            for iface in cls.interfaces:
                add_type(iface)
            for name, type_desc, _access in cls.fields:
                add_field((cls.desc, name, type_desc))
            for method in cls.methods:
                add_method(method.key(cls.desc))
                if method.asm is None:
                    continue
                for kind, key in method.asm.refs():
                    if kind == "s":
                        strings.add(key)
                    elif kind == "t":
                        add_type(key)
                    elif kind == "f":
                        add_field(key)
                    elif kind == "m":
                        add_method(key)

        string_list = sorted(strings)
        string_index = {s: i for i, s in enumerate(string_list)}
        type_list = sorted(types, key=lambda d: string_index[d])
        type_index = {d: i for i, d in enumerate(type_list)}

        def proto_sort_key(item):
            (ret, params), _short = item
            return (type_index[ret], tuple(type_index[p] for p in params))

        proto_items = sorted(protos.items(), key=proto_sort_key)
        proto_index = {key: i for i, (key, _s) in enumerate(proto_items)}

        def field_sort_key(key):
            owner, name, type_desc = key
            return (type_index[owner], string_index[name], type_index[type_desc])

        field_list = sorted(fields, key=field_sort_key)
        field_index = {key: i for i, key in enumerate(field_list)}

        def method_sort_key(key):
            owner, name, ret, params = key
            return (type_index[owner], string_index[name], proto_index[(ret, params)])

        method_list = sorted(methods, key=method_sort_key)
        method_index = {key: i for i, key in enumerate(method_list)}

        class Resolver:
            def string(self, value):
                return string_index[value]

            def type(self, desc):
                return type_index[desc]

            def field(self, key):
                return field_index[key]

            def method(self, key):
                return method_index[key]

        resolver = Resolver()
        for cls in self.classes:
            for method in cls.methods:
                if method.asm is None:
                    continue
                method.words, method.listing = method.asm.encode(resolver)

        # Encode pieces whose sizes we need before offsets are assigned.
        string_blobs = []
        for text in string_list:
            raw = mutf8(text)
            string_blobs.append(uleb(len(text)) + raw + b"\x00")

        type_lists = {}

        def type_list_blob(descs):
            key = tuple(descs)
            if not key:
                return key
            if key not in type_lists:
                blob = struct.pack("<I", len(key))
                for desc in key:
                    blob += struct.pack("<H", type_index[desc])
                if len(key) % 2 == 1:
                    blob += b"\x00\x00"
                type_lists[key] = blob
            return key

        iface_keys = {}
        for cls in self.classes:
            iface_keys[id(cls)] = type_list_blob(cls.interfaces)
        proto_param_keys = {}
        for key, _short in proto_items:
            _ret, params = key
            proto_param_keys[key] = type_list_blob(params)

        code_blobs = []
        code_owners = []
        for cls in self.classes:
            for method in cls.methods:
                if method.asm is None:
                    continue
                insns = method.words
                blob = struct.pack(
                    "<HHHHI",
                    method.registers,
                    method.ins,
                    method.outs,
                    0,
                    0,
                )
                blob += struct.pack("<I", len(insns))
                blob += struct.pack("<" + "H" * len(insns), *insns)
                code_blobs.append(blob)
                code_owners.append(method)

        # id sections
        def align(value, n=4):
            return (value + n - 1) & ~(n - 1)

        header_size = 0x70
        string_ids_size = len(string_list)
        type_ids_size = len(type_list)
        proto_ids_size = len(proto_items)
        field_ids_size = len(field_list)
        method_ids_size = len(method_list)
        class_defs_size = len(self.classes)

        offset = header_size
        string_ids_off = offset if string_ids_size else 0
        offset += 4 * string_ids_size
        type_ids_off = offset if type_ids_size else 0
        offset += 4 * type_ids_size
        proto_ids_off = offset if proto_ids_size else 0
        offset += 12 * proto_ids_size
        field_ids_off = offset if field_ids_size else 0
        offset += 8 * field_ids_size
        method_ids_off = offset if method_ids_size else 0
        offset += 8 * method_ids_size
        # class defs sorted by type index
        classes_sorted = sorted(self.classes, key=lambda c: type_index[c.desc])
        class_defs_off = offset if class_defs_size else 0
        offset += 32 * class_defs_size
        data_off = align(offset)

        cursor = data_off
        string_offs = []
        for blob in string_blobs:
            string_offs.append(cursor)
            cursor += len(blob)
        cursor = align(cursor)
        type_list_off = {}
        for key, blob in type_lists.items():
            cursor = align(cursor)
            type_list_off[key] = cursor
            cursor += len(blob)
        cursor = align(cursor)
        code_off = {}
        for method, blob in zip(code_owners, code_blobs):
            cursor = align(cursor)
            code_off[id(method)] = cursor
            cursor += len(blob)

        class_data_blobs = []
        for cls in classes_sorted:
            field_items = []
            for name, type_desc, access in cls.fields:
                field_items.append((field_index[(cls.desc, name, type_desc)], access))
            field_items.sort()
            direct = []
            virtual = []
            for method in cls.methods:
                idx = method_index[method.key(cls.desc)]
                coff = 0 if method.asm is None else code_off[id(method)]
                bucket = direct if method.direct else virtual
                bucket.append((idx, method.access, coff))
            direct.sort()
            virtual.sort()
            blob = bytearray()
            blob += uleb(0)
            blob += uleb(len(field_items))
            blob += uleb(len(direct))
            blob += uleb(len(virtual))
            prev = 0
            for idx, access in field_items:
                blob += uleb(idx - prev)
                blob += uleb(access)
                prev = idx
            for group in (direct, virtual):
                prev = 0
                for idx, access, coff in group:
                    blob += uleb(idx - prev)
                    blob += uleb(access)
                    blob += uleb(coff)
                    prev = idx
            class_data_blobs.append(bytes(blob))

        class_data_off = []
        for blob in class_data_blobs:
            class_data_off.append(cursor)
            cursor += len(blob)
        cursor = align(cursor)
        map_off = cursor

        # Map items in offset order.
        map_items = [(0x0000, 1, 0)]
        if string_ids_size:
            map_items.append((0x0001, string_ids_size, string_ids_off))
        if type_ids_size:
            map_items.append((0x0002, type_ids_size, type_ids_off))
        if proto_ids_size:
            map_items.append((0x0003, proto_ids_size, proto_ids_off))
        if field_ids_size:
            map_items.append((0x0004, field_ids_size, field_ids_off))
        if method_ids_size:
            map_items.append((0x0005, method_ids_size, method_ids_off))
        if class_defs_size:
            map_items.append((0x0006, class_defs_size, class_defs_off))
        if code_owners:
            map_items.append((0x2001, len(code_owners), code_off[id(code_owners[0])]))
        if string_list:
            map_items.append((0x2002, len(string_list), string_offs[0]))
        if type_lists:
            first_tl = min(type_list_off.values())
            map_items.append((0x1001, len(type_lists), first_tl))
        if class_data_blobs:
            map_items.append((0x2000, len(class_data_blobs), class_data_off[0]))
        map_items.append((0x1000, 1, map_off))
        map_items.sort(key=lambda item: item[2])
        map_size = 4 + 12 * len(map_items)
        file_size = map_off + map_size
        data_size = file_size - data_off

        out = bytearray(file_size)
        out[0:8] = b"dex\n035\x00"
        struct.pack_into("<I", out, 32 + 0, file_size)  # patched below; layout first
        # We'll write the header at the end of the fields. Offsets:
        # 0 magic, 8 checksum, 12 signature, 32 file_size, 36 header_size,
        # 40 endian, 44 link_size, 48 link_off, 52 map_off, then ids.

        def write_header():
            struct.pack_into("<I", out, 32, file_size)
            struct.pack_into("<I", out, 36, header_size)
            struct.pack_into("<I", out, 40, 0x12345678)
            struct.pack_into("<I", out, 44, 0)
            struct.pack_into("<I", out, 48, 0)
            struct.pack_into("<I", out, 52, map_off)
            struct.pack_into("<II", out, 56, string_ids_size, string_ids_off)
            struct.pack_into("<II", out, 64, type_ids_size, type_ids_off)
            struct.pack_into("<II", out, 72, proto_ids_size, proto_ids_off)
            struct.pack_into("<II", out, 80, field_ids_size, field_ids_off)
            struct.pack_into("<II", out, 88, method_ids_size, method_ids_off)
            struct.pack_into("<II", out, 96, class_defs_size, class_defs_off)
            struct.pack_into("<II", out, 104, data_size, data_off)

        write_header()
        cursor = string_ids_off
        for off in string_offs:
            struct.pack_into("<I", out, cursor, off)
            cursor += 4
        cursor = type_ids_off
        for desc in type_list:
            struct.pack_into("<I", out, cursor, string_index[desc])
            cursor += 4
        cursor = proto_ids_off
        for key, shy in proto_items:
            ret, params = key
            param_key = proto_param_keys[key]
            param_off = 0 if not param_key else type_list_off[param_key]
            struct.pack_into("<III", out, cursor, string_index[shy], type_index[ret], param_off)
            cursor += 12
        cursor = field_ids_off
        for key in field_list:
            owner, name, type_desc = key
            struct.pack_into("<HHI", out, cursor, type_index[owner], type_index[type_desc], string_index[name])
            cursor += 8
        cursor = method_ids_off
        for key in method_list:
            owner, name, ret, params = key
            struct.pack_into(
                "<HHI", out, cursor,
                type_index[owner], proto_index[(ret, params)], string_index[name],
            )
            cursor += 8
        cursor = class_defs_off
        for cls, data_at in zip(classes_sorted, class_data_off):
            iface_key = iface_keys[id(cls)]
            iface_off = 0 if not iface_key else type_list_off[iface_key]
            source_idx = string_index[cls.source] if cls.source else NO_INDEX
            struct.pack_into(
                "<IIIIIIII", out, cursor,
                type_index[cls.desc],
                cls.access,
                type_index[cls.super],
                iface_off,
                source_idx,
                0,
                data_at,
                0,
            )
            cursor += 32

        for off, blob in zip(string_offs, string_blobs):
            out[off:off + len(blob)] = blob
        for key, blob in type_lists.items():
            off = type_list_off[key]
            out[off:off + len(blob)] = blob
        for method, blob in zip(code_owners, code_blobs):
            off = code_off[id(method)]
            out[off:off + len(blob)] = blob
        for off, blob in zip(class_data_off, class_data_blobs):
            out[off:off + len(blob)] = blob

        struct.pack_into("<I", out, map_off, len(map_items))
        at = map_off + 4
        for typ, size, item_off in map_items:
            struct.pack_into("<HHII", out, at, typ, 0, size, item_off)
            at += 12

        digest = hashlib.sha1(out[32:]).digest()
        out[12:32] = digest
        checksum = zlib.adler32(out[12:]) & 0xFFFFFFFF
        struct.pack_into("<I", out, 8, checksum)

        image = {
            "strings": string_list,
            "types": type_list,
            "methods": method_list,
            "fields": field_list,
            "classes": {},
        }
        for cls in self.classes:
            methods_out = {}
            for method in cls.methods:
                methods_out[(method.name, method.ret, method.params)] = {
                    "words": list(method.words),
                    "registers": method.registers,
                    "ins": method.ins,
                    "outs": method.outs,
                    "static": bool(method.access & ACC_STATIC),
                    "native": method.asm is None and bool(method.access & ACC_NATIVE),
                    "listing": method.listing,
                    "access": method.access,
                }
            image["classes"][cls.desc] = {
                "methods": methods_out,
                "fields": list(cls.fields),
            }
        self.image = image
        self.listing = []
        for cls in classes_sorted:
            self.listing.append(f"class {cls.desc}")
            for method in cls.methods:
                header = f"  {method.name}{method.params}{method.ret}"
                self.listing.append(header)
                self.listing.extend("    " + line for line in method.listing)
        return bytes(out)


def check_dex(data):
    if data[:8] != b"dex\n035\x00":
        raise CompileError("bad dex magic")
    checksum = struct.unpack_from("<I", data, 8)[0]
    actual = zlib.adler32(data[12:]) & 0xFFFFFFFF
    if checksum != actual:
        raise CompileError("dex checksum mismatch")
    signature = data[12:32]
    if signature != hashlib.sha1(data[32:]).digest():
        raise CompileError("dex signature mismatch")
    file_size = struct.unpack_from("<I", data, 32)[0]
    if file_size != len(data):
        raise CompileError("dex size mismatch")
    endian = struct.unpack_from("<I", data, 40)[0]
    if endian != 0x12345678:
        raise CompileError("dex endian mismatch")
    return True
