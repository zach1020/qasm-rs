use crate::ast::*;
use crate::lexer::{self, Token};
use crate::span::{Span, Spanned};

pub struct Parser {
    tokens: Vec<Spanned<Token>>,
    pos: usize,
    source_len: usize,
}

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "error at {}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for ParseError {}

type Result<T> = std::result::Result<T, ParseError>;

impl Parser {
    pub fn new(source: &str) -> Self {
        let (tokens, _lex_errors) = lexer::lex(source);
        Parser {
            tokens,
            pos: 0,
            source_len: source.len(),
        }
    }

    // ── Helpers ──────────────────────────────────────────────

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|s| &s.node)
    }

    fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|s| s.span.clone())
            .unwrap_or(self.source_len..self.source_len)
    }

    fn advance(&mut self) -> Option<&Spanned<Token>> {
        let tok = self.tokens.get(self.pos);
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn prev_span(&self) -> Span {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span.clone()
        } else {
            0..0
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn error(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            message: msg.into(),
            span: self.peek_span(),
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<Span> {
        match self.tokens.get(self.pos) {
            Some(s) if &s.node == expected => {
                let span = s.span.clone();
                self.pos += 1;
                Ok(span)
            }
            Some(s) => Err(ParseError {
                message: format!("expected {:?}, found {:?}", expected, s.node),
                span: s.span.clone(),
            }),
            None => Err(ParseError {
                message: format!("expected {:?}, found end of file", expected),
                span: self.source_len..self.source_len,
            }),
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span)> {
        match self.tokens.get(self.pos) {
            Some(Spanned {
                node: Token::Ident(name),
                span,
            }) => {
                let name = name.clone();
                let span = span.clone();
                self.pos += 1;
                Ok((name, span))
            }
            Some(s) => Err(ParseError {
                message: format!("expected identifier, found {:?}", s.node),
                span: s.span.clone(),
            }),
            None => Err(ParseError {
                message: "expected identifier, found end of file".into(),
                span: self.source_len..self.source_len,
            }),
        }
    }

    fn merge(a: &Span, b: &Span) -> Span {
        a.start.min(b.start)..a.end.max(b.end)
    }

    fn check(&self, expected: &Token) -> bool {
        self.peek() == Some(expected)
    }

    // ── Top-level ───────────────────────────────────────────

    pub fn parse(&mut self) -> Result<Program> {
        let version = self.parse_version()?;
        let mut statements = Vec::new();
        while !self.at_end() {
            statements.push(self.parse_stmt()?);
        }
        Ok(Program {
            version,
            statements,
        })
    }

    fn parse_version(&mut self) -> Result<String> {
        self.expect(&Token::OpenQasm)?;
        let ver = match self.peek().cloned() {
            Some(Token::FloatLiteral(v)) => {
                self.advance();
                format!("{}", v)
            }
            Some(Token::IntLiteral(v)) => {
                self.advance();
                format!("{}", v)
            }
            _ => return Err(self.error("expected version number after OPENQASM")),
        };
        self.expect(&Token::Semicolon)?;
        Ok(ver)
    }

    // ── Statements ──────────────────────────────────────────

    fn parse_stmt(&mut self) -> Result<Stmt> {
        match self.peek() {
            Some(Token::Qubit) => self.parse_qubit_decl(),
            Some(Token::Bit) => self.parse_bit_decl(),
            Some(Token::Qreg) => self.parse_qreg_decl(),
            Some(Token::Creg) => self.parse_creg_decl(),
            Some(Token::Int) | Some(Token::Float) | Some(Token::Bool) => {
                self.parse_classical_decl()
            }
            Some(Token::Gate) => self.parse_gate_def(),
            Some(Token::Measure) => self.parse_measure_stmt(),
            Some(Token::Reset) => self.parse_reset_stmt(),
            Some(Token::Barrier) => self.parse_barrier_stmt(),
            Some(Token::If) => self.parse_if_stmt(),
            Some(Token::For) => self.parse_for_stmt(),
            Some(Token::While) => self.parse_while_stmt(),
            Some(Token::Ctrl)
            | Some(Token::NegCtrl)
            | Some(Token::Inv)
            | Some(Token::Pow) => self.parse_modified_gate_call(),
            Some(Token::Ident(_)) => self.parse_ident_stmt(),
            _ => Err(self.error("unexpected token at statement level")),
        }
    }

    fn parse_qubit_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `qubit`
        let size = self.parse_optional_size()?;
        let (name, _) = self.expect_ident()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::QubitDecl {
            name,
            size,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_bit_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `bit`
        let size = self.parse_optional_size()?;
        let (name, _) = self.expect_ident()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::BitDecl {
            name,
            size,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_qreg_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `qreg`
        let (name, _) = self.expect_ident()?;
        let size = self.parse_optional_size()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::QubitDecl {
            name,
            size,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_creg_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `creg`
        let (name, _) = self.expect_ident()?;
        let size = self.parse_optional_size()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::BitDecl {
            name,
            size,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_classical_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        let ty = match self.peek() {
            Some(Token::Int) => ClassicalType::Int,
            Some(Token::Float) => ClassicalType::Float,
            Some(Token::Bool) => ClassicalType::Bool,
            _ => return Err(self.error("expected classical type")),
        };
        self.advance(); // consume type keyword
        let (name, _) = self.expect_ident()?;

        let init = if self.check(&Token::Equals) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };

        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::ClassicalDecl {
            ty,
            name,
            init,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_optional_size(&mut self) -> Result<Option<u64>> {
        if self.check(&Token::LBracket) {
            self.advance();
            let n = match self.peek().cloned() {
                Some(Token::IntLiteral(n)) => {
                    self.advance();
                    n
                }
                _ => return Err(self.error("expected integer size in brackets")),
            };
            self.expect(&Token::RBracket)?;
            Ok(Some(n))
        } else {
            Ok(None)
        }
    }

    // ── Classical control flow ──────────────────────────────

    fn parse_if_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `if`
        self.expect(&Token::LParen)?;
        let condition = self.parse_expr()?;
        self.expect(&Token::RParen)?;
        let then_body = self.parse_block()?;

        let else_body = if self.check(&Token::Else) {
            self.advance();
            Some(self.parse_block()?)
        } else {
            None
        };

        let end = self.prev_span();
        Ok(Stmt::If {
            condition,
            then_body,
            else_body,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `for`

        // `for int i in [start:end] { ... }`
        let var_ty = match self.peek() {
            Some(Token::Int) => ClassicalType::Int,
            Some(Token::Float) => ClassicalType::Float,
            Some(Token::Bool) => ClassicalType::Bool,
            _ => return Err(self.error("expected type in for loop")),
        };
        self.advance();
        let (var_name, _) = self.expect_ident()?;
        self.expect(&Token::In)?;

        // Range: [start:end] or [start:step:end]
        self.expect(&Token::LBracket)?;
        let range_start = self.parse_expr()?;
        self.expect(&Token::Colon)?;
        let second = self.parse_expr()?;

        let range = if self.check(&Token::Colon) {
            self.advance();
            let third = self.parse_expr()?;
            ForRange {
                start: range_start,
                end: third,
                step: Some(second),
            }
        } else {
            ForRange {
                start: range_start,
                end: second,
                step: None,
            }
        };
        self.expect(&Token::RBracket)?;

        let body = self.parse_block()?;
        let end = self.prev_span();

        Ok(Stmt::For {
            var_name,
            var_ty,
            range,
            body,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_while_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `while`
        self.expect(&Token::LParen)?;
        let condition = self.parse_expr()?;
        self.expect(&Token::RParen)?;
        let body = self.parse_block()?;
        let end = self.prev_span();

        Ok(Stmt::While {
            condition,
            body,
            span: Self::merge(&start, &end),
        })
    }

    /// Parse a `{ stmt; stmt; ... }` block.
    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.expect(&Token::LBrace)?;
        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) && !self.at_end() {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    // ── Gate definition ─────────────────────────────────────

    fn parse_gate_def(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `gate`

        let (name, _) = self.expect_ident()?;

        // Optional classical parameters: `(theta, phi)`
        let params = if self.check(&Token::LParen) {
            self.advance();
            let list = self.parse_ident_list()?;
            self.expect(&Token::RParen)?;
            list
        } else {
            Vec::new()
        };

        // Qubit parameters: `q` or `c, t`
        let qparams = self.parse_ident_list()?;

        // Body: `{ ... }`
        self.expect(&Token::LBrace)?;
        let mut body = Vec::new();
        while !self.check(&Token::RBrace) && !self.at_end() {
            body.push(self.parse_gate_body_stmt()?);
        }
        let end = self.expect(&Token::RBrace)?;

        Ok(Stmt::GateDef {
            name,
            params,
            qparams,
            body,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_ident_list(&mut self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        if let Some(Token::Ident(_)) = self.peek() {
            let (name, _) = self.expect_ident()?;
            names.push(name);
            while self.check(&Token::Comma) {
                self.advance();
                let (name, _) = self.expect_ident()?;
                names.push(name);
            }
        }
        Ok(names)
    }

    fn parse_gate_body_stmt(&mut self) -> Result<Stmt> {
        match self.peek() {
            Some(Token::Ctrl)
            | Some(Token::NegCtrl)
            | Some(Token::Inv)
            | Some(Token::Pow) => self.parse_modified_gate_call(),
            Some(Token::Ident(_)) => self.parse_gate_call_stmt(),
            _ => Err(self.error("only gate calls are allowed inside a gate body")),
        }
    }

    // ── Gate calls ──────────────────────────────────────────

    fn parse_modified_gate_call(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        let modifiers = self.parse_gate_modifiers()?;
        let (name, _) = self.expect_ident()?;

        let params = self.parse_optional_params()?;
        let args = self.parse_operand_list()?;
        let end = self.expect(&Token::Semicolon)?;

        Ok(Stmt::GateCall {
            name,
            modifiers,
            params,
            args,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_gate_modifiers(&mut self) -> Result<Vec<GateModifier>> {
        let mut mods = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Ctrl) => {
                    let span = self.peek_span();
                    self.advance();
                    let arg = if self.check(&Token::LParen) {
                        self.advance();
                        let e = self.parse_expr()?;
                        self.expect(&Token::RParen)?;
                        Some(e)
                    } else {
                        None
                    };
                    let at = self.expect(&Token::At)?;
                    mods.push(GateModifier::Ctrl(arg, Self::merge(&span, &at)));
                }
                Some(Token::NegCtrl) => {
                    let span = self.peek_span();
                    self.advance();
                    let arg = if self.check(&Token::LParen) {
                        self.advance();
                        let e = self.parse_expr()?;
                        self.expect(&Token::RParen)?;
                        Some(e)
                    } else {
                        None
                    };
                    let at = self.expect(&Token::At)?;
                    mods.push(GateModifier::NegCtrl(arg, Self::merge(&span, &at)));
                }
                Some(Token::Inv) => {
                    let span = self.peek_span();
                    self.advance();
                    let at = self.expect(&Token::At)?;
                    mods.push(GateModifier::Inv(Self::merge(&span, &at)));
                }
                Some(Token::Pow) => {
                    let span = self.peek_span();
                    self.advance();
                    self.expect(&Token::LParen)?;
                    let e = self.parse_expr()?;
                    self.expect(&Token::RParen)?;
                    let at = self.expect(&Token::At)?;
                    mods.push(GateModifier::Pow(e, Self::merge(&span, &at)));
                }
                _ => break,
            }
        }
        Ok(mods)
    }

    fn parse_gate_call_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        let (name, _) = self.expect_ident()?;

        let params = self.parse_optional_params()?;
        let args = self.parse_operand_list()?;
        let end = self.expect(&Token::Semicolon)?;

        Ok(Stmt::GateCall {
            name,
            modifiers: Vec::new(),
            params,
            args,
            span: Self::merge(&start, &end),
        })
    }

    /// Identifier at statement position → assignment or gate call.
    fn parse_ident_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        let (name, name_span) = self.expect_ident()?;

        // Assignment: `x = expr;` or `x += expr;` or `x -= expr;`
        let assign_op = match self.peek() {
            Some(Token::Equals) => Some(AssignOp::Assign),
            Some(Token::PlusEquals) => Some(AssignOp::AddAssign),
            Some(Token::MinusEquals) => Some(AssignOp::SubAssign),
            _ => None,
        };

        if let Some(op) = assign_op {
            self.advance();

            // Special case: `c = measure q;`
            if op == AssignOp::Assign && self.check(&Token::Measure) {
                self.advance();
                let qubit = self.parse_operand()?;
                let end = self.expect(&Token::Semicolon)?;
                return Ok(Stmt::Measure {
                    qubit,
                    target: Some(GateOperand {
                        name,
                        index: None,
                        span: name_span,
                    }),
                    span: Self::merge(&start, &end),
                });
            }

            let value = self.parse_expr()?;
            let end = self.expect(&Token::Semicolon)?;
            return Ok(Stmt::Assignment {
                name,
                op,
                value,
                span: Self::merge(&start, &end),
            });
        }

        // Gate call with optional params: `rx(pi/2) q[0];`
        let params = self.parse_optional_params()?;
        let args = self.parse_operand_list()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::GateCall {
            name,
            modifiers: Vec::new(),
            params,
            args,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_optional_params(&mut self) -> Result<Vec<Expr>> {
        if !self.check(&Token::LParen) {
            return Ok(Vec::new());
        }
        self.advance();

        let mut exprs = Vec::new();
        if !self.check(&Token::RParen) {
            exprs.push(self.parse_expr()?);
            while self.check(&Token::Comma) {
                self.advance();
                exprs.push(self.parse_expr()?);
            }
        }
        self.expect(&Token::RParen)?;
        Ok(exprs)
    }

    fn parse_measure_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `measure`
        let qubit = self.parse_operand()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::Measure {
            qubit,
            target: None,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_reset_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `reset`
        let target = self.parse_operand()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::Reset {
            target,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_barrier_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `barrier`
        let targets = self.parse_operand_list()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::Barrier {
            targets,
            span: Self::merge(&start, &end),
        })
    }

    // ── Operands ────────────────────────────────────────────

    fn parse_operand(&mut self) -> Result<GateOperand> {
        let (name, name_span) = self.expect_ident()?;
        if self.check(&Token::LBracket) {
            self.advance();
            let idx = match self.peek().cloned() {
                Some(Token::IntLiteral(n)) => {
                    self.advance();
                    n
                }
                _ => return Err(self.error("expected integer index")),
            };
            let end = self.expect(&Token::RBracket)?;
            Ok(GateOperand {
                name,
                index: Some(idx),
                span: Self::merge(&name_span, &end),
            })
        } else {
            Ok(GateOperand {
                name,
                index: None,
                span: name_span,
            })
        }
    }

    fn parse_operand_list(&mut self) -> Result<Vec<GateOperand>> {
        let mut ops = Vec::new();
        if matches!(self.peek(), Some(Token::Ident(_))) {
            ops.push(self.parse_operand()?);
            while self.check(&Token::Comma) {
                self.advance();
                ops.push(self.parse_operand()?);
            }
        }
        Ok(ops)
    }

    // ── Expression parser (Pratt / precedence climbing) ─────

    pub fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_comparison()
    }

    /// Comparison operators have the lowest precedence.
    fn parse_comparison(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_expr_bp(0)?;

        loop {
            let op = match self.peek() {
                Some(Token::DoubleEquals) => CompareOp::Eq,
                Some(Token::NotEquals) => CompareOp::Ne,
                Some(Token::Less) => CompareOp::Lt,
                Some(Token::LessEquals) => CompareOp::Le,
                Some(Token::Greater) => CompareOp::Gt,
                Some(Token::GreaterEquals) => CompareOp::Ge,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_expr_bp(0)?;
            let span = Self::merge(lhs.span(), rhs.span());
            lhs = Expr::Compare {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }

        Ok(lhs)
    }

    /// Pratt parser: parse expression with minimum binding power `min_bp`.
    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr> {
        let mut lhs = self.parse_prefix()?;

        loop {
            let (op, l_bp, r_bp) = match self.peek() {
                Some(Token::Plus) => (BinOp::Add, 1, 2),
                Some(Token::Minus) => (BinOp::Sub, 1, 2),
                Some(Token::Star) => (BinOp::Mul, 3, 4),
                Some(Token::Slash) => (BinOp::Div, 3, 4),
                Some(Token::DoubleStar) => (BinOp::Pow, 6, 5), // right-associative
                _ => break,
            };

            if l_bp < min_bp {
                break;
            }

            self.advance();
            let rhs = self.parse_expr_bp(r_bp)?;
            let span = Self::merge(lhs.span(), rhs.span());
            lhs = Expr::BinOp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }

        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<Expr> {
        match self.peek().cloned() {
            Some(Token::Minus) => {
                let start = self.peek_span();
                self.advance();
                let operand = self.parse_expr_bp(5)?;
                let span = Self::merge(&start, operand.span());
                Ok(Expr::Neg(Box::new(operand), span))
            }
            Some(Token::LParen) => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Some(Token::IntLiteral(n)) => {
                let span = self.peek_span();
                self.advance();
                Ok(Expr::IntLit(n, span))
            }
            Some(Token::FloatLiteral(f)) => {
                let span = self.peek_span();
                self.advance();
                Ok(Expr::FloatLit(f, span))
            }
            Some(Token::True) => {
                let span = self.peek_span();
                self.advance();
                Ok(Expr::BoolLit(true, span))
            }
            Some(Token::False) => {
                let span = self.peek_span();
                self.advance();
                Ok(Expr::BoolLit(false, span))
            }
            Some(Token::Ident(ref name)) => {
                let span = self.peek_span();
                let name = name.clone();
                self.advance();
                match name.as_str() {
                    "pi" => Ok(Expr::Const(ConstKind::Pi, span)),
                    "tau" => Ok(Expr::Const(ConstKind::Tau, span)),
                    "euler" => Ok(Expr::Const(ConstKind::Euler, span)),
                    _ => Ok(Expr::Ident(name, span)),
                }
            }
            _ => Err(self.error("expected expression")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bell_pair() {
        let source = "OPENQASM 3.0; qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 6);
    }

    #[test]
    fn parse_scalar_qubit() {
        let source = "OPENQASM 3.0; qubit q; h q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 2);
    }

    #[test]
    fn parse_measure_and_reset() {
        let source = "OPENQASM 3.0; qubit q; measure q; reset q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 3);
    }

    #[test]
    fn parse_barrier() {
        let source = "OPENQASM 3.0; qubit[3] q; barrier q[0], q[1], q[2];";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 2);
    }

    #[test]
    fn parse_parameterized_gate() {
        let source = "OPENQASM 3.0; qubit q; rx(pi/2) q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            Stmt::GateCall { name, params, .. } => {
                assert_eq!(name, "rx");
                assert_eq!(params.len(), 1);
            }
            _ => panic!("expected gate call"),
        }
    }

    #[test]
    fn parse_gate_def() {
        let source = "OPENQASM 3.0; gate h q { u3(pi/2, 0, pi) q; }";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 1);
        match &program.statements[0] {
            Stmt::GateDef {
                name,
                params,
                qparams,
                body,
                ..
            } => {
                assert_eq!(name, "h");
                assert!(params.is_empty());
                assert_eq!(qparams, &["q"]);
                assert_eq!(body.len(), 1);
            }
            _ => panic!("expected gate def"),
        }
    }

    #[test]
    fn parse_modified_gate() {
        let source = "OPENQASM 3.0; qubit[2] q; ctrl @ x q[0], q[1];";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            Stmt::GateCall {
                name, modifiers, ..
            } => {
                assert_eq!(name, "x");
                assert_eq!(modifiers.len(), 1);
            }
            _ => panic!("expected modified gate call"),
        }
    }

    #[test]
    fn parse_expression_precedence() {
        let source = "OPENQASM 3.0; qubit q; rx(pi/2 + 1) q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        match &program.statements[1] {
            Stmt::GateCall { params, .. } => {
                assert_eq!(params.len(), 1);
                match &params[0] {
                    Expr::BinOp {
                        op: BinOp::Add, ..
                    } => {}
                    other => panic!("expected Add at top, got {:?}", other),
                }
            }
            _ => panic!("expected gate call"),
        }
    }

    #[test]
    fn parse_qreg_creg_compat() {
        let source = "OPENQASM 3.0; qreg q[2]; creg c[2]; h q[0];";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 3);
    }

    #[test]
    fn parse_classical_decl() {
        let source = "OPENQASM 3.0; int x = 42; float y; bool flag = true;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 3);
        match &program.statements[0] {
            Stmt::ClassicalDecl { ty, name, init, .. } => {
                assert_eq!(*ty, ClassicalType::Int);
                assert_eq!(name, "x");
                assert!(init.is_some());
            }
            _ => panic!("expected classical decl"),
        }
    }

    #[test]
    fn parse_assignment() {
        let source = "OPENQASM 3.0; int x = 0; x = 5; x += 1;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 3);
        match &program.statements[2] {
            Stmt::Assignment { name, op, .. } => {
                assert_eq!(name, "x");
                assert_eq!(*op, AssignOp::AddAssign);
            }
            _ => panic!("expected assignment"),
        }
    }

    #[test]
    fn parse_if_else() {
        let source = "OPENQASM 3.0; qubit q; bit c; c = measure q; if (c == 1) { h q; } else { x q; }";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 4);
        match &program.statements[3] {
            Stmt::If { else_body, .. } => {
                assert!(else_body.is_some());
            }
            _ => panic!("expected if stmt"),
        }
    }

    #[test]
    fn parse_for_loop() {
        let source = "OPENQASM 3.0; qubit[4] q; for int i in [0:4] { h q; }";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            Stmt::For { var_name, .. } => {
                assert_eq!(var_name, "i");
            }
            _ => panic!("expected for stmt"),
        }
    }

    #[test]
    fn parse_while_loop() {
        let source = "OPENQASM 3.0; int count = 0; while (count < 10) { count += 1; }";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        assert_eq!(program.statements.len(), 2);
        match &program.statements[1] {
            Stmt::While { .. } => {}
            _ => panic!("expected while stmt"),
        }
    }

    #[test]
    fn parse_comparison_expr() {
        let source = "OPENQASM 3.0; int x = 0; if (x == 0) { x = 1; }";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        match &program.statements[1] {
            Stmt::If { condition, .. } => match condition {
                Expr::Compare { op, .. } => assert_eq!(*op, CompareOp::Eq),
                other => panic!("expected comparison, got {:?}", other),
            },
            _ => panic!("expected if"),
        }
    }
}
