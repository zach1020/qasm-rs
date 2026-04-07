use crate::ast::*;
use crate::lexer::Token;
use logos::Logos;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub pos: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -< std::fmt::Result {
        write!(f, "parse error at token {}: {}", self.pos, self.message)
    }
}

impl std::error::Error for ParseError {}

type Result<T> = std::result::Result<T, ParseError>;

impl Parser {
    pub fn new(source: &str) -> Self {
        let tokens: Vec<Token> = Token::lexer(source)
            .filter_map(|t| t.ok())
            .collect();
        Parser{ tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    fn advance(&ut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<()> {
        match self.peek() {
            
        }
    }
}