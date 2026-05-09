//! High-level intermediate representation.
//!
//! HIR preserves OpenQASM program structure while allowing straight-line
//! quantum regions to be represented as circuit DAGs. This keeps classical
//! control flow explicit instead of forcing it into the circuit graph.

use crate::{ast, codegen, ir::CircuitDAG};

pub struct HirProgram {
    pub version: String,
    pub statements: Vec<HirStmt>,
    pub num_qubits: usize,
    pub num_bits: usize,
}

pub enum HirStmt {
    Ast(ast::Stmt),
    Circuit(CircuitDAG),
    If {
        condition: ast::Expr,
        then_body: Vec<HirStmt>,
        else_body: Option<Vec<HirStmt>>,
    },
    For {
        var_name: String,
        var_ty: ast::ClassicalType,
        range: ast::ForRange,
        body: Vec<HirStmt>,
    },
    While {
        condition: ast::Expr,
        body: Vec<HirStmt>,
    },
}

impl HirProgram {
    pub fn gate_count(&self) -> usize {
        gate_count_stmts(&self.statements)
    }

    pub fn op_count(&self) -> usize {
        op_count_stmts(&self.statements)
    }

    pub fn depth(&self) -> usize {
        depth_stmts(&self.statements)
    }

    pub fn emit_qasm(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("OPENQASM {};\n", self.version));

        for stmt in &self.statements {
            out.push('\n');
            emit_stmt(&mut out, stmt, 0);
        }

        out
    }
}

fn gate_count_stmts(stmts: &[HirStmt]) -> usize {
    stmts
        .iter()
        .map(|stmt| match stmt {
            HirStmt::Ast(_) => 0,
            HirStmt::Circuit(dag) => dag.gate_count(),
            HirStmt::If {
                then_body,
                else_body,
                ..
            } => gate_count_stmts(then_body) + else_body.as_deref().map_or(0, gate_count_stmts),
            HirStmt::For { body, .. } | HirStmt::While { body, .. } => gate_count_stmts(body),
        })
        .sum()
}

fn op_count_stmts(stmts: &[HirStmt]) -> usize {
    stmts
        .iter()
        .map(|stmt| match stmt {
            HirStmt::Ast(_) => 0,
            HirStmt::Circuit(dag) => dag.op_count(),
            HirStmt::If {
                then_body,
                else_body,
                ..
            } => op_count_stmts(then_body) + else_body.as_deref().map_or(0, op_count_stmts),
            HirStmt::For { body, .. } | HirStmt::While { body, .. } => op_count_stmts(body),
        })
        .sum()
}

fn depth_stmts(stmts: &[HirStmt]) -> usize {
    stmts
        .iter()
        .map(|stmt| match stmt {
            HirStmt::Ast(_) => 0,
            HirStmt::Circuit(dag) => dag.depth(),
            HirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                let then_depth = depth_stmts(then_body);
                let else_depth = else_body.as_deref().map_or(0, depth_stmts);
                then_depth.max(else_depth)
            }
            HirStmt::For { body, .. } | HirStmt::While { body, .. } => depth_stmts(body),
        })
        .sum()
}

fn emit_stmt(out: &mut String, stmt: &HirStmt, depth: usize) {
    match stmt {
        HirStmt::Ast(stmt) => codegen::emit_stmt(out, stmt, depth),
        HirStmt::Circuit(dag) => dag.emit_ops_qasm(out, depth),
        HirStmt::If {
            condition,
            then_body,
            else_body,
        } => {
            codegen::indent(out, depth);
            out.push_str("if (");
            codegen::emit_expr(out, condition);
            out.push_str(") {\n");
            for stmt in then_body {
                emit_stmt(out, stmt, depth + 1);
            }
            codegen::indent(out, depth);
            if let Some(else_stmts) = else_body {
                out.push_str("} else {\n");
                for stmt in else_stmts {
                    emit_stmt(out, stmt, depth + 1);
                }
                codegen::indent(out, depth);
                out.push_str("}\n");
            } else {
                out.push_str("}\n");
            }
        }
        HirStmt::For {
            var_name,
            var_ty,
            range,
            body,
        } => {
            codegen::indent(out, depth);
            out.push_str(&format!("for {} {} in [", var_ty, var_name));
            codegen::emit_expr(out, &range.start);
            out.push(':');
            if let Some(step) = &range.step {
                codegen::emit_expr(out, step);
                out.push(':');
            }
            codegen::emit_expr(out, &range.end);
            out.push_str("] {\n");
            for stmt in body {
                emit_stmt(out, stmt, depth + 1);
            }
            codegen::indent(out, depth);
            out.push_str("}\n");
        }
        HirStmt::While { condition, body } => {
            codegen::indent(out, depth);
            out.push_str("while (");
            codegen::emit_expr(out, condition);
            out.push_str(") {\n");
            for stmt in body {
                emit_stmt(out, stmt, depth + 1);
            }
            codegen::indent(out, depth);
            out.push_str("}\n");
        }
    }
}
