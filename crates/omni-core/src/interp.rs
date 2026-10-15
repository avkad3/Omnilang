//! Tree-walking interpreter, used by `omni run`.
//!
//! Integer arithmetic is checked: overflow and division by zero stop the
//! program with a diagnostic (in the user's language) instead of silently
//! wrapping, which is friendlier for learners than C's undefined behaviour.

use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::glossary::Glossary;
use crate::pack::{Kw, Pack};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Str(Rc<str>),
    Unit,
}

/// How printed values look.
#[derive(Debug, Clone)]
pub struct OutputStyle<'a> {
    /// Pack whose words are used for `true`/`false` and whose digits are used
    /// when `native_digits` is set.
    pub pack: &'a Pack,
    pub native_digits: bool,
}

impl OutputStyle<'_> {
    pub fn format(&self, v: &Value) -> String {
        match v {
            Value::Int(n) => self.number(*n),
            Value::Bool(b) => {
                let kw = if *b { Kw::True } else { Kw::False };
                self.pack
                    .word(kw)
                    .unwrap_or(if *b { "true" } else { "false" })
                    .to_string()
            }
            Value::Str(s) => s.to_string(),
            Value::Unit => String::new(),
        }
    }

    pub fn number(&self, n: i64) -> String {
        let ascii = n.to_string();
        match (self.native_digits, self.pack.digits) {
            (true, Some(ds)) => ascii
                .chars()
                .map(|c| c.to_digit(10).map_or(c, |d| ds[d as usize]))
                .collect(),
            _ => ascii,
        }
    }
}

pub const DEFAULT_MAX_DEPTH: usize = 5_000;

/// Stack reserved for the interpreter thread. A tree-walking interpreter
/// recurses on the native stack (over 10 KiB per call in debug builds), so it
/// runs on its own thread with room for [`DEFAULT_MAX_DEPTH`] nested calls.
const INTERPRETER_STACK: usize = 512 * 1024 * 1024;

/// Runs `f` on a thread with a large stack and returns its result.
pub fn with_large_stack<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("omni-interp".into())
            .stack_size(INTERPRETER_STACK)
            .spawn_scoped(scope, f)
            .expect("spawn interpreter thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e))
    })
}

/// Runs an already-checked program's entry point on a large stack.
pub fn run_program(
    program: &Program,
    glossary: &Glossary,
    style: OutputStyle,
    entry: usize,
    out: &mut (dyn Write + Send),
) -> Result<(), Diagnostic> {
    with_large_stack(|| {
        let mut it = Interpreter::new(program, glossary, style, out);
        it.run(&program.funcs[entry])
    })
}

pub struct Interpreter<'a> {
    funcs: HashMap<String, &'a Func>,
    glossary: &'a Glossary,
    style: OutputStyle<'a>,
    out: &'a mut (dyn Write + Send),
    scopes: Vec<HashMap<String, Value>>,
    depth: usize,
    pub max_depth: usize,
}

enum Flow {
    Next,
    Return(Value),
}

type RResult<T> = Result<T, Diagnostic>;

impl<'a> Interpreter<'a> {
    pub fn new(
        program: &'a Program,
        glossary: &'a Glossary,
        style: OutputStyle<'a>,
        out: &'a mut (dyn Write + Send),
    ) -> Self {
        let funcs = program
            .funcs
            .iter()
            .map(|f| (glossary.key(&f.name.text), f))
            .collect();
        Interpreter {
            funcs,
            glossary,
            style,
            out,
            scopes: Vec::new(),
            depth: 0,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    /// Runs `func` (normally the checked entry point) with no arguments.
    pub fn run(&mut self, func: &'a Func) -> RResult<()> {
        self.call(func, Vec::new(), func.name.span).map(|_| ())
    }

    fn call(&mut self, f: &'a Func, args: Vec<Value>, at: Span) -> RResult<Value> {
        if self.depth >= self.max_depth {
            return Err(Diagnostic::error("E0403", at).arg("limit", self.max_depth.to_string()));
        }
        self.depth += 1;
        let saved = std::mem::take(&mut self.scopes);
        let mut frame = HashMap::new();
        for (p, v) in f.params.iter().zip(args) {
            frame.insert(self.glossary.key(&p.name.text), v);
        }
        self.scopes.push(frame);
        let result = self.block_inner(&f.body);
        self.scopes = saved;
        self.depth -= 1;
        match result? {
            Flow::Return(v) => Ok(v),
            Flow::Next => Ok(Value::Unit),
        }
    }

    fn block(&mut self, b: &'a Block) -> RResult<Flow> {
        self.scopes.push(HashMap::new());
        let r = self.block_inner(b);
        self.scopes.pop();
        r
    }

    fn block_inner(&mut self, b: &'a Block) -> RResult<Flow> {
        for s in &b.stmts {
            if let Flow::Return(v) = self.stmt(s)? {
                return Ok(Flow::Return(v));
            }
        }
        Ok(Flow::Next)
    }

    fn stmt(&mut self, s: &'a Stmt) -> RResult<Flow> {
        match &s.kind {
            StmtKind::Let { name, value, .. } => {
                let v = self.expr(value)?;
                let key = self.glossary.key(&name.text);
                self.scopes.last_mut().expect("scope").insert(key, v);
            }
            StmtKind::Assign { name, value } => {
                let v = self.expr(value)?;
                let key = self.glossary.key(&name.text);
                if let Some(slot) = self.scopes.iter_mut().rev().find_map(|sc| sc.get_mut(&key)) {
                    *slot = v;
                }
            }
            StmtKind::If { cond, then, els } => {
                if self.truthy(cond)? {
                    return self.block(then);
                }
                match els {
                    Some(Else::Block(b)) => return self.block(b),
                    Some(Else::If(stmt)) => return self.stmt(stmt),
                    None => {}
                }
            }
            StmtKind::While { cond, body } => {
                while self.truthy(cond)? {
                    if let Flow::Return(v) = self.block(body)? {
                        return Ok(Flow::Return(v));
                    }
                }
            }
            StmtKind::Return(value) => {
                let v = match value {
                    Some(e) => self.expr(e)?,
                    None => Value::Unit,
                };
                return Ok(Flow::Return(v));
            }
            StmtKind::Print(values) => {
                let mut parts = Vec::with_capacity(values.len());
                for e in values {
                    let v = self.expr(e)?;
                    parts.push(self.style.format(&v));
                }
                let _ = writeln!(self.out, "{}", parts.join(" "));
            }
            StmtKind::Expr(e) => {
                self.expr(e)?;
            }
        }
        Ok(Flow::Next)
    }

    fn truthy(&mut self, e: &'a Expr) -> RResult<bool> {
        Ok(matches!(self.expr(e)?, Value::Bool(true)))
    }

    fn int(&mut self, e: &'a Expr) -> RResult<i64> {
        match self.expr(e)? {
            Value::Int(n) => Ok(n),
            other => unreachable!("checker guarantees int, got {other:?}"),
        }
    }

    fn expr(&mut self, e: &'a Expr) -> RResult<Value> {
        Ok(match &e.kind {
            ExprKind::Int(n) => Value::Int(*n),
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Str(s) => Value::Str(Rc::from(s.as_str())),
            ExprKind::Var(name) => {
                let key = self.glossary.key(name);
                self.scopes
                    .iter()
                    .rev()
                    .find_map(|sc| sc.get(&key))
                    .cloned()
                    .expect("checked")
            }
            ExprKind::Unary(UnOp::Neg, inner) => {
                let n = self.int(inner)?;
                Value::Int(
                    n.checked_neg()
                        .ok_or_else(|| Diagnostic::error("E0402", e.span))?,
                )
            }
            ExprKind::Unary(UnOp::Not, inner) => Value::Bool(!self.truthy(inner)?),
            ExprKind::Binary(BinOp::And, l, r) => Value::Bool(self.truthy(l)? && self.truthy(r)?),
            ExprKind::Binary(BinOp::Or, l, r) => Value::Bool(self.truthy(l)? || self.truthy(r)?),
            ExprKind::Binary(op @ (BinOp::Eq | BinOp::Ne), l, r) => {
                let (a, b) = (self.expr(l)?, self.expr(r)?);
                Value::Bool((a == b) == (*op == BinOp::Eq))
            }
            ExprKind::Binary(op, l, r) => {
                let (a, b) = (self.int(l)?, self.int(r)?);
                let overflow = || Diagnostic::error("E0402", e.span);
                match op {
                    BinOp::Add => Value::Int(a.checked_add(b).ok_or_else(overflow)?),
                    BinOp::Sub => Value::Int(a.checked_sub(b).ok_or_else(overflow)?),
                    BinOp::Mul => Value::Int(a.checked_mul(b).ok_or_else(overflow)?),
                    BinOp::Div | BinOp::Rem => {
                        if b == 0 {
                            return Err(Diagnostic::error("E0401", r.span));
                        }
                        let v = if *op == BinOp::Div {
                            a.checked_div(b)
                        } else {
                            a.checked_rem(b)
                        };
                        Value::Int(v.ok_or_else(overflow)?)
                    }
                    BinOp::Lt => Value::Bool(a < b),
                    BinOp::Le => Value::Bool(a <= b),
                    BinOp::Gt => Value::Bool(a > b),
                    BinOp::Ge => Value::Bool(a >= b),
                    BinOp::And | BinOp::Or | BinOp::Eq | BinOp::Ne => unreachable!(),
                }
            }
            ExprKind::Call(name, args) => {
                let mut values = Vec::with_capacity(args.len());
                for a in args {
                    values.push(self.expr(a)?);
                }
                let f = *self
                    .funcs
                    .get(&self.glossary.key(&name.text))
                    .expect("checked");
                self.call(f, values, e.span)?
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::pack::Packs;
    use crate::parser::parse;

    fn run_with(src: &str, lang: &str, native: bool) -> Result<String, Diagnostic> {
        let packs = Packs::builtin();
        let glossary = Glossary::default();
        let p = parse(src, &packs);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let c = check(&p.program, &packs, &glossary, true);
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        let mut out: Vec<u8> = Vec::new();
        let style = OutputStyle {
            pack: packs.get(lang).unwrap(),
            native_digits: native,
        };
        run_program(&p.program, &glossary, style, c.entry.unwrap(), &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn run(src: &str) -> String {
        run_with(src, "en", false).expect("runs")
    }

    #[test]
    fn arithmetic_loops_and_recursion() {
        let src = "
            fn fact(n: int) -> int { if n <= 1 { return 1; } return n * fact(n - 1); }
            fn main() {
                var i = 0;
                var sum = 0;
                while i < 5 { sum = sum + i; i = i + 1; }
                print \"sum\", sum;
                print fact(10), 7 / 2, -7 % 3, 1 < 2 and not false;
            }";
        assert_eq!(run(src), "sum 10\n3628800 3 -1 true\n");
    }

    #[test]
    fn same_program_three_languages_same_output() {
        let en = "fn sq(x: int) -> int { return x * x; } fn main() { var i = 1; while i <= 3 { print sq(i); i = i + 1; } }";
        let hi = "कार्य वर्ग(क: पूर्णांक) -> पूर्णांक { क * क लौटाओ। } कार्य मुख्य() { चर i = 1; जबतक i <= 3 { वर्ग(i) छापो। i = i + 1। } }";
        let ta = "செயல்பாடு வர்க்கம்(எ: முழுஎண்) -> முழுஎண் { எ * எ திருப்பு; } செயல்பாடு முதன்மை() { மாறி i = 1; i <= 3 இருக்கும்வரை { வர்க்கம்(i) அச்சிடு; i = i + 1; } }";
        assert_eq!(run(en), "1\n4\n9\n");
        assert_eq!(run(hi), run(en));
        assert_eq!(run(ta), run(en));
    }

    #[test]
    fn localized_output() {
        let src = "fn main() { print true, 2026; }";
        assert_eq!(run_with(src, "hi", true).unwrap(), "सत्य २०२६\n");
        assert_eq!(run_with(src, "ta", false).unwrap(), "மெய் 2026\n");
    }

    #[test]
    fn short_circuit_and_scoping() {
        let src = "fn boom() -> bool { print \"boom\"; return true; }
            fn main() { print false and boom(); let x = 1; if true { let x = 2; print x; } print x; }";
        assert_eq!(run(src), "false\n2\n1\n");
    }

    #[test]
    fn runtime_errors_are_diagnostics() {
        let d = run_with("fn main() { let z = 0; print 1 / z; }", "en", false).unwrap_err();
        assert_eq!(d.code, "E0401");
        let d = run_with("fn main() { print 9223372036854775807 + 1; }", "en", false).unwrap_err();
        assert_eq!(d.code, "E0402");
        let d = run_with(
            "fn f(n: int) -> int { return f(n + 1); } fn main() { print f(0); }",
            "en",
            false,
        )
        .unwrap_err();
        assert_eq!(d.code, "E0403");
    }
}
