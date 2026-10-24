"""Recursive-descent parsers for Droide source and native source."""

from .ast import (
    Assign, Binary, Block, CallExpr, CallStmt, Designator, ExitStmt, Ident,
    IfStmt, Literal, Param, Program, Routine, Unary, WhileStmt,
    NAssign, NBinary, NBlock, NCall, NFun, NIdent, NIf, NLit, NProgram,
    NReturn, NUnary, NVar, NWhile,
)
from .errors import CompileError
from .lexer import Lexer, NLexer


class Parser:
    def __init__(self, text, file=""):
        self.file = file
        self.tokens = Lexer(text, file).tokens()
        self.i = 0

    def _tok(self):
        return self.tokens[self.i]

    def _eat(self, kind):
        tok = self._tok()
        if tok.kind != kind:
            raise CompileError(
                f"expected {kind}, found {tok.kind} {tok.value!r}",
                tok.line, tok.col, self.file,
            )
        self.i += 1
        return tok

    def _accept(self, kind):
        if self._tok().kind == kind:
            tok = self._tok()
            self.i += 1
            return tok
        return None

    def parse(self):
        unit = None
        uses = []
        routines = []
        while self._tok().kind != "eof":
            k = self._tok().kind
            if k == "unit":
                self._eat("unit")
                name = self._eat("ident")
                self._eat("semi")
                unit = name.value
            elif k == "uses":
                self._eat("uses")
                first = self._eat("ident")
                uses = [first.value]
                while self._accept("comma"):
                    uses.append(self._eat("ident").value)
                self._eat("semi")
            elif k in ("procedure", "function"):
                routines.append(self._routine())
            elif k == "semi":
                self._eat("semi")
            else:
                tok = self._tok()
                raise CompileError(
                    f"expected a procedure or function, found {tok.value!r}",
                    tok.line, tok.col, self.file,
                )
        return Program(unit, uses, routines)

    def _routine(self):
        kind_tok = self._tok()
        kind = kind_tok.kind
        self.i += 1
        name = self._eat("ident")
        self._eat("lparen")
        params = []
        if self._tok().kind != "rparen":
            params = self._params()
        self._eat("rparen")
        ret = None
        if kind == "function":
            self._eat("colon")
            ret = self._eat("type").value
        self._eat("semi")
        locals_ = []
        if self._tok().kind == "var":
            self._eat("var")
            while self._tok().kind == "ident":
                locals_.extend(self._vardecl())
        body_tok = self._tok()
        body = self._block_stmts()
        end_line = self.tokens[self.i - 1].line
        return Routine(kind, name.value, params, ret, locals_, body, end_line, name.line, name.col)

    def _params(self):
        out = []
        while True:
            names = [self._eat("ident")]
            while self._accept("comma"):
                names.append(self._eat("ident"))
            self._eat("colon")
            typ = self._eat("type").value
            for n in names:
                out.append(Param(n.value, typ, n.line, n.col))
            if not self._accept("semi"):
                break
        return out

    def _vardecl(self):
        names = [self._eat("ident")]
        while self._accept("comma"):
            names.append(self._eat("ident"))
        self._eat("colon")
        typ = self._eat("type").value
        self._eat("semi")
        return [Param(n.value, typ, n.line, n.col) for n in names]

    def _block_stmts(self):
        self._eat("begin")
        stmts = self._stmts_until({"end"})
        self._eat("end")
        self._accept("semi")
        return stmts

    def _stmts_until(self, stop):
        stmts = []
        while self._tok().kind not in stop and self._tok().kind != "eof":
            if self._accept("semi"):
                continue
            stmts.append(self._stmt())
        return stmts

    def _stmt(self):
        k = self._tok().kind
        if k == "if":
            return self._if()
        if k == "while":
            return self._while()
        if k == "begin":
            tok = self._tok()
            return Block(self._block_stmts(), tok.line, tok.col)
        if k == "exit":
            tok = self._eat("exit")
            self._accept("semi")
            return ExitStmt(tok.line, tok.col)
        return self._assign_or_call()

    def _if(self):
        tok = self._eat("if")
        cond = self._expr()
        self._eat("then")
        then = self._stmt()
        else_ = None
        if self._accept("else"):
            else_ = self._stmt()
        return IfStmt(cond, then, else_, tok.line, tok.col)

    def _while(self):
        tok = self._eat("while")
        cond = self._expr()
        self._eat("do")
        body = self._stmt()
        return WhileStmt(cond, body, tok.line, tok.col)

    def _assign_or_call(self):
        des = self._designator()
        if self._accept("assign"):
            expr = self._expr()
            self._accept("semi")
            return Assign(des, expr, des.line, des.col)
        if self._tok().kind == "lparen":
            self._eat("lparen")
            args = self._args()
            self._eat("rparen")
            self._accept("semi")
            if len(des.parts) != 1:
                raise CompileError("calls are not qualified", des.line, des.col, self.file)
            return CallStmt(des.parts[0], args, des.line, des.col)
        raise CompileError("expected ':=' or '('", des.line, des.col, self.file)

    def _designator(self):
        ident = self._eat("ident")
        parts = [ident.value]
        while self._accept("dot"):
            parts.append(self._eat("ident").value)
        return Designator(parts, ident.line, ident.col)

    def _args(self):
        if self._tok().kind == "rparen":
            return []
        args = [self._expr()]
        while self._accept("comma"):
            args.append(self._expr())
        return args

    def _expr(self):
        return self._binary(0)

    _PRECEDENCE = [
        ("=", "<>", "<", ">", "<=", ">="),
        ("+", "-", "or"),
        ("*", "/", "div", "mod", "and"),
    ]

    def _binary(self, level):
        if level >= len(self._PRECEDENCE):
            return self._unary()
        ops = self._PRECEDENCE[level]
        left = self._binary(level + 1)
        while self._tok().kind == "op" and self._tok().value in ops or self._tok().kind in ops:
            op_tok = self._tok()
            self.i += 1
            right = self._binary(level + 1)
            left = Binary(op_tok.value, left, right, op_tok.line, op_tok.col)
        return left

    def _unary(self):
        tok = self._tok()
        if tok.kind == "not" or (tok.kind == "op" and tok.value in ("+", "-")):
            self.i += 1
            expr = self._unary()
            return Unary(tok.value if tok.kind == "op" else "not", expr, tok.line, tok.col)
        return self._primary()

    def _primary(self):
        tok = self._tok()
        if tok.kind == "number":
            self.i += 1
            return Literal(tok.value, "Integer", tok.line, tok.col)
        if tok.kind == "string":
            self.i += 1
            return Literal(tok.value, "String", tok.line, tok.col)
        if tok.kind == "true":
            self.i += 1
            return Literal(True, "Boolean", tok.line, tok.col)
        if tok.kind == "false":
            self.i += 1
            return Literal(False, "Boolean", tok.line, tok.col)
        if tok.kind == "lparen":
            self.i += 1
            expr = self._expr()
            self._eat("rparen")
            return expr
        if tok.kind == "ident":
            ident = self._eat("ident")
            if self._tok().kind == "lparen":
                self._eat("lparen")
                args = self._args()
                self._eat("rparen")
                return CallExpr(ident.value, args, ident.line, ident.col)
            parts = [ident.value]
            line, col = ident.line, ident.col
            while self._accept("dot"):
                parts.append(self._eat("ident").value)
            if len(parts) == 1:
                return Ident(parts[0], line, col)
            return Designator(parts, line, col)
        raise CompileError(f"expected an expression, found {tok.value!r}", tok.line, tok.col, self.file)


def parse_droid(text, file=""):
    return Parser(text, file).parse()


class NParser:
    def __init__(self, text, file=""):
        self.file = file
        self.tokens = NLexer(text, file).tokens()
        self.i = 0

    def _tok(self):
        return self.tokens[self.i]

    def _eat(self, kind):
        tok = self._tok()
        if tok.kind != kind:
            raise CompileError(
                f"expected {kind}, found {tok.kind} {tok.value!r}",
                tok.line, tok.col, self.file,
            )
        self.i += 1
        return tok

    def _accept(self, kind):
        if self._tok().kind == kind:
            tok = self._tok()
            self.i += 1
            return tok
        return None

    def parse(self):
        funs = []
        while self._tok().kind != "eof":
            funs.append(self._fun())
        return NProgram(funs)

    def _fun(self):
        tok = self._eat("fun")
        name = self._eat("ident")
        self._eat("lparen")
        params = []
        if self._tok().kind != "rparen":
            while True:
                pname = self._eat("ident")
                self._eat("colon")
                self._eat("i32")
                params.append(pname.value)
                if not self._accept("comma"):
                    break
        self._eat("rparen")
        ret = "i32"
        if self._accept("op") if False else self._tok().kind == "op" and self._tok().value == "->":
            self.i += 1
            self._eat("i32")
        elif self._tok().kind == "colon":
            self._eat("colon")
            self._eat("i32")
        body = self._block()
        end_line = self.tokens[self.i - 1].line
        return NFun(name.value, params, ret, body, end_line, name.line, name.col)

    def _block(self):
        tok = self._eat("lbrace")
        stmts = []
        while self._tok().kind != "rbrace" and self._tok().kind != "eof":
            stmts.append(self._stmt())
        self._eat("rbrace")
        return stmts

    def _stmt(self):
        k = self._tok().kind
        if k == "return":
            tok = self._eat("return")
            expr = self._expr()
            self._eat("semi")
            return NReturn(expr, tok.line, tok.col)
        if k == "if":
            tok = self._eat("if")
            cond = self._expr()
            then = self._block()
            else_ = []
            if self._accept("else"):
                if self._tok().kind == "if":
                    else_ = [self._stmt()]
                else:
                    else_ = self._block()
            return NIf(cond, then, else_, tok.line, tok.col)
        if k == "while":
            tok = self._eat("while")
            cond = self._expr()
            body = self._block()
            return NWhile(cond, body, tok.line, tok.col)
        if k == "var":
            tok = self._eat("var")
            name = self._eat("ident")
            self._eat("colon")
            self._eat("i32")
            expr = None
            if self._accept("op") if False else (self._tok().kind == "op" and self._tok().value == "="):
                self.i += 1
                expr = self._expr()
            self._eat("semi")
            return NVar(name.value, expr, tok.line, tok.col)
        if k == "ident" and self.tokens[self.i + 1].kind == "op" and self.tokens[self.i + 1].value == "=":
            name = self._eat("ident")
            self._eat("op")
            expr = self._expr()
            self._eat("semi")
            return NAssign(name.value, expr, name.line, name.col)
        if k == "ident" and self.tokens[self.i + 1].kind == "lparen":
            name = self._eat("ident")
            self._eat("lparen")
            args = self._args()
            self._eat("rparen")
            self._eat("semi")
            return NCall(name.value, args, name.line, name.col)  # used as statement; checker requires use
        tok = self._tok()
        raise CompileError(f"expected a statement, found {tok.value!r}", tok.line, tok.col, self.file)

    def _args(self):
        if self._tok().kind == "rparen":
            return []
        args = [self._expr()]
        while self._accept("comma"):
            args.append(self._expr())
        return args

    def _expr(self):
        return self._binary(0)

    _OPS = [
        ("==", "!=", "<", ">", "<=", ">="),
        ("+", "-"),
        ("*", "/", "%"),
    ]

    def _binary(self, level):
        if level >= len(self._OPS):
            return self._unary()
        ops = self._OPS[level]
        left = self._binary(level + 1)
        while self._tok().kind == "op" and self._tok().value in ops:
            op = self._tok()
            self.i += 1
            right = self._binary(level + 1)
            left = NBinary(op.value, left, right, op.line, op.col)
        return left

    def _unary(self):
        tok = self._tok()
        if tok.kind == "op" and tok.value in ("+", "-"):
            self.i += 1
            return NUnary(tok.value, self._unary(), tok.line, tok.col)
        return self._primary()

    def _primary(self):
        tok = self._tok()
        if tok.kind == "number":
            self.i += 1
            return NLit(tok.value, tok.line, tok.col)
        if tok.kind == "ident":
            self.i += 1
            if self._tok().kind == "lparen":
                self._eat("lparen")
                args = self._args()
                self._eat("rparen")
                return NCall(tok.value, args, tok.line, tok.col)
            return NIdent(tok.value, tok.line, tok.col)
        if tok.kind == "lparen":
            self.i += 1
            expr = self._expr()
            self._eat("rparen")
            return expr
        raise CompileError(f"expected an expression, found {tok.value!r}", tok.line, tok.col, self.file)


def parse_native(text, file=""):
    return NParser(text, file).parse()
