//! Recursive-descent parsers for Droide source and native source.

use crate::ast::*;
use crate::error::CompileError;
use crate::lex::{lex_droid, lex_native, Token};

const DROID_PREC: &[&[&str]] = &[
    &["=", "<>", "<", ">", "<=", ">="],
    &["+", "-", "or"],
    &["*", "/", "div", "mod", "and"],
];

const NATIVE_PREC: &[&[&str]] = &[
    &["==", "!=", "<", ">", "<=", ">="],
    &["+", "-"],
    &["*", "/", "%"],
];

struct Parser {
    file: String,
    tokens: Vec<Token>,
    i: usize,
}

impl Parser {
    fn tok(&self) -> &Token {
        &self.tokens[self.i]
    }

    fn at(&self, kind: &str) -> bool {
        self.tok().kind == kind
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.i].clone();
        self.i += 1;
        tok
    }

    fn eat(&mut self, kind: &str) -> Result<Token, CompileError> {
        let tok = self.tok().clone();
        if tok.kind != kind {
            return Err(CompileError::new(
                format!("expected {kind}, found {} {}", tok.kind, tok.py_repr()),
                tok.line,
                tok.col,
                &self.file,
            ));
        }
        self.i += 1;
        Ok(tok)
    }

    fn accept(&mut self, kind: &str) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }

    fn fail(&self, message: impl Into<String>, line: i32, col: i32) -> CompileError {
        CompileError::new(message, line, col, &self.file)
    }

    fn parse_program(&mut self) -> Result<Program, CompileError> {
        let mut unit = None;
        let mut uses = Vec::new();
        let mut routines = Vec::new();
        while !self.at("eof") {
            let kind = self.tok().kind;
            if kind == "unit" {
                self.bump();
                let name = self.eat("ident")?;
                self.eat("semi")?;
                unit = Some(name.text().to_string());
            } else if kind == "uses" {
                self.bump();
                let first = self.eat("ident")?;
                uses = vec![first.text().to_string()];
                while self.accept("comma").is_some() {
                    uses.push(self.eat("ident")?.text().to_string());
                }
                self.eat("semi")?;
            } else if kind == "procedure" || kind == "function" {
                routines.push(self.routine()?);
            } else if kind == "semi" {
                self.bump();
            } else {
                let tok = self.tok().clone();
                return Err(self.fail(
                    format!("expected a procedure or function, found {}", tok.py_repr()),
                    tok.line,
                    tok.col,
                ));
            }
        }
        Ok(Program {
            line: 1,
            unit,
            uses,
            routines,
        })
    }

    fn routine(&mut self) -> Result<Routine, CompileError> {
        let kind_tok = self.bump();
        let kind = kind_tok.kind;
        let name = self.eat("ident")?;
        let name_line = name.line;
        let name_col = name.col;
        let name_text = name.text().to_string();
        self.eat("lparen")?;
        let params = if self.at("rparen") {
            Vec::new()
        } else {
            self.params()?
        };
        self.eat("rparen")?;
        let ret = if kind == "function" {
            self.eat("colon")?;
            Some(self.eat("type")?.text().to_string())
        } else {
            None
        };
        self.eat("semi")?;
        let mut locals = Vec::new();
        if self.at("var") {
            self.bump();
            while self.at("ident") {
                locals.extend(self.vardecl()?);
            }
        }
        let (body, end_line) = self.block_body()?;
        Ok(Routine {
            line: name_line,
            col: name_col,
            end_line,
            kind: kind.to_string(),
            name: name_text,
            params,
            ret,
            locals,
            body,
        })
    }

    fn params(&mut self) -> Result<Vec<Param>, CompileError> {
        let mut out = Vec::new();
        loop {
            let mut names = vec![self.eat("ident")?];
            while self.accept("comma").is_some() {
                names.push(self.eat("ident")?);
            }
            self.eat("colon")?;
            let typ = self.eat("type")?.text().to_string();
            for n in names {
                out.push(Param {
                    line: n.line,
                    col: n.col,
                    name: n.text().to_string(),
                    type_name: typ.clone(),
                });
            }
            if self.accept("semi").is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn vardecl(&mut self) -> Result<Vec<Param>, CompileError> {
        let mut names = vec![self.eat("ident")?];
        while self.accept("comma").is_some() {
            names.push(self.eat("ident")?);
        }
        self.eat("colon")?;
        let typ = self.eat("type")?.text().to_string();
        self.eat("semi")?;
        Ok(names
            .into_iter()
            .map(|n| Param {
                line: n.line,
                col: n.col,
                name: n.text().to_string(),
                type_name: typ.clone(),
            })
            .collect())
    }

    fn block_body(&mut self) -> Result<(Vec<Stmt>, i32), CompileError> {
        self.eat("begin")?;
        let stmts = self.stmts_until(&["end"])?;
        let end = self.eat("end")?;
        self.accept("semi");
        Ok((stmts, end.line))
    }

    fn stmts_until(&mut self, stop: &[&str]) -> Result<Vec<Stmt>, CompileError> {
        let mut stmts = Vec::new();
        while !stop.contains(&self.tok().kind) && !self.at("eof") {
            if self.accept("semi").is_some() {
                continue;
            }
            stmts.push(self.stmt()?);
        }
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<Stmt, CompileError> {
        match self.tok().kind {
            "if" => self.if_stmt(),
            "while" => self.while_stmt(),
            "begin" => {
                let tok = self.tok().clone();
                let (stmts, _) = self.block_body()?;
                Ok(Stmt::Block(Block {
                    line: tok.line,
                    col: tok.col,
                    stmts,
                }))
            }
            "exit" => {
                let tok = self.eat("exit")?;
                self.accept("semi");
                Ok(Stmt::Exit(ExitStmt {
                    line: tok.line,
                    col: tok.col,
                }))
            }
            _ => self.assign_or_call(),
        }
    }

    fn if_stmt(&mut self) -> Result<Stmt, CompileError> {
        let tok = self.eat("if")?;
        let cond = self.expr()?;
        self.eat("then")?;
        let then = self.stmt()?;
        let else_ = if self.accept("else").is_some() {
            self.stmt()?
        } else {
            // `else` is optional. An empty block walks like a missing else.
            Stmt::Block(Block {
                line: tok.line,
                col: tok.col,
                stmts: Vec::new(),
            })
        };
        Ok(Stmt::If(IfStmt {
            line: tok.line,
            col: tok.col,
            cond,
            then: Box::new(then),
            else_: Box::new(else_),
        }))
    }

    fn while_stmt(&mut self) -> Result<Stmt, CompileError> {
        let tok = self.eat("while")?;
        let cond = self.expr()?;
        self.eat("do")?;
        let body = self.stmt()?;
        Ok(Stmt::While(WhileStmt {
            line: tok.line,
            col: tok.col,
            cond,
            body: Box::new(body),
        }))
    }

    fn assign_or_call(&mut self) -> Result<Stmt, CompileError> {
        let des = self.designator()?;
        if self.accept("assign").is_some() {
            let expr = self.expr()?;
            self.accept("semi");
            return Ok(Stmt::Assign(Assign {
                line: des.line,
                col: des.col,
                target: des,
                expr,
            }));
        }
        if self.at("lparen") {
            self.bump();
            let args = self.args()?;
            self.eat("rparen")?;
            self.accept("semi");
            if des.parts.len() != 1 {
                return Err(self.fail("calls are not qualified", des.line, des.col));
            }
            return Ok(Stmt::Call(CallStmt {
                line: des.line,
                col: des.col,
                name: des.parts[0].clone(),
                args,
                resolved: None,
                form_name: None,
            }));
        }
        Err(self.fail("expected ':=' or '('", des.line, des.col))
    }

    fn designator(&mut self) -> Result<Designator, CompileError> {
        let ident = self.eat("ident")?;
        let mut parts = vec![ident.text().to_string()];
        while self.accept("dot").is_some() {
            parts.push(self.eat("ident")?.text().to_string());
        }
        Ok(Designator {
            line: ident.line,
            col: ident.col,
            parts,
            ty: None,
            resolved: None,
        })
    }

    fn args(&mut self) -> Result<Vec<Expr>, CompileError> {
        if self.at("rparen") {
            return Ok(Vec::new());
        }
        let mut args = vec![self.expr()?];
        while self.accept("comma").is_some() {
            args.push(self.expr()?);
        }
        Ok(args)
    }

    fn expr(&mut self) -> Result<Expr, CompileError> {
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> Result<Expr, CompileError> {
        if level >= DROID_PREC.len() {
            return self.unary();
        }
        let ops = DROID_PREC[level];
        let mut left = self.binary(level + 1)?;
        while self.is_droid_op(ops) {
            let op_tok = self.bump();
            let right = self.binary(level + 1)?;
            left = Expr::Binary(Binary {
                line: op_tok.line,
                col: op_tok.col,
                op: op_tok.text().to_string(),
                left: Box::new(left),
                right: Box::new(right),
                ty: None,
                string_add: false,
            });
        }
        Ok(left)
    }

    fn is_droid_op(&self, ops: &[&str]) -> bool {
        let tok = self.tok();
        (tok.kind == "op" && ops.contains(&tok.text())) || ops.contains(&tok.kind)
    }

    fn unary(&mut self) -> Result<Expr, CompileError> {
        let tok = self.tok().clone();
        if tok.kind == "not" || (tok.kind == "op" && (tok.text() == "+" || tok.text() == "-")) {
            self.bump();
            let expr = self.unary()?;
            let op = if tok.kind == "op" {
                tok.text().to_string()
            } else {
                "not".to_string()
            };
            return Ok(Expr::Unary(Unary {
                line: tok.line,
                col: tok.col,
                op,
                expr: Box::new(expr),
                ty: None,
            }));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, CompileError> {
        let tok = self.tok().clone();
        match tok.kind {
            "number" => {
                self.bump();
                let Some(n) = tok.as_int() else {
                    return Err(self.expr_err(&tok));
                };
                Ok(lit_int(n, tok.line, tok.col))
            }
            "string" => {
                self.bump();
                Ok(Expr::Literal(Literal {
                    line: tok.line,
                    col: tok.col,
                    value: Lit::Str(tok.text().to_string()),
                    type_name: "String".into(),
                    ty: None,
                }))
            }
            "true" | "false" => {
                self.bump();
                Ok(Expr::Literal(Literal {
                    line: tok.line,
                    col: tok.col,
                    value: Lit::Bool(tok.kind == "true"),
                    type_name: "Boolean".into(),
                    ty: None,
                }))
            }
            "lparen" => {
                self.bump();
                let expr = self.expr()?;
                self.eat("rparen")?;
                Ok(expr)
            }
            "ident" => {
                self.bump();
                if self.at("lparen") {
                    self.bump();
                    let args = self.args()?;
                    self.eat("rparen")?;
                    return Ok(Expr::Call(CallExpr {
                        line: tok.line,
                        col: tok.col,
                        name: tok.text().to_string(),
                        args,
                        ty: None,
                        resolved: None,
                        form_name: None,
                    }));
                }
                let mut parts = vec![tok.text().to_string()];
                while self.accept("dot").is_some() {
                    parts.push(self.eat("ident")?.text().to_string());
                }
                if parts.len() == 1 {
                    Ok(Expr::Ident(Ident {
                        line: tok.line,
                        col: tok.col,
                        name: parts.pop().unwrap(),
                        ty: None,
                        resolved: None,
                    }))
                } else {
                    Ok(Expr::Designator(Designator {
                        line: tok.line,
                        col: tok.col,
                        parts,
                        ty: None,
                        resolved: None,
                    }))
                }
            }
            _ => Err(self.expr_err(&tok)),
        }
    }

    fn expr_err(&self, tok: &Token) -> CompileError {
        self.fail(
            format!("expected an expression, found {}", tok.py_repr()),
            tok.line,
            tok.col,
        )
    }
}

fn lit_int(value: i64, line: i32, col: i32) -> Expr {
    Expr::Literal(Literal {
        line,
        col,
        value: Lit::Int(value),
        type_name: "Integer".into(),
        ty: None,
    })
}

pub fn parse_droid(text: &str, file: &str) -> Result<Program, CompileError> {
    let tokens = lex_droid(text, file)?;
    Parser {
        file: file.to_string(),
        tokens,
        i: 0,
    }
    .parse_program()
}

struct NParser {
    file: String,
    tokens: Vec<Token>,
    i: usize,
}

impl NParser {
    fn tok(&self) -> &Token {
        &self.tokens[self.i]
    }

    fn at(&self, kind: &str) -> bool {
        self.tok().kind == kind
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.i].clone();
        self.i += 1;
        tok
    }

    fn eat(&mut self, kind: &str) -> Result<Token, CompileError> {
        let tok = self.tok().clone();
        if tok.kind != kind {
            return Err(CompileError::new(
                format!("expected {kind}, found {} {}", tok.kind, tok.py_repr()),
                tok.line,
                tok.col,
                &self.file,
            ));
        }
        self.i += 1;
        Ok(tok)
    }

    fn accept(&mut self, kind: &str) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }

    fn fail(&self, message: impl Into<String>, line: i32, col: i32) -> CompileError {
        CompileError::new(message, line, col, &self.file)
    }

    fn ahead_is(&self, n: usize, kind: &str) -> bool {
        self.tokens.get(self.i + n).is_some_and(|t| t.kind == kind)
    }

    fn ahead_op(&self, n: usize, op: &str) -> bool {
        self.tokens
            .get(self.i + n)
            .is_some_and(|t| t.kind == "op" && t.text() == op)
    }

    fn parse_program(&mut self) -> Result<NProgram, CompileError> {
        let mut funs = Vec::new();
        while !self.at("eof") {
            funs.push(self.fun()?);
        }
        Ok(NProgram { funs })
    }

    fn fun(&mut self) -> Result<NFun, CompileError> {
        self.eat("fun")?;
        let name = self.eat("ident")?;
        let name_line = name.line;
        let name_col = name.col;
        let name_text = name.text().to_string();
        self.eat("lparen")?;
        let mut params = Vec::new();
        if !self.at("rparen") {
            loop {
                let pname = self.eat("ident")?;
                self.eat("colon")?;
                self.eat("i32")?;
                params.push(pname.text().to_string());
                if self.accept("comma").is_none() {
                    break;
                }
            }
        }
        self.eat("rparen")?;
        if (self.tok().kind == "op" && self.tok().text() == "->") || self.at("colon") {
            self.bump();
            self.eat("i32")?;
        }
        let body = self.block()?;
        let end_line = self.tokens[self.i - 1].line;
        Ok(NFun {
            line: name_line,
            col: name_col,
            end_line,
            name: name_text,
            params,
            ret: "i32".into(),
            body,
        })
    }

    fn block(&mut self) -> Result<Vec<NStmt>, CompileError> {
        self.eat("lbrace")?;
        let mut stmts = Vec::new();
        while !self.at("rbrace") && !self.at("eof") {
            stmts.push(self.stmt()?);
        }
        self.eat("rbrace")?;
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<NStmt, CompileError> {
        let kind = self.tok().kind;
        if kind == "return" {
            let tok = self.eat("return")?;
            let expr = self.expr()?;
            self.eat("semi")?;
            return Ok(NStmt::Return(NReturn {
                line: tok.line,
                col: tok.col,
                expr: Some(expr),
            }));
        }
        if kind == "if" {
            let tok = self.eat("if")?;
            let cond = self.expr()?;
            let then = self.block()?;
            let else_ = if self.accept("else").is_some() {
                if self.at("if") {
                    vec![self.stmt()?]
                } else {
                    self.block()?
                }
            } else {
                Vec::new()
            };
            return Ok(NStmt::If(NIf {
                line: tok.line,
                col: tok.col,
                cond,
                then,
                else_,
            }));
        }
        if kind == "while" {
            let tok = self.eat("while")?;
            let cond = self.expr()?;
            let body = self.block()?;
            return Ok(NStmt::While(NWhile {
                line: tok.line,
                col: tok.col,
                cond,
                body,
            }));
        }
        if kind == "var" {
            let tok = self.eat("var")?;
            let name = self.eat("ident")?;
            self.eat("colon")?;
            self.eat("i32")?;
            let expr = if self.tok().kind == "op" && self.tok().text() == "=" {
                self.bump();
                self.expr()?
            } else {
                // NVar.expr is required. A missing initializer is zero, as codegen treats it.
                NExpr::Lit(NLit {
                    line: tok.line,
                    col: tok.col,
                    value: 0,
                })
            };
            self.eat("semi")?;
            return Ok(NStmt::Var(NVar {
                line: tok.line,
                col: tok.col,
                name: name.text().to_string(),
                expr,
            }));
        }
        if kind == "ident" && self.ahead_op(1, "=") {
            let name = self.eat("ident")?;
            self.eat("op")?;
            let expr = self.expr()?;
            self.eat("semi")?;
            return Ok(NStmt::Assign(NAssign {
                line: name.line,
                col: name.col,
                name: name.text().to_string(),
                expr,
            }));
        }
        if kind == "ident" && self.ahead_is(1, "lparen") {
            let name = self.bump();
            let line = name.line;
            let col = name.col;
            let call = self.finish_call(name)?;
            self.eat("semi")?;
            // NStmt has no call form. Keep the call as a local so it is still checked and run.
            return Ok(NStmt::Var(NVar {
                line,
                col,
                name: format!(".discard.{line}.{col}"),
                expr: call,
            }));
        }
        let tok = self.tok().clone();
        Err(self.fail(
            format!("expected a statement, found {}", tok.py_repr()),
            tok.line,
            tok.col,
        ))
    }

    fn finish_call(&mut self, name: Token) -> Result<NExpr, CompileError> {
        self.eat("lparen")?;
        let args = self.args()?;
        self.eat("rparen")?;
        Ok(NExpr::Call(NCall {
            line: name.line,
            col: name.col,
            name: name.text().to_string(),
            args,
        }))
    }

    fn args(&mut self) -> Result<Vec<NExpr>, CompileError> {
        if self.at("rparen") {
            return Ok(Vec::new());
        }
        let mut args = vec![self.expr()?];
        while self.accept("comma").is_some() {
            args.push(self.expr()?);
        }
        Ok(args)
    }

    fn expr(&mut self) -> Result<NExpr, CompileError> {
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> Result<NExpr, CompileError> {
        if level >= NATIVE_PREC.len() {
            return self.unary();
        }
        let ops = NATIVE_PREC[level];
        let mut left = self.binary(level + 1)?;
        while self.is_native_op(ops) {
            let op_tok = self.bump();
            let right = self.binary(level + 1)?;
            left = NExpr::Binary(NBinary {
                line: op_tok.line,
                col: op_tok.col,
                op: op_tok.text().to_string(),
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn is_native_op(&self, ops: &[&str]) -> bool {
        let tok = self.tok();
        tok.kind == "op" && ops.contains(&tok.text())
    }

    fn unary(&mut self) -> Result<NExpr, CompileError> {
        let tok = self.tok().clone();
        if tok.kind == "op" && (tok.text() == "+" || tok.text() == "-") {
            self.bump();
            let expr = self.unary()?;
            return Ok(NExpr::Unary(NUnary {
                line: tok.line,
                col: tok.col,
                op: tok.text().to_string(),
                expr: Box::new(expr),
            }));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<NExpr, CompileError> {
        let tok = self.tok().clone();
        if tok.kind == "number" {
            self.bump();
            let Some(n) = tok.as_int() else {
                return Err(self.expr_err(&tok));
            };
            return Ok(NExpr::Lit(NLit {
                line: tok.line,
                col: tok.col,
                value: n,
            }));
        }
        if tok.kind == "ident" {
            self.bump();
            if self.at("lparen") {
                return self.finish_call(tok);
            }
            return Ok(NExpr::Ident(NIdent {
                line: tok.line,
                col: tok.col,
                name: tok.text().to_string(),
            }));
        }
        if tok.kind == "lparen" {
            self.bump();
            let expr = self.expr()?;
            self.eat("rparen")?;
            return Ok(expr);
        }
        Err(self.expr_err(&tok))
    }

    fn expr_err(&self, tok: &Token) -> CompileError {
        self.fail(
            format!("expected an expression, found {}", tok.py_repr()),
            tok.line,
            tok.col,
        )
    }
}

pub fn parse_native(text: &str, file: &str) -> Result<NProgram, CompileError> {
    let tokens = lex_native(text, file)?;
    NParser {
        file: file.to_string(),
        tokens,
        i: 0,
    }
    .parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_MAIN: &str = include_str!("/home/yy/code/droide/examples/Hello/forms/Main.droid");
    const HELLO_FIB: &str = include_str!("/home/yy/code/droide/examples/Hello/native/fib.nc");

    #[test]
    fn parses_hello_main() {
        let program = parse_droid(
            HELLO_MAIN,
            "/home/yy/code/droide/examples/Hello/forms/Main.droid",
        )
        .unwrap();
        assert!(program
            .routines
            .iter()
            .any(|r| r.name == "OkClick" || r.name == "FibClick"));
    }

    #[test]
    fn parses_fib_nc() {
        let program = parse_native(
            HELLO_FIB,
            "/home/yy/code/droide/examples/Hello/native/fib.nc",
        )
        .unwrap();
        assert_eq!(program.funs.len(), 1);
        assert_eq!(program.funs[0].name, "fib");
        assert_eq!(program.funs[0].params.len(), 1);
    }

    #[test]
    fn parses_shipped_sources() {
        let droid = [
            "/home/yy/code/droide/examples/Hello/forms/Main.droid",
            "/home/yy/code/droide/examples/Hello/forms/About.droid",
            "/home/yy/code/droide/examples/Hello/units/Utils.droid",
            "/home/yy/code/droide/examples/NativeCalc/forms/Calc.droid",
        ];
        for path in droid {
            let text = std::fs::read_to_string(path).unwrap();
            parse_droid(&text, path).unwrap();
        }
        let native = [
            "/home/yy/code/droide/examples/Hello/native/fib.nc",
            "/home/yy/code/droide/examples/NativeCalc/native/calc.nc",
        ];
        for path in native {
            let text = std::fs::read_to_string(path).unwrap();
            parse_native(&text, path).unwrap();
        }
    }

    #[test]
    fn optional_semicolon_before_else() {
        let program = parse_droid(
            "function F(): Integer;\nbegin\n  if 1 < 2 then\n    Result := 1\n  else\n    Result := 2\nend;\n",
            "F.droid",
        )
        .unwrap();
        assert_eq!(program.routines[0].name, "F");
        let Stmt::If(stmt) = &program.routines[0].body[0] else {
            panic!("if")
        };
        let Stmt::Assign(assign) = stmt.else_.as_ref() else {
            panic!("else")
        };
        let Expr::Literal(lit) = &assign.expr else {
            panic!("lit")
        };
        assert!(matches!(lit.value, Lit::Int(2)));
    }

    #[test]
    fn hello_main_shape() {
        let program = parse_droid(HELLO_MAIN, "Main.droid").unwrap();
        assert_eq!(program.uses, vec!["Utils".to_string()]);
        assert!(program.unit.is_none());
        let ok = program
            .routines
            .iter()
            .find(|r| r.name == "OkClick")
            .unwrap();
        assert_eq!(ok.kind, "procedure");
        assert_eq!(ok.line, 9);
        assert_eq!(ok.col, 11);
        assert_eq!(ok.end_line, 20);
        assert!(ok.ret.is_none());
        assert_eq!(ok.params.len(), 1);
        assert_eq!(ok.params[0].name, "Sender");
        assert_eq!(ok.params[0].type_name, "View");
        assert_eq!(ok.locals.len(), 1);
        assert_eq!(ok.locals[0].name, "Name");
        assert_eq!(ok.locals[0].type_name, "String");
        let fib = program
            .routines
            .iter()
            .find(|r| r.name == "FibClick")
            .unwrap();
        assert_eq!(fib.locals.len(), 2);
        assert_eq!(fib.locals[0].name, "N");
        assert_eq!(fib.locals[1].name, "F");
        assert_eq!(fib.end_line, 40);
    }

    #[test]
    fn utils_and_literals() {
        let text = std::fs::read_to_string("/home/yy/code/droide/examples/Hello/units/Utils.droid")
            .unwrap();
        let program = parse_droid(&text, "Utils.droid").unwrap();
        assert_eq!(program.unit.as_deref(), Some("Utils"));
        let clamp = &program.routines[0];
        assert_eq!(clamp.name, "Clamp");
        assert_eq!(clamp.kind, "function");
        assert_eq!(clamp.ret.as_deref(), Some("Integer"));
        assert_eq!(clamp.params.len(), 3);
        assert_eq!(clamp.end_line, 11);
        let sum = &program.routines[1];
        assert_eq!(sum.name, "SumTo");
        let Stmt::While(w) = &sum.body[2] else {
            panic!("while")
        };
        let Stmt::Block(body) = w.body.as_ref() else {
            panic!("block")
        };
        assert_eq!(body.stmts.len(), 2);
    }

    #[test]
    fn strings_hex_and_precedence() {
        let program = parse_droid(
            "procedure P();\nvar X: Boolean;\nbegin\n  X := 'it''s';\n  X := $Ff + 0x10 div 2 * 3\nend\n",
            "t.droid",
        )
        .unwrap();
        let Stmt::Assign(a) = &program.routines[0].body[0] else {
            panic!("assign")
        };
        let Expr::Literal(lit) = &a.expr else {
            panic!("lit")
        };
        assert_eq!(lit.type_name, "String");
        assert!(matches!(&lit.value, Lit::Str(s) if s == "it's"));
        let Stmt::Assign(a) = &program.routines[0].body[1] else {
            panic!("assign")
        };
        let Expr::Binary(add) = &a.expr else {
            panic!("add")
        };
        assert_eq!(add.op, "+");
        let Expr::Literal(hex) = add.left.as_ref() else {
            panic!("hex")
        };
        assert!(matches!(hex.value, Lit::Int(255)));
        let Expr::Binary(mul) = add.right.as_ref() else {
            panic!("mul")
        };
        assert_eq!(mul.op, "*");
        let Expr::Binary(div) = mul.left.as_ref() else {
            panic!("div")
        };
        assert_eq!(div.op, "div");
        assert!(!div.string_add);
    }

    #[test]
    fn droid_errors_match_python() {
        let err = parse_droid("procedure", "t.droid").unwrap_err();
        assert_eq!(err.message, "expected ident, found eof ''");
        assert_eq!((err.line, err.col, err.file.as_str()), (1, 10, "t.droid"));
        let err = parse_droid("procedure P;", "t.droid").unwrap_err();
        assert_eq!(err.message, "expected lparen, found semi ';'");
        assert_eq!((err.line, err.col), (1, 12));
        let err = parse_droid("@", "t.droid").unwrap_err();
        assert_eq!(err.message, "unexpected character '@'");
        let err = parse_droid("'abc", "t.droid").unwrap_err();
        assert_eq!(err.message, "unterminated string");
        let err = parse_droid("$", "t.droid").unwrap_err();
        assert_eq!(err.message, "unexpected character '$'");
        let err = parse_droid("$G", "t.droid").unwrap_err();
        assert_eq!(err.message, "hex literal is empty");
        let err = parse_droid("procedure P();\nbegin\n  A.B()\nend\n", "t.droid").unwrap_err();
        assert_eq!(err.message, "calls are not qualified");
    }

    #[test]
    fn fib_and_native_calc_shape() {
        let program = parse_native(HELLO_FIB, "fib.nc").unwrap();
        let fun = &program.funs[0];
        assert_eq!(fun.params, vec!["n".to_string()]);
        assert_eq!(fun.ret, "i32");
        assert_eq!(fun.line, 1);
        assert_eq!(fun.col, 5);
        assert_eq!(fun.end_line, 6);
        assert_eq!(fun.body.len(), 2);
        let text =
            std::fs::read_to_string("/home/yy/code/droide/examples/NativeCalc/native/calc.nc")
                .unwrap();
        let program = parse_native(&text, "calc.nc").unwrap();
        assert_eq!(program.funs.len(), 9);
        let sum = program.funs.iter().find(|f| f.name == "sumto").unwrap();
        assert_eq!(sum.end_line, 43);
        assert!(matches!(sum.body[0], NStmt::Var(_)));
        assert!(matches!(sum.body[2], NStmt::While(_)));
        let dist = program.funs.iter().find(|f| f.name == "dist").unwrap();
        let NStmt::Return(ret) = &dist.body[0] else {
            panic!("return")
        };
        let Some(NExpr::Call(call)) = &ret.expr else {
            panic!("call")
        };
        assert_eq!(call.name, "abs");
    }

    #[test]
    fn native_arrow_else_if_and_errors() {
        let program = parse_native(
            "fun F(A: i32, B: I32) -> i32 { var x: i32; x = 1 + 2 * 3; if x == 1 { return -x; } else if x != 2 { return x; } return 0; }\n",
            "e.nc",
        )
        .unwrap();
        let fun = &program.funs[0];
        assert_eq!(fun.name, "F");
        assert_eq!(fun.params, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(fun.ret, "i32");
        let NStmt::Var(var) = &fun.body[0] else {
            panic!("var")
        };
        assert_eq!(var.name, "x");
        assert!(matches!(var.expr, NExpr::Lit(NLit { value: 0, .. })));
        let NStmt::If(iff) = &fun.body[2] else {
            panic!("if")
        };
        assert_eq!(iff.else_.len(), 1);
        assert!(matches!(iff.else_[0], NStmt::If(_)));

        let err = parse_native("fun", "n.nc").unwrap_err();
        assert_eq!(err.message, "expected ident, found eof ''");
        assert_eq!((err.line, err.col), (1, 4));
        let err = parse_native("fun f(a: i32) { return; }", "n.nc").unwrap_err();
        assert_eq!(err.message, "expected an expression, found ';'");
        assert_eq!((err.line, err.col), (1, 23));
        let err = parse_native("fun f() { foo }", "n.nc").unwrap_err();
        assert_eq!(err.message, "expected a statement, found 'foo'");
        assert_eq!((err.line, err.col), (1, 11));
        let err = parse_native("!", "n.nc").unwrap_err();
        assert_eq!(err.message, "expected fun, found op '!'");
        let err =
            parse_native("fun f() { if 1 { return 1; } else return 2; }", "n.nc").unwrap_err();
        assert_eq!(err.message, "expected lbrace, found return 'return'");
        assert_eq!((err.line, err.col), (1, 35));
    }
}
