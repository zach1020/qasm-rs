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
        modifiers: Vec<GateModifier>,
        params: Vec<Expr>,
        args: Vec<GateOperand>,
        span: Span,
    },
    GateDef {
        name: String,
        params: Vec<String>,
        qparams: Vec<String>,
        body: Vec<Stmt>,
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
            | Stmt::GateDef { span, .. }
            | Stmt::Measure { span, .. }
            | Stmt::Reset { span, .. }
            | Stmt::Barrier { span, .. } => span,
        }
    }
}

/// A gate modifier: ctrl @, negctrl @, inv @, pow(k) @
#[derive(Debug, Clone)]
pub enum GateModifier {
    Ctrl(Option<Expr>, Span),
    NegCtrl(Option<Expr>, Span),
    Inv(Span),
    Pow(Expr, Span),
}

/// Expression tree for classical parameters (gate angles, etc.)
#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(u64, Span),
    FloatLit(f64, Span),
    Ident(String, Span),
    /// Built-in constants: pi, tau, euler
    Const(ConstKind, Span),
    Neg(Box<Expr>, Span),
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> &Span {
        match self {
            Expr::IntLit(_, s)
            | Expr::FloatLit(_, s)
            | Expr::Ident(_, s)
            | Expr::Const(_, s)
            | Expr::Neg(_, s)
            | Expr::BinOp { span: s, .. } => s,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

impl std::fmt::Display for BinOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::Pow => write!(f, "**"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ConstKind {
    Pi,
    Tau,
    Euler,
}

impl std::fmt::Display for ConstKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstKind::Pi => write!(f, "pi"),
            ConstKind::Tau => write!(f, "tau"),
            ConstKind::Euler => write!(f, "euler"),
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
