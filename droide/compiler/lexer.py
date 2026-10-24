"""Lexer for the Droide language (Delphi-flavoured) and the native subset."""

from .errors import CompileError

KEYWORDS = {
    "unit", "uses", "procedure", "function", "var", "begin", "end",
    "if", "then", "else", "while", "do", "and", "or", "not", "div", "mod",
    "exit", "true", "false",
}


class Token:
    __slots__ = ("kind", "value", "line", "col")

    def __init__(self, kind, value, line, col):
        self.kind = kind
        self.value = value
        self.line = line
        self.col = col

    def __repr__(self):
        return f"Token({self.kind}, {self.value!r}, {self.line}:{self.col})"


class Lexer:
    def __init__(self, text, file=""):
        self.text = text
        self.file = file
        self.n = len(text)
        self.i = 0
        self.line = 1
        self.col = 1

    def _peek(self, k=0):
        j = self.i + k
        return self.text[j] if j < self.n else ""

    def _adv(self):
        ch = self.text[self.i]
        self.i += 1
        if ch == "\n":
            self.line += 1
            self.col = 1
        else:
            self.col += 1
        return ch

    def tokens(self):
        out = []
        while True:
            tok = self._next()
            out.append(tok)
            if tok.kind == "eof":
                return out

    def _next(self):
        self._skip()
        if self.i >= self.n:
            return Token("eof", "", self.line, self.col)
        line, col = self.line, self.col
        ch = self._peek()
        if ch.isalpha() or ch == "_":
            return self._ident(line, col)
        if ch.isdigit() or (ch == "$" and self._peek(1).isalnum()):
            return self._number(line, col)
        if ch in ("'", '"'):
            return self._string(line, col)
        two = ch + self._peek(1)
        if two == ":=":
            self._adv(); self._adv()
            return Token("assign", ":=", line, col)
        if two == "<>":
            self._adv(); self._adv()
            return Token("op", "<>", line, col)
        if two == "<=":
            self._adv(); self._adv()
            return Token("op", "<=", line, col)
        if two == ">=":
            self._adv(); self._adv()
            return Token("op", ">=", line, col)
        if ch in "+-*/=<>":
            self._adv()
            return Token("op", ch, line, col)
        singles = {
            "(": "lparen", ")": "rparen", ",": "comma", ";": "semi",
            ":": "colon", ".": "dot",
        }
        if ch in singles:
            self._adv()
            return Token(singles[ch], ch, line, col)
        raise CompileError(f"unexpected character {ch!r}", line, col, self.file)

    def _skip(self):
        while self.i < self.n:
            ch = self._peek()
            if ch in " \t\r\n":
                self._adv()
                continue
            if ch == "/" and self._peek(1) == "/":
                while self.i < self.n and self._peek() != "\n":
                    self._adv()
                continue
            if ch == "{":
                self._adv()
                while self.i < self.n and self._peek() != "}":
                    self._adv()
                if self.i < self.n:
                    self._adv()
                continue
            if ch == "(" and self._peek(1) == "*":
                self._adv(); self._adv()
                while self.i < self.n and not (self._peek() == "*" and self._peek(1) == ")"):
                    self._adv()
                if self.i < self.n:
                    self._adv(); self._adv()
                continue
            break

    def _ident(self, line, col):
        start = self.i
        while self.i < self.n and (self._peek().isalnum() or self._peek() == "_"):
            self._adv()
        word = self.text[start:self.i]
        low = word.lower()
        if low in KEYWORDS:
            return Token(low, low, line, col)
        if low in ("integer", "string", "boolean", "view"):
            return Token("type", low.capitalize() if low != "integer" else "Integer", line, col)
        return Token("ident", word, line, col)

    def _number(self, line, col):
        if self._peek() == "$":
            self._adv()
            start = self.i
            while self.i < self.n and self._peek() in "0123456789abcdefABCDEF":
                self._adv()
            raw = self.text[start:self.i]
            if not raw:
                raise CompileError("hex literal is empty", line, col, self.file)
            return Token("number", int(raw, 16), line, col)
        start = self.i
        while self.i < self.n and self._peek().isdigit():
            self._adv()
        if self._peek() in "xX" and self.text[start:self.i] == "0":
            self._adv()
            hstart = self.i
            while self.i < self.n and self._peek() in "0123456789abcdefABCDEF":
                self._adv()
            return Token("number", int(self.text[hstart:self.i], 16), line, col)
        return Token("number", int(self.text[start:self.i]), line, col)

    def _string(self, line, col):
        quote = self._adv()
        chars = []
        while self.i < self.n and self._peek() != quote and self._peek() != "\n":
            if quote == "'" and self._peek() == "'" and self._peek(1) == "'":
                chars.append("'")
                self._adv(); self._adv()
                continue
            chars.append(self._adv())
        if self._peek() != quote:
            raise CompileError("unterminated string", line, col, self.file)
        self._adv()
        return Token("string", "".join(chars), line, col)


# --- native language -------------------------------------------------------

NKEYWORDS = {"fun", "return", "if", "else", "while", "var", "i32"}


class NLexer:
    def __init__(self, text, file=""):
        self.text = text
        self.file = file
        self.n = len(text)
        self.i = 0
        self.line = 1
        self.col = 1

    def _peek(self, k=0):
        j = self.i + k
        return self.text[j] if j < self.n else ""

    def _adv(self):
        ch = self.text[self.i]
        self.i += 1
        if ch == "\n":
            self.line += 1
            self.col = 1
        else:
            self.col += 1
        return ch

    def tokens(self):
        out = []
        while True:
            tok = self._next()
            out.append(tok)
            if tok.kind == "eof":
                return out

    def _next(self):
        while self.i < self.n:
            ch = self._peek()
            if ch in " \t\r\n":
                self._adv()
                continue
            if ch == "/" and self._peek(1) == "/":
                while self.i < self.n and self._peek() != "\n":
                    self._adv()
                continue
            break
        if self.i >= self.n:
            return Token("eof", "", self.line, self.col)
        line, col = self.line, self.col
        ch = self._peek()
        if ch.isalpha() or ch == "_":
            start = self.i
            while self.i < self.n and (self._peek().isalnum() or self._peek() == "_"):
                self._adv()
            word = self.text[start:self.i]
            low = word.lower()
            if low in NKEYWORDS:
                return Token(low, low, line, col)
            return Token("ident", word, line, col)
        if ch.isdigit():
            start = self.i
            while self.i < self.n and self._peek().isdigit():
                self._adv()
            return Token("number", int(self.text[start:self.i]), line, col)
        two = ch + self._peek(1)
        for op in ("==", "!=", "<=", ">=", "->"):
            if two == op:
                self._adv(); self._adv()
                return Token("op", op, line, col)
        if ch in "+-*/%<>=!":
            self._adv()
            return Token("op", ch, line, col)
        singles = {
            "(": "lparen", ")": "rparen", "{": "lbrace", "}": "rbrace",
            ",": "comma", ";": "semi", ":": "colon",
        }
        if ch in singles:
            self._adv()
            return Token(singles[ch], ch, line, col)
        raise CompileError(f"unexpected character {ch!r}", line, col, self.file)
