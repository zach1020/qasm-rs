//! Circuit optimization passes operating on the DAG.
//!
//! Each pass is a graph rewrite: pattern-match on subgraphs, replace
//! with equivalent (or empty) subgraphs. The DAG's tombstone-based
//! removal means passes compose cleanly — no index invalidation.
//!
//! Currently implemented:
//!   - **Adjacent inverse cancellation**: remove pairs of self-inverse
//!     gates (H·H = I, X·X = I, Y·Y = I, Z·Z = I, CX·CX = I).

use crate::ir::*;

/// Statistics returned by an optimization pass.
#[derive(Debug, Default)]
pub struct OptStats {
    /// Number of gate pairs removed.
    pub gates_removed: usize,
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
        "h" | "x" | "y" | "z" | "cx" | "cnot" | "cz" | "swap" | "s" | "sdg" | "t" | "tdg"
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
    if params_a != params_b && !are_inverse_pair(name_a, name_b) {
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

    false
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
    let mut stats = OptStats::default();
    let mut changed = true;

    while changed {
        changed = false;

        for wire in 0..dag.num_qubits {
            // Walk the wire from In to Out.
            let mut current = dag.input_nodes[wire];

            loop {
                let next = match dag.wire_successor(current, wire) {
                    Some(id) => id,
                    None => break,
                };

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
                if gates_cancel(dag, current, next)
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
                    current = next;
                }
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
        let mut dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; cx q[0], q[1]; cx q[0], q[1];",
        );
        assert_eq!(dag.gate_count(), 2);
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 2);
        assert_eq!(dag.gate_count(), 0);
    }

    #[test]
    fn no_cancel_cx_different_order() {
        // cx q[0],q[1] then cx q[1],q[0] — different control/target, should NOT cancel.
        let mut dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; cx q[0], q[1]; cx q[1], q[0];",
        );
        let stats = cancel_inverses(&mut dag);
        assert_eq!(stats.gates_removed, 0);
        assert_eq!(dag.gate_count(), 2);
    }

    #[test]
    fn cancel_preserves_other_gates() {
        // h q[0]; x q[0]; x q[0]; cx q[0],q[1]; → h q[0]; cx q[0],q[1];
        let mut dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; h q[0]; x q[0]; x q[0]; cx q[0], q[1];",
        );
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
    fn optimized_dag_emits_valid_qasm() {
        let mut dag = lower_source(
            "OPENQASM 3.0; qubit[2] q; bit[2] c; \
             h q[0]; x q[0]; x q[0]; cx q[0], q[1]; c = measure q;",
        );
        cancel_inverses(&mut dag);
        let qasm = dag.emit_qasm();
        // x·x removed, h and cx remain.
        assert!(qasm.contains("h q[0]"));
        assert!(qasm.contains("cx q[0], q[1]"));
        assert!(!qasm.contains("x q[0]"));
    }
}
