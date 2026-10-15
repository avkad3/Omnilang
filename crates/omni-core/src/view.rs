//! Multilingual views: render a program in any language pack.
//!
//! Because the AST is language-neutral, a program written in Tamil can be
//! shown to a Hindi-speaking reviewer in Hindi and to an employer in English,
//! with keywords, word order, type names, the entry-point name and (with a
//! glossary) identifiers all translated. Comments are kept as written.

use std::fmt::Write as _;

use crate::ast::*;
use crate::glossary::Glossary;
use crate::pack::{Kw, Pack, Packs, Position};

pub struct ViewOptions<'a> {
    pub pack: &'a Pack,
    pub packs: &'a Packs,
    pub glossary: &'a Glossary,
    /// Write numbers in the pack's native digits (e.g. Devanagari ०-९).
    pub native_digits: bool,
}

const INDENT: &str = "    ";

pub fn render(program: &Program, opts: &ViewOptions) -> String {
    let mut r = Renderer {
        o: opts,
        out: String::new(),
    };
    for (i, f) in program.funcs.iter().enumerate() {
        if i > 0 {
            r.out.push('\n');
        }
        r.func(f);
    }
    for c in &program.trailing_comments {
        r.comment(c, 0);
    }
    r.out
}

struct Renderer<'a> {
    o: &'a ViewOptions<'a>,
    out: String,
}

impl Renderer<'_> {
    fn kw(&self, kw: Kw) -> &str {
        self.o.pack.word(kw).expect("pack defines every required keyword")
    }

    fn term(&self) -> &str {
        &self.o.pack.terminator
    }

    fn name(&self, name: &str) -> String {
        if self.o.packs.is_main(name) {
            return self.o.pack.main.clone();
        }
        self.o
            .glossary
            .translate(name, &self.o.pack.code)
            .unwrap_or(name)
            .to_string()
    }

    fn ty(&self, t: Type) -> &str {
        match t {
            Type::Int => self.kw(Kw::Int),
            Type::Bool => self.kw(Kw::Bool),
            Type::Str => self.kw(Kw::Str),
            Type::Unit => "",
        }
    }

    fn line(&mut self, depth: usize, text: &str) {
        for _ in 0..depth {
            self.out.push_str(INDENT);
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn comment(&mut self, text: &str, depth: usize) {
        self.line(depth, &format!("//{text}"));
    }

    fn func(&mut self, f: &Func) {
        for c in &f.comments {
            self.comment(c, 0);
        }
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| format!("{}: {}", self.name(&p.name.text), self.ty(p.ty)))
            .collect();
        let mut head = format!(
            "{} {}({})",
            self.kw(Kw::Fn),
            self.name(&f.name.text),
            params.join(", ")
        );
        if f.ret != Type::Unit {
            let _ = write!(head, " -> {}", self.ty(f.ret));
        }
        self.out.push_str(&head);
        self.block(&f.body, 0);
        self.out.push('\n');
    }

    /// Writes ` {`, the statements, and the closing `}` (no newline after it).
    fn block(&mut self, b: &Block, depth: usize) {
        self.out.push_str(" {\n");
        for s in &b.stmts {
            self.stmt(s, depth + 1);
        }
        for c in &b.trailing_comments {
            self.comment(c, depth + 1);
        }
        for _ in 0..depth {
            self.out.push_str(INDENT);
        }
        self.out.push('}');
    }

    fn stmt(&mut self, s: &Stmt, depth: usize) {
        for c in &s.comments {
            self.comment(c, depth);
        }
        for _ in 0..depth {
            self.out.push_str(INDENT);
        }
        self.stmt_inline(s, depth);
        self.out.push('\n');
    }

    /// Renders a statement starting at the current position (used directly
    /// for `else if`).
    fn stmt_inline(&mut self, s: &Stmt, depth: usize) {
        let term = self.term().to_string();
        match &s.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let kw = self.kw(if *mutable { Kw::Var } else { Kw::Let }).to_string();
                let mut text = format!("{kw} {}", self.name(&name.text));
                if let Some(t) = ty {
                    let _ = write!(text, ": {}", self.ty(*t));
                }
                let _ = write!(text, " = {}{term}", self.expr(value, 0));
                self.out.push_str(&text);
            }
            StmtKind::Assign { name, value } => {
                let text = format!("{} = {}{term}", self.name(&name.text), self.expr(value, 0));
                self.out.push_str(&text);
            }
            StmtKind::If { cond, then, els } => {
                let head = self.condition(Kw::If, cond);
                self.out.push_str(&head);
                self.block(then, depth);
                match els {
                    None => {}
                    Some(Else::Block(b)) => {
                        let e = format!(" {}", self.kw(Kw::Else));
                        self.out.push_str(&e);
                        self.block(b, depth);
                    }
                    Some(Else::If(inner)) => {
                        let e = format!(" {} ", self.kw(Kw::Else));
                        self.out.push_str(&e);
                        self.stmt_inline(inner, depth);
                    }
                }
            }
            StmtKind::While { cond, body } => {
                let head = self.condition(Kw::While, cond);
                self.out.push_str(&head);
                self.block(body, depth);
            }
            StmtKind::Return(value) => {
                let word = self.kw(Kw::Return).to_string();
                let text = match (value, self.o.pack.position(Kw::Return)) {
                    (None, _) => format!("{word}{term}"),
                    (Some(v), Position::Prefix) => format!("{word} {}{term}", self.expr(v, 0)),
                    (Some(v), Position::Postfix) => format!("{} {word}{term}", self.expr(v, 0)),
                };
                self.out.push_str(&text);
            }
            StmtKind::Print(values) => {
                let word = self.kw(Kw::Print).to_string();
                let list: Vec<String> = values.iter().map(|v| self.expr(v, 0)).collect();
                let list = list.join(", ");
                let text = match self.o.pack.position(Kw::Print) {
                    Position::Prefix => format!("{word} {list}{term}"),
                    Position::Postfix => format!("{list} {word}{term}"),
                };
                self.out.push_str(&text);
            }
            StmtKind::Expr(e) => {
                let text = format!("{}{term}", self.expr(e, 0));
                self.out.push_str(&text);
            }
        }
    }

    /// `if cond` / `अगर cond तो` / `cond என்றால்` (without the block).
    fn condition(&self, kw: Kw, cond: &Expr) -> String {
        let word = self.kw(kw);
        let c = self.expr(cond, 0);
        match self.o.pack.position(kw) {
            Position::Prefix => {
                let then = if kw == Kw::If {
                    self.o.pack.word(Kw::Then)
                } else {
                    None
                };
                match then {
                    Some(t) => format!("{word} {c} {t}"),
                    None => format!("{word} {c}"),
                }
            }
            Position::Postfix => format!("{c} {word}"),
        }
    }

    /// Renders an expression, adding parentheses only where precedence
    /// requires them. `parent` is the precedence of the enclosing operator.
    fn expr(&self, e: &Expr, parent: u8) -> String {
        match &e.kind {
            ExprKind::Int(n) => {
                let s = n.to_string();
                match (self.o.native_digits, self.o.pack.digits) {
                    (true, Some(ds)) => s
                        .chars()
                        .map(|c| c.to_digit(10).map_or(c, |d| ds[d as usize]))
                        .collect(),
                    _ => s,
                }
            }
            ExprKind::Bool(b) => self.kw(if *b { Kw::True } else { Kw::False }).to_string(),
            ExprKind::Str(s) => format!("\"{}\"", escape(s)),
            ExprKind::Var(name) => self.name(name),
            ExprKind::Unary(op, inner) => {
                let inner_s = self.expr(inner, 6);
                match op {
                    UnOp::Neg => format!("-{inner_s}"),
                    UnOp::Not => format!("{} {inner_s}", self.kw(Kw::Not)),
                }
            }
            ExprKind::Binary(op, l, r) => {
                let p = op.precedence();
                let sym = match op {
                    BinOp::And => self.kw(Kw::And).to_string(),
                    BinOp::Or => self.kw(Kw::Or).to_string(),
                    other => other.symbol().to_string(),
                };
                // Left-associative: the right operand needs parens at equal
                // precedence (`a - (b - c)`).
                let text = format!("{} {sym} {}", self.expr(l, p), self.expr(r, p + 1));
                if p < parent {
                    format!("({text})")
                } else {
                    text
                }
            }
            ExprKind::Call(name, args) => {
                let args: Vec<String> = args.iter().map(|a| self.expr(a, 0)).collect();
                format!("{}({})", self.name(&name.text), args.join(", "))
            }
        }
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    const PROGRAM: &str = "// Sum of squares
fn square(x: int) -> int {
    return x * x;
}

fn main() {
    var i = 1;
    var total = 0;
    // loop
    while i <= 3 {
        total = total + square(i);
        i = i + 1;
    }
    if total > 10 and not false {
        print \"big\", total;
    } else if total == 0 {
        return;
    } else {
        print (1 + 2) * 3, 10 - (4 - 1);
    }
}
";

    fn view(src: &str, lang: &str, glossary: &Glossary) -> String {
        let packs = Packs::builtin();
        let p = parse(src, &packs);
        assert!(p.diagnostics.is_empty(), "{lang}: {:?}\n{src}", p.diagnostics);
        let opts = ViewOptions {
            pack: packs.get(lang).unwrap(),
            packs: &packs,
            glossary,
            native_digits: false,
        };
        render(&p.program, &opts)
    }

    #[test]
    fn english_view_is_canonical_formatting() {
        assert_eq!(view(PROGRAM, "en", &Glossary::default()), PROGRAM);
    }

    #[test]
    fn round_trips_through_every_language() {
        let g = Glossary::default();
        for lang in ["hi", "ta"] {
            let translated = view(PROGRAM, lang, &g);
            // Translating back to English recovers the original exactly.
            assert_eq!(view(&translated, "en", &g), PROGRAM, "via {lang}:\n{translated}");
        }
    }

    #[test]
    fn hindi_and_tamil_use_their_own_word_order() {
        let g = Glossary::default();
        let hi = view(PROGRAM, "hi", &g);
        assert!(hi.contains("x * x लौटाओ।"), "{hi}");
        assert!(hi.contains("अगर total > 10 और नहीं असत्य तो {"), "{hi}");
        assert!(hi.contains("कार्य मुख्य() {"), "{hi}");
        let ta = view(PROGRAM, "ta", &g);
        assert!(ta.contains("i <= 3 இருக்கும்வரை {"), "{ta}");
        assert!(ta.contains("\"big\", total அச்சிடு;"), "{ta}");
        assert!(ta.contains("} இல்லையெனில் total == 0 என்றால் {"), "{ta}");
    }

    #[test]
    fn glossary_translates_names_and_native_digits() {
        let g = Glossary::from_toml(
            "[[term]]\nen = \"total\"\nhi = \"कुल\"\nta = \"மொத்தம்\"\n[[term]]\nen = \"square\"\nhi = \"वर्ग\"\n",
        )
        .unwrap();
        let hi = view(PROGRAM, "hi", &g);
        assert!(hi.contains("चर कुल = 0।") && hi.contains("वर्ग(i)"), "{hi}");
        // ...and back to English through the same glossary.
        assert_eq!(view(&hi, "en", &g), PROGRAM);

        let packs = Packs::builtin();
        let p = parse("fn main() { print 2026; }", &packs);
        let opts = ViewOptions {
            pack: packs.get("hi").unwrap(),
            packs: &packs,
            glossary: &g,
            native_digits: true,
        };
        assert!(render(&p.program, &opts).contains("२०२६ छापो।"));
    }
}
