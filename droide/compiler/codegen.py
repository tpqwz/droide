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
from .index import attach as attach_index
from .search import attach as attach_search
from .widgets import GETTERS, SETTERS, WIDGETS, parse_color

SCRATCH = 6


def _parent_name(comp):
    parent = comp.get("parent") or ""
    if not isinstance(parent, str):
        return ""
    return parent.strip()


def _horizontal(comp):
    props = comp.get("props") or {}
    text = props.get("Direction") or ""
    return isinstance(text, str) and text.strip().lower() == "horizontal"
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
            self.emit_setter(wreg, actual, vreg, vty, widget["comp"]["type"])
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

    def emit_setter(self, wreg, prop, vreg, vty, kind):
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
        if mode in ("web-html", "web-url"):
            self.emit_web_assign(wreg, vreg, mode)
            return
        if mode == "orient":
            # Horizontal is orientation 0. Anything else, including Vertical, is 1.
            vertical = self.fresh()
            apply = self.fresh()
            text = self.alloc()
            self.asm.const_string(text, "Horizontal")
            self.invoke(
                "virtual",
                ("Ljava/lang/String;", "equalsIgnoreCase", "Z", ("Ljava/lang/String;",)),
                [vreg, text],
            )
            self.asm.move_result(text, False)
            self.zero_except(wreg, text)
            self.asm.if_zero("eqz", text, vertical)
            self.asm.const(text, 0)
            self.asm.goto(apply)
            self.asm.label(vertical)
            self.asm.const(text, 1)
            self.asm.label(apply)
            self.invoke(
                "virtual",
                ("Landroid/widget/LinearLayout;", "setOrientation", "V", ("I",)),
                [wreg, text],
            )
            return
        if mode == "lines":
            if kind == "Spinner":
                name, view = "setSpinner", "Landroid/widget/Spinner;"
            else:
                name, view = "setLines", "Landroid/widget/ListView;"
            self.invoke(
                "static",
                (self.cx.rt_desc, name, "V", (
                    view,
                    "Landroid/content/Context;",
                    "Ljava/lang/String;",
                )),
                [wreg, self.frame.this, vreg],
            )
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

    def emit_web_assign(self, wreg, vreg, mode):
        settings = self.alloc()
        self.invoke(
            "virtual",
            ("Landroid/webkit/WebView;", "getSettings", "Landroid/webkit/WebSettings;", ()),
            [wreg],
        )
        self.asm.move_result(settings, True)
        one = self.alloc()
        self.asm.const(one, 1)
        self.invoke(
            "virtual",
            ("Landroid/webkit/WebSettings;", "setJavaScriptEnabled", "V", ("Z",)),
            [settings, one],
        )
        if mode == "web-url":
            self.invoke(
                "virtual",
                ("Landroid/webkit/WebView;", "loadUrl", "V", ("Ljava/lang/String;",)),
                [wreg, vreg],
            )
            return
        mime = self.alloc()
        enc = self.alloc()
        self.asm.const_string(mime, "text/html")
        self.asm.const_string(enc, "utf-8")
        self.invoke(
            "virtual",
            (
                "Landroid/webkit/WebView;",
                "loadData",
                "V",
                ("Ljava/lang/String;", "Ljava/lang/String;", "Ljava/lang/String;"),
            ),
            [wreg, vreg, mime, enc],
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

    def _new_builder(self):
        builder = self.alloc()
        self.asm.new_instance(builder, "Landroid/app/AlertDialog$Builder;")
        self.invoke(
            "direct",
            (
                "Landroid/app/AlertDialog$Builder;",
                "<init>",
                "V",
                ("Landroid/content/Context;",),
            ),
            [builder, self.frame.this],
        )
        return builder

    def _alert_message(self, builder, text):
        self.invoke(
            "virtual",
            (
                "Landroid/app/AlertDialog$Builder;",
                "setMessage",
                "Landroid/app/AlertDialog$Builder;",
                ("Ljava/lang/CharSequence;",),
            ),
            [builder, text],
        )
        self.asm.move_result(builder, True)

    def _alert_button(self, name, builder, label, listener):
        self.invoke(
            "virtual",
            (
                "Landroid/app/AlertDialog$Builder;",
                name,
                "Landroid/app/AlertDialog$Builder;",
                ("Ljava/lang/CharSequence;", "Landroid/content/DialogInterface$OnClickListener;"),
            ),
            [builder, label, listener],
        )
        self.asm.move_result(builder, True)

    def _alert_show(self, builder, dest):
        self.invoke(
            "virtual",
            (
                "Landroid/app/AlertDialog$Builder;",
                "show",
                "Landroid/app/AlertDialog;",
                (),
            ),
            [builder],
        )
        self.asm.move_result(dest, True)

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
        if name == "Message":
            builder = self._new_builder()
            self._alert_message(builder, arg_regs[0])
            ok = self.alloc()
            self.asm.const_string(ok, "OK")
            none = self.alloc()
            self.asm.const(none, 0)
            self._alert_button("setPositiveButton", builder, ok, none)
            self._alert_show(builder, ok)
            self.release_to(start)
            return None
        if name == "Question":
            handler = getattr(node, "form_name", "") or ""
            if not handler:
                raise CompileError("Question needs a procedure name")
            # The procedure name is fixed at check time. Drop its register
            # so the builder, the label, and the listener fit in scratch.
            if len(arg_regs) > 1 and self.sp > start + 1 and arg_regs[1] == self.sp - 1:
                self.release_to(self.sp - 1)
            builder = self._new_builder()
            self._alert_message(builder, arg_regs[0])
            yes = self.alloc()
            self.asm.const_string(yes, "Yes")
            form_desc = self.owner
            ldesc = form_desc[:-1] + "$Ask$" + handler + ";"
            listen = self.alloc()
            self.asm.new_instance(listen, ldesc)
            self.invoke("direct", (ldesc, "<init>", "V", (form_desc,)), [listen, self.frame.this])
            self._alert_button("setPositiveButton", builder, yes, listen)
            self.release_to(builder + 1)
            no = self.alloc()
            self.asm.const_string(no, "No")
            none = self.alloc()
            self.asm.const(none, 0)
            self._alert_button("setNegativeButton", builder, no, none)
            self._alert_show(builder, no)
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
        if name == "IsInt":
            self.invoke(
                "static",
                (self.cx.rt_desc, "isInt", "Z", ("Ljava/lang/String;",)),
                [arg_regs[0]],
            )
            self.asm.move_result(start, False)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "Boolean")
        if name == "Length":
            self.invoke("virtual", ("Ljava/lang/String;", "length", "I", ()), [arg_regs[0]])
            self.asm.move_result(start, False)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "Integer")
        rt = self.cx.rt_desc
        if name == "WriteText":
            self.invoke(
                "static",
                (rt, "writeText", "V", ("Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;")),
                [self.frame.this, arg_regs[0], arg_regs[1]],
            )
            self.release_to(start)
            return None
        if name == "ReadText":
            self.invoke(
                "static",
                (rt, "readText", "Ljava/lang/String;", ("Landroid/content/Context;", "Ljava/lang/String;")),
                [self.frame.this, arg_regs[0]],
            )
            self.asm.move_result(start, True)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "String")
        if name == "Where":
            self.invoke(
                "static",
                (rt, "where", "Ljava/lang/String;", ("Landroid/content/Context;",)),
                [self.frame.this],
            )
            self.asm.move_result(start, True)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "String")
        if name == "OpenCamera":
            self.invoke(
                "static",
                (rt, "openCamera", "V", ("Landroid/content/Context;",)),
                [self.frame.this],
            )
            self.release_to(start)
            return None
        if name == "Share":
            self.invoke(
                "static",
                (rt, "share", "V", ("Landroid/content/Context;", "Ljava/lang/String;")),
                [self.frame.this, arg_regs[0]],
            )
            self.release_to(start)
            return None
        if name == "CopyText":
            self.invoke(
                "static",
                (rt, "copyText", "V", ("Landroid/content/Context;", "Ljava/lang/String;")),
                [self.frame.this, arg_regs[0]],
            )
            self.release_to(start)
            return None
        if name == "PasteText":
            self.invoke(
                "static",
                (rt, "pasteText", "Ljava/lang/String;", ("Landroid/content/Context;",)),
                [self.frame.this],
            )
            self.asm.move_result(start, True)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "String")
        if name == "Search":
            self.invoke(
                "static",
                (rt, "search", "Ljava/lang/String;", ("Landroid/content/Context;", "Ljava/lang/String;")),
                [self.frame.this, arg_regs[0]],
            )
            self.asm.move_result(start, True)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "String")
        if name == "Lookup":
            self.invoke(
                "static",
                (rt, "lookup", "Ljava/lang/String;", ("Landroid/content/Context;", "Ljava/lang/String;")),
                [self.frame.this, arg_regs[0]],
            )
            self.asm.move_result(start, True)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "String")
        if name == "Ingest":
            self.invoke(
                "static",
                (rt, "ingest", "Ljava/lang/String;", (
                    "Landroid/content/Context;", "Ljava/lang/String;",
                    "Ljava/lang/String;", "Ljava/lang/String;",
                )),
                [self.frame.this, arg_regs[0], arg_regs[1], arg_regs[2]],
            )
            self.asm.move_result(start, True)
            self.release_to(start + 1)
            self.sp = start + 1
            return None if as_stmt else (start, "String")
        raise CompileError(f"unknown builtin {name}")


def emit_is_int(asm):
    """True for an optional sign and one to nine digits.

    The string parameter is v4 (registers=5, ins=1). A false result lets a
    form skip StrToInt. There is no null test: if-eqz rejects a reference.
    """
    asm.invoke("virtual", ("Ljava/lang/String;", "length", "I", ()), [4])
    asm.move_result(1, False)
    asm.if_zero("eqz", 1, "is_fail")
    asm.const(0, 0)
    asm.invoke("virtual", ("Ljava/lang/String;", "charAt", "C", ("I",)), [4, 0])
    asm.move_result(2, False)
    asm.const(3, 43)
    asm.if_cmp("eq", 2, 3, "is_signed")
    asm.const(3, 45)
    asm.if_cmp("ne", 2, 3, "is_bounds")
    asm.label("is_signed")
    asm.const(0, 1)
    asm.label("is_bounds")
    asm.if_cmp("ge", 0, 1, "is_fail")
    asm.const(3, 9)
    asm.binop(0x90, 3, 3, 0, "add-int")
    asm.if_cmp("gt", 1, 3, "is_fail")
    asm.label("is_loop")
    asm.if_cmp("ge", 0, 1, "is_ok")
    asm.invoke("virtual", ("Ljava/lang/String;", "charAt", "C", ("I",)), [4, 0])
    asm.move_result(2, False)
    asm.const(3, 48)
    asm.if_cmp("lt", 2, 3, "is_fail")
    asm.const(3, 57)
    asm.if_cmp("gt", 2, 3, "is_fail")
    asm.const(3, 1)
    asm.binop(0x90, 0, 0, 3, "add-int")
    asm.goto("is_loop")
    asm.label("is_ok")
    asm.const(0, 1)
    asm.ret(0, "int")
    asm.label("is_fail")
    asm.const(0, 0)
    asm.ret(0, "int")


def emit_write_text(asm):
    """v5 context, v6 name, v7 text. The file is under the app's files directory."""
    asm.invoke("virtual", ("Landroid/content/Context;", "getFilesDir", "Ljava/io/File;", ()), [5])
    asm.move_result(0, True)
    asm.new_instance(1, "Ljava/io/File;")
    asm.invoke("direct", ("Ljava/io/File;", "<init>", "V", ("Ljava/io/File;", "Ljava/lang/String;")), [1, 0, 6])
    asm.new_instance(2, "Ljava/io/FileOutputStream;")
    asm.invoke("direct", ("Ljava/io/FileOutputStream;", "<init>", "V", ("Ljava/io/File;",)), [2, 1])
    asm.invoke("virtual", ("Ljava/lang/String;", "getBytes", "[B", ()), [7])
    asm.move_result(3, True)
    asm.invoke("virtual", ("Ljava/io/FileOutputStream;", "write", "V", ("[B",)), [2, 3])
    asm.invoke("virtual", ("Ljava/io/FileOutputStream;", "close", "V", ()), [2])
    asm.ret(0, "void")


def emit_read_text(asm):
    """Missing and empty files both return an empty string, on separate returns."""
    asm.invoke("virtual", ("Landroid/content/Context;", "getFilesDir", "Ljava/io/File;", ()), [6])
    asm.move_result(0, True)
    asm.new_instance(1, "Ljava/io/File;")
    asm.invoke("direct", ("Ljava/io/File;", "<init>", "V", ("Ljava/io/File;", "Ljava/lang/String;")), [1, 0, 7])
    asm.invoke("virtual", ("Ljava/io/File;", "exists", "Z", ()), [1])
    asm.move_result(2, False)
    asm.if_zero("eqz", 2, "missing")
    asm.new_instance(2, "Ljava/io/FileInputStream;")
    asm.invoke("direct", ("Ljava/io/FileInputStream;", "<init>", "V", ("Ljava/io/File;",)), [2, 1])
    asm.new_instance(3, "Ljava/util/Scanner;")
    asm.invoke("direct", ("Ljava/util/Scanner;", "<init>", "V", ("Ljava/io/InputStream;",)), [3, 2])
    asm.const_string(4, "\\A")
    asm.invoke(
        "virtual",
        ("Ljava/util/Scanner;", "useDelimiter", "Ljava/util/Scanner;", ("Ljava/lang/String;",)),
        [3, 4],
    )
    asm.move_result(3, True)
    asm.invoke("virtual", ("Ljava/util/Scanner;", "hasNext", "Z", ()), [3])
    asm.move_result(4, False)
    asm.if_zero("eqz", 4, "empty")
    asm.invoke("virtual", ("Ljava/util/Scanner;", "next", "Ljava/lang/String;", ()), [3])
    asm.move_result(0, True)
    asm.invoke("virtual", ("Ljava/util/Scanner;", "close", "V", ()), [3])
    asm.ret(0, "obj")
    asm.label("empty")
    asm.invoke("virtual", ("Ljava/util/Scanner;", "close", "V", ()), [3])
    asm.const_string(0, "")
    asm.ret(0, "obj")
    asm.label("missing")
    asm.const_string(0, "")
    asm.ret(0, "obj")


def emit_where(asm):
    """v11 is the context. Latitude and longitude stay inside this method."""
    asm.const_string(0, "android.permission.ACCESS_FINE_LOCATION")
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "checkSelfPermission", "I", ("Ljava/lang/String;",)),
        [11, 0],
    )
    asm.move_result(0, False)
    asm.if_zero("eqz", 0, "granted")
    asm.const(0, 2)
    asm.new_array(1, 0, "[Ljava/lang/String;")
    asm.const_string(2, "android.permission.ACCESS_FINE_LOCATION")
    asm.const(0, 0)
    asm.aput_object(2, 1, 0)
    asm.const_string(2, "android.permission.ACCESS_COARSE_LOCATION")
    asm.const(0, 1)
    asm.aput_object(2, 1, 0)
    asm.check_cast(11, "Landroid/app/Activity;")
    asm.const(0, 1)
    asm.invoke(
        "virtual",
        ("Landroid/app/Activity;", "requestPermissions", "V", ("[Ljava/lang/String;", "I")),
        [11, 1, 0],
    )
    asm.const_string(0, "Allow location")
    asm.ret(0, "obj")
    asm.label("granted")
    asm.const_string(0, "location")
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "getSystemService", "Ljava/lang/Object;", ("Ljava/lang/String;",)),
        [11, 0],
    )
    asm.move_result(0, True)
    asm.check_cast(0, "Landroid/location/LocationManager;")
    loc = (
        "Landroid/location/LocationManager;",
        "getLastKnownLocation",
        "Landroid/location/Location;",
        ("Ljava/lang/String;",),
    )
    asm.const_string(1, "gps")
    asm.invoke("virtual", loc, [0, 1])
    asm.move_result(1, True)
    asm.if_zero("nez", 1, "have")
    asm.const_string(1, "network")
    asm.invoke("virtual", loc, [0, 1])
    asm.move_result(1, True)
    asm.if_zero("nez", 1, "have")
    asm.const_string(1, "passive")
    asm.invoke("virtual", loc, [0, 1])
    asm.move_result(1, True)
    asm.if_zero("eqz", 1, "none")
    asm.label("have")
    asm.invoke("virtual", ("Landroid/location/Location;", "getLatitude", "D", ()), [1])
    asm.move_result_wide(2)
    value_of = ("Ljava/lang/String;", "valueOf", "Ljava/lang/String;", ("D",))
    asm.invoke("static", value_of, [2, 3])
    asm.move_result(4, True)
    asm.invoke("virtual", ("Landroid/location/Location;", "getLongitude", "D", ()), [1])
    asm.move_result_wide(2)
    asm.invoke("static", value_of, [2, 3])
    asm.move_result(5, True)
    asm.const_string(6, ", ")
    concat = ("Ljava/lang/String;", "concat", "Ljava/lang/String;", ("Ljava/lang/String;",))
    asm.invoke("virtual", concat, [4, 6])
    asm.move_result(4, True)
    asm.invoke("virtual", concat, [4, 5])
    asm.move_result(4, True)
    asm.ret(4, "obj")
    asm.label("none")
    asm.const_string(0, "No location")
    asm.ret(0, "obj")


def emit_open_camera(asm):
    asm.new_instance(0, "Landroid/content/Intent;")
    asm.const_string(1, "android.media.action.IMAGE_CAPTURE")
    asm.invoke("direct", ("Landroid/content/Intent;", "<init>", "V", ("Ljava/lang/String;",)), [0, 1])
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "getPackageManager", "Landroid/content/pm/PackageManager;", ()),
        [5],
    )
    asm.move_result(2, True)
    asm.const(3, 0)
    asm.invoke(
        "virtual",
        (
            "Landroid/content/pm/PackageManager;",
            "queryIntentActivities",
            "Ljava/util/List;",
            ("Landroid/content/Intent;", "I"),
        ),
        [2, 0, 3],
    )
    asm.move_result(2, True)
    asm.invoke("interface", ("Ljava/util/List;", "isEmpty", "Z", ()), [2])
    asm.move_result(3, False)
    asm.if_zero("nez", 3, "done")
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "startActivity", "V", ("Landroid/content/Intent;",)),
        [5, 0],
    )
    asm.label("done")
    asm.ret(0, "void")


def emit_share(asm):
    # v6 context, v7 text. An empty handler list skips the chooser.
    asm.new_instance(0, "Landroid/content/Intent;")
    asm.const_string(1, "android.intent.action.SEND")
    asm.invoke("direct", ("Landroid/content/Intent;", "<init>", "V", ("Ljava/lang/String;",)), [0, 1])
    asm.const_string(1, "text/plain")
    asm.invoke(
        "virtual",
        ("Landroid/content/Intent;", "setType", "Landroid/content/Intent;", ("Ljava/lang/String;",)),
        [0, 1],
    )
    asm.move_result(0, True)
    asm.const_string(1, "android.intent.extra.TEXT")
    asm.invoke(
        "virtual",
        (
            "Landroid/content/Intent;",
            "putExtra",
            "Landroid/content/Intent;",
            ("Ljava/lang/String;", "Ljava/lang/String;"),
        ),
        [0, 1, 7],
    )
    asm.move_result(0, True)
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "getPackageManager", "Landroid/content/pm/PackageManager;", ()),
        [6],
    )
    asm.move_result(2, True)
    asm.const(3, 0)
    asm.invoke(
        "virtual",
        (
            "Landroid/content/pm/PackageManager;",
            "queryIntentActivities",
            "Ljava/util/List;",
            ("Landroid/content/Intent;", "I"),
        ),
        [2, 0, 3],
    )
    asm.move_result(2, True)
    asm.invoke("interface", ("Ljava/util/List;", "isEmpty", "Z", ()), [2])
    asm.move_result(3, False)
    asm.if_zero("nez", 3, "done")
    asm.const_string(1, "Share")
    asm.invoke(
        "static",
        (
            "Landroid/content/Intent;",
            "createChooser",
            "Landroid/content/Intent;",
            ("Landroid/content/Intent;", "Ljava/lang/CharSequence;"),
        ),
        [0, 1],
    )
    asm.move_result(0, True)
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "startActivity", "V", ("Landroid/content/Intent;",)),
        [6, 0],
    )
    asm.label("done")
    asm.ret(0, "void")


def emit_copy_text(asm):
    # v6 context, v7 text. A missing clipboard service returns.
    asm.const_string(0, "clipboard")
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "getSystemService", "Ljava/lang/Object;", ("Ljava/lang/String;",)),
        [6, 0],
    )
    asm.move_result(0, True)
    asm.if_zero("eqz", 0, "done")
    asm.check_cast(0, "Landroid/content/ClipboardManager;")
    asm.const_string(1, "text")
    asm.invoke(
        "static",
        (
            "Landroid/content/ClipData;",
            "newPlainText",
            "Landroid/content/ClipData;",
            ("Ljava/lang/CharSequence;", "Ljava/lang/CharSequence;"),
        ),
        [1, 7],
    )
    asm.move_result(1, True)
    asm.invoke(
        "virtual",
        ("Landroid/content/ClipboardManager;", "setPrimaryClip", "V", ("Landroid/content/ClipData;",)),
        [0, 1],
    )
    asm.label("done")
    asm.ret(0, "void")


def emit_paste_text(asm):
    # v7 context. v1 stays an int and v2 stays a reference so the empty exits share one label.
    asm.const_string(0, "clipboard")
    asm.invoke(
        "virtual",
        ("Landroid/content/Context;", "getSystemService", "Ljava/lang/Object;", ("Ljava/lang/String;",)),
        [7, 0],
    )
    asm.move_result(0, True)
    asm.if_zero("eqz", 0, "none")
    asm.check_cast(0, "Landroid/content/ClipboardManager;")
    asm.invoke("virtual", ("Landroid/content/ClipboardManager;", "hasPrimaryClip", "Z", ()), [0])
    asm.move_result(1, False)
    asm.if_zero("eqz", 1, "none")
    asm.invoke(
        "virtual",
        ("Landroid/content/ClipboardManager;", "getPrimaryClip", "Landroid/content/ClipData;", ()),
        [0],
    )
    asm.move_result(2, True)
    asm.if_zero("eqz", 2, "none")
    asm.invoke("virtual", ("Landroid/content/ClipData;", "getItemCount", "I", ()), [2])
    asm.move_result(1, False)
    asm.if_zero("eqz", 1, "none")
    asm.const(1, 0)
    asm.invoke(
        "virtual",
        ("Landroid/content/ClipData;", "getItemAt", "Landroid/content/ClipData$Item;", ("I",)),
        [2, 1],
    )
    asm.move_result(2, True)
    asm.if_zero("eqz", 2, "none")
    asm.invoke(
        "virtual",
        ("Landroid/content/ClipData$Item;", "getText", "Ljava/lang/CharSequence;", ()),
        [2],
    )
    asm.move_result(2, True)
    asm.if_zero("eqz", 2, "none")
    asm.invoke("virtual", ("Ljava/lang/Object;", "toString", "Ljava/lang/String;", ()), [2])
    asm.move_result(2, True)
    asm.ret(2, "obj")
    asm.label("none")
    asm.const_string(0, "")
    asm.ret(0, "obj")


def emit_set_lines(asm):
    # v5 list, v6 context, v7 text. registers 8, ins 3.
    # android.R.layout.simple_list_item_1 is 0x01090003. Empty text is a zero-length array.
    asm.invoke("virtual", ("Ljava/lang/String;", "length", "I", ()), [7])
    asm.move_result(0, False)
    asm.if_zero("nez", 0, "lines_have")
    asm.const(3, 0)
    asm.new_array(0, 3, "[Ljava/lang/String;")
    asm.goto("lines_ready")
    asm.label("lines_have")
    asm.const_string(1, "\n")
    asm.invoke(
        "virtual",
        ("Ljava/lang/String;", "split", "[Ljava/lang/String;", ("Ljava/lang/String;",)),
        [7, 1],
    )
    asm.move_result(0, True)
    asm.label("lines_ready")
    asm.check_cast(0, "[Ljava/lang/Object;")
    asm.const(2, 0x01090003)
    asm.new_instance(1, "Landroid/widget/ArrayAdapter;")
    asm.invoke(
        "direct",
        (
            "Landroid/widget/ArrayAdapter;",
            "<init>",
            "V",
            ("Landroid/content/Context;", "I", "[Ljava/lang/Object;"),
        ),
        [1, 6, 2, 0],
    )
    asm.invoke(
        "virtual",
        ("Landroid/widget/AbsListView;", "setAdapter", "V", ("Landroid/widget/ListAdapter;",)),
        [5, 1],
    )
    asm.ret(0, "void")


def emit_set_spinner(asm):
    # v5 spinner, v6 context, v7 text. registers 8, ins 3.
    # Same rows as setLines. The adapter call is AbsSpinner.setAdapter(SpinnerAdapter).
    asm.invoke("virtual", ("Ljava/lang/String;", "length", "I", ()), [7])
    asm.move_result(0, False)
    asm.if_zero("nez", 0, "lines_have")
    asm.const(3, 0)
    asm.new_array(0, 3, "[Ljava/lang/String;")
    asm.goto("lines_ready")
    asm.label("lines_have")
    asm.const_string(1, "\n")
    asm.invoke(
        "virtual",
        ("Ljava/lang/String;", "split", "[Ljava/lang/String;", ("Ljava/lang/String;",)),
        [7, 1],
    )
    asm.move_result(0, True)
    asm.label("lines_ready")
    asm.check_cast(0, "[Ljava/lang/Object;")
    asm.const(2, 0x01090003)
    asm.new_instance(1, "Landroid/widget/ArrayAdapter;")
    asm.invoke(
        "direct",
        (
            "Landroid/widget/ArrayAdapter;",
            "<init>",
            "V",
            ("Landroid/content/Context;", "I", "[Ljava/lang/Object;"),
        ),
        [1, 6, 2, 0],
    )
    asm.invoke(
        "virtual",
        ("Landroid/widget/AbsSpinner;", "setAdapter", "V", ("Landroid/widget/SpinnerAdapter;",)),
        [5, 1],
    )
    asm.ret(0, "void")


def emit_web_settings(asm, web, tmp, flag):
    asm.invoke("virtual", ("Landroid/webkit/WebView;", "getSettings", "Landroid/webkit/WebSettings;", ()), [web])
    asm.move_result(tmp, True)
    asm.const(flag, 1)
    asm.invoke("virtual", ("Landroid/webkit/WebSettings;", "setJavaScriptEnabled", "V", ("Z",)), [tmp, flag])


def emit_web_load(asm, props):
    html = str((props or {}).get("Html") or "").strip()
    url = str((props or {}).get("Url") or "").strip()
    if not html and not url:
        return
    emit_web_settings(asm, 2, 3, 4)
    if html:
        asm.const_string(3, html)
        asm.const_string(4, "text/html")
        asm.const_string(5, "utf-8")
        asm.invoke(
            "virtual",
            (
                "Landroid/webkit/WebView;",
                "loadData",
                "V",
                ("Ljava/lang/String;", "Ljava/lang/String;", "Ljava/lang/String;"),
            ),
            [2, 3, 4, 5],
        )
    else:
        asm.const_string(3, url)
        asm.invoke("virtual", ("Landroid/webkit/WebView;", "loadUrl", "V", ("Ljava/lang/String;",)), [2, 3])


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
        is_int = Asm()
        emit_is_int(is_int)
        cls.method(MethodModel("isInt", "Z", ("Ljava/lang/String;",), ACC_PUBLIC | ACC_STATIC, is_int, 5, 1, 2))
        write_text = Asm()
        emit_write_text(write_text)
        cls.method(MethodModel(
            "writeText", "V",
            ("Landroid/content/Context;", "Ljava/lang/String;", "Ljava/lang/String;"),
            ACC_PUBLIC | ACC_STATIC, write_text, 8, 3, 3,
        ))
        read_text = Asm()
        emit_read_text(read_text)
        cls.method(MethodModel(
            "readText", "Ljava/lang/String;",
            ("Landroid/content/Context;", "Ljava/lang/String;"),
            ACC_PUBLIC | ACC_STATIC, read_text, 8, 2, 3,
        ))
        where = Asm()
        emit_where(where)
        cls.method(MethodModel(
            "where", "Ljava/lang/String;",
            ("Landroid/content/Context;",),
            ACC_PUBLIC | ACC_STATIC, where, 12, 1, 3,
        ))
        camera = Asm()
        emit_open_camera(camera)
        cls.method(MethodModel(
            "openCamera", "V",
            ("Landroid/content/Context;",),
            ACC_PUBLIC | ACC_STATIC, camera, 6, 1, 3,
        ))
        share = Asm()
        emit_share(share)
        cls.method(MethodModel(
            "share", "V",
            ("Landroid/content/Context;", "Ljava/lang/String;"),
            ACC_PUBLIC | ACC_STATIC, share, 8, 2, 3,
        ))
        copy_text = Asm()
        emit_copy_text(copy_text)
        cls.method(MethodModel(
            "copyText", "V",
            ("Landroid/content/Context;", "Ljava/lang/String;"),
            ACC_PUBLIC | ACC_STATIC, copy_text, 8, 2, 2,
        ))
        paste_text = Asm()
        emit_paste_text(paste_text)
        cls.method(MethodModel(
            "pasteText", "Ljava/lang/String;",
            ("Landroid/content/Context;",),
            ACC_PUBLIC | ACC_STATIC, paste_text, 8, 1, 2,
        ))
        lines = Asm()
        emit_set_lines(lines)
        cls.method(MethodModel(
            "setLines", "V",
            ("Landroid/widget/ListView;", "Landroid/content/Context;", "Ljava/lang/String;"),
            ACC_PUBLIC | ACC_STATIC, lines, 8, 3, 4,
        ))
        spinner = Asm()
        emit_set_spinner(spinner)
        cls.method(MethodModel(
            "setSpinner", "V",
            ("Landroid/widget/Spinner;", "Landroid/content/Context;", "Ljava/lang/String;"),
            ACC_PUBLIC | ACC_STATIC, spinner, 8, 3, 4,
        ))
        attach_search(self.dex, self.package, cls)
        attach_index(self.dex, self.package, cls)
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
            events = widget["comp"].get("events") or {}
            handler = events.get("OnClick")
            if handler:
                self.dex.add(self.listener(form, desc, widget, handler))
            handler = events.get("OnItemClick")
            if handler:
                self.dex.add(self.item_listener(form, desc, widget, handler))
            handler = events.get("OnLongClick")
            if handler:
                self.dex.add(self.long_listener(form, desc, widget, handler))
            handler = events.get("OnItemLongClick")
            if handler:
                self.dex.add(self.item_long_listener(form, desc, widget, handler))
            handler = events.get("OnItemSelected")
            if handler:
                self.dex.add(self.select_listener(form, desc, widget, handler))
            handler = events.get("OnChange")
            if handler:
                self.dex.add(self.change_listener(form, desc, widget, handler))
        # The method that names the listener is already encoded. The class
        # can follow it; DEX refs are resolved when the file is built.
        for handler in _ask_handlers(form):
            self.dex.add(self.ask_listener(form, desc, handler))

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
            # A child's left and top are margins inside its panel, not form coordinates.
            if _parent_name(comp):
                continue
            width = max(width, int(comp["left"]) + int(comp["width"]))
            height = max(height, int(comp["top"]) + int(comp["height"]))
        self.place_frame(asm, this, width, height)
        for comp in comps:
            self.place_widget(asm, form, desc, this, comp)
        self.place_children(asm, desc, this, comps)
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
        # ProgressBar's constructor takes four words. DatePicker.init takes five.
        return MethodModel("onCreate", "V", ("Landroid/os/Bundle;",), ACC_PUBLIC, asm, 8, 2, 5)

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
            if kind == "Web" and prop in ("Html", "Url"):
                continue
            if kind == "List" and prop == "Items":
                continue
            if kind == "Spinner" and prop in ("Items", "Selection"):
                continue
            if kind == "Panel" and prop == "Direction":
                continue
            if prop not in props:
                continue
            self.emit_const_prop(asm, 2, prop, props[prop])
        if kind == "Date" and not (comp.get("events") or {}).get("OnChange"):
            self.emit_update_date(asm, props)
        if kind == "Web":
            emit_web_load(asm, props)
        if kind == "List":
            asm.const_string(3, str((props or {}).get("Items") or ""))
            asm.invoke(
                "static",
                (self.rt_desc, "setLines", "V", (
                    "Landroid/widget/ListView;",
                    "Landroid/content/Context;",
                    "Ljava/lang/String;",
                )),
                [2, this, 3],
            )
        if kind == "Spinner":
            asm.const_string(3, str((props or {}).get("Items") or ""))
            asm.invoke(
                "static",
                (self.rt_desc, "setSpinner", "V", (
                    "Landroid/widget/Spinner;",
                    "Landroid/content/Context;",
                    "Ljava/lang/String;",
                )),
                [2, this, 3],
            )
            raw = (props or {}).get("Selection", 0)
            try:
                selection = int(raw)
            except (TypeError, ValueError):
                selection = 0
            asm.const(4, selection)
            asm.invoke(
                "virtual",
                ("Landroid/widget/AdapterView;", "setSelection", "V", ("I",)),
                [2, 4],
            )
        if kind == "Panel":
            # LinearLayout.VERTICAL is 1. HORIZONTAL is 0.
            asm.const(4, 0 if _horizontal(comp) else 1)
            asm.invoke(
                "virtual",
                ("Landroid/widget/LinearLayout;", "setOrientation", "V", ("I",)),
                [2, 4],
            )
        parented = bool(_parent_name(comp))
        params = (
            "Landroid/widget/LinearLayout$LayoutParams;"
            if parented
            else "Landroid/widget/FrameLayout$LayoutParams;"
        )
        self.emit_dp(asm, this, 4, int(comp["width"]))
        self.emit_dp(asm, this, 5, int(comp["height"]))
        asm.new_instance(3, params)
        asm.invoke("direct", (params, "<init>", "V", ("I", "I")), [3, 4, 5])
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
        handler = (comp.get("events") or {}).get("OnItemClick")
        if handler:
            ldesc = class_desc(self.package, form["name"] + "$Item$" + comp["name"])
            asm.new_instance(3, ldesc)
            asm.invoke("direct", (ldesc, "<init>", "V", (desc,)), [3, this])
            asm.invoke(
                "virtual",
                (
                    "Landroid/widget/AdapterView;",
                    "setOnItemClickListener",
                    "V",
                    ("Landroid/widget/AdapterView$OnItemClickListener;",),
                ),
                [2, 3],
            )
        handler = (comp.get("events") or {}).get("OnLongClick")
        if handler:
            ldesc = class_desc(self.package, form["name"] + "$Long$" + comp["name"])
            asm.new_instance(3, ldesc)
            asm.invoke("direct", (ldesc, "<init>", "V", (desc,)), [3, this])
            asm.invoke(
                "virtual",
                ("Landroid/view/View;", "setOnLongClickListener", "V", ("Landroid/view/View$OnLongClickListener;",)),
                [2, 3],
            )
        handler = (comp.get("events") or {}).get("OnItemLongClick")
        if handler:
            ldesc = class_desc(self.package, form["name"] + "$ItemLong$" + comp["name"])
            asm.new_instance(3, ldesc)
            asm.invoke("direct", (ldesc, "<init>", "V", (desc,)), [3, this])
            asm.invoke(
                "virtual",
                (
                    "Landroid/widget/AdapterView;",
                    "setOnItemLongClickListener",
                    "V",
                    ("Landroid/widget/AdapterView$OnItemLongClickListener;",),
                ),
                [2, 3],
            )
        handler = (comp.get("events") or {}).get("OnItemSelected")
        if handler:
            ldesc = class_desc(self.package, form["name"] + "$Select$" + comp["name"])
            asm.new_instance(3, ldesc)
            asm.invoke("direct", (ldesc, "<init>", "V", (desc,)), [3, this])
            asm.invoke(
                "virtual",
                (
                    "Landroid/widget/AdapterView;",
                    "setOnItemSelectedListener",
                    "V",
                    ("Landroid/widget/AdapterView$OnItemSelectedListener;",),
                ),
                [2, 3],
            )
        if (comp.get("events") or {}).get("OnChange"):
            if kind == "Date":
                self.wire_date(asm, form, desc, this, comp["name"], props)
            else:
                self.wire_change(asm, form, desc, this, kind, comp["name"])
        if not parented:
            asm.invoke("virtual", ("Landroid/view/ViewGroup;", "addView", "V", ("Landroid/view/View;",)), [1, 2])
        asm.iput_obj(2, this, (desc, comp["name"], wdesc))

    def place_children(self, asm, desc, this, comps):
        # Attached after every widget field exists, so a child may be listed first.
        for comp in comps:
            parent_name = _parent_name(comp)
            if not parent_name:
                continue
            child_name = comp["name"]
            child_desc = WIDGETS[comp["type"]]["desc"]
            parent = next(
                (other for other in comps if (other.get("name") or "").lower() == parent_name.lower()),
                None,
            )
            if parent is None:
                raise CompileError(f"{child_name} parent {parent_name} is unknown")
            parent_desc = WIDGETS[parent["type"]]["desc"]
            asm.iget_obj(3, this, (desc, parent["name"], parent_desc))
            asm.iget_obj(2, this, (desc, child_name, child_desc))
            asm.invoke(
                "virtual",
                ("Landroid/view/ViewGroup;", "addView", "V", ("Landroid/view/View;",)),
                [3, 2],
            )

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

    def item_listener(self, form, form_desc, widget, handler):
        lname = form["name"] + "$Item$" + widget["name"]
        desc = class_desc(self.package, lname)
        cls = ClassModel(
            desc,
            "Ljava/lang/Object;",
            ACC_PUBLIC,
            interfaces=["Landroid/widget/AdapterView$OnItemClickListener;"],
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
        # registers 8, ins 6. v2 this, v3 parent, v4 row, v5 position, v6-v7 the long id.
        click.iget_obj(0, 2, (desc, "this$0", form_desc))
        key = self.handler_key(form_desc, rec)
        kind = rec["params"][0] if rec["params"] else None
        if kind is None:
            click.invoke("virtual", key, [0])
            outs = 1
        elif kind == "Integer":
            click.invoke("virtual", key, [0, 5])
            outs = 2
        elif kind == "View":
            click.invoke("virtual", key, [0, 4])
            outs = 2
        elif kind == "String":
            click.invoke(
                "virtual",
                ("Landroid/widget/AdapterView;", "getItemAtPosition", "Ljava/lang/Object;", ("I",)),
                [3, 5],
            )
            click.move_result(1, True)
            click.invoke("virtual", ("Ljava/lang/Object;", "toString", "Ljava/lang/String;", ()), [1])
            click.move_result(1, True)
            click.invoke("virtual", key, [0, 1])
            outs = 2
        else:
            raise CompileError(f"OnItemClick parameter {kind} is not supported")
        click.ret(0, "void")
        cls.method(MethodModel(
            "onItemClick",
            "V",
            ("Landroid/widget/AdapterView;", "Landroid/view/View;", "I", "J"),
            ACC_PUBLIC,
            click,
            8,
            6,
            outs,
        ))
        return cls

    def select_listener(self, form, form_desc, widget, handler):
        lname = form["name"] + "$Select$" + widget["name"]
        desc = class_desc(self.package, lname)
        cls = ClassModel(
            desc,
            "Ljava/lang/Object;",
            ACC_PUBLIC,
            interfaces=["Landroid/widget/AdapterView$OnItemSelectedListener;"],
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
        # registers 8, ins 6. v2 this, v3 parent, v4 row, v5 position, v6-v7 the long id.
        click.iget_obj(0, 2, (desc, "this$0", form_desc))
        key = self.handler_key(form_desc, rec)
        kind = rec["params"][0] if rec["params"] else None
        if kind is None:
            click.invoke("virtual", key, [0])
            outs = 1
        elif kind == "Integer":
            click.invoke("virtual", key, [0, 5])
            outs = 2
        elif kind == "View":
            click.invoke("virtual", key, [0, 4])
            outs = 2
        elif kind == "String":
            click.invoke(
                "virtual",
                ("Landroid/widget/AdapterView;", "getItemAtPosition", "Ljava/lang/Object;", ("I",)),
                [3, 5],
            )
            click.move_result(1, True)
            click.invoke("virtual", ("Ljava/lang/Object;", "toString", "Ljava/lang/String;", ()), [1])
            click.move_result(1, True)
            click.invoke("virtual", key, [0, 1])
            outs = 2
        else:
            raise CompileError(f"OnItemSelected parameter {kind} is not supported")
        click.ret(0, "void")
        cls.method(MethodModel(
            "onItemSelected",
            "V",
            ("Landroid/widget/AdapterView;", "Landroid/view/View;", "I", "J"),
            ACC_PUBLIC,
            click,
            8,
            6,
            outs,
        ))
        _empty_method(cls, "onNothingSelected", ("Landroid/widget/AdapterView;",), 2, 2)
        return cls

    def listener_shell(self, form_name, suffix, widget, form_desc, iface):
        lname = form_name + "$" + suffix + "$" + widget
        desc = class_desc(self.package, lname)
        cls = ClassModel(
            desc,
            "Ljava/lang/Object;",
            ACC_PUBLIC,
            interfaces=[iface],
            source=form_name + ".droid",
        )
        cls.field("this$0", form_desc, ACC_PUBLIC)
        init = Asm()
        init.invoke("direct", ("Ljava/lang/Object;", "<init>", "V", ()), [0])
        init.iput_obj(1, 0, (desc, "this$0", form_desc))
        init.ret(0, "void")
        cls.method(MethodModel("<init>", "V", (form_desc,), ACC_PUBLIC | ACC_CONSTRUCTOR, init, 2, 2, 1))
        return desc, cls

    def wire_change(self, asm, form, form_desc, this, kind, comp_name):
        hook = _change_hook(kind)
        if hook is None:
            raise CompileError(f"{kind} has no OnChange event")
        owner, name, listener = hook
        ldesc = class_desc(self.package, form["name"] + "$Change$" + comp_name)
        asm.new_instance(3, ldesc)
        asm.invoke("direct", (ldesc, "<init>", "V", (form_desc,)), [3, this])
        asm.invoke("virtual", (owner, name, "V", (listener,)), [2, 3])

    def wire_date(self, asm, form, form_desc, this, comp_name, props):
        ldesc = class_desc(self.package, form["name"] + "$Change$" + comp_name)
        asm.new_instance(7, ldesc)
        asm.invoke("direct", (ldesc, "<init>", "V", (form_desc,)), [7, this])
        defaults = WIDGETS["Date"]["defaults"]["props"]
        year = int(props.get("Year", defaults["Year"]))
        month = int(props.get("Month", defaults["Month"]))
        day = int(props.get("Day", defaults["Day"]))
        asm.const(3, year)
        asm.const(4, month - 1)
        asm.const(5, day)
        asm.invoke(
            "virtual",
            (
                "Landroid/widget/DatePicker;",
                "init",
                "V",
                ("I", "I", "I", "Landroid/widget/DatePicker$OnDateChangedListener;"),
            ),
            [2, 3, 4, 5, 7],
        )

    def long_listener(self, form, form_desc, widget, handler):
        desc, cls = self.listener_shell(
            form["name"], "Long", widget["name"], form_desc, "Landroid/view/View$OnLongClickListener;"
        )
        rec = form["routines"][handler.lower()]
        click = Asm()
        click.iget_obj(0, 1, (desc, "this$0", form_desc))
        key = self.handler_key(form_desc, rec)
        if rec["params"]:
            click.invoke("virtual", key, [0, 2])
            outs = 2
        else:
            click.invoke("virtual", key, [0])
            outs = 1
        click.const(0, 1)
        click.ret(0, "int")
        cls.method(MethodModel("onLongClick", "Z", ("Landroid/view/View;",), ACC_PUBLIC, click, 3, 2, outs))
        return cls

    def item_long_listener(self, form, form_desc, widget, handler):
        desc, cls = self.listener_shell(
            form["name"],
            "ItemLong",
            widget["name"],
            form_desc,
            "Landroid/widget/AdapterView$OnItemLongClickListener;",
        )
        rec = form["routines"][handler.lower()]
        click = Asm()
        click.iget_obj(0, 2, (desc, "this$0", form_desc))
        key = self.handler_key(form_desc, rec)
        kind = rec["params"][0] if rec["params"] else None
        if kind is None:
            click.invoke("virtual", key, [0])
            outs = 1
        elif kind == "Integer":
            click.invoke("virtual", key, [0, 5])
            outs = 2
        elif kind == "View":
            click.invoke("virtual", key, [0, 4])
            outs = 2
        elif kind == "String":
            click.invoke(
                "virtual",
                ("Landroid/widget/AdapterView;", "getItemAtPosition", "Ljava/lang/Object;", ("I",)),
                [3, 5],
            )
            click.move_result(1, True)
            click.invoke("virtual", ("Ljava/lang/Object;", "toString", "Ljava/lang/String;", ()), [1])
            click.move_result(1, True)
            click.invoke("virtual", key, [0, 1])
            outs = 2
        else:
            raise CompileError(f"OnItemLongClick parameter {kind} is not supported")
        click.const(0, 1)
        click.ret(0, "int")
        cls.method(MethodModel(
            "onItemLongClick",
            "Z",
            ("Landroid/widget/AdapterView;", "Landroid/view/View;", "I", "J"),
            ACC_PUBLIC,
            click,
            8,
            6,
            outs,
        ))
        return cls

    def ask_listener(self, form, form_desc, handler):
        lname = form["name"] + "$Ask$" + handler
        desc = class_desc(self.package, lname)
        cls = ClassModel(
            desc,
            "Ljava/lang/Object;",
            ACC_PUBLIC,
            interfaces=["Landroid/content/DialogInterface$OnClickListener;"],
            source=form["name"] + ".droid",
        )
        cls.field("this$0", form_desc, ACC_PUBLIC)
        init = Asm()
        init.invoke("direct", ("Ljava/lang/Object;", "<init>", "V", ()), [0])
        init.iput_obj(1, 0, (desc, "this$0", form_desc))
        init.ret(0, "void")
        cls.method(MethodModel("<init>", "V", (form_desc,), ACC_PUBLIC | ACC_CONSTRUCTOR, init, 2, 2, 1))
        rec = form["routines"].get(handler.lower())
        if rec is None:
            raise CompileError(f"{handler} is not declared")
        if rec["ret"]:
            raise CompileError(f"{handler} must be a procedure")
        if rec["params"]:
            raise CompileError(f"{handler} takes no parameters")
        click = Asm()
        # v1 listener, v2 dialog, v3 which. v0 is the form. registers 4, ins 3.
        click.iget_obj(0, 1, (desc, "this$0", form_desc))
        click.invoke("virtual", self.handler_key(form_desc, rec), [0])
        click.ret(0, "void")
        cls.method(MethodModel(
            "onClick",
            "V",
            ("Landroid/content/DialogInterface;", "I"),
            ACC_PUBLIC,
            click,
            4,
            3,
            1,
        ))
        return cls

    def change_listener(self, form, form_desc, widget, handler):
        kind = widget["comp"]["type"]
        rec = form["routines"][handler.lower()]
        desc, cls = self.listener_shell(
            form["name"], "Change", widget["name"], form_desc, _change_iface(kind)
        )
        key = self.handler_key(form_desc, rec)
        if kind in ("Edit", "Memo", "Number", "Password", "Email", "Phone"):
            _emit_text_watcher(cls, desc, form_desc, rec, key)
        elif kind in ("Check", "Radio", "Switch", "Toggle"):
            _emit_checked(cls, desc, form_desc, rec, key)
        elif kind == "Seek":
            _emit_seek(cls, desc, form_desc, rec, key)
        elif kind == "Rating":
            _emit_rating(cls, desc, form_desc, rec, key)
        elif kind == "Picker":
            _emit_picker(cls, desc, form_desc, rec, key)
        elif kind == "Time":
            _emit_time(cls, desc, form_desc, rec, key)
        elif kind == "Date":
            _emit_date_changed(cls, desc, form_desc, rec, key)
        else:
            raise CompileError(f"{kind} has no OnChange event")
        return cls


def _ask_handlers(form):
    seen = []

    def add(handler):
        if handler and handler not in seen:
            seen.append(handler)

    def note(node):
        resolved = getattr(node, "resolved", None)
        if not resolved or resolved[0] != "builtin" or resolved[1] != "Question":
            return
        add(getattr(node, "form_name", "") or "")

    def walk_expr(expr):
        if isinstance(expr, ast.CallExpr):
            note(expr)
            for arg in expr.args:
                walk_expr(arg)
        elif isinstance(expr, ast.Unary):
            walk_expr(expr.expr)
        elif isinstance(expr, ast.Binary):
            walk_expr(expr.left)
            walk_expr(expr.right)

    def walk_stmt(stmt):
        if isinstance(stmt, ast.Block):
            for inner in stmt.stmts:
                walk_stmt(inner)
        elif isinstance(stmt, ast.IfStmt):
            walk_expr(stmt.cond)
            walk_stmt(stmt.then)
            walk_stmt(stmt.else_)
        elif isinstance(stmt, ast.WhileStmt):
            walk_expr(stmt.cond)
            walk_stmt(stmt.body)
        elif isinstance(stmt, ast.Assign):
            walk_expr(stmt.expr)
        elif isinstance(stmt, ast.CallStmt):
            note(stmt)
            for arg in stmt.args:
                walk_expr(arg)

    for rec in form["routines"].values():
        for stmt in rec["routine"].body:
            walk_stmt(stmt)
    return seen


def _canon(name):
    return {"integer": "Integer", "string": "String", "boolean": "Boolean", "view": "View"}.get(name.lower(), name)


def _change_hook(kind):
    if kind in ("Edit", "Memo", "Number", "Password", "Email", "Phone"):
        return (
            "Landroid/widget/TextView;",
            "addTextChangedListener",
            "Landroid/text/TextWatcher;",
        )
    if kind in ("Check", "Radio", "Switch", "Toggle"):
        return (
            "Landroid/widget/CompoundButton;",
            "setOnCheckedChangeListener",
            "Landroid/widget/CompoundButton$OnCheckedChangeListener;",
        )
    if kind == "Seek":
        return (
            "Landroid/widget/SeekBar;",
            "setOnSeekBarChangeListener",
            "Landroid/widget/SeekBar$OnSeekBarChangeListener;",
        )
    if kind == "Rating":
        return (
            "Landroid/widget/RatingBar;",
            "setOnRatingBarChangeListener",
            "Landroid/widget/RatingBar$OnRatingBarChangeListener;",
        )
    if kind == "Picker":
        return (
            "Landroid/widget/NumberPicker;",
            "setOnValueChangedListener",
            "Landroid/widget/NumberPicker$OnValueChangeListener;",
        )
    if kind == "Time":
        return (
            "Landroid/widget/TimePicker;",
            "setOnTimeChangedListener",
            "Landroid/widget/TimePicker$OnTimeChangedListener;",
        )
    return None


def _change_iface(kind):
    hook = _change_hook(kind)
    if hook is not None:
        return hook[2]
    if kind == "Date":
        return "Landroid/widget/DatePicker$OnDateChangedListener;"
    return "Ljava/lang/Object;"


def _empty_method(cls, name, params, registers, ins):
    asm = Asm()
    asm.ret(0, "void")
    cls.method(MethodModel(name, "V", params, ACC_PUBLIC, asm, registers, ins, 1))


def _emit_text_watcher(cls, desc, form_desc, rec, key):
    _empty_method(cls, "beforeTextChanged", ("Ljava/lang/CharSequence;", "I", "I", "I"), 5, 5)
    _empty_method(cls, "onTextChanged", ("Ljava/lang/CharSequence;", "I", "I", "I"), 5, 5)
    asm = Asm()
    # registers 4, ins 2. v2 is this, v3 is the Editable, v0 is the form, v1 is the text.
    asm.iget_obj(0, 2, (desc, "this$0", form_desc))
    params = rec["params"]
    if not params:
        asm.invoke("virtual", key, [0])
        outs = 1
    elif params[0] == "String":
        asm.if_zero("eqz", 3, "text_blank")
        asm.invoke("virtual", ("Ljava/lang/Object;", "toString", "Ljava/lang/String;", ()), [3])
        asm.move_result(1, True)
        asm.goto("text_call")
        asm.label("text_blank")
        asm.const_string(1, "")
        asm.label("text_call")
        asm.invoke("virtual", key, [0, 1])
        outs = 2
    else:
        raise CompileError(f"OnChange parameter {params[0]} is not supported")
    asm.ret(0, "void")
    cls.method(MethodModel(
        "afterTextChanged", "V", ("Landroid/text/Editable;",), ACC_PUBLIC, asm, 4, 2, outs
    ))


def _emit_checked(cls, desc, form_desc, rec, key):
    asm = Asm()
    asm.iget_obj(0, 1, (desc, "this$0", form_desc))
    params = rec["params"]
    if not params:
        asm.invoke("virtual", key, [0])
        outs = 1
    elif params[0] == "Boolean":
        asm.invoke("virtual", key, [0, 3])
        outs = 2
    else:
        raise CompileError(f"OnChange parameter {params[0]} is not supported")
    asm.ret(0, "void")
    cls.method(MethodModel(
        "onCheckedChanged",
        "V",
        ("Landroid/widget/CompoundButton;", "Z"),
        ACC_PUBLIC,
        asm,
        4,
        3,
        outs,
    ))


def _emit_seek(cls, desc, form_desc, rec, key):
    _empty_method(cls, "onStartTrackingTouch", ("Landroid/widget/SeekBar;",), 2, 2)
    _empty_method(cls, "onStopTrackingTouch", ("Landroid/widget/SeekBar;",), 2, 2)
    asm = Asm()
    asm.iget_obj(0, 1, (desc, "this$0", form_desc))
    params = rec["params"]
    if not params:
        asm.invoke("virtual", key, [0])
        outs = 1
    elif params[0] == "Integer":
        asm.invoke("virtual", key, [0, 3])
        outs = 2
    else:
        raise CompileError(f"OnChange parameter {params[0]} is not supported")
    asm.ret(0, "void")
    cls.method(MethodModel(
        "onProgressChanged",
        "V",
        ("Landroid/widget/SeekBar;", "I", "Z"),
        ACC_PUBLIC,
        asm,
        5,
        4,
        outs,
    ))


def _emit_rating(cls, desc, form_desc, rec, key):
    asm = Asm()
    asm.iget_obj(0, 1, (desc, "this$0", form_desc))
    params = rec["params"]
    if not params:
        asm.invoke("virtual", key, [0])
        outs = 1
    elif params[0] == "Integer":
        asm.unop(0x87, 3, 3, "float-to-int")
        asm.invoke("virtual", key, [0, 3])
        outs = 2
    else:
        raise CompileError(f"OnChange parameter {params[0]} is not supported")
    asm.ret(0, "void")
    cls.method(MethodModel(
        "onRatingChanged",
        "V",
        ("Landroid/widget/RatingBar;", "F", "Z"),
        ACC_PUBLIC,
        asm,
        5,
        4,
        outs,
    ))


def _emit_picker(cls, desc, form_desc, rec, key):
    asm = Asm()
    asm.iget_obj(0, 1, (desc, "this$0", form_desc))
    params = rec["params"]
    if not params:
        asm.invoke("virtual", key, [0])
        outs = 1
    elif params[0] == "Integer":
        asm.invoke("virtual", key, [0, 4])
        outs = 2
    else:
        raise CompileError(f"OnChange parameter {params[0]} is not supported")
    asm.ret(0, "void")
    cls.method(MethodModel(
        "onValueChange",
        "V",
        ("Landroid/widget/NumberPicker;", "I", "I"),
        ACC_PUBLIC,
        asm,
        5,
        4,
        outs,
    ))


def _reject_change_params(rec):
    if rec["params"]:
        raise CompileError(f"{rec['routine'].name} takes no parameters")


def _emit_time(cls, desc, form_desc, rec, key):
    _reject_change_params(rec)
    asm = Asm()
    asm.iget_obj(0, 1, (desc, "this$0", form_desc))
    asm.invoke("virtual", key, [0])
    asm.ret(0, "void")
    cls.method(MethodModel(
        "onTimeChanged",
        "V",
        ("Landroid/widget/TimePicker;", "I", "I"),
        ACC_PUBLIC,
        asm,
        5,
        4,
        1,
    ))


def _emit_date_changed(cls, desc, form_desc, rec, key):
    _reject_change_params(rec)
    asm = Asm()
    asm.iget_obj(0, 1, (desc, "this$0", form_desc))
    asm.invoke("virtual", key, [0])
    asm.ret(0, "void")
    cls.method(MethodModel(
        "onDateChanged",
        "V",
        ("Landroid/widget/DatePicker;", "I", "I", "I"),
        ACC_PUBLIC,
        asm,
        6,
        5,
        1,
    ))


def compile_dex(package, checked):
    return Compiler(package, checked).emit()
