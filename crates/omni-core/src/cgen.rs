//! C backend: emits readable, portable C99.
//!
//! * Names are mangled to ASCII so any C toolchain and linker accepts them:
//!   `om_` + the name, with `_` doubled and every other non-alphanumeric
//!   character written as `_uXXXX_`. The scheme is injective, and the
//!   original spelling is kept in a comment.
//! * Integer arithmetic is checked (GCC/Clang overflow builtins), matching
//!   the interpreter: overflow and division by zero print the translated
//!   message and exit with status 101.
//! * Source comments are carried over, so the C is a readable companion to
//!   the original program.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::glossary::Glossary;
use crate::interp::OutputStyle;
use crate::pack::Kw;

pub struct COptions<'a> {
    pub glossary: &'a Glossary,
    /// Controls the words for true/false and native digits in output.
    pub style: OutputStyle<'a>,
    /// Language of runtime error messages compiled into the program.
    pub message_lang: &'a str,
}

/// Mangles a name into a C identifier.
pub fn mangle(name: &str) -> String {
    let mut out = String::from("om_");
    for c in name.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => out.push(c),
            '_' => out.push_str("__"),
            _ => {
                let _ = write!(out, "_u{:04X}_", c as u32);
            }
        }
    }
    out
}

/// A C string literal. Non-ASCII text stays as UTF-8 so the generated C is
/// readable in any script; only quotes, backslashes, control characters and
/// `?` (trigraph safety) are escaped.
pub fn c_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '?' => out.push_str("\\?"),
            c if (c as u32) < 0x20 || c == '\u{7F}' => {
                let _ = write!(out, "\\{:03o}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn c_type(t: Type) -> &'static str {
    match t {
        Type::Int => "int64_t",
        Type::Bool => "bool",
        Type::Str => "const char *",
        Type::Unit => "void",
    }
}

/// Generates a complete C translation unit for a checked program.
pub fn generate(program: &Program, entry: usize, opts: &COptions) -> String {
    let funcs = program
        .funcs
        .iter()
        .map(|f| (opts.glossary.key(&f.name.text), f.ret))
        .collect();
    let mut g = Gen {
        o: opts,
        out: String::new(),
        funcs,
        scopes: Vec::new(),
    };
    g.prelude();
    for (i, f) in program.funcs.iter().enumerate() {
        let proto = g.prototype(f, i == entry);
        let _ = writeln!(g.out, "{proto};");
    }
    g.out.push('\n');
    for (i, f) in program.funcs.iter().enumerate() {
        g.func(f, i == entry);
        g.out.push('\n');
    }
    for c in &program.trailing_comments {
        let _ = writeln!(g.out, "//{c}");
    }
    g.out.push_str(concat!(
        "int main(void) {\n",
        "#ifdef _WIN32\n",
        "    SetConsoleOutputCP(65001);            /* UTF-8 console */\n",
        "    _setmode(_fileno(stdout), _O_BINARY); /* LF line endings, as elsewhere */\n",
        "#endif\n",
        "    om_entry();\n",
        "    return 0;\n",
        "}\n",
    ));
    g.out
}

struct Gen<'a> {
    o: &'a COptions<'a>,
    out: String,
    /// Return type of each function, by glossary key.
    funcs: HashMap<String, Type>,
    /// Variable types in the scopes currently being generated.
    scopes: Vec<HashMap<String, Type>>,
}

impl Gen<'_> {
    /// C name for a source name. Every spelling of a glossary term must map
    /// to the same C name, so prefer the term's English spelling.
    fn name(&self, name: &str) -> String {
        let canonical = match self.o.glossary.translate(name, "en") {
            Some(en) => en.to_string(),
            None => {
                let key = self.o.glossary.key(name);
                match key.strip_prefix('#') {
                    Some(n) => format!("term{n}"),
                    None => key,
                }
            }
        };
        mangle(&canonical)
    }

    /// C name plus the original spelling as a comment, for declarations.
    fn named(&self, name: &str) -> String {
        let m = self.name(name);
        if name.is_ascii() {
            m
        } else {
            format!("{m} /* {name} */")
        }
    }

    fn prelude(&mut self) {
        let lang = self.o.message_lang;
        let msg = |code: &'static str| c_string(&Diagnostic::error(code, Default::default()).message(lang));
        let (div0, overflow) = (msg("E0401"), msg("E0402"));
        let style = &self.o.style;
        let t = c_string(style.pack.word(Kw::True).unwrap_or("true"));
        let f = c_string(style.pack.word(Kw::False).unwrap_or("false"));
        let digits: Vec<String> = match (style.native_digits, style.pack.digits) {
            (true, Some(ds)) => ds.iter().map(|d| c_string(&d.to_string())).collect(),
            _ => (0..10).map(|d| c_string(&d.to_string())).collect(),
        };
        let digits = digits.join(", ");
        let _ = write!(
            self.out,
            r#"/* Generated by the OmniLang compiler (omni). */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#include <windows.h>
#endif

static void om_fail(const char *msg) {{
    fflush(stdout);
    fprintf(stderr, "%s\n", msg);
    exit(101);
}}

static int64_t om_add(int64_t a, int64_t b) {{ int64_t r; if (__builtin_add_overflow(a, b, &r)) om_fail({overflow}); return r; }}
static int64_t om_sub(int64_t a, int64_t b) {{ int64_t r; if (__builtin_sub_overflow(a, b, &r)) om_fail({overflow}); return r; }}
static int64_t om_mul(int64_t a, int64_t b) {{ int64_t r; if (__builtin_mul_overflow(a, b, &r)) om_fail({overflow}); return r; }}
static int64_t om_div(int64_t a, int64_t b) {{
    if (b == 0) om_fail({div0});
    if (a == INT64_MIN && b == -1) om_fail({overflow});
    return a / b;
}}
static int64_t om_rem(int64_t a, int64_t b) {{
    if (b == 0) om_fail({div0});
    if (a == INT64_MIN && b == -1) om_fail({overflow});
    return a % b;
}}
static int64_t om_neg(int64_t a) {{ if (a == INT64_MIN) om_fail({overflow}); return -a; }}

static const char *const om_digits[10] = {{{digits}}};
static void om_print_int(int64_t v) {{
    char buf[24];
    snprintf(buf, sizeof buf, "%lld", (long long)v);
    for (const char *p = buf; *p; p++) {{
        if (*p >= '0' && *p <= '9') fputs(om_digits[*p - '0'], stdout);
        else putchar(*p);
    }}
}}
static void om_print_bool(bool v) {{ fputs(v ? {t} : {f}, stdout); }}
static void om_print_str(const char *v) {{ fputs(v, stdout); }}

"#
        );
    }

    fn prototype(&self, f: &Func, is_entry: bool) -> String {
        let name = if is_entry {
            "om_entry".to_string()
        } else {
            self.name(&f.name.text)
        };
        let params: Vec<String> = f
            .params
            .iter()
            .map(|p| format!("{} {}", c_type(p.ty), self.name(&p.name.text)))
            .collect();
        let params = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        format!("static {} {name}({params})", c_type(f.ret))
    }

    fn func(&mut self, f: &Func, is_entry: bool) {
        for c in &f.comments {
            let _ = writeln!(self.out, "//{c}");
        }
        if !f.name.text.is_ascii() {
            let _ = writeln!(self.out, "/* {} */", f.name.text);
        }
        let proto = self.prototype(f, is_entry);
        self.out.push_str(&proto);
        let params = f
            .params
            .iter()
            .map(|p| (self.o.glossary.key(&p.name.text), p.ty))
            .collect();
        self.scopes.push(params);
        self.block(&f.body, 0);
        self.scopes.pop();
        self.out.push('\n');
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str("    ");
        }
    }

    fn block(&mut self, b: &Block, depth: usize) {
        self.out.push_str(" {\n");
        self.scopes.push(HashMap::new());
        for s in &b.stmts {
            self.stmt(s, depth + 1);
        }
        self.scopes.pop();
        for c in &b.trailing_comments {
            self.indent(depth + 1);
            let _ = writeln!(self.out, "//{c}");
        }
        self.indent(depth);
        self.out.push('}');
    }

    fn stmt(&mut self, s: &Stmt, depth: usize) {
        for c in &s.comments {
            self.indent(depth);
            let _ = writeln!(self.out, "//{c}");
        }
        self.indent(depth);
        self.stmt_inline(s, depth);
        self.out.push('\n');
    }

    fn stmt_inline(&mut self, s: &Stmt, depth: usize) {
        match &s.kind {
            StmtKind::Let {
                mutable,
                name,
                value,
                ty,
            } => {
                let t = ty.unwrap_or_else(|| self.type_of(value));
                let konst = if *mutable { "" } else { "const " };
                let text = format!(
                    "{konst}{} {} = {};",
                    c_type(t),
                    self.named(&name.text),
                    self.expr(value)
                );
                self.out.push_str(&text);
                let key = self.o.glossary.key(&name.text);
                self.scopes.last_mut().expect("inside a block").insert(key, t);
            }
            StmtKind::Assign { name, value } => {
                let text = format!("{} = {};", self.name(&name.text), self.expr(value));
                self.out.push_str(&text);
            }
            StmtKind::If { cond, then, els } => {
                let head = format!("if ({})", self.expr(cond));
                self.out.push_str(&head);
                self.block(then, depth);
                match els {
                    None => {}
                    Some(Else::Block(b)) => {
                        self.out.push_str(" else");
                        self.block(b, depth);
                    }
                    Some(Else::If(inner)) => {
                        self.out.push_str(" else ");
                        self.stmt_inline(inner, depth);
                    }
                }
            }
            StmtKind::While { cond, body } => {
                let head = format!("while ({})", self.expr(cond));
                self.out.push_str(&head);
                self.block(body, depth);
            }
            StmtKind::Return(None) => self.out.push_str("return;"),
            StmtKind::Return(Some(v)) => {
                let text = format!("return {};", self.expr(v));
                self.out.push_str(&text);
            }
            StmtKind::Print(values) => {
                let mut parts = Vec::new();
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        parts.push("putchar(' ');".to_string());
                    }
                    let printer = match self.type_of(v) {
                        Type::Int => "om_print_int",
                        Type::Bool => "om_print_bool",
                        _ => "om_print_str",
                    };
                    parts.push(format!("{printer}({});", self.expr(v)));
                }
                parts.push("putchar('\\n');".into());
                let text = format!("{{ {} }}", parts.join(" "));
                self.out.push_str(&text);
            }
            StmtKind::Expr(e) => {
                let text = format!("(void)({});", self.expr(e));
                self.out.push_str(&text);
            }
        }
    }

    fn expr(&self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(n) => format!("INT64_C({n})"),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => c_string(s),
            ExprKind::Var(name) => self.name(name),
            ExprKind::Unary(UnOp::Neg, inner) => format!("om_neg({})", self.expr(inner)),
            ExprKind::Unary(UnOp::Not, inner) => format!("!({})", self.expr(inner)),
            ExprKind::Binary(op, l, r) => {
                let (a, b) = (self.expr(l), self.expr(r));
                match op {
                    BinOp::Add => format!("om_add({a}, {b})"),
                    BinOp::Sub => format!("om_sub({a}, {b})"),
                    BinOp::Mul => format!("om_mul({a}, {b})"),
                    BinOp::Div => format!("om_div({a}, {b})"),
                    BinOp::Rem => format!("om_rem({a}, {b})"),
                    BinOp::Eq | BinOp::Ne if self.type_of(l) == Type::Str => {
                        let cmp = if *op == BinOp::Eq { "==" } else { "!=" };
                        format!("(strcmp({a}, {b}) {cmp} 0)")
                    }
                    other => format!("({a} {} {b})", other.symbol()),
                }
            }
            ExprKind::Call(name, args) => {
                let args: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                format!("{}({})", self.name(&name.text), args.join(", "))
            }
        }
    }

    /// Type of an expression in an already type-checked program.
    fn type_of(&self, e: &Expr) -> Type {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Unary(UnOp::Neg, _) => Type::Int,
            ExprKind::Bool(_) | ExprKind::Unary(UnOp::Not, _) => Type::Bool,
            ExprKind::Str(_) => Type::Str,
            ExprKind::Binary(op, ..) => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => Type::Int,
                _ => Type::Bool,
            },
            ExprKind::Var(n) => {
                let key = self.o.glossary.key(n);
                self.scopes
                    .iter()
                    .rev()
                    .find_map(|s| s.get(&key))
                    .copied()
                    .expect("checked program")
            }
            ExprKind::Call(n, _) => self.funcs[&self.o.glossary.key(&n.text)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::pack::Packs;
    use crate::parser::parse;

    fn emit(src: &str, glossary: &Glossary) -> String {
        let packs = Packs::builtin();
        let p = parse(src, &packs);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let c = check(&p.program, &packs, glossary, true);
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        let style = OutputStyle {
            pack: packs.get("en").unwrap(),
            native_digits: false,
        };
        generate(
            &p.program,
            c.entry.unwrap(),
            &COptions {
                glossary,
                style,
                message_lang: "en",
            },
        )
    }

    #[test]
    fn mangling_is_ascii_and_injective() {
        assert_eq!(mangle("count"), "om_count");
        assert_eq!(mangle("a_b"), "om_a__b");
        assert_eq!(mangle("गिनती"), "om__u0917__u093F__u0928__u0924__u0940_");
        assert!(mangle("எண்ணிக்கை").is_ascii());
        // A literal "_u0041_" can't collide with the escape for a character,
        // because literal underscores are doubled.
        assert_ne!(mangle("_u0041_"), mangle("\u{C0}"));
    }

    #[test]
    fn strings_stay_readable_utf8() {
        assert_eq!(c_string("नमस्ते \"x\"\n"), r#""नमस्ते \"x\"\n""#);
        assert_eq!(c_string("a??b"), r#""a\?\?b""#);
    }

    #[test]
    fn generates_typed_checked_code() {
        let c = emit(
            r#"fn half(n: int) -> int { return n / 2; }
               fn main() { let x = half(10); var ok = x == 5; print "x", x, ok; if "a" == "a" { print 1; } }"#,
            &Glossary::default(),
        );
        assert!(c.contains("static int64_t om_half(int64_t om_n)"), "{c}");
        assert!(c.contains("const int64_t om_x = om_half(INT64_C(10));"), "{c}");
        assert!(c.contains("bool om_ok = (om_x == INT64_C(5));"), "{c}");
        assert!(c.contains(r#"om_print_str("x");"#), "{c}");
        assert!(c.contains("om_print_bool(om_ok);"), "{c}");
        assert!(c.contains(r#"(strcmp("a", "a") == 0)"#), "{c}");
        assert!(c.contains("static void om_entry(void)"), "{c}");
    }

    #[test]
    fn glossary_spellings_become_one_c_name() {
        let g = Glossary::from_toml("[[term]]\nen = \"count\"\nhi = \"गिनती\"\n").unwrap();
        let c = emit("fn main() { var गिनती = 1; count = count + 1; print गिनती; }", &g);
        assert!(c.contains("int64_t om_count /* गिनती */ = INT64_C(1);"), "{c}");
        assert!(c.contains("om_count = om_add(om_count, INT64_C(1));"), "{c}");
    }
}
