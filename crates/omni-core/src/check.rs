//! Name resolution and type checking.
//!
//! Names are compared by their glossary key (see [`crate::glossary`]), so
//! `count` and `गिनती` resolve to the same variable when the project
//! glossary says they are the same term.

use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::glossary::Glossary;
use crate::pack::Packs;

/// Facts the later stages need.
#[derive(Debug, Clone)]
pub struct Checked {
    pub diagnostics: Vec<Diagnostic>,
    /// Index into `program.funcs` of the entry point, when one was required.
    pub entry: Option<usize>,
}

struct FuncSig {
    params: Vec<Type>,
    ret: Type,
    display: String,
}

struct Var {
    ty: Type,
    mutable: bool,
    display: String,
}

struct Checker<'a> {
    glossary: &'a Glossary,
    funcs: HashMap<String, FuncSig>,
    scopes: Vec<HashMap<String, Var>>,
    ret: Type,
    diags: Vec<Diagnostic>,
}

/// Checks `program`. With `require_entry`, exactly one function must be
/// named `main` (in any active language) and take no arguments.
pub fn check(program: &Program, packs: &Packs, glossary: &Glossary, require_entry: bool) -> Checked {
    let mut c = Checker {
        glossary,
        funcs: HashMap::new(),
        scopes: Vec::new(),
        ret: Type::Unit,
        diags: Vec::new(),
    };

    for f in &program.funcs {
        let key = glossary.key(&f.name.text);
        if let Some(prev) = c.funcs.get(&key) {
            c.diags.push(redefined(&f.name, &prev.display));
            continue;
        }
        c.funcs.insert(
            key,
            FuncSig {
                params: f.params.iter().map(|p| p.ty).collect(),
                ret: f.ret,
                display: f.name.text.clone(),
            },
        );
    }
    for f in &program.funcs {
        c.func(f);
    }

    let mut entry = None;
    if require_entry {
        let mains: Vec<usize> = (0..program.funcs.len())
            .filter(|&i| packs.is_main(&program.funcs[i].name.text))
            .collect();
        match mains.as_slice() {
            [] => {
                let names: Vec<String> = packs.iter().map(|p| format!("'{}'", p.main)).collect();
                c.diags
                    .push(Diagnostic::error("E0309", Span::default()).arg("names", names.join(" / ")));
            }
            [i] => {
                let f = &program.funcs[*i];
                if !f.params.is_empty() || f.ret != Type::Unit {
                    c.diags.push(Diagnostic::error("E0311", f.name.span));
                }
                entry = Some(*i);
            }
            [a, b, ..] => {
                let (fa, fb) = (&program.funcs[*a], &program.funcs[*b]);
                c.diags.push(
                    Diagnostic::error("E0310", fb.name.span)
                        .arg("name", &fa.name.text)
                        .arg("other", &fb.name.text),
                );
            }
        }
    }
    Checked {
        diagnostics: c.diags,
        entry,
    }
}

fn redefined(name: &Name, previous: &str) -> Diagnostic {
    if previous == name.text {
        Diagnostic::error("E0301", name.span).arg("name", &name.text)
    } else {
        // Different spellings of the same glossary term.
        Diagnostic::error("E0313", name.span)
            .arg("name", format!("'{}'", name.text))
            .arg("other", format!("'{previous}'"))
    }
}

fn mismatch(span: Span, expected: Type, found: Type) -> Diagnostic {
    Diagnostic::error("E0302", span)
        .arg("expected", expected.phrase())
        .arg("found", found.phrase())
}

impl Checker<'_> {
    fn func(&mut self, f: &Func) {
        self.ret = f.ret;
        self.scopes.push(HashMap::new());
        for p in &f.params {
            self.declare(&p.name, p.ty, false);
        }
        let returns = self.block(&f.body, false);
        self.scopes.pop();
        if f.ret != Type::Unit && !returns {
            self.diags.push(
                Diagnostic::error("E0307", f.name.span)
                    .arg("name", &f.name.text)
                    .arg("expected", f.ret.phrase()),
            );
        }
    }

    fn declare(&mut self, name: &Name, ty: Type, mutable: bool) {
        let key = self.glossary.key(&name.text);
        let scope = self.scopes.last_mut().expect("inside a scope");
        if let Some(prev) = scope.get(&key) {
            let d = redefined(name, &prev.display);
            self.diags.push(d);
            return;
        }
        scope.insert(
            key,
            Var {
                ty,
                mutable,
                display: name.text.clone(),
            },
        );
    }

    fn lookup(&self, name: &str) -> Option<&Var> {
        let key = self.glossary.key(name);
        self.scopes.iter().rev().find_map(|s| s.get(&key))
    }

    /// Checks a block; returns true if it always returns.
    fn block(&mut self, b: &Block, new_scope: bool) -> bool {
        if new_scope {
            self.scopes.push(HashMap::new());
        }
        let mut returns = false;
        for s in &b.stmts {
            returns |= self.stmt(s);
        }
        if new_scope {
            self.scopes.pop();
        }
        returns
    }

    fn stmt(&mut self, s: &Stmt) -> bool {
        match &s.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let found = self.expr(value);
                let declared = match (ty, found) {
                    (Some(t), Some(f)) if *t != f => {
                        self.diags.push(mismatch(value.span, *t, f));
                        *t
                    }
                    (Some(t), _) => *t,
                    (None, Some(f)) => f,
                    (None, None) => Type::Int, // error already reported
                };
                if declared == Type::Unit {
                    self.diags
                        .push(Diagnostic::error("E0312", value.span).arg("found", Type::Unit.phrase()));
                }
                self.declare(name, declared, *mutable);
                false
            }
            StmtKind::Assign { name, value } => {
                let found = self.expr(value);
                match self.lookup(&name.text) {
                    None => self
                        .diags
                        .push(Diagnostic::error("E0201", name.span).arg("name", &name.text)),
                    Some(v) if !v.mutable => {
                        self.diags
                            .push(Diagnostic::error("E0303", name.span).arg("name", &name.text));
                    }
                    Some(v) => {
                        if let Some(f) = found {
                            if f != v.ty {
                                let d = mismatch(value.span, v.ty, f);
                                self.diags.push(d);
                            }
                        }
                    }
                }
                false
            }
            StmtKind::If { cond, then, els } => {
                self.expect(cond, Type::Bool);
                let t = self.block(then, true);
                let e = match els {
                    None => false,
                    Some(Else::Block(b)) => self.block(b, true),
                    Some(Else::If(stmt)) => self.stmt(stmt),
                };
                t && e
            }
            StmtKind::While { cond, body } => {
                self.expect(cond, Type::Bool);
                self.block(body, true);
                false
            }
            StmtKind::Return(value) => {
                let found = match value {
                    Some(v) => self.expr(v),
                    None => Some(Type::Unit),
                };
                if let Some(f) = found {
                    if f != self.ret {
                        self.diags
                            .push(mismatch(value.as_ref().map_or(s.span, |v| v.span), self.ret, f));
                    }
                }
                true
            }
            StmtKind::Print(values) => {
                for v in values {
                    if let Some(Type::Unit) = self.expr(v) {
                        self.diags
                            .push(Diagnostic::error("E0312", v.span).arg("found", Type::Unit.phrase()));
                    }
                }
                false
            }
            StmtKind::Expr(e) => {
                self.expr(e);
                false
            }
        }
    }

    fn expect(&mut self, e: &Expr, want: Type) {
        if let Some(found) = self.expr(e) {
            if found != want {
                self.diags.push(mismatch(e.span, want, found));
            }
        }
    }

    /// Type of an expression, or None if an error was reported.
    fn expr(&mut self, e: &Expr) -> Option<Type> {
        match &e.kind {
            ExprKind::Int(_) => Some(Type::Int),
            ExprKind::Bool(_) => Some(Type::Bool),
            ExprKind::Str(_) => Some(Type::Str),
            ExprKind::Var(name) => match self.lookup(name) {
                Some(v) => Some(v.ty),
                None => {
                    // Functions aren't values, so a function name used as a
                    // variable is also "not defined" (as a variable).
                    self.diags
                        .push(Diagnostic::error("E0201", e.span).arg("name", name));
                    None
                }
            },
            ExprKind::Unary(op, inner) => {
                let want = match op {
                    UnOp::Neg => Type::Int,
                    UnOp::Not => Type::Bool,
                };
                let t = self.expr(inner)?;
                if t != want {
                    self.diags.push(mismatch(inner.span, want, t));
                    return None;
                }
                Some(want)
            }
            ExprKind::Binary(op, l, r) => {
                let (lt, rt) = (self.expr(l), self.expr(r));
                let (lt, rt) = (lt?, rt?);
                let bad = || {
                    Diagnostic::error("E0306", e.span)
                        .arg("op", op.symbol())
                        .arg("left", lt.phrase())
                        .arg("right", rt.phrase())
                };
                let result = match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                        (lt == Type::Int && rt == Type::Int).then_some(Type::Int)
                    }
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        (lt == Type::Int && rt == Type::Int).then_some(Type::Bool)
                    }
                    BinOp::Eq | BinOp::Ne => (lt == rt && lt != Type::Unit).then_some(Type::Bool),
                    BinOp::And | BinOp::Or => (lt == Type::Bool && rt == Type::Bool).then_some(Type::Bool),
                };
                if result.is_none() {
                    self.diags.push(bad());
                }
                result
            }
            ExprKind::Call(name, args) => {
                let arg_types: Vec<Option<Type>> = args.iter().map(|a| self.expr(a)).collect();
                let key = self.glossary.key(&name.text);
                if self.lookup(&name.text).is_some() && !self.funcs.contains_key(&key) {
                    self.diags
                        .push(Diagnostic::error("E0305", name.span).arg("name", &name.text));
                    return None;
                }
                let Some(sig) = self.funcs.get(&key) else {
                    self.diags
                        .push(Diagnostic::error("E0201", name.span).arg("name", &name.text));
                    return None;
                };
                if sig.params.len() != args.len() {
                    let d = Diagnostic::error("E0304", e.span)
                        .arg("name", &name.text)
                        .arg("expected", sig.params.len().to_string())
                        .arg("found", args.len().to_string());
                    self.diags.push(d);
                    return Some(sig.ret);
                }
                let ret = sig.ret;
                let params = sig.params.clone();
                for ((a, want), found) in args.iter().zip(params).zip(arg_types) {
                    if let Some(f) = found {
                        if f != want {
                            self.diags.push(mismatch(a.span, want, f));
                        }
                    }
                }
                Some(ret)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn diags_with(src: &str, glossary: &Glossary) -> Vec<&'static str> {
        let packs = Packs::builtin();
        let p = parse(src, &packs);
        assert!(p.diagnostics.is_empty(), "parse: {:?}", p.diagnostics);
        check(&p.program, &packs, glossary, true)
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect()
    }

    fn diags(src: &str) -> Vec<&'static str> {
        diags_with(src, &Glossary::default())
    }

    #[test]
    fn a_correct_program_in_each_language() {
        assert!(
            diags("fn add(a: int, b: int) -> int { return a + b; } fn main() { print add(1, 2); }")
                .is_empty()
        );
        assert!(diags(
            "कार्य जोड़(क: पूर्णांक, ख: पूर्णांक) -> पूर्णांक { क + ख लौटाओ। } कार्य मुख्य() { जोड़(1, 2) छापो। }"
        )
        .is_empty());
        assert!(
            diags("செயல்பாடு முதன்மை() { மாறி i = 0; i < 3 இருக்கும்வரை { i = i + 1; } i அச்சிடு; }").is_empty()
        );
    }

    #[test]
    fn names_and_mutability() {
        assert_eq!(diags("fn main() { print y; }"), ["E0201"]);
        assert_eq!(diags("fn main() { let x = 1; x = 2; }"), ["E0303"]);
        assert_eq!(diags("fn main() { let x = 1; let x = 2; }"), ["E0301"]);
        // Shadowing in an inner block is fine.
        assert!(diags("fn main() { let x = 1; if true { let x = 2; print x; } }").is_empty());
        assert_eq!(diags("fn main() { let f = 1; f(); }"), ["E0305"]);
    }

    #[test]
    fn types() {
        assert_eq!(diags("fn main() { let x: int = true; }"), ["E0302"]);
        assert_eq!(diags("fn main() { if 1 { } }"), ["E0302"]);
        assert_eq!(diags("fn main() { print 1 + true; }"), ["E0306"]);
        assert_eq!(diags("fn main() { print \"a\" < \"b\"; }"), ["E0306"]);
        assert!(diags("fn main() { print \"a\" == \"b\"; }").is_empty());
        assert_eq!(
            diags("fn f(a: int) {} fn main() { f(true); f(); }"),
            ["E0302", "E0304"]
        );
        assert_eq!(diags("fn f() {} fn main() { print f(); }"), ["E0312"]);
    }

    #[test]
    fn every_path_must_return() {
        assert_eq!(
            diags("fn f(x: int) -> int { if x > 0 { return 1; } } fn main() {}"),
            ["E0307"]
        );
        assert!(
            diags("fn f(x: int) -> int { if x > 0 { return 1; } else { return 2; } } fn main() {}")
                .is_empty()
        );
        assert_eq!(diags("fn f() -> int { return true; } fn main() {}"), ["E0302"]);
    }

    #[test]
    fn entry_point_rules() {
        assert_eq!(diags("fn f() {}"), ["E0309"]);
        assert_eq!(diags("fn main() {} कार्य मुख्य() {}"), ["E0310"]);
        assert_eq!(diags("fn main(x: int) {}"), ["E0311"]);
    }

    #[test]
    fn glossary_spellings_are_one_variable() {
        let g = Glossary::from_toml("[[term]]\nen = \"count\"\nhi = \"गिनती\"\n").unwrap();
        // Declared in Hindi, updated and printed in English.
        assert!(diags_with("fn main() { var गिनती = 1; count = count + 1; print count; }", &g).is_empty());
        // Declaring both spellings in one scope is a clash.
        assert_eq!(
            diags_with("fn main() { let count = 1; let गिनती = 2; }", &g),
            ["E0313"]
        );
    }

    #[test]
    fn error_messages_render_in_hindi_and_tamil() {
        let packs = Packs::builtin();
        let p = parse("fn main() { let x: int = true; }", &packs);
        let d = &check(&p.program, &packs, &Glossary::default(), true).diagnostics[0];
        assert!(d.message("hi").contains("पूर्णांक"), "{}", d.message("hi"));
        assert!(d.message("ta").contains("முழுஎண்"), "{}", d.message("ta"));
    }
}
