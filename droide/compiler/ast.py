"""AST nodes. Fields are plain attributes so the checker can hang types on them."""


class Node:
    def __init__(self, line=1, col=1):
        self.line = line
        self.col = col


class Program(Node):
    def __init__(self, unit, uses, routines, line=1):
        super().__init__(line, 1)
        self.unit = unit
        self.uses = uses
        self.routines = routines


class Routine(Node):
    def __init__(self, kind, name, params, ret, locals_, body, end_line, line, col):
        super().__init__(line, col)
        self.kind = kind
        self.name = name
        self.params = params
        self.ret = ret
        self.locals = locals_
        self.body = body
        self.end_line = end_line


class Param(Node):
    def __init__(self, name, type_name, line, col):
        super().__init__(line, col)
        self.name = name
        self.type_name = type_name


class Block(Node):
    def __init__(self, stmts, line, col):
        super().__init__(line, col)
        self.stmts = stmts


class Assign(Node):
    def __init__(self, target, expr, line, col):
        super().__init__(line, col)
        self.target = target
        self.expr = expr


class CallStmt(Node):
    def __init__(self, name, args, line, col):
        super().__init__(line, col)
        self.name = name
        self.args = args


class IfStmt(Node):
    def __init__(self, cond, then, else_, line, col):
        super().__init__(line, col)
        self.cond = cond
        self.then = then
        self.else_ = else_


class WhileStmt(Node):
    def __init__(self, cond, body, line, col):
        super().__init__(line, col)
        self.cond = cond
        self.body = body


class ExitStmt(Node):
    pass


class Designator(Node):
    def __init__(self, parts, line, col):
        super().__init__(line, col)
        self.parts = parts

    @property
    def name(self):
        return self.parts[0]


class Ident(Node):
    def __init__(self, name, line, col):
        super().__init__(line, col)
        self.name = name


class Literal(Node):
    def __init__(self, value, type_name, line, col):
        super().__init__(line, col)
        self.value = value
        self.type_name = type_name


class Unary(Node):
    def __init__(self, op, expr, line, col):
        super().__init__(line, col)
        self.op = op
        self.expr = expr


class Binary(Node):
    def __init__(self, op, left, right, line, col):
        super().__init__(line, col)
        self.op = op
        self.left = left
        self.right = right


class CallExpr(Node):
    def __init__(self, name, args, line, col):
        super().__init__(line, col)
        self.name = name
        self.args = args


# Native AST

class NProgram(Node):
    def __init__(self, funs):
        super().__init__(1, 1)
        self.funs = funs


class NFun(Node):
    def __init__(self, name, params, ret, body, end_line, line, col):
        super().__init__(line, col)
        self.name = name
        self.params = params
        self.ret = ret
        self.body = body
        self.end_line = end_line


class NVar(Node):
    def __init__(self, name, expr, line, col):
        super().__init__(line, col)
        self.name = name
        self.expr = expr


class NAssign(Node):
    def __init__(self, name, expr, line, col):
        super().__init__(line, col)
        self.name = name
        self.expr = expr


class NReturn(Node):
    def __init__(self, expr, line, col):
        super().__init__(line, col)
        self.expr = expr


class NIf(Node):
    def __init__(self, cond, then, else_, line, col):
        super().__init__(line, col)
        self.cond = cond
        self.then = then
        self.else_ = else_


class NWhile(Node):
    def __init__(self, cond, body, line, col):
        super().__init__(line, col)
        self.cond = cond
        self.body = body


class NBlock(Node):
    def __init__(self, stmts, line, col):
        super().__init__(line, col)
        self.stmts = stmts


class NIdent(Node):
    def __init__(self, name, line, col):
        super().__init__(line, col)
        self.name = name


class NLit(Node):
    def __init__(self, value, line, col):
        super().__init__(line, col)
        self.value = value


class NUnary(Node):
    def __init__(self, op, expr, line, col):
        super().__init__(line, col)
        self.op = op
        self.expr = expr


class NBinary(Node):
    def __init__(self, op, left, right, line, col):
        super().__init__(line, col)
        self.op = op
        self.left = left
        self.right = right


class NCall(Node):
    def __init__(self, name, args, line, col):
        super().__init__(line, col)
        self.name = name
        self.args = args
