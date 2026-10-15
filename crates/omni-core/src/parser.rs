//! Parser: tokens from any mix of language packs → language-neutral AST.
//!
//! Statement keywords may be *prefix* (`return x;`, `अगर x > 1 { … }`) or
//! *postfix* (`x लौटाओ।`, `x > 1 என்றால் { … }`) depending on the word used.
//! The grammar handles both in one file:
//!
//! ```text
//! stmt := (let|var) NAME (':' type)? '=' expr ';'
//!       | NAME '=' expr ';'
//!       | PREFIX-if expr then? block else?     | expr POSTFIX-if block else?
//!       | PREFIX-while expr block              | expr POSTFIX-while block
//!       | PREFIX-return expr? ';'              | expr? POSTFIX-return ';'
//!       | PREFIX-print expr ';'                | expr POSTFIX-print ';'
//!       | expr ';'
//! ```

use unicode_normalization::UnicodeNormalization;

use crate::ast::*;
use crate::diag::{Diagnostic, Span};
use crate::lexer::{lex, Comment, Tok, Token};
use crate::pack::{Kw, Packs, Position};

pub struct Parsed {
    pub program: Program,
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses `src`. Diagnostics from both stages are returned together.
pub fn parse(src: &str, packs: &Packs) -> Parsed {
    let lexed = lex(src, packs);
    let mut p = Parser {
        src,
        packs,
        toks: lexed.tokens,
        pos: 0,
        comments: lexed.comments,
        next_comment: 0,
        diags: lexed.diagnostics,
    };
    let program = p.program();
    Parsed {
        program,
        diagnostics: p.diags,
    }
}

/// Internal marker: an error was already reported.
struct Reported;
type PResult<T> = Result<T, Reported>;

struct Parser<'a> {
    src: &'a str,
    packs: &'a Packs,
    toks: Vec<Token>,
    pos: usize,
    comments: Vec<Comment>,
    next_comment: usize,
    diags: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    // ---------- token helpers ----------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.toks[self.pos.saturating_sub(1)].span
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.peek() == tok {
            self.bump();
            true
        } else {
            false
        }
    }

    fn at_kw(&self, kw: Kw) -> bool {
        *self.peek() == Tok::Kw(kw)
    }

    /// Source text of the current token, or the "end of file" phrase.
    fn found(&self) -> String {
        match self.peek() {
            Tok::Eof => "@what.eof".into(),
            _ => {
                let s = self.span();
                format!("'{}'", &self.src[s.start..s.end])
            }
        }
    }

    fn error_expected(&mut self, expected: &str) -> Reported {
        let d = Diagnostic::error("E0200", self.span())
            .arg("expected", expected)
            .arg("found", self.found());
        self.diags.push(d);
        Reported
    }

    fn expect(&mut self, tok: Tok, expected: &str) -> PResult<Span> {
        if *self.peek() == tok {
            Ok(self.bump().span)
        } else {
            Err(self.error_expected(expected))
        }
    }

    fn expect_terminator(&mut self) -> PResult<()> {
        self.expect(Tok::Semi, "@what.stmt_end").map(|_| ())
    }

    /// The keyword word as written, and its word order.
    fn keyword_word(&self, span: Span) -> (String, Option<Position>) {
        let word: String = self.src[span.start..span.end].nfkc().collect();
        let pos = self.packs.keyword_position(&word);
        (word, pos)
    }

    /// Comments that appear before byte offset `before`.
    fn take_comments(&mut self, before: usize) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(c) = self.comments.get(self.next_comment) {
            if c.span.start >= before {
                break;
            }
            out.push(c.text.clone());
            self.next_comment += 1;
        }
        out
    }

    /// After an error, skip to a point where parsing can resume.
    fn synchronize(&mut self) {
        loop {
            match self.peek() {
                Tok::Eof | Tok::RBrace => return,
                Tok::Semi => {
                    self.bump();
                    return;
                }
                Tok::Kw(Kw::Let | Kw::Var | Kw::Fn) => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    // ---------- items ----------

    fn program(&mut self) -> Program {
        let mut funcs = Vec::new();
        while *self.peek() != Tok::Eof {
            if self.at_kw(Kw::Fn) {
                match self.func() {
                    Ok(f) => funcs.push(f),
                    Err(Reported) => self.skip_item(),
                }
            } else {
                self.error_expected("@what.item");
                self.skip_item();
            }
        }
        let trailing_comments = self.take_comments(usize::MAX);
        Program {
            funcs,
            trailing_comments,
        }
    }

    /// Skips to the next `fn` at top level.
    fn skip_item(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Kw(Kw::Fn) if depth == 0 => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.bump();
        }
    }

    fn func(&mut self) -> PResult<Func> {
        let start = self.span();
        let comments = self.take_comments(start.start);
        self.bump(); // fn
        let name = self.name()?;
        self.expect(Tok::LParen, "'('")?;
        let mut params = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                let pname = self.name()?;
                self.expect(Tok::Colon, "':'")?;
                let ty = self.ty()?;
                params.push(Param { name: pname, ty });
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "')'")?;
        let ret = if self.eat(&Tok::Arrow) {
            self.ty()?
        } else {
            Type::Unit
        };
        let body = self.block()?;
        let span = start.to(body.span);
        Ok(Func {
            name,
            params,
            ret,
            body,
            span,
            comments,
        })
    }

    fn name(&mut self) -> PResult<Name> {
        match self.peek().clone() {
            Tok::Ident(text) => {
                let span = self.bump().span;
                Ok(Name { text, span })
            }
            _ => Err(self.error_expected("@what.name")),
        }
    }

    fn ty(&mut self) -> PResult<Type> {
        let ty = match self.peek() {
            Tok::Kw(Kw::Int) => Type::Int,
            Tok::Kw(Kw::Bool) => Type::Bool,
            Tok::Kw(Kw::Str) => Type::Str,
            _ => return Err(self.error_expected("@what.type")),
        };
        self.bump();
        Ok(ty)
    }

    // ---------- statements ----------

    fn block(&mut self) -> PResult<Block> {
        let open = self.expect(Tok::LBrace, "'{'")?;
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
            match self.stmt() {
                Ok(s) => stmts.push(s),
                Err(Reported) => self.synchronize(),
            }
        }
        let close_start = self.span().start;
        let trailing_comments = self.take_comments(close_start);
        let close = self.expect(Tok::RBrace, "'}'")?;
        Ok(Block {
            stmts,
            span: open.to(close),
            trailing_comments,
        })
    }

    fn stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        let comments = self.take_comments(start.start);
        let kind = self.stmt_kind()?;
        Ok(Stmt {
            kind,
            span: start.to(self.prev_span()),
            comments,
        })
    }

    fn stmt_kind(&mut self) -> PResult<StmtKind> {
        match self.peek().clone() {
            Tok::Kw(k @ (Kw::Let | Kw::Var)) => {
                self.bump();
                let name = self.name()?;
                let ty = if self.eat(&Tok::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.expect(Tok::Assign, "'='")?;
                let value = self.expr()?;
                self.expect_terminator()?;
                Ok(StmtKind::Let {
                    mutable: k == Kw::Var,
                    name,
                    ty,
                    value,
                })
            }
            Tok::Ident(_) if *self.peek_at(1) == Tok::Assign => {
                let name = self.name()?;
                self.bump(); // =
                let value = self.expr()?;
                self.expect_terminator()?;
                Ok(StmtKind::Assign { name, value })
            }
            Tok::Kw(k @ (Kw::If | Kw::While | Kw::Return | Kw::Print)) => {
                let span = self.span();
                let (word, pos) = self.keyword_word(span);
                if pos == Some(Position::Postfix) {
                    // Only a value-less postfix return may start a statement.
                    if k == Kw::Return && *self.peek_at(1) == Tok::Semi {
                        self.bump();
                        self.bump();
                        return Ok(StmtKind::Return(None));
                    }
                    return Err(self.wrong_position("E0202", span, k, &word));
                }
                self.bump();
                self.prefix_statement(k)
            }
            _ => {
                let expr = self.expr()?;
                self.after_expr(expr)
            }
        }
    }

    fn prefix_statement(&mut self, k: Kw) -> PResult<StmtKind> {
        match k {
            Kw::If => {
                let cond = self.expr()?;
                self.eat(&Tok::Kw(Kw::Then)); // optional "तो"
                self.finish_if(cond)
            }
            Kw::While => {
                let cond = self.expr()?;
                let body = self.block()?;
                Ok(StmtKind::While { cond, body })
            }
            Kw::Return => {
                let value = if *self.peek() == Tok::Semi {
                    None
                } else {
                    Some(self.expr()?)
                };
                self.expect_terminator()?;
                Ok(StmtKind::Return(value))
            }
            Kw::Print => {
                let mut values = vec![self.expr()?];
                while self.eat(&Tok::Comma) {
                    values.push(self.expr()?);
                }
                self.expect_terminator()?;
                Ok(StmtKind::Print(values))
            }
            _ => unreachable!("not a statement keyword"),
        }
    }

    /// An expression has been parsed at the start of a statement: it is
    /// either the operand of a postfix keyword or a plain expression statement.
    fn after_expr(&mut self, expr: Expr) -> PResult<StmtKind> {
        // `a, b छापो।` — a comma list is only valid before a postfix print.
        if *self.peek() == Tok::Comma {
            let comma = self.span();
            let mut values = vec![expr];
            while self.eat(&Tok::Comma) {
                values.push(self.expr()?);
            }
            if *self.peek() == Tok::Kw(Kw::Print)
                && self.keyword_word(self.span()).1 == Some(Position::Postfix)
            {
                self.bump();
                self.expect_terminator()?;
                return Ok(StmtKind::Print(values));
            }
            self.diags.push(
                Diagnostic::error("E0200", comma)
                    .arg("expected", "@what.stmt_end")
                    .arg("found", "','"),
            );
            return Err(Reported);
        }
        if let Tok::Kw(k @ (Kw::If | Kw::While | Kw::Return | Kw::Print)) = *self.peek() {
            let span = self.span();
            let (word, pos) = self.keyword_word(span);
            if pos != Some(Position::Postfix) {
                return Err(self.wrong_position("E0203", span, k, &word));
            }
            self.bump();
            return match k {
                Kw::If => self.finish_if(expr),
                Kw::While => {
                    let body = self.block()?;
                    Ok(StmtKind::While { cond: expr, body })
                }
                Kw::Return => {
                    self.expect_terminator()?;
                    Ok(StmtKind::Return(Some(expr)))
                }
                _ => {
                    self.expect_terminator()?;
                    Ok(StmtKind::Print(vec![expr]))
                }
            };
        }
        self.expect_terminator()?;
        Ok(StmtKind::Expr(expr))
    }

    fn finish_if(&mut self, cond: Expr) -> PResult<StmtKind> {
        let then = self.block()?;
        let els = if self.eat(&Tok::Kw(Kw::Else)) {
            if *self.peek() == Tok::LBrace {
                Some(Else::Block(self.block()?))
            } else {
                let start = self.span();
                let kind = self.stmt_kind()?;
                if !matches!(kind, StmtKind::If { .. }) {
                    return Err(self.error_expected("'{'"));
                }
                Some(Else::If(Box::new(Stmt {
                    kind,
                    span: start.to(self.prev_span()),
                    comments: vec![],
                })))
            }
        } else {
            None
        };
        Ok(StmtKind::If { cond, then, els })
    }

    fn wrong_position(&mut self, code: &'static str, span: Span, k: Kw, word: &str) -> Reported {
        let example = match (k, code) {
            (Kw::If | Kw::While, "E0202") => format!("x > 0 {word} {{ … }}"),
            (Kw::If | Kw::While, _) => format!("{word} x > 0 {{ … }}"),
            (_, "E0202") => format!("x {word}"),
            _ => format!("{word} x"),
        };
        self.diags.push(
            Diagnostic::error(code, span)
                .arg("word", word)
                .arg("example", example),
        );
        Reported
    }

    // ---------- expressions ----------

    fn expr(&mut self) -> PResult<Expr> {
        self.binary(1)
    }

    fn binop(&self) -> Option<BinOp> {
        Some(match self.peek() {
            Tok::Plus => BinOp::Add,
            Tok::Minus => BinOp::Sub,
            Tok::Star => BinOp::Mul,
            Tok::Slash => BinOp::Div,
            Tok::Percent => BinOp::Rem,
            Tok::EqEq => BinOp::Eq,
            Tok::NotEq => BinOp::Ne,
            Tok::Lt => BinOp::Lt,
            Tok::Le => BinOp::Le,
            Tok::Gt => BinOp::Gt,
            Tok::Ge => BinOp::Ge,
            Tok::AndAnd | Tok::Kw(Kw::And) => BinOp::And,
            Tok::OrOr | Tok::Kw(Kw::Or) => BinOp::Or,
            _ => return None,
        })
    }

    /// Precedence climbing.
    fn binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut left = self.unary()?;
        while let Some(op) = self.binop() {
            let prec = op.precedence();
            if prec < min_prec {
                break;
            }
            self.bump();
            let right = self.binary(prec + 1)?;
            let span = left.span.to(right.span);
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Bang | Tok::Kw(Kw::Not) => UnOp::Not,
            _ => return self.primary(),
        };
        let start = self.bump().span;
        let inner = self.unary()?;
        let span = start.to(inner.span);
        Ok(Expr {
            kind: ExprKind::Unary(op, Box::new(inner)),
            span,
        })
    }

    fn primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                ExprKind::Int(v)
            }
            Tok::Str(s) => {
                self.bump();
                ExprKind::Str(s)
            }
            Tok::Kw(Kw::True) => {
                self.bump();
                ExprKind::Bool(true)
            }
            Tok::Kw(Kw::False) => {
                self.bump();
                ExprKind::Bool(false)
            }
            Tok::LParen => {
                self.bump();
                let inner = self.expr()?;
                let close = self.expect(Tok::RParen, "')'")?;
                return Ok(Expr {
                    kind: inner.kind,
                    span: span.to(close),
                });
            }
            Tok::Ident(text) => {
                self.bump();
                if *self.peek() == Tok::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RParen {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                    }
                    let close = self.expect(Tok::RParen, "')'")?;
                    let name = Name { text, span };
                    return Ok(Expr {
                        kind: ExprKind::Call(name, args),
                        span: span.to(close),
                    });
                }
                ExprKind::Var(text)
            }
            _ => return Err(self.error_expected("@what.expr")),
        };
        Ok(Expr { kind, span })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Program {
        let p = parse(src, &Packs::builtin());
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        p.program
    }

    fn codes(src: &str) -> Vec<&'static str> {
        parse(src, &Packs::builtin())
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect()
    }

    fn body(p: &Program) -> &[Stmt] {
        &p.funcs[0].body.stmts
    }

    #[test]
    fn english_function() {
        let p = parse_ok("fn add(a: int, b: int) -> int { return a + b; }");
        let f = &p.funcs[0];
        assert_eq!(f.name.text, "add");
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.ret, Type::Int);
        assert!(matches!(body(&p)[0].kind, StmtKind::Return(Some(_))));
    }

    #[test]
    fn hindi_postfix_return_and_print() {
        let p = parse_ok("कार्य जोड़(क: पूर्णांक, ख: पूर्णांक) -> पूर्णांक { क + ख लौटाओ। }");
        assert!(matches!(body(&p)[0].kind, StmtKind::Return(Some(_))));
        let p = parse_ok("कार्य मुख्य() { \"नमस्ते\" छापो। }");
        assert!(
            matches!(&body(&p)[0].kind, StmtKind::Print(v) if v[0].kind == ExprKind::Str("नमस्ते".into()))
        );
    }

    #[test]
    fn hindi_if_with_optional_then() {
        let p = parse_ok("कार्य मुख्य() { अगर 1 < 2 तो { 1 छापो। } वरना { 2 छापो। } }");
        assert!(matches!(
            &body(&p)[0].kind,
            StmtKind::If {
                els: Some(Else::Block(_)),
                ..
            }
        ));
    }

    #[test]
    fn tamil_postfix_if_while_and_else_if_chain() {
        let src = "செயல்பாடு முதன்மை() {
            மாறி i = 0;
            i < 3 இருக்கும்வரை { i அச்சிடு; i = i + 1; }
            i == 0 என்றால் { 0 அச்சிடு; } இல்லையெனில் i == 3 என்றால் { 3 அச்சிடு; }
        }";
        let p = parse_ok(src);
        assert!(matches!(body(&p)[1].kind, StmtKind::While { .. }));
        assert!(matches!(
            &body(&p)[2].kind,
            StmtKind::If {
                els: Some(Else::If(_)),
                ..
            }
        ));
    }

    #[test]
    fn languages_can_be_mixed_in_one_file() {
        let p = parse_ok("fn main() { let x = 1; अगर x > 0 { x அச்சிடு; } }");
        assert_eq!(body(&p).len(), 2);
    }

    #[test]
    fn precedence_and_word_operators() {
        let p = parse_ok("fn main() { print 1 + 2 * 3 == 7 and not false; }");
        let StmtKind::Print(v) = &body(&p)[0].kind else {
            panic!()
        };
        // (((1 + (2*3)) == 7) and (not false))
        assert!(matches!(&v[0].kind, ExprKind::Binary(BinOp::And, l, _)
            if matches!(&l.kind, ExprKind::Binary(BinOp::Eq, ..))));
    }

    #[test]
    fn keyword_in_the_wrong_position_explains_word_order() {
        let d = parse("कार्य मुख्य() { लौटाओ 5। }", &Packs::builtin()).diagnostics;
        assert_eq!(d[0].code, "E0202");
        assert!(
            d[0].message("en").contains("`x लौटाओ`"),
            "{}",
            d[0].message("en")
        );
        assert_eq!(codes("fn main() { 5 return; }"), ["E0203"]);
    }

    #[test]
    fn print_takes_several_values_in_either_order() {
        let p = parse_ok("fn main() { print \"sum:\", 1 + 2; }");
        assert!(matches!(&body(&p)[0].kind, StmtKind::Print(v) if v.len() == 2));
        let p = parse_ok("कार्य मुख्य() { \"योग:\", 1 + 2 छापो। }");
        assert!(matches!(&body(&p)[0].kind, StmtKind::Print(v) if v.len() == 2));
        assert_eq!(codes("fn main() { 1, 2; }"), ["E0200"]);
    }

    #[test]
    fn bare_postfix_return() {
        let p = parse_ok("कार्य मुख्य() { लौटाओ। }");
        assert!(matches!(body(&p)[0].kind, StmtKind::Return(None)));
    }

    #[test]
    fn comments_attach_to_following_statement() {
        let p = parse_ok("// adds\nfn main() {\n  // first\n  print 1;\n  // end\n}\n// bye\n");
        assert_eq!(p.funcs[0].comments, [" adds"]);
        assert_eq!(body(&p)[0].comments, [" first"]);
        assert_eq!(p.funcs[0].body.trailing_comments, [" end"]);
        assert_eq!(p.trailing_comments, [" bye"]);
    }

    #[test]
    fn recovers_and_reports_several_errors() {
        let c = codes("fn main() { let = 1; print ; let y = 2; }\nfn other( { }");
        assert!(c.len() >= 3, "{c:?}");
        assert!(c.iter().all(|&c| c == "E0200"));
    }
}
