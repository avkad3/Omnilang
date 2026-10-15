//! OmniLang compiler core.
//!
//! Source code may be written in any mix of the active language packs; every
//! pack lowers to the same language-neutral syntax tree.

pub mod ast;
pub mod cgen;
pub mod check;
pub mod diag;
pub mod glossary;
pub mod interp;
pub mod lexer;
pub mod pack;
pub mod parser;
pub mod security;
pub mod translit;
pub mod view;
