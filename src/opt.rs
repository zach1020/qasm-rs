//! Circuit optimization passes operating on the DAG.
//!
//! Each pass is a graph rewrite: pattern-match on subgraphs, replace
//! with equivalent (or empty) subgraphs. The DAG's tombstone-based
//! removal means passes compose cleanly — no index invalidation.
//!
//! Currently implemented:
//!   - **Adjacent inverse cancellation**: remove pairs of self-inverse
//!     gates (H·H = I, X·X = I, Y·Y = I, Z·Z = I, CX·CX = I).

use std::collections::HashSet;

use crate::ir::*;
use crate::{ast, hir};

/// Statistics returned by an optimization pass.
#[derive(Debug, Default)]
pub struct OptStats {
    /// Number of gate pairs removed.
    pub gates_removed: usize,
}

#[derive(Debug, Default)]
pub struct DecomposeStats {
    pub gates_decomposed: usize,
}

// ── Adjacent inverse cancellation ───────────────────────────
//
// Walk each qubit wire. When two consecutive gate nodes are both
// self-inverse, have identical names, identical parameters, and
// operate on the same set of wires — remove both.
//
// For single-qubit gates (H, X, Y, Z): adjacency on one wire
// is sufficient since they only touch one wire.
//
// For multi-qubit gates (CX, CZ, SWAP): we additionally verify
// that the same pair of nodes are adjacent on ALL their wires.
// CX·CX cancels only if both CX gates connect the same control
// and target in the same order.
//
// This is a fixed-point iteration: we keep scanning until no
// more cancellations are found, because removing a pair may
// expose a new adjacent pair behind it.

/// Set of self-inverse gate names (case-insensitive).
fn is_self_inverse(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "h" | "x" | "y" | "z" | "cx" | "cnot" | "cz" | "swap"
    )
}

/// Check if two gates are inverse pairs (e.g., S and Sdg).
fn are_inverse_pair(a: &str, b: &str) -> bool {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    // S/Sdg, T/Tdg
    (a == "s" && b == "sdg")
        || (a == "sdg" && b == "s")
        || (a == "t" && b == "tdg")
        || (a == "tdg" && b == "t")
}

/// Check if two gates cancel each other:
/// either both self-inverse with same name, or an inverse pair.
fn gates_cancel(dag: &CircuitDAG, a: NodeId, b: NodeId) -> bool {
    let node_a = dag.node(a);
    let node_b = dag.node(b);

    let (name_a, mods_a, params_a, qubits_a) = match &node_a.op {
        Op::Gate {
            name,
            modifiers,
            params,
            qubits,
        } => (name, modifiers, params, qubits),
        _ => return false,
    };

    let (name_b, mods_b, params_b, qubits_b) = match &node_b.op {
        Op::Gate {
            name,
            modifiers,
            params,
            qubits,
        } => (name, modifiers, params, qubits),
        _ => return false,
    };

    // Must have no modifiers (inv @ h would need different handling).
    if !mods_a.is_empty() || !mods_b.is_empty() {
        return false;
    }

    // Must operate on exactly the same wires in the same order.
    if qubits_a != qubits_b {
        return false;
    }

    // Must have identical parameters.
    let inverse_rotation_params = name_a.eq_ignore_ascii_case(name_b)
        && matches!(
            name_a.to_ascii_lowercase().as_str(),
            "rx" | "ry" | "rz" | "p"
        )
        && params_a.len() == 1
        && params_b.len() == 1
        && params_are_negations(&params_a[0], &params_b[0]);
    if params_a != params_b && !are_inverse_pair(name_a, name_b) && !inverse_rotation_params {
        return false;
    }

    // Self-inverse check: same name, same params.
    if name_a.to_lowercase() == name_b.to_lowercase()
        && params_a == params_b
        && is_self_inverse(name_a)
    {
        return true;
    }

    // Inverse pair check: S/Sdg, T/Tdg (params must both be empty).
    if are_inverse_pair(name_a, name_b) && params_a.is_empty() && params_b.is_empty() {
        return true;
    }

    if inverse_rotation_params {
        return true;
    }

    false
}

fn params_are_negations(a: &Param, b: &Param) -> bool {
    matches!(a, Param::Neg(inner) if inner.as_ref() == b)
        || matches!(b, Param::Neg(inner) if inner.as_ref() == a)
}

fn gates_commute(dag: &CircuitDAG, a: NodeId, b: NodeId) -> bool {
    let (
        Op::Gate {
            name: name_a,
            modifiers: modifiers_a,
            qubits: qubits_a,
            ..
        },
        Op::Gate {
            name: name_b,
            modifiers: modifiers_b,
            qubits: qubits_b,
            ..
        },
    ) = (&dag.node(a).op, &dag.node(b).op)
    else {
        return false;
    };
    if !modifiers_a.is_empty()
        || !modifiers_b.is_empty()
        || qubits_a.len() != 1
        || qubits_b.len() != 1
        || qubits_a != qubits_b
    {
        return false;
    }
    let diagonal = |name: &str| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "z" | "s" | "sdg" | "t" | "tdg" | "rz" | "p"
        )
    };
    let x_axis = |name: &str| matches!(name.to_ascii_lowercase().as_str(), "x" | "sx" | "rx");
    (diagonal(name_a) && diagonal(name_b)) || (x_axis(name_a) && x_axis(name_b))
}

fn commuting_separator_on_all_wires(
    dag: &CircuitDAG,
    a: NodeId,
    middle: NodeId,
    b: NodeId,
    middle_wire: usize,
) -> bool {
    for wire in dag.node(a).op.qubits() {
        let successor = dag.wire_successor(a, *wire);
        if *wire == middle_wire {
            if successor != Some(middle) || dag.wire_successor(middle, *wire) != Some(b) {
                return false;
            }
        } else if successor != Some(b) {
            return false;
        }
    }
    true
}

/// For a multi-qubit gate, check that the two candidate nodes are
/// adjacent on ALL their shared wires — not just the wire we
/// discovered them on.
fn adjacent_on_all_wires(dag: &CircuitDAG, a: NodeId, b: NodeId) -> bool {
    let qubits = dag.node(a).op.qubits().to_vec();
    for w in &qubits {
        match dag.wire_successor(a, *w) {
            Some(succ) if succ == b => {}
            _ => return false,
        }
    }
    true
}

/// Run adjacent inverse cancellation. Returns statistics.
pub fn cancel_inverses(dag: &mut CircuitDAG) -> OptStats {
    cancel_inverses_except(dag, &HashSet::new())
}

fn cancel_inverses_except(dag: &mut CircuitDAG, excluded_gates: &HashSet<String>) -> OptStats {
    let mut stats = OptStats::default();
    let mut changed = true;

    while changed {
        changed = false;

        for wire in 0..dag.num_qubits {
            // Walk the wire from In to Out.
            let mut current = dag.input_nodes[wire];

            while let Some(next) = dag.wire_successor(current, wire) {
                // Skip non-gate nodes.
                if !dag.node(current).op.is_gate() {
                    current = next;
                    continue;
                }
                if !dag.node(next).op.is_gate() {
                    current = next;
                    continue;
                }

                // Check cancellation.
                let gate_is_excluded = match &dag.node(current).op {
                    Op::Gate { name, .. } => excluded_gates.contains(&name.to_ascii_lowercase()),
                    _ => false,
                };
                if !gate_is_excluded
                    && gates_cancel(dag, current, next)
                    && adjacent_on_all_wires(dag, current, next)
                {
                    // Get the predecessor of `current` on this wire
                    // before we remove nodes, so we can continue from there.
                    let prev = dag.wire_predecessor(current, wire);

                    dag.remove_node(current);
                    dag.remove_node(next);
                    stats.gates_removed += 2;
                    changed = true;

                    // Continue from the predecessor (may expose new pair).
                    current = match prev {
                        Some(id) => id,
                        None => break,
                    };
                } else {
                    if !gate_is_excluded && gates_commute(dag, current, next) {
                        if let Some(candidate) = dag.wire_successor(next, wire) {
                            if dag.node(candidate).op.is_gate()
                                && gates_cancel(dag, current, candidate)
                                && commuting_separator_on_all_wires(
                                    dag, current, next, candidate, wire,
                                )
                            {
                                let prev = dag.wire_predecessor(current, wire);
                                dag.remove_node(current);
                                dag.remove_node(candidate);
                                stats.gates_removed += 2;
                                changed = true;
                                current = prev.unwrap_or(dag.input_nodes[wire]);
                                continue;
                            }
                        }
                    }
                    current = next;
                }
            }
        }
    }

    stats
}

/// Run DAG optimizations over every straight-line circuit region in HIR.
pub fn optimize_hir(program: &mut hir::HirProgram) -> OptStats {
    let mut custom_gates = HashSet::new();
    collect_custom_gates(&program.statements, &mut custom_gates);
    optimize_hir_stmts(&mut program.statements, &custom_gates)
}

/// Decompose `h` and `x` gates into the common `{rz, sx, cx}` basis.
///
/// Modified gates and user-defined gates are preserved. The Hadamard
/// decomposition is equivalent up to global phase.
pub fn decompose_hir_to_rz_sx_cx(program: &mut hir::HirProgram) -> DecomposeStats {
    let mut custom_gates = HashSet::new();
    collect_custom_gates(&program.statements, &mut custom_gates);
    decompose_hir_stmts(&mut program.statements, &custom_gates)
}

fn decompose_hir_stmts(
    stmts: &mut [hir::HirStmt],
    excluded_gates: &HashSet<String>,
) -> DecomposeStats {
    let mut stats = DecomposeStats::default();
    for stmt in stmts {
        match stmt {
            hir::HirStmt::Circuit(dag) => {
                let (replacement, region_stats) = decompose_dag_to_rz_sx_cx(dag, excluded_gates);
                *dag = replacement;
                stats.gates_decomposed += region_stats.gates_decomposed;
            }
            hir::HirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                stats.gates_decomposed +=
                    decompose_hir_stmts(then_body, excluded_gates).gates_decomposed;
                if let Some(else_body) = else_body {
                    stats.gates_decomposed +=
                        decompose_hir_stmts(else_body, excluded_gates).gates_decomposed;
                }
            }
            hir::HirStmt::For { body, .. } | hir::HirStmt::While { body, .. } => {
                stats.gates_decomposed +=
                    decompose_hir_stmts(body, excluded_gates).gates_decomposed;
            }
            hir::HirStmt::Ast(_) => {}
        }
    }
    stats
}

fn decompose_dag_to_rz_sx_cx(
    dag: &CircuitDAG,
    excluded_gates: &HashSet<String>,
) -> (CircuitDAG, DecomposeStats) {
    let mut replacement = CircuitDAG::new(dag.num_qubits, dag.num_bits);
    replacement.qubit_names = dag.qubit_names.clone();
    replacement.bit_names = dag.bit_names.clone();
    let mut stats = DecomposeStats::default();
    for node_id in dag.ops_topo() {
        match &dag.node(node_id).op {
            Op::Gate {
                name,
                modifiers,
                params,
                qubits,
            } if modifiers.is_empty()
                && params.is_empty()
                && !excluded_gates.contains(&name.to_ascii_lowercase())
                && name.eq_ignore_ascii_case("h") =>
            {
                let half_pi = Param::BinOp {
                    op: ParamOp::Div,
                    lhs: Box::new(Param::Pi),
                    rhs: Box::new(Param::Int(2)),
                };
                replacement.append_gate("rz".into(), vec![], vec![half_pi.clone()], qubits.clone());
                replacement.append_gate("sx".into(), vec![], vec![], qubits.clone());
                replacement.append_gate("rz".into(), vec![], vec![half_pi], qubits.clone());
                stats.gates_decomposed += 1;
            }
            Op::Gate {
                name,
                modifiers,
                params,
                qubits,
            } if modifiers.is_empty()
                && params.is_empty()
                && !excluded_gates.contains(&name.to_ascii_lowercase())
                && name.eq_ignore_ascii_case("x") =>
            {
                replacement.append_gate("sx".into(), vec![], vec![], qubits.clone());
                replacement.append_gate("sx".into(), vec![], vec![], qubits.clone());
                stats.gates_decomposed += 1;
            }
            Op::Gate {
                name,
                modifiers,
                params,
                qubits,
            } => {
                replacement.append_gate(
                    name.clone(),
                    modifiers.clone(),
                    params.clone(),
                    qubits.clone(),
                );
            }
            Op::Measure { qubit, bit } => {
                replacement.append_measure(*qubit, *bit);
            }
            Op::Reset { qubit } => {
                replacement.append_reset(*qubit);
            }
            Op::Barrier { qubits } => {
                replacement.append_barrier(qubits.clone());
            }
            Op::Delay { duration, qubits } => {
                replacement.append_delay(duration.clone(), qubits.clone());
            }
            Op::In { .. } | Op::Out { .. } => {}
        }
    }
    replacement.finalize();
    (replacement, stats)
}

fn collect_custom_gates(stmts: &[hir::HirStmt], custom_gates: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            hir::HirStmt::Ast(ast::Stmt::GateDef { name, .. }) => {
                custom_gates.insert(name.to_ascii_lowercase());
            }
            hir::HirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_custom_gates(then_body, custom_gates);
                if let Some(else_body) = else_body {
                    collect_custom_gates(else_body, custom_gates);
                }
            }
            hir::HirStmt::For { body, .. } | hir::HirStmt::While { body, .. } => {
                collect_custom_gates(body, custom_gates);
            }
            hir::HirStmt::Ast(_) | hir::HirStmt::Circuit(_) => {}
        }
    }
}

fn optimize_hir_stmts(stmts: &mut [hir::HirStmt], excluded_gates: &HashSet<String>) -> OptStats {
    let mut stats = OptStats::default();

    for stmt in stmts {
        match stmt {
            hir::HirStmt::Ast(_) => {}
            hir::HirStmt::Circuit(dag) => {
                stats.gates_removed += cancel_inverses_except(dag, excluded_gates).gates_removed;
            }
            hir::HirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                stats.gates_removed += optimize_hir_stmts(then_body, excluded_gates).gates_removed;
                if let Some(else_body) = else_body {
                    stats.gates_removed +=
                        optimize_hir_stmts(else_body, excluded_gates).gates_removed;
                }
            }
            hir::HirStmt::For { body, .. } | hir::HirStmt::While { body, .. } => {
                stats.gates_removed += optimize_hir_stmts(body, excluded_gates).gates_removed;
            }
        }
    }

    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower;
    use crate::parser::Parser;

    fn lower_source(source: &str) -> CircuitDAG {
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("parse failed");
        lower::lower(&program).expect("lowering failed")
    }

    #[test]
    fn cancel_adjacent_h() {
        let mut dag = lower_source("OPENQASM 3.0; qubit q; h q; h q;");
        assert_eq!(dag.gate_count(), 2);
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 0);
    }

    #[test]
    fn cancel_adjacent_x() {
        let mut dag = lower_source("OPENQASM 3.0; qubit q; x q; x q;");
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 0);
    }

    #[test]
    fn no_cancel_different_gates() {
        let mut dag = lower_source("OPENQASM 3.0; qubit q; h q; x q;");
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 0);
        assert_eq!(dag.gate_count(), 2);
    }

    #[test]
    fn cancel_cascading() {
        // h · x · x · h → remove x·x → h · h → remove h·h → empty
        let mut dag = lower_source("OPENQASM 3.0; qubit q; h q; x q; x q; h q;");
        assert_eq!(dag.gate_count(), 4);
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 4);
        assert_eq!(dag.gate_count(), 0);
    }

    #[test]
    fn cancel_cx_pair() {
        let mut dag = lower_source("OPENQASM 3.0; qubit[2] q; cx q[0], q[1]; cx q[0], q[1];");
        assert_eq!(dag.gate_count(), 2);
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 0);
    }

    #[test]
    fn no_cancel_cx_different_order() {
        // cx q[0],q[1] then cx q[1],q[0] — different control/target, should NOT cancel.
        let mut dag = lower_source("OPENQASM 3.0; qubit[2] q; cx q[0], q[1]; cx q[1], q[0];");
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 0);
        assert_eq!(dag.gate_count(), 2);
    }

    #[test]
    fn cancel_preserves_other_gates() {
        // h q[0]; x q[0]; x q[0]; cx q[0],q[1]; → h q[0]; cx q[0],q[1];
        let mut dag =
            lower_source("OPENQASM 3.0; qubit[2] q; h q[0]; x q[0]; x q[0]; cx q[0], q[1];");
        assert_eq!(dag.gate_count(), 4);
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 2);
    }

    #[test]
    fn cancel_s_sdg_pair() {
        let mut dag = lower_source("OPENQASM 3.0; qubit q; s q; sdg q;");
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 0);
    }

    #[test]
    fn does_not_cancel_repeated_phase_gates() {
        for gate in ["s", "sdg", "t", "tdg"] {
            let source = format!("OPENQASM 3.0; qubit q; {gate} q; {gate} q;");
            let mut dag = lower_source(&source);
            let stats = cancel_inverses(&mut dag);
            assert_eq!(stats.gates_removed, 0, "{gate} is not self-inverse");
            assert_eq!(dag.gate_count(), 2);
        }
    }

    #[test]
    fn does_not_optimize_user_defined_gate_by_name() {
        let mut parser = Parser::new("OPENQASM 3.0; gate h q { x q; } qubit q; h q; h q;");
        let program = parser.parse().expect("parse failed");
        let mut hir = lower::lower_hir(&program).expect("lowering failed");
        let stats = optimize_hir(&mut hir);
        assert_eq!(stats.gates_removed, 0);
        assert_eq!(hir.gate_count(), 2);
    }

    #[test]
    fn cancels_inverse_rotations() {
        let mut dag = lower_source("OPENQASM 3.0; qubit q; rz(pi) q; rz(-pi) q;");
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 0);
    }

    #[test]
    fn cancels_inverse_pair_across_commuting_gate() {
        let mut dag = lower_source("OPENQASM 3.0; qubit q; z q; s q; z q;");
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 1);
        assert!(dag.emit_qasm().contains("s q;"));
    }

    #[test]
    fn does_not_cancel_across_non_commuting_gate() {
        let mut dag = lower_source("OPENQASM 3.0; qubit q; z q; x q; z q;");
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 0);
        assert_eq!(dag.gate_count(), 3);
    }

    #[test]
    fn decomposes_to_rz_sx_cx_basis() {
        let mut parser = Parser::new("OPENQASM 3.0; qubit q; h q; x q;");
        let program = parser.parse().unwrap();
        let mut hir = lower::lower_hir(&program).unwrap();
        let stats = decompose_hir_to_rz_sx_cx(&mut hir);
        assert_eq!(stats.gates_decomposed, 2);
        let qasm = hir.emit_qasm();
        assert!(!qasm.contains("\nh q;"));
        assert!(!qasm.contains("\nx q;"));
        assert_eq!(hir.gate_count(), 5);
    }

    #[test]
    fn optimized_dag_emits_valid_qasm() {
        let mut dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; bit[2] c; \
             h q[0]; x q[0]; x q[0]; cx q[0], q[1]; c = measure q;",
        );
        cancel_inverses(&mut dag);
        let qasm = dag.emit_qasm();
        // x·x removed, h and cx remain.
        assert!(qasm.lines().any(|line| line == "h q[0];"));
        assert!(qasm.lines().any(|line| line == "cx q[0], q[1];"));
        assert!(!qasm.lines().any(|line| line == "x q[0];"));
    }
}
