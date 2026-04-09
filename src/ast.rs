use crate::span::Span;

/// AST for OpenQASM 3 subset.

#[derive(Debug, Clone)]
pub struct Program {
    pub version: String,
    pub statements: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    QubitDecl {
        name: String,
        size: Option<u64>,
        span: Span,
    },
    BitDecl {
        name: String,
        size: Option<u64>,
        span: Span,
    },
    GateCall {
        name: String,
        args: Vec<GateOperand>,
        span: Span,
    },
    Measure {
        qubit: GateOperand,
        target: Option<GateOperand>,
        span: Span,
    },
    Reset {
        target: GateOperand,
        span: Span,
    },
    Barrier {
        targets: Vec<GateOperand>,
        span: Span,
    },
}

impl Stmt {
    pub fn span(&self) -> &Span {
        match self {
            Stmt::QubitDecl { span, .. }
            | Stmt::BitDecl { span, .. }
            | Stmt::GateCall { span, .. }
            | Stmt::Measure { span, .. }
            | Stmt::Reset { span, .. }
            | Stmt::Barrier { span, .. } => span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GateOperand {
    pub name: String,
    pub index: Option<u64>,
    pub span: Span,
}

impl std::fmt::Display for GateOperand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            Some(i) => write!(f, "{}[{}]", self.name, i),
            None => write!(f, "{}", self.name),
        }
    }
}
