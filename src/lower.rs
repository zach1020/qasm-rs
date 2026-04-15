//! AST → CircuitDAG lowering.
//!
//! Walks the AST, resolves qubit/bit names to wire indices, and builds
//! the circuit DAG. Classical declarations and assignments are skipped
//! (they don't produce circuit operations). Control flow (if/for/while)
//! is not yet supported at the IR level — the lowering pass errors on
//! these constructs.
//!
//! Gate definitions are recorded but not inlined — the DAG preserves
//! the original gate calls. A future pass could inline/decompose gates
//! into a basis gate set.

use std::collections::HashMap;

use crate::ast;
use crate::ir::*;

// ── Lowering errors ─────────────────────────────────────────

#[derive(Debug)]
pub struct LowerError {
    pub message: String,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lowering error: {}", self.message)
    }
}

impl std::error::Error for LowerError {}

type Result<T> = std::result::Result<T, LowerError>;

fn err(msg: impl Into<String>) -> LowerError {
    LowerError {
        message: msg.into(),
    }
}

// ── Wire mapping ────────────────────────────────────────────

/// Maps AST qubit/bit names to DAG wire indices.
struct WireMap {
    /// (register_name, index) → wire_id
    qubits: HashMap<(String, Option<u64>), usize>,
    /// (register_name, index) → bit_id
    bits: HashMap<(String, Option<u64>), usize>,
    next_qubit: usize,
    next_bit: usize,
    /// Ordered list for re-emission.
    qubit_names: Vec<(String, Option<u64>)>,
    bit_names: Vec<(String, Option<u64>)>,
}

impl WireMap {
    fn new() -> Self {
        Self {
            qubits: HashMap::new(),
            bits: HashMap::new(),
            next_qubit: 0,
            next_bit: 0,
            qubit_names: Vec::new(),
            bit_names: Vec::new(),
        }
    }

    fn add_qubit_register(&mut self, name: &str, size: Option<u64>) {
        match size {
            Some(n) => {
                for i in 0..n {
                    let id = self.next_qubit;
                    self.qubits.insert((name.to_string(), Some(i)), id);
                    self.qubit_names.push((name.to_string(), Some(i)));
                    self.next_qubit += 1;
                }
            }
            None => {
                let id = self.next_qubit;
                self.qubits.insert((name.to_string(), None), id);
                self.qubit_names.push((name.to_string(), None));
                self.next_qubit += 1;
            }
        }
    }

    fn add_bit_register(&mut self, name: &str, size: Option<u64>) {
        match size {
            Some(n) => {
                for i in 0..n {
                    let id = self.next_bit;
                    self.bits.insert((name.to_string(), Some(i)), id);
                    self.bit_names.push((name.to_string(), Some(i)));
                    self.next_bit += 1;
                }
            }
            None => {
                let id = self.next_bit;
                self.bits.insert((name.to_string(), None), id);
                self.bit_names.push((name.to_string(), None));
                self.next_bit += 1;
            }
        }
    }

    fn resolve_qubit(&self, op: &ast::GateOperand) -> Result<usize> {
        let key = (op.name.clone(), op.index);
        self.qubits
            .get(&key)
            .copied()
            .ok_or_else(|| err(format!("unresolved qubit `{}`", op)))
    }

    fn resolve_bit(&self, op: &ast::GateOperand) -> Result<usize> {
        let key = (op.name.clone(), op.index);
        self.bits
            .get(&key)
            .copied()
            .ok_or_else(|| err(format!("unresolved bit `{}`", op)))
    }

    /// Resolve a qubit operand that might be a whole register.
    /// If no index is given and it's a register, returns all wire IDs.
    fn resolve_qubit_operand(&self, op: &ast::GateOperand) -> Result<Vec<usize>> {
        if op.index.is_some() {
            return Ok(vec![self.resolve_qubit(op)?]);
        }
        // No index — might be scalar or whole register.
        // Try scalar first.
        if let Some(&id) = self.qubits.get(&(op.name.clone(), None)) {
            return Ok(vec![id]);
        }
        // Try as a register — collect all indices.
        let mut wires = Vec::new();
        let mut i = 0u64;
        while let Some(&id) = self.qubits.get(&(op.name.clone(), Some(i))) {
            wires.push(id);
            i += 1;
        }
        if wires.is_empty() {
            Err(err(format!("unresolved qubit `{}`", op)))
        } else {
            Ok(wires)
        }
    }
}

// ── Expression lowering ─────────────────────────────────────

fn lower_expr(expr: &ast::Expr) -> Param {
    match expr {
        ast::Expr::IntLit(n, _) => Param::Int(*n),
        ast::Expr::FloatLit(f, _) => Param::Float(*f),
        ast::Expr::BoolLit(b, _) => Param::Int(if *b { 1 } else { 0 }),
        ast::Expr::Ident(name, _) => Param::Ident(name.clone()),
        ast::Expr::Const(kind, _) => match kind {
            ast::ConstKind::Pi => Param::Pi,
            ast::ConstKind::Tau => Param::Tau,
            ast::ConstKind::Euler => Param::Euler,
        },
        ast::Expr::Neg(inner, _) => Param::Neg(Box::new(lower_expr(inner))),
        ast::Expr::BinOp { op, lhs, rhs, .. } => {
            let param_op = match op {
                ast::BinOp::Add => ParamOp::Add,
                ast::BinOp::Sub => ParamOp::Sub,
                ast::BinOp::Mul => ParamOp::Mul,
                ast::BinOp::Div => ParamOp::Div,
                ast::BinOp::Pow => ParamOp::Pow,
            };
            Param::BinOp {
                op: param_op,
                lhs: Box::new(lower_expr(lhs)),
                rhs: Box::new(lower_expr(rhs)),
            }
        }
        ast::Expr::Compare { .. } => {
            // Comparisons don't appear in gate parameters.
            Param::Int(0)
        }
    }
}

fn lower_modifier(m: &ast::GateModifier) -> Modifier {
    match m {
        ast::GateModifier::Ctrl(arg, _) => {
            let n = arg.as_ref().and_then(|e| match e {
                ast::Expr::IntLit(n, _) => Some(*n),
                _ => None,
            });
            Modifier::Ctrl(n)
        }
        ast::GateModifier::NegCtrl(arg, _) => {
            let n = arg.as_ref().and_then(|e| match e {
                ast::Expr::IntLit(n, _) => Some(*n),
                _ => None,
            });
            Modifier::NegCtrl(n)
        }
        ast::GateModifier::Inv(_) => Modifier::Inv,
        ast::GateModifier::Pow(e, _) => Modifier::Pow(lower_expr(e)),
    }
}

// ── Main lowering pass ──────────────────────────────────────

/// Lower a type-checked AST into a CircuitDAG.
pub fn lower(program: &ast::Program) -> Result<CircuitDAG> {
    // First pass: collect declarations to determine wire counts.
    let mut wires = WireMap::new();

    for stmt in &program.statements {
        match stmt {
            ast::Stmt::QubitDecl { name, size, .. } => {
                wires.add_qubit_register(name, *size);
            }
            ast::Stmt::BitDecl { name, size, .. } => {
                wires.add_bit_register(name, *size);
            }
            _ => {}
        }
    }

    // Create DAG with the right number of wires.
    let mut dag = CircuitDAG::new(wires.next_qubit, wires.next_bit);
    dag.qubit_names = wires.qubit_names.clone();
    dag.bit_names = wires.bit_names.clone();

    // Second pass: lower operations.
    lower_stmts(&program.statements, &wires, &mut dag)?;

    // Finalize: connect wire heads to output nodes.
    dag.finalize();

    Ok(dag)
}

fn lower_stmts(
    stmts: &[ast::Stmt],
    wires: &WireMap,
    dag: &mut CircuitDAG,
) -> Result<()> {
    for stmt in stmts {
        lower_stmt(stmt, wires, dag)?;
    }
    Ok(())
}

fn lower_stmt(
    stmt: &ast::Stmt,
    wires: &WireMap,
    dag: &mut CircuitDAG,
) -> Result<()> {
    match stmt {
        // Declarations are handled in the first pass.
        ast::Stmt::QubitDecl { .. }
        | ast::Stmt::BitDecl { .. }
        | ast::Stmt::ClassicalDecl { .. }
        | ast::Stmt::Assignment { .. }
        | ast::Stmt::GateDef { .. } => Ok(()),

        ast::Stmt::GateCall {
            name,
            modifiers,
            params,
            args,
            ..
        } => {
            let ir_params: Vec<Param> = params.iter().map(lower_expr).collect();
            let ir_mods: Vec<Modifier> = modifiers.iter().map(lower_modifier).collect();
            let mut qubit_wires = Vec::new();
            for arg in args {
                qubit_wires.extend(wires.resolve_qubit_operand(arg)?);
            }
            dag.append_gate(name.clone(), ir_mods, ir_params, qubit_wires);
            Ok(())
        }

        ast::Stmt::Measure { qubit, target, .. } => {
            let qubit_wires = wires.resolve_qubit_operand(qubit)?;
            let bit_id = match target {
                Some(t) => Some(wires.resolve_bit(t)?),
                None => None,
            };
            // If measuring a whole register, emit one measure per wire.
            for (i, qw) in qubit_wires.iter().enumerate() {
                let bw = bit_id.map(|b| b + i);
                dag.append_measure(*qw, bw);
            }
            Ok(())
        }

        ast::Stmt::Reset { target, .. } => {
            let qubit_wires = wires.resolve_qubit_operand(target)?;
            for qw in &qubit_wires {
                dag.append_reset(*qw);
            }
            Ok(())
        }

        ast::Stmt::Barrier { targets, .. } => {
            let mut qubit_wires = Vec::new();
            for t in targets {
                qubit_wires.extend(wires.resolve_qubit_operand(t)?);
            }
            dag.append_barrier(qubit_wires);
            Ok(())
        }

        // Control flow: not yet lowered to IR.
        ast::Stmt::If { .. } | ast::Stmt::For { .. } | ast::Stmt::While { .. } => {
            Err(err(
                "classical control flow is not yet supported in IR lowering — \
                 only straight-line quantum circuits can be lowered",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn lower_source(source: &str) -> CircuitDAG {
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        lower(&program).expect("lowering failed")
    }

    #[test]
    fn lower_bell_pair() {
        let dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;",
        );
        assert_eq!(dag.num_qubits, 2);
        assert_eq!(dag.num_bits, 2);
        assert_eq!(dag.gate_count(), 2); // h, cx
        assert_eq!(dag.op_count(), 4); // h, cx, measure×2
    }

    #[test]
    fn lower_scalar_qubit() {
        let dag = lower_source("OPENQASM 3.0; qubit q; h q;");
        assert_eq!(dag.num_qubits, 1);
        assert_eq!(dag.gate_count(), 1);
    }

    #[test]
    fn lower_parameterized_gate() {
        let dag = lower_source("OPENQASM 3.0; qubit q; rx(pi/2) q;");
        assert_eq!(dag.gate_count(), 1);
    }

    #[test]
    fn lower_modified_gate() {
        let dag = lower_source("OPENQASM 3.0; qubit[2] q; ctrl @ x q[0], q[1];");
        assert_eq!(dag.gate_count(), 1);
    }

    #[test]
    fn lower_preserves_depth() {
        // h q[0]; x q[1]; are parallel → depth 1
        // cx q[0], q[1]; depends on both → depth 2
        let dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; h q[0]; x q[1]; cx q[0], q[1];",
        );
        assert_eq!(dag.depth(), 2);
    }

    #[test]
    fn lower_emits_valid_qasm() {
        let dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;",
        );
        let qasm = dag.emit_qasm();
        assert!(qasm.contains("h q[0]"));
        assert!(qasm.contains("cx q[0], q[1]"));
        assert!(qasm.contains("measure"));
    }

    #[test]
    fn lower_rejects_control_flow() {
        let mut parser = Parser::new(
            "OPENQASM 3.0; qubit q; int x = 0; if (x == 0) { h q; }",
        );
        let program = parser.parse().expect("parse failed");
        assert!(lower(&program).is_err());
    }
}
