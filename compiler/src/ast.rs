//! AST. The checker writes `ty`, `resolved`, `string_add`, and `form_name`.
//! Codegen reads those fields. Shapes match the Python compiler.

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct WidgetRef {
    pub name: String,
    pub id: i64,
    pub comp: Value,
}

#[derive(Clone, Debug)]
pub struct NativeRef {
    pub name: String,
    pub params: Vec<String>,
    pub id: i64,
}

/// A resolved Droide routine call. `unit` is the form or unit class name.
#[derive(Clone, Debug)]
pub struct RoutineRef {
    pub unit: String,
    pub name: String,
    pub ret: Option<String>,
    pub params: Vec<String>,
    pub is_static: bool,
}

#[derive(Clone, Debug)]
pub enum Resolved {
    /// Ident read. Python `(kind, ty, spelling)` where kind is `local` or `result`.
    Slot { kind: String, ty: String, spelling: String },
    /// Assign target. Python `(kind, ty)`.
    SlotWrite { kind: String, ty: String },
    Widget(WidgetRef),
    Prop { actual: String, widget: WidgetRef },
    Builtin(String),
    Native(NativeRef),
    Routine(RoutineRef),
}

#[derive(Clone, Debug)]
pub enum Lit {
    Int(i64),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug)]
pub struct Program {
    pub line: i32,
    pub unit: Option<String>,
    pub uses: Vec<String>,
    pub routines: Vec<Routine>,
}

#[derive(Clone, Debug)]
pub struct Routine {
    pub line: i32,
    pub col: i32,
    pub end_line: i32,
    pub kind: String,
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<String>,
    pub locals: Vec<Param>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub line: i32,
    pub col: i32,
    pub name: String,
    pub type_name: String,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Block(Block),
    Assign(Assign),
    Call(CallStmt),
    If(IfStmt),
    While(WhileStmt),
    Exit(ExitStmt),
}

#[derive(Clone, Debug)]
pub struct Block {
    pub line: i32,
    pub col: i32,
    pub stmts: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Assign {
    pub line: i32,
    pub col: i32,
    pub target: Designator,
    pub expr: Expr,
}

#[derive(Clone, Debug)]
pub struct CallStmt {
    pub line: i32,
    pub col: i32,
    pub name: String,
    pub args: Vec<Expr>,
    pub resolved: Option<Resolved>,
    pub form_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct IfStmt {
    pub line: i32,
    pub col: i32,
    pub cond: Expr,
    pub then: Box<Stmt>,
    pub else_: Box<Stmt>,
}

#[derive(Clone, Debug)]
pub struct WhileStmt {
    pub line: i32,
    pub col: i32,
    pub cond: Expr,
    pub body: Box<Stmt>,
}

#[derive(Clone, Debug)]
pub struct ExitStmt {
    pub line: i32,
    pub col: i32,
}

#[derive(Clone, Debug)]
pub struct Designator {
    pub line: i32,
    pub col: i32,
    pub parts: Vec<String>,
    pub ty: Option<String>,
    pub resolved: Option<Resolved>,
}

impl Designator {
    pub fn name(&self) -> &str {
        self.parts.first().map(|s| s.as_str()).unwrap_or("")
    }
}

#[derive(Clone, Debug)]
pub enum Expr {
    Ident(Ident),
    Literal(Literal),
    Unary(Unary),
    Binary(Binary),
    Call(CallExpr),
    Designator(Designator),
}

#[derive(Clone, Debug)]
pub struct Ident {
    pub line: i32,
    pub col: i32,
    pub name: String,
    pub ty: Option<String>,
    pub resolved: Option<Resolved>,
}

#[derive(Clone, Debug)]
pub struct Literal {
    pub line: i32,
    pub col: i32,
    pub value: Lit,
    pub type_name: String,
    pub ty: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Unary {
    pub line: i32,
    pub col: i32,
    pub op: String,
    pub expr: Box<Expr>,
    pub ty: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Binary {
    pub line: i32,
    pub col: i32,
    pub op: String,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
    pub ty: Option<String>,
    pub string_add: bool,
}

#[derive(Clone, Debug)]
pub struct CallExpr {
    pub line: i32,
    pub col: i32,
    pub name: String,
    pub args: Vec<Expr>,
    pub ty: Option<String>,
    pub resolved: Option<Resolved>,
    pub form_name: Option<String>,
}

impl Expr {
    pub fn line(&self) -> i32 {
        match self {
            Expr::Ident(n) => n.line,
            Expr::Literal(n) => n.line,
            Expr::Unary(n) => n.line,
            Expr::Binary(n) => n.line,
            Expr::Call(n) => n.line,
            Expr::Designator(n) => n.line,
        }
    }

    pub fn col(&self) -> i32 {
        match self {
            Expr::Ident(n) => n.col,
            Expr::Literal(n) => n.col,
            Expr::Unary(n) => n.col,
            Expr::Binary(n) => n.col,
            Expr::Call(n) => n.col,
            Expr::Designator(n) => n.col,
        }
    }

    pub fn ty_mut(&mut self) -> &mut Option<String> {
        match self {
            Expr::Ident(n) => &mut n.ty,
            Expr::Literal(n) => &mut n.ty,
            Expr::Unary(n) => &mut n.ty,
            Expr::Binary(n) => &mut n.ty,
            Expr::Call(n) => &mut n.ty,
            Expr::Designator(n) => &mut n.ty,
        }
    }
}

impl Stmt {
    pub fn line(&self) -> i32 {
        match self {
            Stmt::Block(s) => s.line,
            Stmt::Assign(s) => s.line,
            Stmt::Call(s) => s.line,
            Stmt::If(s) => s.line,
            Stmt::While(s) => s.line,
            Stmt::Exit(s) => s.line,
        }
    }

    pub fn col(&self) -> i32 {
        match self {
            Stmt::Block(s) => s.col,
            Stmt::Assign(s) => s.col,
            Stmt::Call(s) => s.col,
            Stmt::If(s) => s.col,
            Stmt::While(s) => s.col,
            Stmt::Exit(s) => s.col,
        }
    }
}

#[derive(Clone, Debug)]
pub struct NProgram {
    pub funs: Vec<NFun>,
}

#[derive(Clone, Debug)]
pub struct NFun {
    pub line: i32,
    pub col: i32,
    pub end_line: i32,
    pub name: String,
    pub params: Vec<String>,
    pub ret: String,
    pub body: Vec<NStmt>,
}

#[derive(Clone, Debug)]
pub enum NStmt {
    Var(NVar),
    Assign(NAssign),
    Return(NReturn),
    If(NIf),
    While(NWhile),
    Block(NBlock),
}

#[derive(Clone, Debug)]
pub struct NVar {
    pub line: i32,
    pub col: i32,
    pub name: String,
    pub expr: NExpr,
}

#[derive(Clone, Debug)]
pub struct NAssign {
    pub line: i32,
    pub col: i32,
    pub name: String,
    pub expr: NExpr,
}

#[derive(Clone, Debug)]
pub struct NReturn {
    pub line: i32,
    pub col: i32,
    pub expr: Option<NExpr>,
}

#[derive(Clone, Debug)]
pub struct NIf {
    pub line: i32,
    pub col: i32,
    pub cond: NExpr,
    pub then: Vec<NStmt>,
    pub else_: Vec<NStmt>,
}

#[derive(Clone, Debug)]
pub struct NWhile {
    pub line: i32,
    pub col: i32,
    pub cond: NExpr,
    pub body: Vec<NStmt>,
}

#[derive(Clone, Debug)]
pub struct NBlock {
    pub line: i32,
    pub col: i32,
    pub stmts: Vec<NStmt>,
}

#[derive(Clone, Debug)]
pub enum NExpr {
    Ident(NIdent),
    Lit(NLit),
    Unary(NUnary),
    Binary(NBinary),
    Call(NCall),
}

#[derive(Clone, Debug)]
pub struct NIdent {
    pub line: i32,
    pub col: i32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct NLit {
    pub line: i32,
    pub col: i32,
    pub value: i64,
}

#[derive(Clone, Debug)]
pub struct NUnary {
    pub line: i32,
    pub col: i32,
    pub op: String,
    pub expr: Box<NExpr>,
}

#[derive(Clone, Debug)]
pub struct NBinary {
    pub line: i32,
    pub col: i32,
    pub op: String,
    pub left: Box<NExpr>,
    pub right: Box<NExpr>,
}

#[derive(Clone, Debug)]
pub struct NCall {
    pub line: i32,
    pub col: i32,
    pub name: String,
    pub args: Vec<NExpr>,
}

impl NExpr {
    pub fn line(&self) -> i32 {
        match self {
            NExpr::Ident(n) => n.line,
            NExpr::Lit(n) => n.line,
            NExpr::Unary(n) => n.line,
            NExpr::Binary(n) => n.line,
            NExpr::Call(n) => n.line,
        }
    }

    pub fn col(&self) -> i32 {
        match self {
            NExpr::Ident(n) => n.col,
            NExpr::Lit(n) => n.col,
            NExpr::Unary(n) => n.col,
            NExpr::Binary(n) => n.col,
            NExpr::Call(n) => n.col,
        }
    }
}

impl NStmt {
    pub fn line(&self) -> i32 {
        match self {
            NStmt::Var(s) => s.line,
            NStmt::Assign(s) => s.line,
            NStmt::Return(s) => s.line,
            NStmt::If(s) => s.line,
            NStmt::While(s) => s.line,
            NStmt::Block(s) => s.line,
        }
    }
}
