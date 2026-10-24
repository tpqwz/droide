"""Compile diagnostics."""


class CompileError(Exception):
    def __init__(self, message, line=1, col=1, file=""):
        super().__init__(message)
        self.message = message
        self.line = line
        self.col = col
        self.file = file

    def to_dict(self):
        return {
            "message": self.message,
            "line": self.line,
            "col": self.col,
            "file": self.file,
        }


class ErrorLog:
    def __init__(self, file=""):
        self.file = file
        self.errors = []

    def add(self, message, line=1, col=1, file=None):
        err = CompileError(message, line, col, self.file if file is None else file)
        self.errors.append(err)
        return err

    def __bool__(self):
        return bool(self.errors)

    def to_list(self):
        return [e.to_dict() for e in self.errors]
