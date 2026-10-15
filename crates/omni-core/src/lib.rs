//! OmniLang compiler core.
//!
//! Source code may be written in any mix of the active language packs; every
//! pack lowers to the same language-neutral syntax tree.

pub mod diag;
pub mod lexer;
pub mod pack;
pub mod security;
