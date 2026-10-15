//! The language-neutral syntax tree.
//!
//! Nothing in here records which human language a program was written in:
//! `if` and `अगर` and `என்றால்` all become [`StmtKind::If`]. That is what lets
//! the same program be shown in any language (see [`crate::view`]).

use crate::diag::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub funcs: Vec<Func>,
    /// Comments after the last function.
    pub trailing_comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    Int,
    Bool,
    Str,
    /// The "type" of a function that returns nothing.
    Unit,
}

impl Type {
    /// Catalog key for this type's localized name (see `messages.toml`).
    pub fn phrase(self) -> &'static str {
        match self {
            Type::Int => "@type.int",
            Type::Bool => "@type.bool",
            Type::Str => "@type.str",
            Type::Unit => "@what.nothing",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Name,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Func {
    pub name: Name,
    pub params: Vec<Param>,
    pub ret: Type,
    pub body: Block,
    pub span: Span,
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
    /// Comments between the last statement and the closing brace.
    pub trailing_comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
    /// Comments on the lines just above this statement.
    pub comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Let {
        mutable: bool,
        name: Name,
        ty: Option<Type>,
        value: Expr,
    },
    Assign {
        name: Name,
        value: Expr,
    },
    If {
        cond: Expr,
        then: Block,
        els: Option<Else>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    Return(Option<Expr>),
    Print(Expr),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Else {
    Block(Block),
    /// `else if …` chains; always holds a [`StmtKind::If`].
    If(Box<Stmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Bool(bool),
    Str(String),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(Name, Vec<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    /// The symbol, for diagnostics and C output (`and`/`or` are words in
    /// source but `&&`/`||` here).
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }

    /// Binding strength; higher binds tighter.
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Or => 1,
            BinOp::And => 2,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 3,
            BinOp::Add | BinOp::Sub => 4,
            BinOp::Mul | BinOp::Div | BinOp::Rem => 5,
        }
    }
}
