//! Circuit intermediate representation — a directed acyclic graph (DAG).
//!
//! The DAG is the standard representation used in quantum compilers
//! (cf. Qiskit's DAGCircuit, tket's Circuit). Each node is a quantum
//! operation; edges represent qubit-wire dependencies. The structure
//! naturally exposes parallelism — gates on disjoint qubits are
//! unordered in the DAG and can be scheduled independently.
//!
//! Optimization passes operate as graph rewrites: pattern-match on
//! local subgraphs, then replace with equivalent (or empty) subgraphs.
//! Gate cancellation, commutation, and template matching all reduce
//! to subgraph isomorphism + replacement on this structure.
//!
//! Each qubit wire is anchored by an `In` boundary node at the start
//! and an `Out` boundary node at the end. The wire threads through
//! gate nodes in program order. To find the next gate on a wire, walk
//! the successor edge labeled with that wire index.

use std::collections::VecDeque;
use std::fmt;

// ── Core types ──────────────────────────────────────────────

pub type NodeId = usize;

/// A quantum operation — the payload of a DAG node.
#[derive(Debug, Clone)]
pub enum Op {
    /// Input boundary for qubit wire `wire`. Every wire has exactly one.
    In { wire: usize },
    /// Output boundary for qubit wire `wire`. Every wire has exactly one.
    Out { wire: usize },
    /// A gate application on one or more qubit wires.
    Gate {
        name: String,
        modifiers: Vec<Modifier>,
        params: Vec<Param>,
        qubits: Vec<usize>,
    },
    /// Measurement: collapse qubit wire to classical bit.
    Measure { qubit: usize, bit: Option<usize> },
    /// Reset qubit to |0⟩, restoring it as a usable resource.
    Reset { qubit: usize },
    /// Barrier: scheduling fence across specified wires.
    Barrier { qubits: Vec<usize> },
}

impl Op {
    /// The qubit wires this operation touches.
    pub fn qubits(&self) -> &[usize] {
        match self {
            Op::In { wire } | Op::Out { wire } => std::slice::from_ref(wire),
            Op::Gate { qubits, .. } | Op::Barrier { qubits } => qubits,
            Op::Measure { qubit, .. } | Op::Reset { qubit } => std::slice::from_ref(qubit),
        }
    }

    /// True if this is a gate operation (not boundary/measure/reset/barrier).
    pub fn is_gate(&self) -> bool {
        matches!(self, Op::Gate { .. })
    }
}

/// Gate modifier carried through to the IR.
#[derive(Debug, Clone, PartialEq)]
pub enum Modifier {
    Ctrl(Option<u64>),
    NegCtrl(Option<u64>),
    Inv,
    Pow(Param),
}

/// A parameter value — simplified from the AST expression tree.
/// Keeps enough structure for symbolic manipulation while being
/// independent of source spans and AST lifetime.
#[derive(Debug, Clone)]
pub enum Param {
    Float(f64),
    Int(u64),
    Pi,
    Tau,
    Euler,
    Ident(String),
    Neg(Box<Param>),
    BinOp {
        op: ParamOp,
        lhs: Box<Param>,
        rhs: Box<Param>,
    },
}

impl PartialEq for Param {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Param::Float(a), Param::Float(b)) => (a - b).abs() < 1e-12,
            (Param::Int(a), Param::Int(b)) => a == b,
            (Param::Pi, Param::Pi) => true,
            (Param::Tau, Param::Tau) => true,
            (Param::Euler, Param::Euler) => true,
            (Param::Ident(a), Param::Ident(b)) => a == b,
            (Param::Neg(a), Param::Neg(b)) => a == b,
            (
                Param::BinOp {
                    op: op1,
                    lhs: l1,
                    rhs: r1,
                },
                Param::BinOp {
                    op: op2,
                    lhs: l2,
                    rhs: r2,
                },
            ) => op1 == op2 && l1 == l2 && r1 == r2,
            _ => false,
        }
    }
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Param::Float(v) => write!(f, "{}", v),
            Param::Int(v) => write!(f, "{}", v),
            Param::Pi => write!(f, "pi"),
            Param::Tau => write!(f, "tau"),
            Param::Euler => write!(f, "euler"),
            Param::Ident(s) => write!(f, "{}", s),
            Param::Neg(inner) => write!(f, "-{}", inner),
            Param::BinOp { op, lhs, rhs } => write!(f, "({} {} {})", lhs, op, rhs),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

impl fmt::Display for ParamOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamOp::Add => write!(f, "+"),
            ParamOp::Sub => write!(f, "-"),
            ParamOp::Mul => write!(f, "*"),
            ParamOp::Div => write!(f, "/"),
            ParamOp::Pow => write!(f, "**"),
        }
    }
}

// ── DAG node ────────────────────────────────────────────────

/// A node in the circuit DAG. Nodes are arena-allocated by index.
#[derive(Debug)]
pub struct DAGNode {
    pub id: NodeId,
    pub op: Op,
    /// Tombstone flag — removed nodes are skipped during iteration.
    /// This avoids index invalidation during optimization passes.
    pub removed: bool,
}

// ── Circuit DAG ─────────────────────────────────────────────

/// The circuit DAG — the central data structure of the compiler's
/// middle-end. All optimization passes read and rewrite this.
pub struct CircuitDAG {
    nodes: Vec<DAGNode>,
    /// `succ[node_id]` = vec of `(successor_node_id, wire_index)`
    succ: Vec<Vec<(NodeId, usize)>>,
    /// `pred[node_id]` = vec of `(predecessor_node_id, wire_index)`
    pred: Vec<Vec<(NodeId, usize)>>,
    pub num_qubits: usize,
    pub num_bits: usize,
    /// Boundary node IDs: one In per qubit wire.
    pub input_nodes: Vec<NodeId>,
    /// Boundary node IDs: one Out per qubit wire.
    pub output_nodes: Vec<NodeId>,
    /// Original qubit register names for re-emission.
    pub qubit_names: Vec<(String, Option<u64>)>,
    /// Original bit register names for re-emission.
    pub bit_names: Vec<(String, Option<u64>)>,
    /// Per-wire head: the last node appended on each wire.
    /// Used during construction; not meaningful after optimization.
    wire_heads: Vec<NodeId>,
}

impl CircuitDAG {
    /// Create a new DAG with `nq` qubit wires and `nb` classical bit wires.
    /// Allocates In/Out boundary nodes for each qubit wire.
    pub fn new(nq: usize, nb: usize) -> Self {
        let total_boundary = 2 * nq;
        let mut nodes = Vec::with_capacity(total_boundary + 64);
        let mut succ = Vec::with_capacity(total_boundary + 64);
        let mut pred = Vec::with_capacity(total_boundary + 64);
        let mut input_nodes = Vec::with_capacity(nq);
        let mut output_nodes = Vec::with_capacity(nq);

        // Create In and Out boundary nodes for each qubit wire.
        for w in 0..nq {
            let in_id = nodes.len();
            nodes.push(DAGNode {
                id: in_id,
                op: Op::In { wire: w },
                removed: false,
            });
            succ.push(Vec::new());
            pred.push(Vec::new());
            input_nodes.push(in_id);

            let out_id = nodes.len();
            nodes.push(DAGNode {
                id: out_id,
                op: Op::Out { wire: w },
                removed: false,
            });
            succ.push(Vec::new());
            pred.push(Vec::new());
            output_nodes.push(out_id);
        }

        // Wire heads start at the In nodes.
        let wire_heads = input_nodes.clone();

        CircuitDAG {
            nodes,
            succ,
            pred,
            num_qubits: nq,
            num_bits: nb,
            input_nodes,
            output_nodes,
            qubit_names: Vec::new(),
            bit_names: Vec::new(),
            wire_heads,
        }
    }

    /// Total number of allocated nodes (including removed and boundary).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of live gate operations (excludes boundary, measure, reset, barrier, removed).
    pub fn gate_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| !n.removed && n.op.is_gate())
            .count()
    }

    /// Number of all live operation nodes (gates + measure + reset + barrier).
    pub fn op_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| {
                !n.removed
                    && !matches!(n.op, Op::In { .. } | Op::Out { .. })
            })
            .count()
    }

    /// Get a node by ID.
    pub fn node(&self, id: NodeId) -> &DAGNode {
        &self.nodes[id]
    }

    /// Successors of a node: `(successor_id, wire)` pairs.
    pub fn successors(&self, id: NodeId) -> &[(NodeId, usize)] {
        &self.succ[id]
    }

    /// Predecessors of a node: `(predecessor_id, wire)` pairs.
    pub fn predecessors(&self, id: NodeId) -> &[(NodeId, usize)] {
        &self.pred[id]
    }

    /// Find the direct successor of `node_id` on a specific `wire`.
    pub fn wire_successor(&self, node_id: NodeId, wire: usize) -> Option<NodeId> {
        self.succ[node_id]
            .iter()
            .find(|(_, w)| *w == wire)
            .map(|(id, _)| *id)
    }

    /// Find the direct predecessor of `node_id` on a specific `wire`.
    pub fn wire_predecessor(&self, node_id: NodeId, wire: usize) -> Option<NodeId> {
        self.pred[node_id]
            .iter()
            .find(|(_, w)| *w == wire)
            .map(|(id, _)| *id)
    }

    // ── Construction ────────────────────────────────────────

    /// Allocate a new node and return its ID. Does NOT wire it up —
    /// use `append_on_wire` or `append_op` for that.
    fn alloc_node(&mut self, op: Op) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(DAGNode {
            id,
            op,
            removed: false,
        });
        self.succ.push(Vec::new());
        self.pred.push(Vec::new());
        id
    }

    /// Add a directed edge: `from --wire--> to`.
    fn add_edge(&mut self, from: NodeId, to: NodeId, wire: usize) {
        self.succ[from].push((to, wire));
        self.pred[to].push((from, wire));
    }

    /// Wire a node onto the end of a qubit wire, updating the head.
    fn append_on_wire(&mut self, node_id: NodeId, wire: usize) {
        let head = self.wire_heads[wire];
        self.add_edge(head, node_id, wire);
        self.wire_heads[wire] = node_id;
    }

    /// High-level: add a gate and wire it to all its qubit wires.
    pub fn append_gate(
        &mut self,
        name: String,
        modifiers: Vec<Modifier>,
        params: Vec<Param>,
        qubits: Vec<usize>,
    ) -> NodeId {
        let wires = qubits.clone();
        let id = self.alloc_node(Op::Gate {
            name,
            modifiers,
            params,
            qubits,
        });
        for w in &wires {
            self.append_on_wire(id, *w);
        }
        id
    }

    /// High-level: add a measure operation.
    pub fn append_measure(&mut self, qubit: usize, bit: Option<usize>) -> NodeId {
        let id = self.alloc_node(Op::Measure { qubit, bit });
        self.append_on_wire(id, qubit);
        id
    }

    /// High-level: add a reset operation.
    pub fn append_reset(&mut self, qubit: usize) -> NodeId {
        let id = self.alloc_node(Op::Reset { qubit });
        self.append_on_wire(id, qubit);
        id
    }

    /// High-level: add a barrier.
    pub fn append_barrier(&mut self, qubits: Vec<usize>) -> NodeId {
        let wires = qubits.clone();
        let id = self.alloc_node(Op::Barrier { qubits });
        for w in &wires {
            self.append_on_wire(id, *w);
        }
        id
    }

    /// Finalize the DAG: connect all wire heads to their Out nodes.
    /// Call this after all operations have been appended.
    pub fn finalize(&mut self) {
        for w in 0..self.num_qubits {
            let head = self.wire_heads[w];
            let out = self.output_nodes[w];
            self.add_edge(head, out, w);
        }
    }

    // ── Optimization support ────────────────────────────────

    /// Remove a node by setting its tombstone. Rewires predecessors
    /// to successors on each wire so the DAG remains connected.
    pub fn remove_node(&mut self, id: NodeId) {
        if self.nodes[id].removed {
            return;
        }
        self.nodes[id].removed = true;

        // For each wire this node touches, splice it out:
        // pred --wire--> [removed] --wire--> succ  →  pred --wire--> succ
        let pred_edges: Vec<(NodeId, usize)> = self.pred[id].clone();
        let succ_edges: Vec<(NodeId, usize)> = self.succ[id].clone();

        for (pred_id, wire) in &pred_edges {
            // Remove the edge pred → id on this wire.
            self.succ[*pred_id].retain(|(to, w)| !(*to == id && *w == *wire));
            // Find the successor on this same wire.
            if let Some((succ_id, _)) = succ_edges.iter().find(|(_, w)| w == wire) {
                // Remove the edge id → succ on this wire.
                self.pred[*succ_id].retain(|(from, w)| !(*from == id && *w == *wire));
                // Add direct edge pred → succ.
                self.succ[*pred_id].push((*succ_id, *wire));
                self.pred[*succ_id].push((*pred_id, *wire));
            }
        }

        // Clear this node's adjacency.
        self.succ[id].clear();
        self.pred[id].clear();
    }

    // ── Traversal ───────────────────────────────────────────

    /// Topological sort using Kahn's algorithm.
    /// Returns node IDs in dependency order, skipping removed nodes.
    pub fn topo_order(&self) -> Vec<NodeId> {
        let n = self.nodes.len();
        let mut in_degree = vec![0usize; n];

        for id in 0..n {
            if self.nodes[id].removed {
                continue;
            }
            for (succ, _) in &self.succ[id] {
                if !self.nodes[*succ].removed {
                    in_degree[*succ] += 1;
                }
            }
        }

        let mut queue = VecDeque::new();
        for id in 0..n {
            if !self.nodes[id].removed && in_degree[id] == 0 {
                queue.push_back(id);
            }
        }

        let mut order = Vec::with_capacity(n);
        while let Some(id) = queue.pop_front() {
            order.push(id);
            for (succ, _) in &self.succ[id] {
                if !self.nodes[*succ].removed {
                    in_degree[*succ] -= 1;
                    if in_degree[*succ] == 0 {
                        queue.push_back(*succ);
                    }
                }
            }
        }

        order
    }

    /// Iterate live gate nodes in topological order.
    pub fn gates_topo(&self) -> Vec<NodeId> {
        self.topo_order()
            .into_iter()
            .filter(|id| !self.nodes[*id].removed && self.nodes[*id].op.is_gate())
            .collect()
    }

    /// Iterate all live operation nodes (gates + measure + reset + barrier)
    /// in topological order.
    pub fn ops_topo(&self) -> Vec<NodeId> {
        self.topo_order()
            .into_iter()
            .filter(|id| {
                let node = &self.nodes[*id];
                !node.removed && !matches!(node.op, Op::In { .. } | Op::Out { .. })
            })
            .collect()
    }

    // ── Circuit metrics ─────────────────────────────────────

    /// Circuit depth: length of the longest path through gate nodes.
    pub fn depth(&self) -> usize {
        let n = self.nodes.len();
        let order = self.topo_order();
        let mut dist = vec![0usize; n];

        for id in &order {
            let d = dist[*id];
            let increment = if self.nodes[*id].op.is_gate() { 1 } else { 0 };
            for (succ, _) in &self.succ[*id] {
                if !self.nodes[*succ].removed {
                    let new_d = d + increment;
                    if new_d > dist[*succ] {
                        dist[*succ] = new_d;
                    }
                }
            }
        }

        *dist.iter().max().unwrap_or(&0)
    }

    // ── QASM emission ───────────────────────────────────────

    /// Emit OpenQASM 3 from the DAG by topological traversal.
    pub fn emit_qasm(&self) -> String {
        let mut out = String::new();
        out.push_str("OPENQASM 3;\n");

        // Emit declarations.
        let mut emitted_regs = std::collections::HashSet::new();
        for (name, idx) in &self.qubit_names {
            if emitted_regs.insert(("qubit", name.clone())) {
                if let Some(_) = idx {
                    // Find register size.
                    let size = self
                        .qubit_names
                        .iter()
                        .filter(|(n, _)| n == name)
                        .count();
                    out.push_str(&format!("qubit[{}] {};\n", size, name));
                } else {
                    out.push_str(&format!("qubit {};\n", name));
                }
            }
        }
        for (name, idx) in &self.bit_names {
            if emitted_regs.insert(("bit", name.clone())) {
                if let Some(_) = idx {
                    let size = self.bit_names.iter().filter(|(n, _)| n == name).count();
                    out.push_str(&format!("bit[{}] {};\n", size, name));
                } else {
                    out.push_str(&format!("bit {};\n", name));
                }
            }
        }

        // Emit operations in topological order.
        for id in self.ops_topo() {
            let node = &self.nodes[id];
            match &node.op {
                Op::Gate {
                    name,
                    modifiers,
                    params,
                    qubits,
                } => {
                    for m in modifiers {
                        match m {
                            Modifier::Ctrl(arg) => {
                                out.push_str("ctrl");
                                if let Some(n) = arg {
                                    out.push_str(&format!("({})", n));
                                }
                                out.push_str(" @ ");
                            }
                            Modifier::NegCtrl(arg) => {
                                out.push_str("negctrl");
                                if let Some(n) = arg {
                                    out.push_str(&format!("({})", n));
                                }
                                out.push_str(" @ ");
                            }
                            Modifier::Inv => out.push_str("inv @ "),
                            Modifier::Pow(p) => {
                                out.push_str(&format!("pow({}) @ ", p));
                            }
                        }
                    }
                    out.push_str(name);
                    if !params.is_empty() {
                        out.push('(');
                        for (i, p) in params.iter().enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            out.push_str(&p.to_string());
                        }
                        out.push(')');
                    }
                    out.push(' ');
                    for (i, w) in qubits.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        self.emit_wire_name(&mut out, *w);
                    }
                    out.push_str(";\n");
                }
                Op::Measure { qubit, bit } => {
                    if let Some(b) = bit {
                        self.emit_bit_name(&mut out, *b);
                        out.push_str(" = measure ");
                    } else {
                        out.push_str("measure ");
                    }
                    self.emit_wire_name(&mut out, *qubit);
                    out.push_str(";\n");
                }
                Op::Reset { qubit } => {
                    out.push_str("reset ");
                    self.emit_wire_name(&mut out, *qubit);
                    out.push_str(";\n");
                }
                Op::Barrier { qubits } => {
                    out.push_str("barrier ");
                    for (i, w) in qubits.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        self.emit_wire_name(&mut out, *w);
                    }
                    out.push_str(";\n");
                }
                _ => {}
            }
        }

        out
    }

    /// Write the original name for a qubit wire index.
    fn emit_wire_name(&self, out: &mut String, wire: usize) {
        if wire < self.qubit_names.len() {
            let (name, idx) = &self.qubit_names[wire];
            out.push_str(name);
            if let Some(i) = idx {
                out.push_str(&format!("[{}]", i));
            }
        } else {
            out.push_str(&format!("q{}", wire));
        }
    }

    /// Write the original name for a classical bit wire index.
    fn emit_bit_name(&self, out: &mut String, bit: usize) {
        if bit < self.bit_names.len() {
            let (name, idx) = &self.bit_names[bit];
            out.push_str(name);
            if let Some(i) = idx {
                out.push_str(&format!("[{}]", i));
            }
        } else {
            out.push_str(&format!("c{}", bit));
        }
    }
}

impl fmt::Display for CircuitDAG {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "CircuitDAG({} qubits, {} ops, depth {})",
            self.num_qubits, self.op_count(), self.depth())?;
        for id in self.ops_topo() {
            let node = &self.nodes[id];
            write!(f, "  [{}] ", id)?;
            match &node.op {
                Op::Gate { name, qubits, params, modifiers } => {
                    for m in modifiers {
                        match m {
                            Modifier::Ctrl(_) => write!(f, "ctrl @ ")?,
                            Modifier::NegCtrl(_) => write!(f, "negctrl @ ")?,
                            Modifier::Inv => write!(f, "inv @ ")?,
                            Modifier::Pow(p) => write!(f, "pow({}) @ ", p)?,
                        }
                    }
                    write!(f, "{}", name)?;
                    if !params.is_empty() {
                        write!(f, "(")?;
                        for (i, p) in params.iter().enumerate() {
                            if i > 0 { write!(f, ", ")?; }
                            write!(f, "{}", p)?;
                        }
                        write!(f, ")")?;
                    }
                    write!(f, " ")?;
                    for (i, w) in qubits.iter().enumerate() {
                        if i > 0 { write!(f, ", ")?; }
                        write!(f, "w{}", w)?;
                    }
                    writeln!(f)?;
                }
                Op::Measure { qubit, bit } => {
                    write!(f, "measure w{}", qubit)?;
                    if let Some(b) = bit { write!(f, " -> b{}", b)?; }
                    writeln!(f)?;
                }
                Op::Reset { qubit } => writeln!(f, "reset w{}", qubit)?,
                Op::Barrier { qubits } => {
                    write!(f, "barrier ")?;
                    for (i, w) in qubits.iter().enumerate() {
                        if i > 0 { write!(f, ", ")?; }
                        write!(f, "w{}", w)?;
                    }
                    writeln!(f)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_circuit() {
        let mut dag = CircuitDAG::new(2, 0);
        dag.finalize();
        assert_eq!(dag.gate_count(), 0);
        assert_eq!(dag.depth(), 0);
        assert_eq!(dag.num_qubits, 2);
    }

    #[test]
    fn single_gate() {
        let mut dag = CircuitDAG::new(1, 0);
        dag.append_gate("h".into(), vec![], vec![], vec![0]);
        dag.finalize();
        assert_eq!(dag.gate_count(), 1);
        assert_eq!(dag.depth(), 1);
    }

    #[test]
    fn two_gates_same_wire() {
        let mut dag = CircuitDAG::new(1, 0);
        let g1 = dag.append_gate("h".into(), vec![], vec![], vec![0]);
        let g2 = dag.append_gate("x".into(), vec![], vec![], vec![0]);
        dag.finalize();

        assert_eq!(dag.gate_count(), 2);
        assert_eq!(dag.depth(), 2);
        // g1 should precede g2 on wire 0.
        assert_eq!(dag.wire_successor(g1, 0), Some(g2));
        assert_eq!(dag.wire_predecessor(g2, 0), Some(g1));
    }

    #[test]
    fn parallel_gates() {
        let mut dag = CircuitDAG::new(2, 0);
        dag.append_gate("h".into(), vec![], vec![], vec![0]);
        dag.append_gate("x".into(), vec![], vec![], vec![1]);
        dag.finalize();

        assert_eq!(dag.gate_count(), 2);
        // Parallel gates → depth 1.
        assert_eq!(dag.depth(), 1);
    }

    #[test]
    fn cx_gate() {
        let mut dag = CircuitDAG::new(2, 0);
        dag.append_gate("h".into(), vec![], vec![], vec![0]);
        dag.append_gate("cx".into(), vec![], vec![], vec![0, 1]);
        dag.finalize();

        assert_eq!(dag.gate_count(), 2);
        assert_eq!(dag.depth(), 2);
    }

    #[test]
    fn remove_node_rewires() {
        let mut dag = CircuitDAG::new(1, 0);
        let g1 = dag.append_gate("h".into(), vec![], vec![], vec![0]);
        let g2 = dag.append_gate("x".into(), vec![], vec![], vec![0]);
        let g3 = dag.append_gate("h".into(), vec![], vec![], vec![0]);
        dag.finalize();

        // Remove g2 — g1 should now connect directly to g3.
        dag.remove_node(g2);
        assert_eq!(dag.gate_count(), 2);
        assert_eq!(dag.wire_successor(g1, 0), Some(g3));
        assert_eq!(dag.wire_predecessor(g3, 0), Some(g1));
    }

    #[test]
    fn topo_order_respects_dependencies() {
        let mut dag = CircuitDAG::new(2, 0);
        let h = dag.append_gate("h".into(), vec![], vec![], vec![0]);
        let cx = dag.append_gate("cx".into(), vec![], vec![], vec![0, 1]);
        dag.finalize();

        let order = dag.gates_topo();
        let h_pos = order.iter().position(|&id| id == h).unwrap();
        let cx_pos = order.iter().position(|&id| id == cx).unwrap();
        assert!(h_pos < cx_pos, "h must come before cx");
    }
}
