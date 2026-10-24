"""Name resolution, types, and the code-exploration database."""

from . import ast
from .errors import ErrorLog
from .widgets import GETTERS, PROP_TYPES, WIDGETS

BUILTINS = {
    "Toast": {"args": ["String"], "ret": None, "where": "form"},
    "Log": {"args": ["String"], "ret": None, "where": "any"},
    "OpenForm": {"args": ["String"], "ret": None, "where": "form"},
    "Finish": {"args": [], "ret": None, "where": "form"},
    "IntToStr": {"args": ["Integer"], "ret": "String", "where": "any"},
    "StrToInt": {"args": ["String"], "ret": "Integer", "where": "any"},
    "Length": {"args": ["String"], "ret": "Integer", "where": "any"},
}


def _low(name):
    return name.lower()


def _canon_type(name):
    key = name.lower()
    return {"integer": "Integer", "string": "String", "boolean": "Boolean", "view": "View"}.get(key, name)


class DB:
    """In-memory cross-reference database, in the spirit of Understand."""

    def __init__(self):
        self.entities = []
        self.refs = []
        self.by_q = {}
        self.cfgs = {}

    def add(self, kind, name, qname, file, line, end_line=None, parent=None, type_name="", extra=None):
        ent = {
            "id": len(self.entities),
            "kind": kind,
            "name": name,
            "qname": qname,
            "file": file,
            "line": line,
            "endLine": end_line if end_line is not None else line,
            "parent": parent,
            "type": type_name,
            "loc": (end_line - line + 1) if end_line else 1,
            "complexity": 1,
            "depth": 0,
            "params": 0,
            "fanIn": 0,
            "fanOut": 0,
        }
        if extra:
            ent.update(extra)
        self.entities.append(ent)
        self.by_q[qname] = ent["id"]
        return ent["id"]

    def ref(self, src, dst, kind, file, line, col):
        if src is None or dst is None:
            return
        self.refs.append({
            "src": src, "dst": dst, "kind": kind,
            "file": file, "line": line, "col": col,
        })

    def finish(self):
        for ref in self.refs:
            if ref["kind"] in ("call", "uses"):
                self.entities[ref["src"]]["fanOut"] += 1
                self.entities[ref["dst"]]["fanIn"] += 1
        return self

    def to_dict(self):
        return {
            "entities": self.entities,
            "refs": self.refs,
            "cfgs": self.cfgs,
        }


def _stmt_list(stmt):
    if stmt is None:
        return []
    if isinstance(stmt, ast.Block):
        return stmt.stmts
    return [stmt]


def _expr_decisions(expr):
    if isinstance(expr, ast.Binary) and expr.op in ("and", "or"):
        return 1 + _expr_decisions(expr.left) + _expr_decisions(expr.right)
    if isinstance(expr, ast.Binary):
        return _expr_decisions(expr.left) + _expr_decisions(expr.right)
    if isinstance(expr, ast.Unary):
        return _expr_decisions(expr.expr)
    if isinstance(expr, ast.CallExpr):
        return sum(_expr_decisions(a) for a in expr.args)
    return 0


def _complexity(stmts):
    total = 0
    for stmt in stmts:
        if isinstance(stmt, ast.IfStmt):
            total += 1 + _expr_decisions(stmt.cond)
            total += _complexity(_stmt_list(stmt.then))
            total += _complexity(_stmt_list(stmt.else_))
        elif isinstance(stmt, ast.WhileStmt):
            total += 1 + _expr_decisions(stmt.cond)
            total += _complexity(_stmt_list(stmt.body))
        elif isinstance(stmt, ast.Block):
            total += _complexity(stmt.stmts)
        elif isinstance(stmt, (ast.Assign, ast.CallStmt, ast.ExitStmt)):
            expr = getattr(stmt, "expr", None)
            if expr is not None:
                total += _expr_decisions(expr)
            for arg in getattr(stmt, "args", []):
                total += _expr_decisions(arg)
    return total


def _depth(stmts, base=1):
    best = base
    for stmt in stmts:
        if isinstance(stmt, ast.IfStmt):
            best = max(best, _depth(_stmt_list(stmt.then), base + 1), _depth(_stmt_list(stmt.else_), base + 1))
        elif isinstance(stmt, ast.WhileStmt):
            best = max(best, _depth(_stmt_list(stmt.body), base + 1))
        elif isinstance(stmt, ast.Block):
            best = max(best, _depth(stmt.stmts, base))
    return best


def _fmt(expr):
    if isinstance(expr, ast.Literal):
        if isinstance(expr.value, str):
            return "'" + expr.value.replace("'", "''") + "'"
        if isinstance(expr.value, bool):
            return "true" if expr.value else "false"
        return str(expr.value)
    if isinstance(expr, ast.Ident):
        return expr.name
    if isinstance(expr, ast.Designator):
        return ".".join(expr.parts)
    if isinstance(expr, ast.CallExpr):
        return expr.name + "(" + ", ".join(_fmt(a) for a in expr.args) + ")"
    if isinstance(expr, ast.Unary):
        return expr.op + " " + _fmt(expr.expr)
    if isinstance(expr, ast.Binary):
        return "(" + _fmt(expr.left) + " " + expr.op + " " + _fmt(expr.right) + ")"
    return "?"


def _fmt_stmt(stmt):
    if isinstance(stmt, ast.Assign):
        return _fmt(stmt.target) + " := " + _fmt(stmt.expr)
    if isinstance(stmt, ast.CallStmt):
        return stmt.name + "(" + ", ".join(_fmt(a) for a in stmt.args) + ")"
    if isinstance(stmt, ast.ExitStmt):
        return "Exit"
    if isinstance(stmt, ast.IfStmt):
        return "if " + _fmt(stmt.cond)
    if isinstance(stmt, ast.WhileStmt):
        return "while " + _fmt(stmt.cond)
    if isinstance(stmt, ast.Block):
        return "begin"
    return "stmt"


def build_cfg(routine):
    blocks = []

    def new(label, text, line):
        block = {"id": len(blocks), "label": label, "text": text, "line": line, "succs": []}
        blocks.append(block)
        return block

    exit_b = new("exit", ["exit"], routine.end_line)

    def link(src, dst):
        if src is None or dst is None:
            return
        if dst["id"] not in src["succs"]:
            src["succs"].append(dst["id"])

    def sequence(stmts, cont):
        straight = []
        idx = 0
        while idx < len(stmts) and not isinstance(stmts[idx], (ast.IfStmt, ast.WhileStmt, ast.Block, ast.ExitStmt)):
            straight.append(stmts[idx])
            idx += 1
        if idx == len(stmts):
            if not straight:
                return cont
            block = new("code", [_fmt_stmt(s) for s in straight], straight[0].line)
            link(block, cont)
            return block
        if straight:
            head = new("code", [_fmt_stmt(s) for s in straight], straight[0].line)
            link(head, one(stmts[idx], stmts[idx + 1:], cont))
            return head
        return one(stmts[idx], stmts[idx + 1:], cont)

    def one(stmt, rest, cont):
        after = sequence(rest, cont) if rest else cont
        if isinstance(stmt, ast.Block):
            return sequence(stmt.stmts, after)
        if isinstance(stmt, ast.ExitStmt):
            block = new("exit-jump", ["Exit"], stmt.line)
            link(block, exit_b)
            return block
        if isinstance(stmt, ast.IfStmt):
            cond = new("if", ["if " + _fmt(stmt.cond)], stmt.line)
            then_b = sequence(_stmt_list(stmt.then), after)
            if stmt.else_ is not None:
                else_b = sequence(_stmt_list(stmt.else_), after)
            else:
                else_b = after
            link(cond, then_b)
            link(cond, else_b)
            return cond
        if isinstance(stmt, ast.WhileStmt):
            cond = new("while", ["while " + _fmt(stmt.cond)], stmt.line)
            body = sequence(_stmt_list(stmt.body), cond)
            link(cond, body)
            link(cond, after)
            return cond
        block = new("code", [_fmt_stmt(stmt)], stmt.line)
        link(block, after)
        return block

    entry = sequence(routine.body, exit_b)
    if entry is None:
        entry = exit_b
    return {"entry": entry["id"], "blocks": blocks}


def _n_complexity(stmts):
    total = 0
    for stmt in stmts:
        if isinstance(stmt, ast.NIf):
            total += 1 + _n_complexity(stmt.then) + _n_complexity(stmt.else_)
        elif isinstance(stmt, ast.NWhile):
            total += 1 + _n_complexity(stmt.body)
    return total


def _n_depth(stmts, base=1):
    best = base
    for stmt in stmts:
        if isinstance(stmt, ast.NIf):
            best = max(best, _n_depth(stmt.then, base + 1), _n_depth(stmt.else_, base + 1))
        elif isinstance(stmt, ast.NWhile):
            best = max(best, _n_depth(stmt.body, base + 1))
    return best


def _nfmt(expr):
    if expr is None:
        return ""
    if isinstance(expr, ast.NLit):
        return str(expr.value)
    if isinstance(expr, ast.NIdent):
        return expr.name
    if isinstance(expr, ast.NUnary):
        return expr.op + _nfmt(expr.expr)
    if isinstance(expr, ast.NBinary):
        return "(" + _nfmt(expr.left) + " " + expr.op + " " + _nfmt(expr.right) + ")"
    if isinstance(expr, ast.NCall):
        return expr.name + "(" + ", ".join(_nfmt(arg) for arg in expr.args) + ")"
    return "?"


def _nfmt_stmt(stmt):
    if isinstance(stmt, ast.NVar):
        return "var " + stmt.name + " = " + _nfmt(stmt.expr)
    if isinstance(stmt, ast.NAssign):
        return stmt.name + " = " + _nfmt(stmt.expr)
    if isinstance(stmt, ast.NReturn):
        text = _nfmt(stmt.expr)
        return "return " + text if text else "return"
    if isinstance(stmt, ast.NIf):
        return "if " + _nfmt(stmt.cond)
    if isinstance(stmt, ast.NWhile):
        return "while " + _nfmt(stmt.cond)
    return type(stmt).__name__


def build_native_cfg(fun):
    blocks = []

    def new(label, text, line):
        block = {"id": len(blocks), "label": label, "text": text, "line": line, "succs": []}
        blocks.append(block)
        return block

    exit_b = new("exit", ["return"], fun.end_line)

    def link(src, dst):
        if src is None or dst is None:
            return
        if dst["id"] not in src["succs"]:
            src["succs"].append(dst["id"])

    def sequence(stmts, cont):
        straight = []
        idx = 0
        while idx < len(stmts) and isinstance(stmts[idx], (ast.NVar, ast.NAssign)):
            straight.append(stmts[idx])
            idx += 1
        if idx == len(stmts):
            if not straight:
                return cont
            block = new("code", [_nfmt_stmt(stmt) for stmt in straight], straight[0].line)
            link(block, cont)
            return block
        if straight:
            head = new("code", [_nfmt_stmt(stmt) for stmt in straight], straight[0].line)
            link(head, one(stmts[idx], stmts[idx + 1:], cont))
            return head
        return one(stmts[idx], stmts[idx + 1:], cont)

    def one(stmt, rest, cont):
        after = sequence(rest, cont) if rest else cont
        if isinstance(stmt, ast.NBlock):
            return sequence(stmt.stmts, after)
        if isinstance(stmt, ast.NReturn):
            block = new("return", [_nfmt_stmt(stmt)], stmt.line)
            link(block, exit_b)
            return block
        if isinstance(stmt, ast.NIf):
            cond = new("if", ["if " + _nfmt(stmt.cond)], stmt.line)
            link(cond, sequence(stmt.then, after))
            link(cond, sequence(stmt.else_, after) if stmt.else_ else after)
            return cond
        if isinstance(stmt, ast.NWhile):
            cond = new("while", ["while " + _nfmt(stmt.cond)], stmt.line)
            link(cond, sequence(stmt.body, cond))
            link(cond, after)
            return cond
        block = new("code", [_nfmt_stmt(stmt)], stmt.line)
        link(block, after)
        return block

    entry = sequence(fun.body, exit_b)
    return {"entry": entry["id"] if entry else exit_b["id"], "blocks": blocks}


class Checker:
    def __init__(self, model):
        self.model = model
        self.log = ErrorLog()
        self.db = DB()
        self.forms = {}
        self.units = {}
        self.natives = {}
        self.routine_index = {}  # (unit_lower, name_lower) -> (entity_id, routine, ret, param_types, is_static)
        self.builtin_ids = {}

    def error(self, message, node=None, file="", line=1, col=1):
        if node is not None:
            line, col = node.line, getattr(node, "col", 1)
        self.log.add(message, line, col, file)

    def run(self):
        self._parse_natives()
        self._collect_signatures()
        self._check_bodies()
        self.db.finish()
        return {
            "errors": self.log.to_list(),
            "forms": self.forms,
            "units": self.units,
            "natives": self.natives,
            "db": self.db,
        }

    def _parse_natives(self):
        file_ids = {}
        for path, info in self.model["natives"].items():
            program = info["program"]
            fid = self.db.add("native-file", path, "native:" + path, info["file"], 1, 1)
            file_ids[path] = fid
            for fun in program.funs:
                key = _low(fun.name)
                if key in self.natives:
                    self.error(f"duplicate native function {fun.name}", fun, info["file"])
                    continue
                if "_" in fun.name:
                    # allowed; JNI encoding escapes underscores
                    pass
                if len(fun.params) > 4:
                    self.error("native functions support at most 4 parameters", fun, info["file"])
                qn = "native." + fun.name
                eid = self.db.add(
                    "native", fun.name, qn, info["file"], fun.line, fun.end_line, fid, "i32",
                    {"params": len(fun.params), "complexity": 1 + _n_complexity(fun.body), "depth": _n_depth(fun.body)},
                )
                self.db.cfgs[str(eid)] = build_native_cfg(fun)
                self.natives[key] = {
                    "name": fun.name, "params": list(fun.params), "fun": fun,
                    "file": info["file"], "id": eid, "path": path,
                }
                for index, pname in enumerate(fun.params):
                    self.db.add("param", pname, qn + "." + pname, info["file"], fun.line, fun.line, eid, "i32")
                self._scan_native_body(fun, eid, info["file"])

    def _scan_native_body(self, fun, eid, file):
        def walk(stmts):
            for stmt in stmts:
                if isinstance(stmt, ast.NCall):
                    target = self.natives.get(_low(stmt.name))
                    if target:
                        self.db.ref(eid, target["id"], "call", file, stmt.line, stmt.col)
                elif isinstance(stmt, ast.NIf):
                    walk(stmt.then)
                    walk(stmt.else_)
                elif isinstance(stmt, ast.NWhile):
                    walk(stmt.body)
                elif isinstance(stmt, ast.NReturn):
                    self._scan_native_expr(stmt.expr, eid, file)
                else:
                    self._scan_native_expr(getattr(stmt, "expr", None), eid, file)
        walk(fun.body)

    def _scan_native_expr(self, expr, eid, file):
        if expr is None:
            return
        if isinstance(expr, ast.NCall):
            target = self.natives.get(_low(expr.name))
            if target:
                self.db.ref(eid, target["id"], "call", file, expr.line, expr.col)
            for arg in expr.args:
                self._scan_native_expr(arg, eid, file)
        elif isinstance(expr, ast.NBinary):
            self._scan_native_expr(expr.left, eid, file)
            self._scan_native_expr(expr.right, eid, file)
        elif isinstance(expr, ast.NUnary):
            self._scan_native_expr(expr.expr, eid, file)

    def _collect_signatures(self):
        for name, info in self.model["forms"].items():
            program = info["program"]
            form = info["form"]
            fid = self.db.add("form", name, name, info["file"], 1, 1, type_name="Activity")
            widgets = {}
            for comp in form.get("components", []):
                cname = comp["name"]
                if _low(cname) in widgets:
                    self.error(f"duplicate component {cname}", file=info["file"])
                if comp["type"] not in WIDGETS:
                    self.error(f"unknown component type {comp['type']}", file=info["file"])
                    continue
                qn = name + "." + cname
                cid = self.db.add("widget", cname, qn, info["file"], 1, 1, fid, comp["type"])
                widgets[_low(cname)] = {"comp": comp, "id": cid, "name": cname}
            routines = {}
            for routine in program.routines:
                key = _low(routine.name)
                if key in routines or key in widgets:
                    self.error(f"duplicate name {routine.name}", routine, info["file"])
                    continue
                if len(routine.params) > 4:
                    self.error(f"{routine.name} has too many parameters (max 4 on a form)", routine, info["file"])
                qn = name + "." + routine.name
                ret = _canon_type(routine.ret) if routine.ret else ""
                eid = self.db.add(
                    "function" if routine.kind == "function" else "procedure",
                    routine.name, qn, info["file"], routine.line, routine.end_line, fid, ret,
                    {
                        "params": len(routine.params),
                        "complexity": 1 + _complexity(routine.body),
                        "depth": _depth(routine.body),
                        "loc": routine.end_line - routine.line + 1,
                    },
                )
                self.db.cfgs[str(eid)] = build_cfg(routine)
                param_types = []
                for param in routine.params:
                    ptype = _canon_type(param.type_name)
                    param_types.append(ptype)
                    self.db.add("param", param.name, qn + "." + param.name, info["file"], param.line, param.line, eid, ptype)
                for local in routine.locals:
                    self.db.add("local", local.name, qn + "." + local.name, info["file"], local.line, local.line, eid, _canon_type(local.type_name))
                rec = {
                    "routine": routine, "id": eid, "ret": routine.ret and _canon_type(routine.ret),
                    "params": param_types, "static": False, "unit": name,
                }
                routines[key] = rec
                self.routine_index[(name.lower(), key)] = rec
            for used in program.uses:
                pass
            self.forms[name] = {
                "name": name,
                "file": info["file"],
                "form": form,
                "program": program,
                "widgets": widgets,
                "routines": routines,
                "id": fid,
            }
        for name, info in self.model["units"].items():
            program = info["program"]
            declared = program.unit or name
            if program.unit and program.unit.lower() != name.lower():
                self.error(f"unit name {program.unit} does not match file {name}", file=info["file"])
            fid = self.db.add("unit", declared, declared, info["file"], 1, 1)
            routines = {}
            for routine in program.routines:
                key = _low(routine.name)
                if key in routines:
                    self.error(f"duplicate routine {routine.name}", routine, info["file"])
                    continue
                if len(routine.params) > 5:
                    self.error(f"{routine.name} has too many parameters (max 5 in a unit)", routine, info["file"])
                qn = declared + "." + routine.name
                ret = _canon_type(routine.ret) if routine.ret else ""
                eid = self.db.add(
                    "function" if routine.kind == "function" else "procedure",
                    routine.name, qn, info["file"], routine.line, routine.end_line, fid, ret,
                    {
                        "params": len(routine.params),
                        "complexity": 1 + _complexity(routine.body),
                        "depth": _depth(routine.body),
                        "loc": routine.end_line - routine.line + 1,
                    },
                )
                self.db.cfgs[str(eid)] = build_cfg(routine)
                param_types = []
                for param in routine.params:
                    ptype = _canon_type(param.type_name)
                    param_types.append(ptype)
                    self.db.add("param", param.name, qn + "." + param.name, info["file"], param.line, param.line, eid, ptype)
                for local in routine.locals:
                    self.db.add("local", local.name, qn + "." + local.name, info["file"], local.line, local.line, eid, _canon_type(local.type_name))
                rec = {
                    "routine": routine, "id": eid, "ret": routine.ret and _canon_type(routine.ret),
                    "params": param_types, "static": True, "unit": declared,
                }
                routines[key] = rec
                self.routine_index[(declared.lower(), key)] = rec
            self.units[name] = {
                "name": declared,
                "file": info["file"],
                "program": program,
                "routines": routines,
                "id": fid,
            }
        # uses edges
        for name, info in list(self.forms.items()) + [(n, self.units[n]) for n in self.units]:
            program = info["program"] if "program" in info else self.units[name]["program"]
            src = info["id"]
            file = info["file"]
            for used in program.uses:
                target = self._unit_by_name(used)
                if target is None:
                    self.error(f"unknown unit {used}", file=file)
                else:
                    self.db.ref(src, target["id"], "uses", file, 1, 1)

    def _unit_by_name(self, name):
        for unit in self.units.values():
            if unit["name"].lower() == name.lower():
                return unit
        for form in self.forms.values():
            if form["name"].lower() == name.lower():
                return form
        return None

    def _check_bodies(self):
        for form in self.forms.values():
            self._check_events(form)
            for rec in form["routines"].values():
                self._check_routine(rec, form["file"], form=form)
        for unit in self.units.values():
            for rec in unit["routines"].values():
                self._check_routine(rec, unit["file"], form=None)
        for native in self.natives.values():
            self._check_native(native)

    def _check_events(self, form):
        file = form["file"]
        form_json = form["form"]
        handler = (form_json.get("events") or {}).get("OnCreate")
        if handler:
            rec = form["routines"].get(_low(handler))
            if rec is None:
                self.error(f"OnCreate handler {handler} is not declared", file=file)
            elif rec["ret"] is not None:
                self.error("OnCreate handler must be a procedure", rec["routine"], file)
            elif len(rec["params"]) > 1:
                self.error("OnCreate handler takes no parameters, or one View", rec["routine"], file)
            elif len(rec["params"]) == 1 and rec["params"][0] != "View":
                self.error("OnCreate handler parameter must be a View", rec["routine"], file)
        for comp in form_json.get("components", []):
            for event, handler in (comp.get("events") or {}).items():
                if not handler:
                    continue
                if event not in WIDGETS.get(comp["type"], {}).get("events", []):
                    self.error(f"{comp['type']} has no {event} event", file=file)
                rec = form["routines"].get(_low(handler))
                if rec is None:
                    self.error(f"{event} handler {handler} is not declared", file=file)
                elif rec["ret"] is not None:
                    self.error(f"{handler} must be a procedure", rec["routine"], file)
                elif len(rec["params"]) > 1:
                    self.error(f"{handler} takes no parameters, or one View", rec["routine"], file)
                elif len(rec["params"]) == 1 and rec["params"][0] != "View":
                    self.error(f"{handler} parameter must be a View", rec["routine"], file)

    def _builtin(self, name):
        if name not in self.builtin_ids:
            spec = BUILTINS[name]
            self.builtin_ids[name] = self.db.add(
                "builtin", name, "builtin." + name, "", 1, 1, type_name=spec["ret"] or "",
                extra={"params": len(spec["args"])},
            )
        return self.builtin_ids[name]

    def _check_routine(self, rec, file, form):
        routine = rec["routine"]
        scope = {}
        for param in routine.params:
            scope[_low(param.name)] = ("local", _canon_type(param.type_name), param.name)
        for local in routine.locals:
            key = _low(local.name)
            if key in scope:
                self.error(f"duplicate local {local.name}", local, file)
            scope[key] = ("local", _canon_type(local.type_name), local.name)
        if routine.kind == "function":
            scope["result"] = ("result", _canon_type(routine.ret), "Result")
        self._check_stmts(routine.body, scope, rec, file, form)

    def _check_stmts(self, stmts, scope, rec, file, form):
        for stmt in stmts:
            if isinstance(stmt, ast.Block):
                self._check_stmts(stmt.stmts, scope, rec, file, form)
            elif isinstance(stmt, ast.IfStmt):
                ty = self._check_expr(stmt.cond, scope, rec, file, form)
                if ty not in ("Boolean", None):
                    self.error("condition must be Boolean", stmt.cond, file)
                self._check_stmts(_stmt_list(stmt.then), scope, rec, file, form)
                self._check_stmts(_stmt_list(stmt.else_), scope, rec, file, form)
            elif isinstance(stmt, ast.WhileStmt):
                ty = self._check_expr(stmt.cond, scope, rec, file, form)
                if ty not in ("Boolean", None):
                    self.error("condition must be Boolean", stmt.cond, file)
                self._check_stmts(_stmt_list(stmt.body), scope, rec, file, form)
            elif isinstance(stmt, ast.ExitStmt):
                pass
            elif isinstance(stmt, ast.Assign):
                self._check_assign(stmt, scope, rec, file, form)
            elif isinstance(stmt, ast.CallStmt):
                self._check_call(stmt.name, stmt.args, stmt, scope, rec, file, form, as_stmt=True)
            else:
                self.error("unsupported statement", stmt, file)

    def _check_assign(self, stmt, scope, rec, file, form):
        target = stmt.target
        if len(target.parts) == 1:
            key = _low(target.parts[0])
            if key not in scope:
                self.error(f"unknown name {target.parts[0]}", target, file)
                self._check_expr(stmt.expr, scope, rec, file, form)
                return
            kind, ty, _spelling = scope[key]
            if kind == "result" or kind == "local":
                expr_ty = self._check_expr(stmt.expr, scope, rec, file, form)
                if expr_ty and expr_ty != ty:
                    self.error(f"cannot assign {expr_ty} to {ty}", stmt.expr, file)
                stmt.target.resolved = (kind, ty)
                return
            self.error(f"cannot assign to {target.parts[0]}", target, file)
            return
        if len(target.parts) == 2 and form is not None:
            wkey = _low(target.parts[0])
            prop = target.parts[1]
            widget = form["widgets"].get(wkey)
            if widget is None:
                self.error(f"unknown component {target.parts[0]}", target, file)
                return
            spec = WIDGETS[widget["comp"]["type"]]
            # property match is case-insensitive
            actual = next((p for p in spec["props"] if p.lower() == prop.lower()), None)
            if actual is None:
                self.error(f"{widget['comp']['type']} has no property {prop}", target, file)
                return
            expr_ty = self._check_expr(stmt.expr, scope, rec, file, form)
            expect = PROP_TYPES[actual]
            if actual in ("TextColor", "Color") and expr_ty == "Integer":
                pass
            elif expr_ty and expr_ty != expect:
                self.error(f"{actual} expects {expect}", stmt.expr, file)
            stmt.target.resolved = ("prop", actual, widget)
            self.db.ref(rec["id"], widget["id"], "write", file, target.line, target.col)
            return
        self.error("invalid assignment target", target, file)

    def _check_expr(self, expr, scope, rec, file, form):
        if isinstance(expr, ast.Literal):
            expr.ty = expr.type_name
            return expr.ty
        if isinstance(expr, ast.Ident):
            return self._resolve_value(expr.name, expr, scope, rec, file, form)
        if isinstance(expr, ast.Designator):
            return self._resolve_designator(expr, scope, rec, file, form)
        if isinstance(expr, ast.Unary):
            ty = self._check_expr(expr.expr, scope, rec, file, form)
            if expr.op == "not":
                if ty not in ("Boolean", None):
                    self.error("not expects Boolean", expr, file)
                expr.ty = "Boolean"
            elif expr.op == "-":
                if ty not in ("Integer", None):
                    self.error("unary minus expects Integer", expr, file)
                expr.ty = "Integer"
            else:
                expr.ty = ty
            return expr.ty
        if isinstance(expr, ast.Binary):
            lt = self._check_expr(expr.left, scope, rec, file, form)
            rt = self._check_expr(expr.right, scope, rec, file, form)
            op = expr.op
            if op in ("and", "or"):
                if lt not in ("Boolean", None) or rt not in ("Boolean", None):
                    self.error(f"{op} expects Boolean operands", expr, file)
                expr.ty = "Boolean"
            elif op in ("=", "<>", "<", ">", "<=", ">="):
                if lt and rt and lt != rt:
                    self.error(f"cannot compare {lt} and {rt}", expr, file)
                if lt not in ("Integer", "String", "Boolean", None):
                    self.error(f"cannot compare {lt}", expr, file)
                expr.ty = "Boolean"
            elif op == "+":
                if lt == "String" or rt == "String":
                    if lt not in ("String", "Integer", None) or rt not in ("String", "Integer", None):
                        self.error("string concatenation needs strings or integers", expr, file)
                    expr.ty = "String"
                    expr.string_add = True
                else:
                    if lt not in ("Integer", None) or rt not in ("Integer", None):
                        self.error("+ expects integers or strings", expr, file)
                    expr.ty = "Integer"
            elif op in ("-", "*", "/", "div", "mod"):
                if lt not in ("Integer", None) or rt not in ("Integer", None):
                    self.error(f"{op} expects integers", expr, file)
                expr.ty = "Integer"
            else:
                self.error(f"unknown operator {op}", expr, file)
                expr.ty = None
            return expr.ty
        if isinstance(expr, ast.CallExpr):
            return self._check_call(expr.name, expr.args, expr, scope, rec, file, form, as_stmt=False)
        self.error("bad expression", expr, file)
        return None

    def _resolve_value(self, name, node, scope, rec, file, form):
        key = _low(name)
        if key in scope:
            kind, ty, spelling = scope[key]
            node.ty = ty
            node.resolved = (kind, ty, spelling)
            return ty
        if form is not None and key in form["widgets"]:
            widget = form["widgets"][key]
            node.ty = "View"
            node.resolved = ("widget", widget)
            self.db.ref(rec["id"], widget["id"], "read", file, node.line, node.col)
            return "View"
        # zero-arg call written without parens is not supported
        self.error(f"unknown name {name}", node, file)
        node.ty = None
        return None

    def _resolve_designator(self, expr, scope, rec, file, form):
        if form is None or len(expr.parts) != 2:
            self.error("invalid name " + ".".join(expr.parts), expr, file)
            expr.ty = None
            return None
        widget = form["widgets"].get(_low(expr.parts[0]))
        if widget is None:
            self.error(f"unknown component {expr.parts[0]}", expr, file)
            expr.ty = None
            return None
        spec = WIDGETS[widget["comp"]["type"]]
        actual = next((p for p in spec["props"] if p.lower() == expr.parts[1].lower()), None)
        if actual is None or actual not in GETTERS and actual not in PROP_TYPES:
            self.error(f"cannot read {expr.parts[1]}", expr, file)
            expr.ty = None
            return None
        if actual not in GETTERS:
            self.error(f"{actual} cannot be read", expr, file)
            expr.ty = None
            return None
        expr.ty = PROP_TYPES[actual]
        expr.resolved = ("prop", actual, widget)
        self.db.ref(rec["id"], widget["id"], "read", file, expr.line, expr.col)
        return expr.ty

    def _lookup_routine(self, name, rec, form):
        key = _low(name)
        if form is not None and key in form["routines"]:
            return form["routines"][key], False
        # same unit
        unit_name = rec["unit"]
        hit = self.routine_index.get((unit_name.lower(), key))
        if hit is not None:
            return hit, hit["static"]
        # used units — search all units with that routine; prefer ones in the uses list
        program = None
        if form is not None:
            program = form["program"]
        else:
            for unit in self.units.values():
                if unit["name"].lower() == unit_name.lower():
                    program = unit["program"]
        uses = [u.lower() for u in (program.uses if program else [])]
        found = []
        for (uname, rname), candidate in self.routine_index.items():
            if rname == key and uname in uses:
                found.append(candidate)
        if len(found) == 1:
            return found[0], True
        if len(found) > 1:
            return "ambiguous", True
        if key in self.natives:
            return ("native", self.natives[key]), True
        if key in {b.lower(): b for b in BUILTINS}:
            canon = {b.lower(): b for b in BUILTINS}[key]
            return ("builtin", canon), True
        return None, False

    def _check_call(self, name, args, node, scope, rec, file, form, as_stmt):
        target, _static = self._lookup_routine(name, rec, form)
        arg_types = [self._check_expr(a, scope, rec, file, form) for a in args]
        if target is None:
            self.error(f"unknown routine {name}", node, file)
            if not as_stmt:
                node.ty = None
            return None
        if target == "ambiguous":
            self.error(f"{name} is declared in more than one used unit", node, file)
            return None
        if isinstance(target, tuple) and target[0] == "builtin":
            spec = BUILTINS[target[1]]
            if spec["where"] == "form" and form is None:
                self.error(f"{target[1]} is only available on a form", node, file)
            if len(arg_types) != len(spec["args"]):
                self.error(f"{target[1]} expects {len(spec['args'])} arguments", node, file)
            else:
                for got, expect in zip(arg_types, spec["args"]):
                    if got and got != expect:
                        self.error(f"{target[1]} expects {expect}, got {got}", node, file)
            if target[1] == "OpenForm":
                if not args or not isinstance(args[0], ast.Literal) or not isinstance(args[0].value, str):
                    self.error("OpenForm needs a string literal form name", node, file)
                else:
                    match = next((n for n in self.forms if n.lower() == args[0].value.lower()), None)
                    if match is None:
                        self.error(f"unknown form {args[0].value}", args[0], file)
                    else:
                        node.form_name = match
            node.resolved = ("builtin", target[1])
            self.db.ref(rec["id"], self._builtin(target[1]), "call", file, node.line, getattr(node, "col", 1))
            if not as_stmt:
                if spec["ret"] is None:
                    self.error(f"{target[1]} does not return a value", node, file)
                    node.ty = None
                    return None
                node.ty = spec["ret"]
                return spec["ret"]
            return None
        if isinstance(target, tuple) and target[0] == "native":
            native = target[1]
            if len(arg_types) != len(native["params"]):
                self.error(f"{native['name']} expects {len(native['params'])} arguments", node, file)
            for got in arg_types:
                if got and got != "Integer":
                    self.error("native arguments must be Integer", node, file)
            node.resolved = ("native", native)
            self.db.ref(rec["id"], native["id"], "call", file, node.line, getattr(node, "col", 1))
            if as_stmt:
                return None
            node.ty = "Integer"
            return "Integer"
        # user routine
        if not target["static"] and (rec["static"] or target["unit"].lower() != rec["unit"].lower()):
            self.error(f"{name} is an instance routine of {target['unit']}", node, file)
        if len(arg_types) != len(target["params"]):
            self.error(f"{name} expects {len(target['params'])} arguments", node, file)
        else:
            for got, expect in zip(arg_types, target["params"]):
                if got and got != expect and not (expect == "View" and got == "View"):
                    self.error(f"expected {expect}, got {got}", node, file)
        node.resolved = ("routine", target)
        self.db.ref(rec["id"], target["id"], "call", file, node.line, getattr(node, "col", 1))
        if as_stmt:
            if target["ret"] is not None:
                # allowing a call statement to discard a value is friendly
                pass
            return None
        if target["ret"] is None:
            self.error(f"{name} does not return a value", node, file)
            node.ty = None
            return None
        node.ty = target["ret"]
        return target["ret"]

    def _check_native(self, native):
        fun = native["fun"]
        file = native["file"]
        defined = set(fun.params)
        assigned = set(fun.params)

        def check_expr(expr):
            if isinstance(expr, ast.NLit):
                return
            if isinstance(expr, ast.NIdent):
                if expr.name not in defined:
                    self.error(f"unknown name {expr.name}", expr, file)
                return
            if isinstance(expr, ast.NUnary):
                check_expr(expr.expr)
                return
            if isinstance(expr, ast.NBinary):
                check_expr(expr.left)
                check_expr(expr.right)
                return
            if isinstance(expr, ast.NCall):
                target = self.natives.get(_low(expr.name))
                if target is None:
                    self.error(f"unknown native function {expr.name}", expr, file)
                elif len(expr.args) != len(target["params"]):
                    self.error(f"{expr.name} expects {len(target['params'])} arguments", expr, file)
                for arg in expr.args:
                    check_expr(arg)
                return
            self.error("bad native expression", expr, file)

        def check_stmts(stmts):
            for stmt in stmts:
                if isinstance(stmt, ast.NVar):
                    if stmt.name in defined:
                        self.error(f"duplicate variable {stmt.name}", stmt, file)
                    defined.add(stmt.name)
                    if stmt.expr is not None:
                        check_expr(stmt.expr)
                    assigned.add(stmt.name)
                elif isinstance(stmt, ast.NAssign):
                    if stmt.name not in defined:
                        self.error(f"unknown name {stmt.name}", stmt, file)
                    check_expr(stmt.expr)
                elif isinstance(stmt, ast.NReturn):
                    check_expr(stmt.expr)
                elif isinstance(stmt, ast.NIf):
                    check_expr(stmt.cond)
                    check_stmts(stmt.then)
                    check_stmts(stmt.else_)
                elif isinstance(stmt, ast.NWhile):
                    check_expr(stmt.cond)
                    check_stmts(stmt.body)
                elif isinstance(stmt, ast.NCall):
                    check_expr(stmt)
                else:
                    self.error("bad native statement", stmt, file)
        check_stmts(fun.body)


def check_model(model):
    return Checker(model).run()
