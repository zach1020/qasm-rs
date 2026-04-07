// AST for a minimal OpenQASM subset.
//
// Covers enough to represent: version declaration, qubit/bit declarations,
// single-qubit gates, two-qubit gates (cx) and measurement.

#[derive(Debug, Clone)]
pub struct Program {
    pub version: String,
    pub statements: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    // 'qubit[N] name;' or 'qubit name;'
    QubitDec1 {
        name: String,
        size: Option<u64>,
    },
    // bit [N] name or bit name;
    BitDec1 {
        name: String,
        size: Option<u64>,
    },
    GateCall {
        name: String,
        args: Vec<GateOperand>,
    },
    Measure {
        qubit: GateOperand,
        target: Option<GateOperand>,
    },
    Reset {
        target: GateOperand,
    },
    Barrier {
        targets: Vec<GateOperand>,
    },
}

// A qubit or bit reference, optionally indexed.
#[derive(Debug, Clone)]
pub struct GateOperand {
    pub name: String,
    pub index: Option<u64>,
}

impl std::fmt::Display for GateOperand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            Some(i) => write!(f, "{}[{}]", self.name, i),
            None => write!(f, "{}", self.name),
        }
    }
}