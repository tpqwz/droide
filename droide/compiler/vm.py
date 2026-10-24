"""A small DEX checker and integer interpreter for the opcodes Droide emits.

The type pass is an abstract interpreter in the style of ART's verifier:
const/4 0 is the zero type, which merges with both ints and references.
It is here so a bad register merge fails in tests before an APK is installed.
"""

from .errors import CompileError


def s32(value):
    value &= 0xFFFFFFFF
    if value >= 0x80000000:
        return value - 0x100000000
    return value


def _s16(value):
    value &= 0xFFFF
    if value >= 0x8000:
        return value - 0x10000
    return value


def _len_at(words, pc):
    op = words[pc] & 0xFF
    if op in (0x00, 0x0E, 0x0F, 0x11, 0x12, 0x01, 0x07, 0x0A, 0x0B, 0x0C, 0x7B, 0x82, 0x87):
        return 1
    if op in (0x13, 0x16, 0x1A, 0x1C, 0x1F, 0x22, 0x23, 0x29, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
              0x38, 0x39, 0x4D, 0x52, 0x54, 0x59, 0x5B, 0x60, 0x90, 0x91, 0x92, 0x93, 0x94, 0xA6,
              0xA7, 0xA8, 0xA9):
        return 2
    if op in (0x14, 0x6E, 0x6F, 0x70, 0x71, 0x72):
        return 3
    raise CompileError(f"unknown opcode {op:#x} at {pc}")


def _merge(a, b):
    if a == b or b == "bottom":
        return a
    if a == "bottom":
        return b
    if a == "zero" and b in ("int", "ref"):
        return b
    if b == "zero" and a in ("int", "ref"):
        return a
    if a == "conflict" or b == "conflict":
        return "conflict"
    return "conflict"


def _ok_int(ty):
    return ty in ("int", "zero")


def _ok_ref(ty):
    return ty in ("ref", "zero")


def _invoke_regs(words, pc):
    """Registers of a 35c invoke. Count is the high nibble; the fifth register is bits 8-11."""
    count = (words[pc] >> 12) & 0xF
    word2 = words[pc + 2]
    regs = [
        word2 & 0xF,
        (word2 >> 4) & 0xF,
        (word2 >> 8) & 0xF,
        (word2 >> 12) & 0xF,
        (words[pc] >> 8) & 0xF,
    ]
    return regs[:count]


def verify_image(image):
    errors = []
    for desc, cls in image["classes"].items():
        for key, method in cls["methods"].items():
            if method["native"] or not method["words"]:
                continue
            try:
                _verify_method(image, desc, key, method)
            except CompileError as exc:
                errors.append(f"{desc}->{key[0]}: {exc}")
    if errors:
        raise CompileError("; ".join(errors))
    return True


def _field_type(image, index):
    _owner, _name, desc = image["fields"][index]
    if desc == "F":
        return "float"
    if desc.startswith("L") or desc.startswith("["):
        return "ref"
    return "int"


def _method_ret(image, index):
    _owner, _name, ret, _params = image["methods"][index]
    if ret == "V":
        return None
    if ret == "F":
        return "float"
    if ret.startswith("L") or ret.startswith("["):
        return "ref"
    return "int"


def _verify_method(image, desc, key, method):
    words = method["words"]
    count = method["registers"]
    entry = ["bottom"] * count
    params = key[2]
    slot = count - method["ins"]
    if not method["static"]:
        entry[slot] = "ref"
        slot += 1
    for desc_p in params:
        if desc_p == "F":
            entry[slot] = "float"
        elif desc_p.startswith("L") or desc_p.startswith("["):
            entry[slot] = "ref"
        else:
            entry[slot] = "int"
        slot += 1
    state = {0: tuple(entry)}
    work = [0]
    seen = 0
    while work:
        pc = work.pop()
        seen += 1
        if seen > 100000:
            raise CompileError("verifier did not converge")
        regs = list(state[pc])
        end = False
        pending = None
        while not end:
            op = words[pc] & 0xFF
            size = _len_at(words, pc)
            nxt = pc + size

            def need(reg, kind):
                ty = regs[reg]
                if kind == "int" and not _ok_int(ty):
                    raise CompileError(f"v{reg} is {ty}, expected int at {pc:#x}")
                if kind == "ref" and not _ok_ref(ty):
                    raise CompileError(f"v{reg} is {ty}, expected ref at {pc:#x}")
                if kind == "float" and ty != "float":
                    raise CompileError(f"v{reg} is {ty}, expected float at {pc:#x}")

            def put(reg, ty):
                regs[reg] = ty

            if op == 0x00:
                pass
            elif op == 0x12:
                reg = (words[pc] >> 8) & 0xF
                nibble = (words[pc] >> 12) & 0xF
                value = nibble if nibble < 8 else nibble - 16
                put(reg, "zero" if value == 0 else "int")
            elif op == 0x13:
                reg = (words[pc] >> 8) & 0xFF
                value = _s16(words[pc + 1])
                put(reg, "zero" if value == 0 else "int")
            elif op == 0x14:
                reg = (words[pc] >> 8) & 0xFF
                raw = words[pc + 1] | (words[pc + 2] << 16)
                put(reg, "zero" if raw == 0 else "int")
            elif op == 0x16:
                # A long is two register words. Search's latch timeout is the only user.
                reg = (words[pc] >> 8) & 0xFF
                if reg % 2:
                    raise CompileError(f"const-wide v{reg} is odd")
                put(reg, "int")
                put(reg + 1, "int")
            elif op == 0x60:
                reg = (words[pc] >> 8) & 0xFF
                put(reg, _field_type(image, words[pc + 1]))
            elif op in (0x1A, 0x1C, 0x22):
                reg = (words[pc] >> 8) & 0xFF
                put(reg, "ref")
            elif op in (0x01, 0x07):
                dest = (words[pc] >> 8) & 0xF
                src = (words[pc] >> 12) & 0xF
                if op == 0x07:
                    need(src, "ref")
                    put(dest, "ref" if regs[src] != "zero" else "zero")
                else:
                    put(dest, regs[src] if regs[src] != "bottom" else "bottom")
                    if regs[dest] == "bottom":
                        raise CompileError(f"move of undefined v{src}")
            elif op in (0x0A, 0x0B, 0x0C):
                reg = (words[pc] >> 8) & 0xFF
                if pending is None:
                    raise CompileError(f"move-result without invoke at {pc:#x}")
                if op == 0x0B:
                    if pending == "ref":
                        raise CompileError("move-result-wide of a reference")
                    if reg % 2:
                        raise CompileError(f"move-result-wide v{reg} is odd")
                    put(reg, "int")
                    put(reg + 1, "int")
                else:
                    if op == 0x0C and pending != "ref":
                        raise CompileError("move-result-object of a non-reference")
                    if op == 0x0A and pending == "ref":
                        raise CompileError("move-result of a reference")
                    put(reg, pending)
                pending = None
            elif op == 0x7B:
                dest = (words[pc] >> 8) & 0xF
                src = (words[pc] >> 12) & 0xF
                need(src, "int")
                put(dest, "int")
            elif op == 0x82:
                dest = (words[pc] >> 8) & 0xF
                src = (words[pc] >> 12) & 0xF
                need(src, "int")
                put(dest, "float")
            elif op == 0x87:
                dest = (words[pc] >> 8) & 0xF
                src = (words[pc] >> 12) & 0xF
                need(src, "float")
                put(dest, "int")
            elif op in (0x90, 0x91, 0x92, 0x93, 0x94, 0xA6, 0xA7, 0xA8, 0xA9):
                dest = (words[pc] >> 8) & 0xFF
                a = words[pc + 1] & 0xFF
                b = (words[pc + 1] >> 8) & 0xFF
                kind = "float" if op >= 0xA6 else "int"
                need(a, kind)
                need(b, kind)
                put(dest, kind)
            elif op in (0x52, 0x54):
                dest = (words[pc] >> 8) & 0xF
                obj = (words[pc] >> 12) & 0xF
                need(obj, "ref")
                put(dest, _field_type(image, words[pc + 1]))
            elif op in (0x59, 0x5B):
                src = (words[pc] >> 8) & 0xF
                obj = (words[pc] >> 12) & 0xF
                need(obj, "ref")
                fty = _field_type(image, words[pc + 1])
                need(src, "ref" if fty == "ref" else ("float" if fty == "float" else "int"))
            elif op in (0x6E, 0x6F, 0x70, 0x71, 0x72):
                method_index = words[pc + 1]
                regs_i = _invoke_regs(words, pc)
                ret = _method_ret(image, method_index)
                _owner, mname, _ret, _params = image["methods"][method_index]
                for reg in regs_i:
                    if regs[reg] == "bottom":
                        raise CompileError(f"invoke uses undefined v{reg}")
                if mname == "<init>" and regs_i:
                    put(regs_i[0], "ref")
                pending = ret
                if ret is None:
                    pending = None
            elif op == 0x0E:
                end = True
                nxt = None
            elif op == 0x0F:
                need((words[pc] >> 8) & 0xFF, "int")
                end = True
                nxt = None
            elif op == 0x11:
                need((words[pc] >> 8) & 0xFF, "ref")
                end = True
                nxt = None
            elif op == 0x29:
                end = True
                nxt = pc + _s16(words[pc + 1])
            elif op == 0x1F:
                reg = (words[pc] >> 8) & 0xFF
                need(reg, "ref")
                put(reg, "ref")
            elif op == 0x23:
                dest = (words[pc] >> 8) & 0xF
                size = (words[pc] >> 12) & 0xF
                need(size, "int")
                put(dest, "ref")
            elif op == 0x4D:
                value = (words[pc] >> 8) & 0xFF
                array = words[pc + 1] & 0xFF
                index = (words[pc + 1] >> 8) & 0xFF
                need(value, "ref")
                need(array, "ref")
                need(index, "int")
            elif op in (0x38, 0x39):
                reg = (words[pc] >> 8) & 0xFF
                ty = regs[reg]
                if not (_ok_int(ty) or _ok_ref(ty)):
                    raise CompileError(f"v{reg} is {ty}, expected int or ref at {pc:#x}")
                _jump(state, work, pc + _s16(words[pc + 1]), regs)
                end = True
                nxt = pc + 2
            elif op in (0x32, 0x33, 0x34, 0x35, 0x36, 0x37):
                a = (words[pc] >> 8) & 0xF
                b = (words[pc] >> 12) & 0xF
                need(a, "int")
                need(b, "int")
                _jump(state, work, pc + _s16(words[pc + 1]), regs)
                end = True
                nxt = pc + 2
            else:
                raise CompileError(f"verifier does not know opcode {op:#x}")
            if pending is not None and op not in (0x6E, 0x6F, 0x70, 0x71, 0x72):
                raise CompileError("invoke result was not moved")
            if not end:
                pc = nxt
                continue
            if nxt is not None:
                _jump(state, work, nxt, regs)
            break
    return True


def _jump(state, work, pc, regs):
    merged = tuple(regs)
    if pc not in state:
        state[pc] = merged
        work.append(pc)
        return
    old = state[pc]
    if old == merged:
        return
    nxt = tuple(_merge(a, b) for a, b in zip(old, merged))
    if "conflict" in nxt:
        raise CompileError(f"register type conflict at {pc:#x}: {old} vs {merged}")
    if nxt != old:
        state[pc] = nxt
        work.append(pc)


def run_static(image, desc, name, args):
    methods = image["classes"][desc]["methods"]
    found = None
    for key, method in methods.items():
        if key[0] == name and method["static"] and not method["native"]:
            found = (key, method)
            break
    if found is None:
        raise CompileError(f"no static method {name}")
    return _run(image, desc, found[0], found[1], args)


def _run(image, desc, key, method, args, depth=0):
    if depth > 40:
        raise CompileError("interpreter recursion limit")
    regs = [0] * method["registers"]
    base = method["registers"] - method["ins"]
    for index, arg in enumerate(args):
        regs[base + index] = s32(arg)
    words = method["words"]
    pc = 0
    pending = None
    steps = 0
    while steps < 100000:
        steps += 1
        op = words[pc] & 0xFF
        if op == 0x12:
            reg = (words[pc] >> 8) & 0xF
            nibble = (words[pc] >> 12) & 0xF
            regs[reg] = nibble if nibble < 8 else nibble - 16
            pc += 1
        elif op == 0x13:
            regs[(words[pc] >> 8) & 0xFF] = _s16(words[pc + 1])
            pc += 2
        elif op == 0x14:
            raw = words[pc + 1] | (words[pc + 2] << 16)
            regs[(words[pc] >> 8) & 0xFF] = s32(raw)
            pc += 3
        elif op == 0x01:
            dest = (words[pc] >> 8) & 0xF
            src = (words[pc] >> 12) & 0xF
            regs[dest] = regs[src]
            pc += 1
        elif op == 0x0A:
            regs[(words[pc] >> 8) & 0xFF] = pending
            pending = None
            pc += 1
        elif op == 0x7B:
            dest = (words[pc] >> 8) & 0xF
            src = (words[pc] >> 12) & 0xF
            regs[dest] = s32(-regs[src])
            pc += 1
        elif op in (0x90, 0x91, 0x92, 0x93, 0x94):
            dest = (words[pc] >> 8) & 0xFF
            a = words[pc + 1] & 0xFF
            b = (words[pc + 1] >> 8) & 0xFF
            left, right = regs[a], regs[b]
            if op == 0x90:
                regs[dest] = s32(left + right)
            elif op == 0x91:
                regs[dest] = s32(left - right)
            elif op == 0x92:
                regs[dest] = s32(left * right)
            elif op == 0x93:
                regs[dest] = int(left / right)
            else:
                regs[dest] = left - int(left / right) * right
            pc += 2
        elif op == 0x0F:
            return s32(regs[(words[pc] >> 8) & 0xFF])
        elif op == 0x0E:
            return None
        elif op == 0x29:
            pc = pc + _s16(words[pc + 1])
        elif op in (0x38, 0x39):
            reg = regs[(words[pc] >> 8) & 0xFF]
            take = (reg == 0) if op == 0x38 else (reg != 0)
            pc = pc + _s16(words[pc + 1]) if take else pc + 2
        elif op in (0x32, 0x33, 0x34, 0x35, 0x36, 0x37):
            a = regs[(words[pc] >> 8) & 0xF]
            b = regs[(words[pc] >> 12) & 0xF]
            take = {
                0x32: a == b, 0x33: a != b, 0x34: a < b, 0x35: a >= b, 0x36: a > b, 0x37: a <= b,
            }[op]
            pc = pc + _s16(words[pc + 1]) if take else pc + 2
        elif op == 0x71:
            arg_regs = _invoke_regs(words, pc)
            owner, mname, ret, params = image["methods"][words[pc + 1]]
            target = image["classes"].get(owner, {}).get("methods", {}).get((mname, ret, params))
            if target is None or target["native"]:
                raise CompileError(f"interpreter cannot call {owner}->{mname}")
            pending = _run(image, owner, (mname, ret, params), target, [regs[r] for r in arg_regs], depth + 1)
            pc += 3
        else:
            raise CompileError(f"interpreter cannot execute opcode {op:#x}")
    raise CompileError("interpreter ran too long")
