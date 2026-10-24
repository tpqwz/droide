//! Lexer for the Droide language and the native subset.

use crate::error::CompileError;

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: &'static str,
    pub value: TokValue,
    pub line: i32,
    pub col: i32,
}

#[derive(Clone, Debug)]
pub enum TokValue {
    Text(String),
    Int(i64),
}

impl Token {
    pub(crate) fn text(&self) -> &str {
        match &self.value {
            TokValue::Text(s) => s,
            TokValue::Int(_) => "",
        }
    }

    pub(crate) fn as_int(&self) -> Option<i64> {
        match self.value {
            TokValue::Int(n) => Some(n),
            TokValue::Text(_) => None,
        }
    }

    /// Python `repr` of the token value, so parser errors match the Python text.
    pub(crate) fn py_repr(&self) -> String {
        match &self.value {
            TokValue::Int(n) => n.to_string(),
            TokValue::Text(s) => py_repr(s),
        }
    }
}

pub(crate) fn py_repr(s: &str) -> String {
    let singles = s.chars().filter(|c| *c == '\'').count();
    let doubles = s.chars().filter(|c| *c == '"').count();
    let quote = if singles > 0 && doubles == 0 {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\'' if quote == '\'' => out.push_str("\\'"),
            '"' if quote == '"' => out.push_str("\\\""),
            c if c.is_control() => {
                let u = c as u32;
                if u < 0x100 {
                    out.push_str(&format!("\\x{u:02x}"));
                } else if u < 0x10000 {
                    out.push_str(&format!("\\u{u:04x}"));
                } else {
                    out.push_str(&format!("\\U{u:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

fn text_token(kind: &'static str, value: impl Into<String>, line: i32, col: i32) -> Token {
    Token {
        kind,
        value: TokValue::Text(value.into()),
        line,
        col,
    }
}

fn int_token(value: i64, line: i32, col: i32) -> Token {
    Token {
        kind: "number",
        value: TokValue::Int(value),
        line,
        col,
    }
}

fn droid_keyword(low: &str) -> Option<&'static str> {
    Some(match low {
        "unit" => "unit",
        "uses" => "uses",
        "procedure" => "procedure",
        "function" => "function",
        "var" => "var",
        "begin" => "begin",
        "end" => "end",
        "if" => "if",
        "then" => "then",
        "else" => "else",
        "while" => "while",
        "do" => "do",
        "and" => "and",
        "or" => "or",
        "not" => "not",
        "div" => "div",
        "mod" => "mod",
        "exit" => "exit",
        "true" => "true",
        "false" => "false",
        _ => return None,
    })
}

fn droid_type(low: &str) -> Option<&'static str> {
    Some(match low {
        "integer" => "Integer",
        "string" => "String",
        "boolean" => "Boolean",
        "view" => "View",
        _ => return None,
    })
}

fn native_keyword(low: &str) -> Option<&'static str> {
    Some(match low {
        "fun" => "fun",
        "return" => "return",
        "if" => "if",
        "else" => "else",
        "while" => "while",
        "var" => "var",
        "i32" => "i32",
        _ => return None,
    })
}

struct Scan<'a> {
    chars: Vec<char>,
    i: usize,
    line: i32,
    col: i32,
    file: &'a str,
}

impl<'a> Scan<'a> {
    fn new(text: &str, file: &'a str) -> Self {
        Self {
            chars: text.chars().collect(),
            i: 0,
            line: 1,
            col: 1,
            file,
        }
    }

    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.i + k).copied()
    }

    fn adv(&mut self) -> char {
        let ch = self.chars[self.i];
        self.i += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn error(&self, message: &str, line: i32, col: i32) -> CompileError {
        CompileError::new(message, line, col, self.file)
    }

    fn slice(&self, start: usize) -> String {
        self.chars[start..self.i].iter().collect()
    }

    fn lex_droid(&mut self) -> Result<Vec<Token>, CompileError> {
        let mut out = Vec::new();
        loop {
            let tok = self.next_droid()?;
            let done = tok.kind == "eof";
            out.push(tok);
            if done {
                return Ok(out);
            }
        }
    }

    fn lex_native(&mut self) -> Result<Vec<Token>, CompileError> {
        let mut out = Vec::new();
        loop {
            let tok = self.next_native()?;
            let done = tok.kind == "eof";
            out.push(tok);
            if done {
                return Ok(out);
            }
        }
    }

    fn next_droid(&mut self) -> Result<Token, CompileError> {
        self.skip_droid();
        if self.i >= self.chars.len() {
            return Ok(text_token("eof", "", self.line, self.col));
        }
        let (line, col) = (self.line, self.col);
        let ch = self.peek(0).unwrap();
        if ch.is_alphabetic() || ch == '_' {
            return Ok(self.ident(line, col, false));
        }
        if ch.is_ascii_digit() || (ch == '$' && self.peek(1).is_some_and(|c| c.is_alphanumeric())) {
            return self.number(line, col);
        }
        if ch == '\'' || ch == '"' {
            return self.string(line, col);
        }
        if let (Some(a), Some(b)) = (self.peek(0), self.peek(1)) {
            let two = match (a, b) {
                (':', '=') => Some(("assign", ":=")),
                ('<', '>') => Some(("op", "<>")),
                ('<', '=') => Some(("op", "<=")),
                ('>', '=') => Some(("op", ">=")),
                _ => None,
            };
            if let Some((kind, value)) = two {
                self.adv();
                self.adv();
                return Ok(text_token(kind, value, line, col));
            }
        }
        if "+-*/=<>".contains(ch) {
            self.adv();
            return Ok(text_token("op", ch.to_string(), line, col));
        }
        let single = match ch {
            '(' => Some("lparen"),
            ')' => Some("rparen"),
            ',' => Some("comma"),
            ';' => Some("semi"),
            ':' => Some("colon"),
            '.' => Some("dot"),
            _ => None,
        };
        if let Some(kind) = single {
            self.adv();
            return Ok(text_token(kind, ch.to_string(), line, col));
        }
        Err(self.unexpected(ch, line, col))
    }

    fn skip_droid(&mut self) {
        loop {
            let Some(ch) = self.peek(0) else { return };
            if matches!(ch, ' ' | '\t' | '\r' | '\n') {
                self.adv();
                continue;
            }
            if ch == '/' && self.peek(1) == Some('/') {
                self.adv();
                self.adv();
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.adv();
                }
                continue;
            }
            if ch == '{' {
                self.adv();
                while self.peek(0).is_some_and(|c| c != '}') {
                    self.adv();
                }
                if self.peek(0) == Some('}') {
                    self.adv();
                }
                continue;
            }
            if ch == '(' && self.peek(1) == Some('*') {
                self.adv();
                self.adv();
                while self.peek(0).is_some()
                    && !(self.peek(0) == Some('*') && self.peek(1) == Some(')'))
                {
                    self.adv();
                }
                if self.peek(0) == Some('*') && self.peek(1) == Some(')') {
                    self.adv();
                    self.adv();
                }
                continue;
            }
            return;
        }
    }

    fn ident(&mut self, line: i32, col: i32, native: bool) -> Token {
        let start = self.i;
        while self
            .peek(0)
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            self.adv();
        }
        let word = self.slice(start);
        let low = word.to_lowercase();
        if native {
            if let Some(kind) = native_keyword(&low) {
                return text_token(kind, low, line, col);
            }
            return text_token("ident", word, line, col);
        }
        if let Some(kind) = droid_keyword(&low) {
            return text_token(kind, low, line, col);
        }
        if let Some(ty) = droid_type(&low) {
            return text_token("type", ty, line, col);
        }
        text_token("ident", word, line, col)
    }

    fn number(&mut self, line: i32, col: i32) -> Result<Token, CompileError> {
        if self.peek(0) == Some('$') {
            self.adv();
            let raw = self.take_hex();
            if raw.is_empty() {
                return Err(self.error("hex literal is empty", line, col));
            }
            let value = i64::from_str_radix(&raw, 16)
                .map_err(|_| self.error("integer literal is too large", line, col))?;
            return Ok(int_token(value, line, col));
        }
        let start = self.i;
        while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
            self.adv();
        }
        if matches!(self.peek(0), Some('x' | 'X'))
            && self.i == start + 1
            && self.chars[start] == '0'
        {
            self.adv();
            let raw = self.take_hex();
            if raw.is_empty() {
                return Err(self.error("hex literal is empty", line, col));
            }
            let value = i64::from_str_radix(&raw, 16)
                .map_err(|_| self.error("integer literal is too large", line, col))?;
            return Ok(int_token(value, line, col));
        }
        let raw = self.slice(start);
        let value = raw
            .parse::<i64>()
            .map_err(|_| self.error("integer literal is too large", line, col))?;
        Ok(int_token(value, line, col))
    }

    fn take_hex(&mut self) -> String {
        let start = self.i;
        while self.peek(0).is_some_and(|c| c.is_ascii_hexdigit()) {
            self.adv();
        }
        self.slice(start)
    }

    /// Single-quoted strings treat `''` as one quote. Double quotes do not.
    fn string(&mut self, line: i32, col: i32) -> Result<Token, CompileError> {
        let quote = self.adv();
        let mut chars = String::new();
        loop {
            let Some(ch) = self.peek(0) else {
                return Err(self.error("unterminated string", line, col));
            };
            if ch == '\n' {
                return Err(self.error("unterminated string", line, col));
            }
            if quote == '\'' && ch == '\'' && self.peek(1) == Some('\'') {
                chars.push('\'');
                self.adv();
                self.adv();
                continue;
            }
            if ch == quote {
                break;
            }
            chars.push(self.adv());
        }
        self.adv();
        Ok(text_token("string", chars, line, col))
    }

    fn next_native(&mut self) -> Result<Token, CompileError> {
        self.skip_native();
        if self.i >= self.chars.len() {
            return Ok(text_token("eof", "", self.line, self.col));
        }
        let (line, col) = (self.line, self.col);
        let ch = self.peek(0).unwrap();
        if ch.is_alphabetic() || ch == '_' {
            return Ok(self.ident(line, col, true));
        }
        if ch.is_ascii_digit() {
            let start = self.i;
            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                self.adv();
            }
            let raw = self.slice(start);
            let value = raw
                .parse::<i64>()
                .map_err(|_| self.error("integer literal is too large", line, col))?;
            return Ok(int_token(value, line, col));
        }
        if let (Some(a), Some(b)) = (self.peek(0), self.peek(1)) {
            let op = match (a, b) {
                ('=', '=') => Some("=="),
                ('!', '=') => Some("!="),
                ('<', '=') => Some("<="),
                ('>', '=') => Some(">="),
                ('-', '>') => Some("->"),
                _ => None,
            };
            if let Some(op) = op {
                self.adv();
                self.adv();
                return Ok(text_token("op", op, line, col));
            }
        }
        if "+-*/%<>=!".contains(ch) {
            self.adv();
            return Ok(text_token("op", ch.to_string(), line, col));
        }
        let single = match ch {
            '(' => Some("lparen"),
            ')' => Some("rparen"),
            '{' => Some("lbrace"),
            '}' => Some("rbrace"),
            ',' => Some("comma"),
            ';' => Some("semi"),
            ':' => Some("colon"),
            _ => None,
        };
        if let Some(kind) = single {
            self.adv();
            return Ok(text_token(kind, ch.to_string(), line, col));
        }
        Err(self.unexpected(ch, line, col))
    }

    fn skip_native(&mut self) {
        loop {
            let Some(ch) = self.peek(0) else { return };
            if matches!(ch, ' ' | '\t' | '\r' | '\n') {
                self.adv();
                continue;
            }
            if ch == '/' && self.peek(1) == Some('/') {
                self.adv();
                self.adv();
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.adv();
                }
                continue;
            }
            return;
        }
    }

    fn unexpected(&self, ch: char, line: i32, col: i32) -> CompileError {
        self.error(
            &format!("unexpected character {}", py_repr(&ch.to_string())),
            line,
            col,
        )
    }
}

pub fn lex_droid(text: &str, file: &str) -> Result<Vec<Token>, CompileError> {
    Scan::new(text, file).lex_droid()
}

pub fn lex_native(text: &str, file: &str) -> Result<Vec<Token>, CompileError> {
    Scan::new(text, file).lex_native()
}
