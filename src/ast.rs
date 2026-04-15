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
    ClassicalDecl {
        ty: ClassicalType,
        name: String,
        init: Option<Expr>,
        span: Span,
    },
    Assignment {
        name: String,
        op: AssignOp,
        value: Expr,
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
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        span: Span,
    },
    For {
        var_name: String,
        var_ty: ClassicalType,
        range: ForRange,
        body: Vec<Stmt>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
}

impl Stmt {
    pub fn span(&self) -> &Span {
        match self {
            Stmt::QubitDecl { span, .. }
            | Stmt::BitDecl { span, .. }
            | Stmt::ClassicalDecl { span, .. }
            | Stmt::Assignment { span, .. }
            | Stmt::GateCall { span, .. }
            | Stmt::GateDef { span, .. }
            | Stmt::Measure { span, .. }
            | Stmt::Reset { span, .. }
            | Stmt::Barrier { span, .. }
            | Stmt::If { span, .. }
            | Stmt::For { span, .. }
            | Stmt::While { span, .. } => span,
        }
    }
}

/// Classical type specifier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClassicalType {
    Int,
    Float,
    Bool,
}

impl std::fmt::Display for ClassicalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassicalType::Int => write!(f, "int"),
            ClassicalType::Float => write!(f, "float"),
            ClassicalType::Bool => write!(f, "bool"),
        }
    }
}

/// Assignment operator kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp {
    Assign,    // =
    AddAssign, // +=
    SubAssign, // -=
}

impl std::fmt::Display for AssignOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssignOp::Assign => write!(f, "="),
            AssignOp::AddAssign => write!(f, "+="),
            AssignOp::SubAssign => write!(f, "-="),
        }
    }
}

/// Range expression for `for` loops: `[start:end]` or `[start:step:end]`.
#[derive(Debug, Clone)]
pub struct ForRange {
    pub start: Expr,
    pub end: Expr,
    pub step: Option<Expr>,
}

/// A gate modifier: ctrl @, negctrl @, inv @, pow(k) @
#[derive(Debug, Clone)]
pub enum GateModifier {
    Ctrl(Option<Expr>, Span),
    NegCtrl(Option<Expr>, Span),
    Inv(Span),
    Pow(Expr, Span),
}

/// Expression tree for classical values.
#[derive(Debug, Clone)]
pub enum Expr {
    IntLit(u64, Span),
    FloatLit(f64, Span),
    BoolLit(bool, Span),
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
    Compare {
        op: CompareOp,
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
            | Expr::BoolLit(_, s)
            | Expr::Ident(_, s)
            | Expr::Const(_, s)
            | Expr::Neg(_, s)
            | Expr::BinOp { span: s, .. }
            | Expr::Compare { span: s, .. } => s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl std::fmt::Display for CompareOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompareOp::Eq => write!(f, "=="),
            CompareOp::Ne => write!(f, "!="),
            CompareOp::Lt => write!(f, "<"),
            CompareOp::Le => write!(f, "<="),
            CompareOp::Gt => write!(f, ">"),
            CompareOp::Ge => write!(f, ">="),
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
