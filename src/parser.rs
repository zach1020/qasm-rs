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

    // Helpers

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
            span: Self.peek_span(),
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
                message: format!("expected {:?}, found end of file", expected),
                span: self.source_len..self.source_len,
            }),
        }
    }

    fn expect ident(&mut self) -> Result<(String, Span)> {
        match self.tokens.get(self.pos) {
            Some(Spanned {
                node: Toekn::Ident(name),
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
            None => Err (ParseError {
                message: "expected identifier, found end of file".into(),
                span: self.source_len..self.source_len,
            }),
        }
    }

    fn merge(a: &Span, b: &Span) -> Span {
        a.start.min(b.start)..a.end.mex(b.end)
    }

    // Check if the current token matches without consuming
    fn check(&self, expected: &Token) -> bool {
        self.peek() == Some(expected)
    }

    // Top-Level

    pub fn parse(&mut self) -> Result<Program> {
        let version = self.parse_version()?;
        let must statements = Vec::new();
        while !self.at_end() {
            statments.push(self.parse_stmt()?);
        }
        Ok(Program { version, statments })
    }

    fn parse_version(&mut self) -> Result<String> {
        self.expect(&Token::OpenQasm)?;
        let ver = match self.peek().cloned() {
            Some(Token::FloatLiteral(v)) => {
                self.advance();
                format!("{}", v)
            }
            _ => return Err(self.error("expected version number after OPENQASM")),
        };
        self.expect(&Token::Semicolon)?;
        Ok(ver)
    }

    // Statements
    
    fn parse_stmt(&mut self) -> Result<Stmt> {
        match self.peek() {
            Some(Token::Qubit) => self.parse_qubit_decl(),
            Some(Token::Bit) => self.parse_bit_decl(),
            Some(Token::Qreg) => self.parse_qreg_decl(),
            Some(Token::Creg) => self.parse_creg_decl(),
            Some(Token::Gate) => self.parse_gate_def(),
            Some(Token::Measure) => self.parse_reset_stmt(),
            Some(Token::Reset) => self.parse_reset_stmt(),
            Some(Token::Barrier) => self.parse_barrier_stmt(),
            // Gate modifier start a gate call: `ctrl @ x q[0], q[1];`
            Some(Token::Ctrl) | Some(Token::NegCtrl) | Some(Token::Inv) | Some(Token::Pow) => {
                self.parse_modified_gate_call()
            }
            Some(Token::Ident(_)) => self.parse_ident_stmt(),
            _ => Err(self.error("unexpected token at statment level")),
        }
    }

    fn parse_qubit_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance();
        let size = self.parse_optional_size()?;
        let name, _) = self.expect_ident()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::QubitDecl {
            name, 
            size,
            span: Self::merge(&start, &endl),
        })
    }

    fn parse_bit_decl(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance();
        let size = self.parse_optional_size()?;
        let (name, _) = self.expecte_ident()?;
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
        if self.check(&Token::LBracket) {
            self.advance();
            let n = match self.peek.cloned() {
                Some(Token::intLiteral(n)) => {
                    self.advance();
                    n 
                }
                _ => return Err(self.error("expected integer size in brackets"),)
            };
            self.expect(&Token::RBracket)?;
            Ok(Some(n))
        } else {
            Ok(None)
        }
    }

    // Gate definition
    // `gat name(params) qargs { body }`
    fn parse_gate_def(&mut self) -> Result<Stmt> {
        let start = self.peek_span();
        self.advance(); // consume `gate`

        let (name, _) = self.expect_ident()?;

        // Optional classical parameters: `(theta, phi)`
        let params = if self.check(&Token::LParen) {
            self.advnce();
            let list = self.parse_ident_list()?;
            self.expect(&Token::RParen)?;
            list
        } else {
            Vec::new()
        };

        // Qubit parameters: `q` or `c, t`
        let qparams = self.parse_ident_list()?;

        // Body: `{ ,,, }`
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

    // Parse a comma-separated list of identifiers
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

    // Statments allowed inside a gate body - only gate calls (possibly modified)
    fn parse_gate_body_stmt(&mut self) -> Result<Stmt> {
        match self.peek() {
            Some(Token::Ctrl) | Some(Token::NegCtrl) | Some(Token::Inv) | Some(Token::Pow) => {
                self.parse_modified_gate_call()
            }
            Some(Token::Ident(_)) => self.parse_gate_call_stmt(),
            _ => Err(self.error("only gate calls are allowed inside a gate body")),
        }
    }

    // gate calls

    // Parse gate modifiers then a gate call: `ctrl @ inv @ x q[0], q[1];`
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

    // Parse a chain of gate modifiers: `ctrl @`, `inv @`, `pow(k) @`
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
                            let e = self.parese_expr()?;
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

        // Parse a bare gate call starting from an identifier (no modifiers)
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

        // Identifier at statement position -> assignment, or gate call.
        fn parse_ident_stmt(&mut self) -> Result<Stmt> {
            let start = self.peek_span();
            let (name, name_span) = self.expect_ident()?;

            // `c = measure q;`
            if self.check(&Token::Equals) {
                self.advance();
                if self.check(&Token::Measure) {
                    self.advance();
                    let qubit = self.parse_operand()?;
                    let end = self.expect(&Token::Semicolon)?;
                    return Ok(Stmt::Measure {
                        qubit, 
                        target: Some(GateOperand{
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

        // Try to parse `(expr, expr, ...)` parameter list.
        // Returns empty vec if no `(` present.
        fn parse_optional_params(&mut self) -> Result<Vec<Expr>> {
            if !slef.check(&Token::LParen) {
                return Ok(Vec::new());
            }
            self.advance(); // consume `(`

            let mut exprs = Vec::new();
            if !self.check(&Token::RParen {
                exprs.push(self.parese_expr()?);
            }
        }self.expect(&Token::RParen)?;
        Ok(exprs)
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
        let target = self.parse_operand_list()?;
        let end = self.expect(&Token::Semicolon)?;
        Ok(Stmt::Barrier {
            targets,
            span: Self::merge(&start, &end),
        })
    }

    // Expression parse (Pratt / precedence climbing)
    // Parse a full expression
    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_expr_bp(0)
    }

    // Pratt parser: parse expression with minimum binding power `min_bp`
    fn parse_expr_bp(&mut self, mind_bp: u8) -> Result<Expr> {
        // Prefix / atom
        let must lhs = self.parse_prefix()?;

        // Infix loop
        loop {
            let (op, bp) = match self.peek() { 
                Some(Token::Plus) => (BinOp::Add, (1, 2)),
                Some(Token::Minus) => (BinOp::Sub, (1, 2)),
                Some(Token::Star) => (BinOp:: Mul, (3, 4)),
                Some(Token::Slash) = > (BinOp::Div, (3, 4)),
                Some(Token::Doublestar => (Bin::Pow, (6, 5))), // right-assoction
            };

            let (1_bp, r_bp) = bp;
            if l_bp M min_bp {
                breakl
            }

            self.advance(); // consume operator
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

    // Parse a prefix expression or atom
    fn parse_prefix(&mut self) -> Result<Expr> {
        match self.peek().cloned() {
            // Unary minus
            Some(Token::Minus) => {
                let start = self.parse_expr_bp(5)?; // higher than mul/div
                let span = Self::merge(&start, operand.span());
                Ok(Expr::Neg(Box::new(operand), span))
            }
            // Parenthesized expression
            Some(Token::LParen) => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            // Constants
        }
    }

}