"""Generate a DEX file from a checked Droide project.

Register plan: six scratch registers at the bottom, then locals, then the
incoming arguments (including `this`) so they occupy the last `ins` slots.
Every scratch register is a const/4 zero at each join, which ART accepts as
either an int or a null. Strings locals start as "". Floats are built with
int-to-float; no raw float bits are loaded with const.
"""

from . import ast
from .dex import (
    ACC_CONSTRUCTOR,
    ACC_NATIVE,
    ACC_PUBLIC,
    ACC_STATIC,
    Asm,
    ClassModel,
    DexFile,
    MethodModel,
)
from .errors import CompileError
from .widgets import GETTERS, SETTERS, WIDGETS, parse_color

SCRATCH = 6
TYPE_DESC = {
    "Integer": "I",
    "Boolean": "Z",
    "String": "Ljava/lang/String;",
    "View": "Landroid/view/View;",
}
INT_BIN = {
    "+": (0x90, "add-int"),
    "-": (0x91, "sub-int"),
    "*": (0x92, "mul-int"),
    "/": (0x93, "div-int"),
    "div": (0x93, "div-int"),
    "mod": (0x94, "rem-int"),
}
CMP = {
    "=": "eq",
    "<>": "ne",
    "<": "lt",
    ">": "gt",
    "<=": "le",
    ">=": "ge",
}


def _obj(ty):
    return ty in ("String", "View")


def class_desc(package, name):
    return "L" + package.replace(".", "/") + "/" + name + ";"


class Frame:
    def __init__(self, static, params, locals_):
        self.static = static
        self.slots = {}
        self.param_names = {name.lower() for name, _ty in params}
        reg = SCRATCH
        for name, ty in locals_:
            self.slots[name.lower()] = (reg, ty)
            reg += 1
        self.local_end = reg
        if not static:
            self.this = reg
            reg += 1
        else:
            self.this = None
        for name, ty in params:
            self.slots[name.lower()] = (reg, ty)
            reg += 1
        self.registers = reg
        self.ins = reg - self.local_end
        if self.registers > 16:
            raise CompileError("routine is too large for the register window (max 16)")


class RoutineGen:
    def __init__(self, compiler, asm, frame, form, owner):
        self.cx = compiler
        self.asm = asm
        self.frame = frame
        self.form = form
        self.owner = owner
        self.sp = 0
        self.outs = 0
        self.ret_label = compiler.fresh()

    def fresh(self):
        return self.cx.fresh()

    def alloc(self):
        if self.sp >= SCRATCH:
            raise CompileError("expression is too complex")
        reg = self.sp
        self.sp += 1
        return reg

    def release_to(self, n):
        while self.sp > n:
            self.sp -= 1
            self.asm.const(self.sp, 0)

    def zero_except(self, *keep):
        kept = set(keep)
        for reg in range(self.sp):
            if reg not in kept:
                self.asm.const(reg, 0)

    def invoke(self, kind, key, regs):
        self.outs = max(self.outs, len(regs))
        self.asm.invoke(kind, key, regs)

    def prologue(self):
        for reg in range(SCRATCH):
            self.asm.const(reg, 0)
        for name, (reg, ty) in self.frame.slots.items():
            if name in self.frame.param_names:
                continue
            if ty == "String":
                self.asm.const_string(reg, "")
            else:
                self.asm.const(reg, 0)

    def finish(self, ret):
        self.asm.label(self.ret_label)
        if ret is None:
            self.asm.ret(0, "void")
            return
        reg, ty = self.frame.slots["result"]
        self.asm.ret(reg, "obj" if _obj(ty) else "int")

    def emit_body(self, stmts):
        self.emit_stmts(stmts)
        self.release_to(0)
        self.asm.goto(self.ret_label)

    def emit_stmts(self, stmts):
        for stmt in stmts:
            self.emit_stmt(stmt)

    def emit_one(self, node):
        if node is None:
            return
        if isinstance(node, ast.Block):
            self.emit_stmts(node.stmts)
        elif isinstance(node, list):
            self.emit_stmts(node)
        else:
            self.emit_stmt(node)

    def emit_stmt(self, stmt):
        if isinstance(stmt, ast.Block):
            self.emit_stmts(stmt.stmts)
        elif isinstance(stmt, ast.IfStmt):
            self.emit_if(stmt)
        elif isinstance(stmt, ast.WhileStmt):
            self.emit_while(stmt)
        elif isinstance(stmt, ast.ExitStmt):
            self.release_to(0)
            self.asm.goto(self.ret_label)
        elif isinstance(stmt, ast.Assign):
            self.emit_assign(stmt)
            self.release_to(0)
        elif isinstance(stmt, ast.CallStmt):
            self.eval_call(stmt, as_stmt=True)
            self.release_to(0)
        else:
            raise CompileError("cannot compile statement")

    def emit_if(self, stmt):
        reg, _ty = self.eval(stmt.cond)
        else_l = self.fresh()
        end_l = self.fresh()
        self.zero_except(reg)
        self.asm.if_zero("eqz", reg, else_l)
        self.asm.const(reg, 0)
        self.sp = 0
        self.emit_one(stmt.then)
        self.release_to(0)
        self.asm.goto(end_l)
        self.asm.label(else_l)
        self.asm.const(reg, 0)
        self.sp = 0
        self.emit_one(stmt.else_)
        self.release_to(0)
        self.asm.label(end_l)
        self.sp = 0

    def emit_while(self, stmt):
        head = self.fresh()
        end = self.fresh()
        self.asm.label(head)
        reg, _ty = self.eval(stmt.cond)
        self.zero_except(reg)
        self.asm.if_zero("eqz", reg, end)
        self.asm.const(reg, 0)
        self.sp = 0
        self.emit_one(stmt.body)
        self.release_to(0)
        self.asm.goto(head)
        self.asm.label(end)
        self.asm.const(reg, 0)
        self.sp = 0

    def emit_assign(self, stmt):
        kind = stmt.target.resolved[0]
        if kind == "prop":
            _prop, actual, widget = stmt.target.resolved
            start = self.sp
            wreg = self.alloc()
            self.load_widget(wreg, widget)
            vreg, vty = self.eval(stmt.expr)
            self.emit_setter(wreg, actual, vreg, vty)
            self.release_to(start)
            return
        vreg, vty = self.eval(stmt.expr)
        self.store(stmt.target.parts[0], vreg, vty)

    def store(self, name, src, ty):
        reg, slot_ty = self.frame.slots[name.lower()]
        self.asm.move(reg, src, _obj(slot_ty or ty))

    def load_widget(self, dest, widget):
        desc = WIDGETS[widget["comp"]["type"]]["desc"]
        key = (self.owner, widget["name"], desc)
        self.asm.iget_obj(dest, self.frame.this, key)

    def emit_setter(self, wreg, prop, vreg, vty):
        owner, method, ret, params, mode = SETTERS[prop]
        if mode == "clock":
            # One Format string drives both hour cycles. Passing null would restore the locale default.
            text = ("Ljava/lang/CharSequence;",)
            self.invoke("virtual", ("Landroid/widget/TextClock;", "setFormat12Hour", "V", text), [wreg, vreg])
            self.invoke("virtual", ("Landroid/widget/TextClock;", "setFormat24Hour", "V", text), [wreg, vreg])
            return
        if mode in ("date-year", "date-month", "date-day"):
            self.emit_date_assign(wreg, mode, vreg)
            return
        key = (owner, method, ret, params)
        if mode == "boxbool":
            # TimePicker.setIs24HourView wants java.lang.Boolean, not Z.
            self.invoke("static", ("Ljava/lang/Boolean;", "valueOf", "Ljava/lang/Boolean;", ("Z",)), [vreg])
            boxed = self.alloc()
            self.asm.move_result(boxed, True)
            self.invoke("virtual", key, [wreg, boxed])
            return
        if mode == "float":
            self.asm.unop(0x82, vreg, vreg, "int-to-float")
        elif mode == "vis":
            no = self.fresh()
            end = self.fresh()
            self.zero_except(wreg, vreg)
            self.asm.if_zero("eqz", vreg, no)
            self.asm.const(vreg, 0)
            self.asm.goto(end)
            self.asm.label(no)
            self.asm.const(vreg, 8)
            self.asm.label(end)
        self.invoke("virtual", key, [wreg, vreg])

    def eval(self, expr):
        return self._eval(expr)

    def _eval(self, expr):
        if isinstance(expr, ast.Literal):
            return self.eval_literal(expr)
        if isinstance(expr, ast.Ident):
            return self.eval_name(expr)
        if isinstance(expr, ast.Designator):
            return self.eval_prop(expr)
        if isinstance(expr, ast.Unary):
            return self.eval_unary(expr)
        if isinstance(expr, ast.Binary):
            if expr.op == "+":
                if getattr(expr, "string_add", False):
                    return self.eval_strcat(expr)
            if expr.op in ("and", "or"):
                return self.eval_logic(expr)
            if expr.op in CMP:
                return self.eval_cmp(expr)
            return self.eval_arith(expr)
        if isinstance(expr, ast.CallExpr):
            return self.eval_call(expr, as_stmt=False)
        raise CompileError("cannot compile expression")

    def eval_literal(self, expr):
        reg = self.alloc()
        if expr.type_name == "String":
            self.asm.const_string(reg, expr.value)
        elif expr.type_name == "Boolean":
            self.asm.const(reg, 1 if expr.value else 0)
        else:
            self.asm.const(reg, int(expr.value))
        return reg, expr.type_name

    def eval_name(self, expr):
        resolved = expr.resolved
        if resolved[0] == "widget":
            reg = self.alloc()
            self.load_widget(reg, resolved[1])
            return reg, "View"
        _kind, ty = resolved[0], resolved[1]
        src, slot_ty = self.frame.slots[expr.name.lower()]
        reg = self.alloc()
        self.asm.move(reg, src, _obj(slot_ty))
        return reg, slot_ty or ty

    def eval_prop(self, expr):
        actual, widget = expr.resolved[1], expr.resolved[2]
        owner, method, ret, params, mode = GETTERS[actual]
        wreg = self.alloc()
        self.load_widget(wreg, widget)
        self.invoke("virtual", (owner, method, ret, params), [wreg])
        if mode == "text":
            self.asm.move_result(wreg, True)
            self.invoke("virtual", ("Ljava/lang/Object;", "toString", "Ljava/lang/String;", ()), [wreg])
            self.asm.move_result(wreg, True)
            return wreg, "String"
        self.asm.move_result(wreg, False)
        if mode == "month":
            # DatePicker.getMonth is 0–11. Droide months are 1–12.
            one = self.alloc()
            self.asm.const(one, 1)
            self.asm.binop(0x90, wreg, wreg, one, "add-int")
            self.release_to(wreg + 1)
            return wreg, "Integer"
        if mode == "float":
            # getRating returns float. Droide properties are integers, so truncate toward zero.
            self.asm.unop(0x87, wreg, wreg, "float-to-int")
            return wreg, "Integer"
        if mode == "vis":
            # getVisibility == 0 means visible.
            zero = self.alloc()
            self.asm.const(zero, 0)
            yes = self.fresh()
            end = self.fresh()
            self.zero_except(wreg, zero)
            self.asm.if_cmp("eq", wreg, zero, yes)
            self.asm.const(wreg, 0)
            self.asm.const(zero, 0)
            self.asm.goto(end)
            self.asm.label(yes)
            self.asm.const(wreg, 1)
            self.asm.const(zero, 0)
            self.asm.label(end)
            self.release_to(wreg + 1)
            return wreg, "Boolean"
        return wreg, "Boolean" if mode == "bool" else "Integer"

    def eval_unary(self, expr):
        reg, ty = self._eval(expr.expr)
        if expr.op == "+":
            return reg, ty
        if expr.op == "-":
            self.asm.unop(0x7B, reg, reg, "neg-int")
            return reg, "Integer"
        yes = self.fresh()
        end = self.fresh()
        self.zero_except(reg)
        self.asm.if_zero("eqz", reg, yes)
        self.asm.const(reg, 0)
        self.asm.goto(end)
        self.asm.label(yes)
        self.asm.const(reg, 1)
        self.asm.label(end)
        return reg, "Boolean"

    def emit_date_assign(self, wreg, mode, vreg):
        # updateDate replaces the whole date. Read the fields this assignment leaves alone.
        # A Month write is 1–12 and is stored as month − 1.
        year = self.alloc()
        month = self.alloc()
        day = self.alloc()
        if mode != "date-year":
            self.invoke("virtual", ("Landroid/widget/DatePicker;", "getYear", "I", ()), [wreg])
            self.asm.move_result(year, False)
        else:
            self.asm.move(year, vreg, False)
        if mode != "date-month":
            self.invoke("virtual", ("Landroid/widget/DatePicker;", "getMonth", "I", ()), [wreg])
            self.asm.move_result(month, False)
        else:
            self.asm.move(month, vreg, False)
            self.asm.const(vreg, 1)
            self.asm.binop(0x91, month, month, vreg, "sub-int")
        if mode != "date-day":
            self.invoke("virtual", ("Landroid/widget/DatePicker;", "getDayOfMonth", "I", ()), [wreg])
            self.asm.move_result(day, False)
        else:
            self.asm.move(day, vreg, False)
        self.invoke(
            "virtual",
            ("Landroid/widget/DatePicker;", "updateDate", "V", ("I", "I", "I")),
            [wreg, year, month, day],
        )

    def eval_arith(self, expr):
        left, _lt = self._eval(expr.left)
        right, _rt = self._eval(expr.right)
        op, name = INT_BIN[expr.op]
        self.asm.binop(op, left, left, right, name)
        self.release_to(left + 1)
        return left, "Integer"

    def eval_logic(self, expr):
        reg, _ty = self._eval(expr.left)
        done = self.fresh()
        self.zero_except(reg)
        if expr.op == "and":
            self.asm.if_zero("eqz", reg, done)
        else:
            self.asm.if_zero("nez", reg, done)
        self.sp = reg
        self._eval(expr.right)
        self.release_to(reg + 1)
        self.asm.label(done)
        self.sp = reg + 1
        return reg, "Boolean"

    def eval_cmp(self, expr):
        left, lt = self._eval(expr.left)
        right, rt = self._eval(expr.right)
        if lt == "String" or rt == "String":
            return self.cmp_string(expr.op, left, right)
        return self.cmp_int(expr.op, left, right)

    def cmp_int(self, op, left, right):
        yes = self.fresh()
        end = self.fresh()
        self.zero_except(left, right)
        self.asm.if_cmp(CMP[op], left, right, yes)
        self.asm.const(left, 0)
        self.asm.const(right, 0)
        self.asm.goto(end)
        self.asm.label(yes)
        self.asm.const(left, 1)
        self.asm.const(right, 0)
        self.asm.label(end)
        self.release_to(left + 1)
        return left, "Boolean"

    def cmp_string(self, op, left, right):
        if op in ("=", "<>"):
            self.invoke(
                "virtual",
                ("Ljava/lang/String;", "equals", "Z", ("Ljava/lang/Object;",)),
                [left, right],
            )
            self.asm.move_result(left, False)
            self.asm.const(right, 0)
            self.release_to(left + 1)
            if op == "<>":
                self.invert(left)
            return left, "Boolean"
        self.invoke(
            "virtual",
            ("Ljava/lang/String;", "compareTo", "I", ("Ljava/lang/String;",)),
            [left, right],
        )
        self.asm.move_result(left, False)
        self.asm.const(right, 0)
        return self.cmp_int(op, left, right)

    def invert(self, reg):
        yes = self.fresh()
        end = self.fresh()
        self.zero_except(reg)
        self.asm.if_zero("eqz", reg, yes)
        self.asm.const(reg, 0)
        self.asm.goto(end)
        self.asm.label(yes)
        self.asm.const(reg, 1)
        self.asm.label(end)

    def eval_strcat(self, expr):
        parts = []

        def flat(node):
            if isinstance(node, ast.Binary) and node.op == "+" and getattr(node, "string_add", False):
                flat(node.left)
                flat(node.right)
            else:
                parts.append(node)

        flat(expr)
        sb = self.alloc()
        self.asm.new_instance(sb, "Ljava/lang/StringBuilder;")
        self.invoke("direct", ("Ljava/lang/StringBuilder;", "<init>", "V", ()), [sb])
        for part in parts:
            reg, ty = self._eval(part)
            if ty == "Integer":
                proto = ("I",)
            else:
                proto = ("Ljava/lang/String;",)
            self.invoke(
                "virtual",
                ("Ljava/lang/StringBuilder;", "append", "Ljava/lang/StringBuilder;", proto),
                [sb, reg],
            )
            self.asm.move_result(sb, True)
            self.release_to(sb + 1)
        self.invoke("virtual", ("Ljava/lang/StringBuilder;", "toString", "Ljava/lang/String;", ()), [sb])
        self.asm.move_result(sb, True)
        return sb, "String"

    def eval_call(self, node, as_stmt):
        kind = node.resolved[0]
        start = self.sp
        arg_regs = []
        for arg in node.args:
            reg, _ty = self._eval(arg)
            arg_regs.append(reg)
        if kind == "builtin":
            return self.emit_builtin(node, arg_regs, start, as_stmt)
        if kind == "native":
            native = node.resolved[1]
            key = (self.cx.native_desc, native["name"], "I", tuple("I" for _ in native["params"]))
            return self.finish_call("static", key, arg_regs, start, "Integer", as_stmt, instance=False)
        target = node.resolved[1]
        owner = class_desc(self.cx.package, target["unit"])
        ret = target["ret"]
        key = (
            owner,
            target["routine"].name,
            "V" if ret is None else TYPE_DESC[ret],
            tuple(TYPE_DESC[t] for t in target["params"]),
        )
        return self.finish_call(
            "static" if target["static"] else "virtual",
            key,
            arg_regs,
            start,
            ret,
            as_stmt,
            instance=not target["static"],
        )

    def finish_call(self, kind, key, arg_regs, start, ret, as_stmt, instance):
        regs = ([self.frame.this] + arg_regs) if instance else list(arg_regs)
        self.invoke(kind, key, regs)
        if ret is None:
            self.release_to(start)
            return None
        if arg_regs:
            dest = arg_regs[0]
        else:
            dest = self.alloc()
        self.asm.move_result(dest, _obj(ret))
        if as_stmt:
            self.release_to(start)
            return None
        if dest != start:
            self.asm.move(start, dest, _obj(ret))
            dest = start
        self.release_to(start + 1)
        self.sp = start + 1
        return start, ret

    def emit_builtin(self, node, arg_regs, start, as_stmt):
        name = node.resolved[1]
        if name == "Toast":
            dur = self.alloc()
            self.asm.const(dur, 0)
            self.invoke(
                "static",
                (
                    "Landroid/widget/Toast;",
                    "makeText",
                    "Landroid/widget/Toast;",
                    ("Landroid/content/Context;", "Ljava/lang/CharSequence;", "I"),
                ),
                [self.frame.this, arg_regs[0], dur],
            )
            self.asm.move_result(arg_regs[0], True)
            self.invoke("virtual", ("Landroid/widget/Toast;", "show", "V", ()), [arg_regs[0]])
            self.release_to(start)
            return None
        if name == "Log":
            tag = self.alloc()
            self.asm.const_string(tag, "Droide")
            self.invoke(
                "static",
                ("Landroid/util/Log;", "i", "I", ("Ljava/lang/String;", "Ljava/lang/String;")),
                [tag, arg_regs[0]],
            )
            self.asm.move_result(tag, False)
            self.release_to(start)
            return None
        if name == "Finish":
            self.invoke("virtual", ("Landroid/app/Activity;", "finish", "V", ()), [self.frame.this])
            self.release_to(start)
            return None
        if name == "OpenForm":
            form_name = getattr(node, "form_name", node.args[0].value)
            intent = self.alloc()
            self.asm.new_instance(intent, "Landroid/content/Intent;")
            self.invoke("direct", ("Landroid/content/Intent;", "<init>", "V", ()), [intent])
            cls = self.alloc()
            self.asm.const_class(cls, class_desc(self.cx.package, form_name))
            self.invoke(
                "virtual",
                (
                    "Landroid/content/Intent;",
                    "setClass",
                    "Landroid/content/Intent;",
                    ("Landroid/content/Context;", "Ljava/lang/Class;"),
                ),
                [intent, self.frame.this, cls],
            )
            self.asm.move_result(intent, True)
            self.invoke(
                "virtual",
                ("Landroid/app/Activity;", "startActivity", "V", ("Landroid/content/Intent;",)),
                [self.frame.this, intent],
            )
            self.release_to(start)
            return None
        if name == "IntToStr":
            self.invoke(
                "static",
                ("Ljava/lang/String;", "valueOf", "Ljava/lang/String;", ("I",)),
                [arg_regs[0]],
            )
            self.asm.move_result(start, True)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "String")
        if name == "StrToInt":
            self.invoke(
                "static",
                ("Ljava/lang/Integer;", "parseInt", "I", ("Ljava/lang/String;",)),
                [arg_regs[0]],
            )
            self.asm.move_result(start, False)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "Integer")
        if name == "Length":
            self.invoke("virtual", ("Ljava/lang/String;", "length", "I", ()), [arg_regs[0]])
            self.asm.move_result(start, False)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "Integer")
        raise CompileError(f"unknown builtin {name}")


class Compiler:
    def __init__(self, package, checked):
        self.package = package
        self.forms = checked["forms"]
        self.units = checked["units"]
        self.natives = checked["natives"]
        self.dex = DexFile()
        self._lab = 0
        self.native_desc = class_desc(package, "Native")
        self.rt_desc = class_desc(package, "DroideRt")

    def fresh(self):
        self._lab += 1
        return f"L{self._lab}"

    def emit(self):
        self.emit_runtime()
        if self.natives:
            self.emit_natives()
        for unit in self.units.values():
            self.emit_unit(unit)
        for form in self.forms.values():
            self.emit_form(form)
        return self.dex

    def emit_runtime(self):
        cls = ClassModel(self.rt_desc, "Ljava/lang/Object;", ACC_PUBLIC, source="DroideRt")
        asm = Asm()
        # v6 = context, v7 = value, ins = 2, registers = 8
        asm.invoke(
            "virtual",
            ("Landroid/content/Context;", "getResources", "Landroid/content/res/Resources;", ()),
            [6],
        )
        asm.move_result(0, True)
        asm.invoke(
            "virtual",
            ("Landroid/content/res/Resources;", "getDisplayMetrics", "Landroid/util/DisplayMetrics;", ()),
            [0],
        )
        asm.move_result(0, True)
        asm.iget(1, 0, ("Landroid/util/DisplayMetrics;", "density", "F"))
        asm.unop(0x82, 2, 7, "int-to-float")
        asm.binop(0xA8, 2, 2, 1, "mul-float")
        asm.const(3, 1)
        asm.unop(0x82, 3, 3, "int-to-float")
        asm.const(4, 2)
        asm.unop(0x82, 4, 4, "int-to-float")
        asm.binop(0xA9, 3, 3, 4, "div-float")
        asm.binop(0xA6, 2, 2, 3, "add-float")
        asm.unop(0x87, 0, 2, "float-to-int")
        asm.ret(0, "int")
        cls.method(MethodModel("dp", "I", ("Landroid/content/Context;", "I"), ACC_PUBLIC | ACC_STATIC, asm, 8, 2, 1))
        self.dex.add(cls)

    def emit_natives(self):
        cls = ClassModel(self.native_desc, "Ljava/lang/Object;", ACC_PUBLIC, source="Native")
        clinit = Asm()
        clinit.const_string(0, "droide")
        clinit.invoke("static", ("Ljava/lang/System;", "loadLibrary", "V", ("Ljava/lang/String;",)), [0])
        clinit.ret(0, "void")
        cls.method(MethodModel("<clinit>", "V", (), ACC_STATIC | ACC_CONSTRUCTOR, clinit, 1, 0, 1))
        for native in self.natives.values():
            params = tuple("I" for _ in native["params"])
            access = ACC_PUBLIC | ACC_STATIC | ACC_NATIVE
            cls.method(MethodModel(native["name"], "I", params, access, None, 0, 0, 0))
        self.dex.add(cls)

    def emit_unit(self, unit):
        if not unit["routines"]:
            return
        desc = class_desc(self.package, unit["name"])
        cls = ClassModel(desc, "Ljava/lang/Object;", ACC_PUBLIC, source=unit["name"] + ".droid")
        for rec in unit["routines"].values():
            cls.method(self.compile_routine(rec, desc, static=True, form=None))
        self.dex.add(cls)

    def compile_routine(self, rec, owner, static, form):
        routine = rec["routine"]
        if routine.name.lower() == "oncreate":
            raise CompileError(f"{routine.name} is reserved", routine.line, routine.col, "")
        params = [(p.name, t) for p, t in zip(routine.params, rec["params"])]
        locals_ = [(loc.name, _canon(loc.type_name)) for loc in routine.locals]
        if routine.kind == "function":
            locals_.append(("Result", rec["ret"]))
        frame = Frame(static, params, locals_)
        asm = Asm()
        gen = RoutineGen(self, asm, frame, form, owner)
        gen.prologue()
        gen.emit_body(routine.body)
        gen.finish(rec["ret"])
        ret = "V" if rec["ret"] is None else TYPE_DESC[rec["ret"]]
        param_descs = tuple(TYPE_DESC[t] for t in rec["params"])
        access = ACC_PUBLIC | (ACC_STATIC if static else 0)
        return MethodModel(routine.name, ret, param_descs, access, asm, frame.registers, frame.ins, gen.outs)

    def emit_form(self, form):
        name = form["name"]
        desc = class_desc(self.package, name)
        cls = ClassModel(desc, "Landroid/app/Activity;", ACC_PUBLIC, source=name + ".droid")
        for widget in form["widgets"].values():
            wdesc = WIDGETS[widget["comp"]["type"]]["desc"]
            cls.field(widget["name"], wdesc, ACC_PUBLIC)
        cls.method(self.activity_init(desc))
        cls.method(self.on_create(form, desc))
        for rec in form["routines"].values():
            cls.method(self.compile_routine(rec, desc, static=False, form=form))
        self.dex.add(cls)
        for widget in form["widgets"].values():
            handler = (widget["comp"].get("events") or {}).get("OnClick")
            if not handler:
                continue
            self.dex.add(self.listener(form, desc, widget, handler))

    def activity_init(self, desc):
        asm = Asm()
        asm.invoke("direct", ("Landroid/app/Activity;", "<init>", "V", ()), [0])
        asm.ret(0, "void")
        return MethodModel("<init>", "V", (), ACC_PUBLIC | ACC_CONSTRUCTOR, asm, 1, 1, 1)

    def on_create(self, form, desc):
        asm = Asm()
        this, bundle = 6, 7
        for reg in range(6):
            asm.const(reg, 0)
        asm.invoke("super", ("Landroid/app/Activity;", "onCreate", "V", ("Landroid/os/Bundle;",)), [this, bundle])
        form_json = form["form"]
        bg = parse_color(form_json.get("background") or "#F7F8FA")
        # v0 scroll, v1 frame
        asm.new_instance(0, "Landroid/widget/ScrollView;")
        asm.invoke("direct", ("Landroid/widget/ScrollView;", "<init>", "V", ("Landroid/content/Context;",)), [0, this])
        asm.const(2, bg)
        asm.invoke("virtual", ("Landroid/view/View;", "setBackgroundColor", "V", ("I",)), [0, 2])
        asm.new_instance(1, "Landroid/widget/FrameLayout;")
        asm.invoke("direct", ("Landroid/widget/FrameLayout;", "<init>", "V", ("Landroid/content/Context;",)), [1, this])
        asm.const(2, bg)
        asm.invoke("virtual", ("Landroid/view/View;", "setBackgroundColor", "V", ("I",)), [1, 2])
        comps = form_json.get("components") or []
        width = int(form_json.get("width") or 360)
        height = int(form_json.get("height") or 640)
        for comp in comps:
            width = max(width, int(comp["left"]) + int(comp["width"]))
            height = max(height, int(comp["top"]) + int(comp["height"]))
        self.place_frame(asm, this, width, height)
        for comp in comps:
            self.place_widget(asm, form, desc, this, comp)
        asm.invoke("virtual", ("Landroid/view/ViewGroup;", "addView", "V", ("Landroid/view/View;",)), [0, 1])
        asm.invoke("virtual", ("Landroid/app/Activity;", "setContentView", "V", ("Landroid/view/View;",)), [this, 0])
        handler = (form_json.get("events") or {}).get("OnCreate")
        if handler:
            rec = form["routines"][handler.lower()]
            key = self.handler_key(desc, rec)
            if rec["params"]:
                asm.invoke("virtual", key, [this, 1])
            else:
                asm.invoke("virtual", key, [this])
        asm.ret(0, "void")
        # ProgressBar's 3-arg constructor is four words (instance, context, attrs, style).
        return MethodModel("onCreate", "V", ("Landroid/os/Bundle;",), ACC_PUBLIC, asm, 8, 2, 4)

    def place_frame(self, asm, this, width, height):
        self.emit_dp(asm, this, 4, width)
        self.emit_dp(asm, this, 5, height)
        asm.new_instance(3, "Landroid/widget/FrameLayout$LayoutParams;")
        asm.invoke(
            "direct",
            ("Landroid/widget/FrameLayout$LayoutParams;", "<init>", "V", ("I", "I")),
            [3, 4, 5],
        )
        asm.invoke(
            "virtual",
            ("Landroid/view/View;", "setLayoutParams", "V", ("Landroid/view/ViewGroup$LayoutParams;",)),
            [1, 3],
        )

    def place_widget(self, asm, form, desc, this, comp):
        spec = WIDGETS[comp["type"]]
        wdesc = spec["desc"]
        kind = comp["type"]
        asm.new_instance(2, wdesc)
        if kind == "Progress":
            # Null AttributeSet (const/4 0) plus android.R.attr.progressBarStyleHorizontal.
            # That attr id is stable since API 1, so the device theme supplies the bar.
            asm.const(4, 0)
            asm.const(5, 0x01010078)
            asm.invoke(
                "direct",
                (wdesc, "<init>", "V", ("Landroid/content/Context;", "Landroid/util/AttributeSet;", "I")),
                [2, this, 4, 5],
            )
            asm.const(4, 0)
            asm.invoke("virtual", ("Landroid/widget/ProgressBar;", "setIndeterminate", "V", ("Z",)), [2, 4])
        elif kind == "Busy":
            # The one-argument constructor is the circular indicator. Leave it indeterminate.
            asm.invoke("direct", (wdesc, "<init>", "V", ("Landroid/content/Context;",)), [2, this])
            asm.const(4, 1)
            asm.invoke("virtual", ("Landroid/widget/ProgressBar;", "setIndeterminate", "V", ("Z",)), [2, 4])
        else:
            asm.invoke("direct", (wdesc, "<init>", "V", ("Landroid/content/Context;",)), [2, this])
        if kind == "Button":
            asm.const(4, 0)
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setAllCaps", "V", ("Z",)), [2, 4])
        if kind in ("Edit", "Number", "Password", "Email", "Phone"):
            asm.const(4, 1)
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setSingleLine", "V", ("Z",)), [2, 4])
        if kind == "Memo":
            asm.const(4, 0)
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setSingleLine", "V", ("Z",)), [2, 4])
            asm.const(4, 0x800033)  # Gravity.TOP | Gravity.START
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setGravity", "V", ("I",)), [2, 4])
        if kind == "Number":
            asm.const(4, 0x1002)  # TYPE_CLASS_NUMBER | TYPE_NUMBER_FLAG_SIGNED
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setInputType", "V", ("I",)), [2, 4])
        if kind == "Password":
            asm.const(4, 0x81)  # TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_PASSWORD
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setInputType", "V", ("I",)), [2, 4])
        if kind == "Email":
            asm.const(4, 0x21)  # TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_EMAIL_ADDRESS
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setInputType", "V", ("I",)), [2, 4])
        if kind == "Phone":
            asm.const(4, 0x3)  # TYPE_CLASS_PHONE
            asm.invoke("virtual", ("Landroid/widget/TextView;", "setInputType", "V", ("I",)), [2, 4])
        if kind == "Rating":
            # 1.0f from int-to-float. A raw const would be the wrong float bits.
            asm.const(4, 1)
            asm.unop(0x82, 4, 4, "int-to-float")
            asm.invoke("virtual", ("Landroid/widget/RatingBar;", "setStepSize", "V", ("F",)), [2, 4])
        props = comp.get("props") or {}
        # NumberPicker's default max is 0. setMinValue above that throws, and setValue
        # before the max is clamped. Write the ceiling first, then the floor, then the value.
        order = spec["props"]
        if kind == "Picker":
            order = ["MaxValue", "MinValue", "Value", "Wrap", "Enabled", "Visible"]
        for prop in order:
            if kind == "Date" and prop in ("Year", "Month", "Day"):
                continue
            if prop not in props:
                continue
            self.emit_const_prop(asm, 2, prop, props[prop])
        if kind == "Date":
            self.emit_update_date(asm, props)
        self.emit_dp(asm, this, 4, int(comp["width"]))
        self.emit_dp(asm, this, 5, int(comp["height"]))
        asm.new_instance(3, "Landroid/widget/FrameLayout$LayoutParams;")
        asm.invoke(
            "direct",
            ("Landroid/widget/FrameLayout$LayoutParams;", "<init>", "V", ("I", "I")),
            [3, 4, 5],
        )
        self.emit_dp(asm, this, 4, int(comp["left"]))
        self.emit_dp(asm, this, 5, int(comp["top"]))
        asm.iput(4, 3, ("Landroid/view/ViewGroup$MarginLayoutParams;", "leftMargin", "I"))
        asm.iput(5, 3, ("Landroid/view/ViewGroup$MarginLayoutParams;", "topMargin", "I"))
        asm.invoke(
            "virtual",
            ("Landroid/view/View;", "setLayoutParams", "V", ("Landroid/view/ViewGroup$LayoutParams;",)),
            [2, 3],
        )
        handler = (comp.get("events") or {}).get("OnClick")
        if handler:
            ldesc = class_desc(self.package, form["name"] + "$Click$" + comp["name"])
            asm.new_instance(3, ldesc)
            asm.invoke("direct", (ldesc, "<init>", "V", (desc,)), [3, this])
            asm.invoke(
                "virtual",
                ("Landroid/view/View;", "setOnClickListener", "V", ("Landroid/view/View$OnClickListener;",)),
                [2, 3],
            )
        asm.invoke("virtual", ("Landroid/view/ViewGroup;", "addView", "V", ("Landroid/view/View;",)), [1, 2])
        asm.iput_obj(2, this, (desc, comp["name"], wdesc))

    def emit_const_prop(self, asm, wreg, prop, value):
        owner, method, ret, params, mode = SETTERS[prop]
        key = (owner, method, ret, params)
        if mode == "string":
            asm.const_string(4, "" if value is None else str(value))
            asm.invoke("virtual", key, [wreg, 4])
        elif mode == "float":
            asm.const(4, int(value))
            asm.unop(0x82, 4, 4, "int-to-float")
            asm.invoke("virtual", key, [wreg, 4])
        elif mode == "int":
            asm.const(4, parse_color(value) if prop in ("Color", "TextColor") else int(value))
            asm.invoke("virtual", key, [wreg, 4])
        elif mode == "bool":
            asm.const(4, 1 if value else 0)
            asm.invoke("virtual", key, [wreg, 4])
        elif mode == "vis":
            asm.const(4, 0 if value else 8)
            asm.invoke("virtual", key, [wreg, 4])
        elif mode == "clock":
            text = ("Ljava/lang/CharSequence;",)
            asm.const_string(4, "" if value is None else str(value))
            asm.invoke("virtual", ("Landroid/widget/TextClock;", "setFormat12Hour", "V", text), [wreg, 4])
            asm.invoke("virtual", ("Landroid/widget/TextClock;", "setFormat24Hour", "V", text), [wreg, 4])
        elif mode == "boxbool":
            asm.const(4, 1 if value else 0)
            asm.invoke("static", ("Ljava/lang/Boolean;", "valueOf", "Ljava/lang/Boolean;", ("Z",)), [4])
            asm.move_result(4, True)
            asm.invoke("virtual", key, [wreg, 4])
        elif mode in ("date-year", "date-month", "date-day"):
            # Design-time dates are one updateDate after the other properties. See emit_update_date.
            return

    def emit_update_date(self, asm, props):
        # v3–v5 are free until LayoutParams. Month on the widget is 0–11.
        defaults = WIDGETS["Date"]["defaults"]["props"]
        year = int(props.get("Year", defaults["Year"]))
        month = int(props.get("Month", defaults["Month"]))
        day = int(props.get("Day", defaults["Day"]))
        asm.const(3, year)
        asm.const(4, month - 1)
        asm.const(5, day)
        asm.invoke(
            "virtual",
            ("Landroid/widget/DatePicker;", "updateDate", "V", ("I", "I", "I")),
            [2, 3, 4, 5],
        )

    def emit_dp(self, asm, this, dest, px):
        # const the argument into dest, invoke, then overwrite dest with the result.
        asm.const(dest, int(px))
        asm.invoke("static", (self.rt_desc, "dp", "I", ("Landroid/content/Context;", "I")), [this, dest])
        asm.move_result(dest, False)

    def handler_key(self, desc, rec):
        ret = "V" if rec["ret"] is None else TYPE_DESC[rec["ret"]]
        params = tuple(TYPE_DESC[t] for t in rec["params"])
        return (desc, rec["routine"].name, ret, params)

    def listener(self, form, form_desc, widget, handler):
        lname = form["name"] + "$Click$" + widget["name"]
        desc = class_desc(self.package, lname)
        cls = ClassModel(
            desc,
            "Ljava/lang/Object;",
            ACC_PUBLIC,
            interfaces=["Landroid/view/View$OnClickListener;"],
            source=form["name"] + ".droid",
        )
        cls.field("this$0", form_desc, ACC_PUBLIC)
        init = Asm()
        init.invoke("direct", ("Ljava/lang/Object;", "<init>", "V", ()), [0])
        init.iput_obj(1, 0, (desc, "this$0", form_desc))
        init.ret(0, "void")
        cls.method(MethodModel("<init>", "V", (form_desc,), ACC_PUBLIC | ACC_CONSTRUCTOR, init, 2, 2, 1))
        rec = form["routines"][handler.lower()]
        click = Asm()
        # v1 = this, v2 = view, v0 = outer. registers 3, ins 2
        click.iget_obj(0, 1, (desc, "this$0", form_desc))
        key = self.handler_key(form_desc, rec)
        if rec["params"]:
            click.invoke("virtual", key, [0, 2])
        else:
            click.invoke("virtual", key, [0])
        click.ret(0, "void")
        outs = 2 if rec["params"] else 1
        cls.method(MethodModel("onClick", "V", ("Landroid/view/View;",), ACC_PUBLIC, click, 3, 2, outs))
        return cls


def _canon(name):
    return {"integer": "Integer", "string": "String", "boolean": "Boolean", "view": "View"}.get(name.lower(), name)


def compile_dex(package, checked):
    return Compiler(package, checked).emit()
