use crate::ast::*;
use crate::lexer::{self, Token};
use crate::span::{Span, Spanned};

pub struct Parser {
    tokens: Vec<Spanned<Token>>,
    pos: usize,
    /// Length of the original source (used for EOF spans).
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

    // ── helpers ──────────────────────────────────────────────

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

    /// Merge two spans into one covering both.
    fn merge(a: &Span, b: &Span) -> Span {
        a.start.min(b.start)..a.end.max(b.end)
    }

    // ── top-level ───────────────────────────────────────────

    pub fn parse(&mut self) -> Result<Program> {
        let version = self.parse_version()?;
        let mut statements = Vec::new();
        while !self.at_end() {
            statements.push(self.parse_stmt()?);
        }
        Ok(Program { version, statements })
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

    // ── statements ──────────────────────────────────────────

    fn parse_stmt(&mut self) -> Result<Stmt> {
        match self.peek() {
            Some(Token::Qubit) => self.parse_qubit_decl(),
            Some(Token::Bit) => self.parse_bit_decl(),
            Some(Token::Qreg) => self.parse_qreg_decl(),
            Some(Token::Creg) => self.parse_creg_decl(),
            Some(Token::Measure) => self.parse_measure_stmt(),
            Some(Token::Reset) => self.parse_reset_stmt(),
            Some(Token::Barrier) => self.parse_barrier_stmt(),
            Some(Token::Ident(_)) => self.parse_ident_stmt(),
            _ => Err(self.error("unexpected token at statement level")),
        }
    }

    fn parse_qubit_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance();
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
        self.advance();
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
        self.advance();
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
        self.advance();
        let (name, _) = self.expect_ident()?;
        let size = self.parse_optional_size()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::BitDecl {
            name,
            size,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_optional_size(&mut self) -> Result<Option<u64>> {
        if self.peek() == Some(&Token::LBracket) {
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

    fn parse_measure_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance();
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
        self.advance();
        let target = self.parse_operand()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::Reset {
            target,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_barrier_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance();
        let targets = self.parse_operand_list()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::Barrier {
            targets,
            span: Self::merge(&start, &end),
        })
    }

    fn parse_ident_stmt(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        let (name, name_span) = self.expect_ident()?;

        // `c = measure q;`
        if self.peek() == Some(&Token::Equals) {
            self.advance();
            if self.peek() == Some(&Token::Measure) {
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
            return Err(self.error(
                "only `name = measure ...` assignments supported so far",
            ));
        }

        // gate call
        let args = self.parse_operand_list()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::GateCall {
            name,
            args,
            span: Self::merge(&start, &end),
        })
    }

    // ── operands ────────────────────────────────────────────

    fn parse_operand(&mut self) -> Result<GateOperand> {
        let (name, name_span) = self.expect_ident()?;
        let (index, end_span) = if self.peek() == Some(&Token::LBracket) {
            self.advance();
            let n = match self.peek().cloned() {
                Some(Token::IntLiteral(n)) => {
                    self.advance();
                    n
                }
                _ => return Err(self.error("expected integer index")),
            };
            let rb = self.expect(&Token::RBracket)?;
            (Some(n), rb)
        } else {
            (None, name_span.clone())
        };
        Ok(GateOperand {
            name,
            index,
            span: Self::merge(&name_span, &end_span),
        })
    }

    fn parse_operand_list(&mut self) -> Result<Vec<GateOperand>> {
        let mut ops = vec![self.parse_operand()?];
        while self.peek() == Some(&Token::Comma) {
            self.advance();
            ops.push(self.parse_operand()?);
        }
        Ok(ops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bell_pair() {
        let source = r#"
        OPENQASM 3.0;
        qubit[2] q;
        bit[2] c;
        h q[0];
        cx q[0], q[1];
        c = measure q;
        "#;
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("should parse");
        assert_eq!(program.statements.len(), 5);
        // Verify spans are populated
        for stmt in &program.statements {
            let s = stmt.span();
            assert!(s.start < s.end, "span should be non-empty: {:?}", s);
        }
    }

    #[test]
    fn parse_single_qubit() {
        let source = "OPENQASM 3.0; qubit q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("should parse");
        assert_eq!(program.statements.len(), 1);
    }

    #[test]
    fn parse_reset_barrier() {
        let source = "OPENQASM 3.0; qubit[2] q; reset q[0]; barrier q[0], q[1];";
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("should parse");
        assert_eq!(program.statements.len(), 3);
    }

    #[test]
    fn error_missing_semicolon() {
        let source = "OPENQASM 3.0; qubit q";
        let mut parser = Parser::new(source);
        let err = parser.parse().unwrap_err();
        // The error span should point at EOF
        assert_eq!(err.span.start, source.len());
    }

    #[test]
    fn error_bad_token_has_span() {
        let source = "OPENQASM 3.0; qubit[2] q; 42 q;";
        let mut parser = Parser::new(source);
        let err = parser.parse().unwrap_err();
        // Should point at the `42` token
        assert!(err.span.start > 0);
        assert!(err.span.start < source.len());
    }
}
