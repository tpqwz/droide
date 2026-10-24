//! Compile diagnostics. Field names match the JSON the studio already reads.

use std::fmt;

#[derive(Clone, Debug)]
pub struct CompileError {
    pub message: String,
    pub line: i32,
    pub col: i32,
    pub file: String,
}

impl CompileError {
    pub fn new(message: impl Into<String>, line: i32, col: i32, file: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line,
            col,
            file: file.into(),
        }
    }

    pub fn bare(message: impl Into<String>) -> Self {
        Self::new(message, 1, 1, "")
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "message": self.message,
            "line": self.line,
            "col": self.col,
            "file": self.file,
        })
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Debug, Default)]
pub struct ErrorLog {
    pub file: String,
    pub errors: Vec<CompileError>,
}

impl ErrorLog {
    pub fn new(file: impl Into<String>) -> Self {
        Self { file: file.into(), errors: Vec::new() }
    }

    pub fn add(&mut self, message: impl Into<String>, line: i32, col: i32, file: Option<&str>) -> &CompileError {
        let file = file.unwrap_or(&self.file).to_string();
        self.errors.push(CompileError::new(message, line, col, file));
        self.errors.last().unwrap()
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}
