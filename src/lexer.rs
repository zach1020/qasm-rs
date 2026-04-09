use logos::Logos;
use crate::span::{Span, Spanned};

#[derive(Logos, Debug, PartialEq, Clone)]
#[logos(skip r"[ \t\r\n\f]+")]
#[logos(skip r"//[^\n]*")]
#[logos(skip r"/\*[^*]*\*+(?:[^/*][^*]*\*+)*/")]
pub enum Token {
    // === Keywords ===
    #[token("OPENQASM")]
    OpenQasm,
    #[token("include")]
    Include,
    #[token("qubit")]
    Qubit,
    #[token("bit")]
    Bit,
    #[token("gate")]
    Gate,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("measure")]
    Measure,
    #[token("reset")]
    Reset,
    #[token("barrier")]
    Barrier,
    #[token("let")]
    Let,
    #[token("const")]
    Const,
    #[token("int")]
    Int,
    #[token("float")]
    Float,
    #[token("bool")]
    Bool,
    #[token("return")]
    Return,
    #[token("def")]
    Def,
    #[token("for")]
    For,
    #[token("while")]
    While,
    #[token("in")]
    In,
    #[token("input")]
    Input,
    #[token("output")]
    Output,
    #[token("creg")]
    Creg,
    #[token("qreg")]
    Qreg,

    // === Literals ===
    #[regex(r"[0-9]+\.[0-9]*([eE][+-]?[0-9]+)?", |lex| lex.slice().parse::<f64>().ok())]
    FloatLiteral(f64),

    #[regex(r"[0-9]+", |lex| lex.slice().parse::<u64>().ok(), priority = 2)]
    IntLiteral(u64),

    #[regex(r#""[^"]*""#, |lex| {
        let s = lex.slice();
        Some(s[1..s.len()-1].to_string())
    })]
    StringLiteral(String),

    // === Identifiers ===
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string(), priority = 1)]
    Ident(String),

    // === Punctuation & Operators ===
    #[token(";")]
    Semicolon,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token(":")]
    Colon,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("->")]
    Arrow,
    #[token("=")]
    Equals,
    #[token("==")]
    DoubleEquals,
    #[token("!=")]
    NotEquals,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("@")]
    At,
    #[token("**")]
    DoubleStar,
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Ident(s) => write!(f, "Ident({})", s),
            Token::IntLiteral(n) => write!(f, "Int({})", n),
            Token::FloatLiteral(n) => write!(f, "Float({})", n),
            Token::StringLiteral(s) => write!(f, "Str(\"{}\")", s),
            other => write!(f, "{:?}", other),
        }
    }
}

/// Lex source into spanned tokens, collecting errors for invalid bytes.
pub fn lex(source: &str) -> (Vec<Spanned<Token>>, Vec<Span>) {
    let mut tokens = Vec::new();
    let mut errors = Vec::new();
    let mut lexer = Token::lexer(source);

    while let Some(result) = lexer.next() {
        let span = lexer.span();
        match result {
            Ok(tok) => tokens.push(Spanned::new(tok, span)),
            Err(()) => errors.push(span),
        }
    }

    (tokens, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_bell_pair() {
        let source = r#"
        OPENQASM 3.0;
        qubit[2] q;
        bit[2] c;
        h q[0];
        cx q[0], q[1];
        c = measure q;
        "#;

        let (tokens, errors) = lex(source);
        assert!(errors.is_empty());
        assert_eq!(tokens[0].node, Token::OpenQasm);
        assert_eq!(tokens[1].node, Token::FloatLiteral(3.0));
        // Verify spans are non-empty
        assert!(tokens[0].span.start < tokens[0].span.end);
    }

    #[test]
    fn lex_comments() {
        let source = "qubit q; // this is a comment\n/* block */ bit c;";
        let (tokens, errors) = lex(source);
        assert!(errors.is_empty());
        assert_eq!(tokens[0].node, Token::Qubit);
        assert_eq!(tokens[3].node, Token::Bit);
    }

    #[test]
    fn lex_error_collected() {
        let source = "qubit # q;";
        let (tokens, errors) = lex(source);
        assert_eq!(errors.len(), 1);
        assert_eq!(tokens.len(), 3); // qubit, q, ;
    }
}
