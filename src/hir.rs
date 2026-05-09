//! High-level intermediate representation.
//!
//! HIR preserves OpenQASM program structure while allowing straight-line
//! quantum regions to be represented as circuit DAGs. This keeps classical
//! control flow explicit instead of forcing it into the circuit graph.

use crate::{ast, codegen, ir::CircuitDAG};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthEstimate {
    Exact(usize),
    AtLeast(usize),
}

impl DepthEstimate {
    pub fn lower_bound(self) -> usize {
        match self {
            DepthEstimate::Exact(n) | DepthEstimate::AtLeast(n) => n,
        }
    }

    fn add(self, other: Self) -> Self {
        match (self, other) {
            (DepthEstimate::Exact(a), DepthEstimate::Exact(b)) => {
                DepthEstimate::Exact(a.saturating_add(b))
            }
            (a, b) => DepthEstimate::AtLeast(a.lower_bound().saturating_add(b.lower_bound())),
        }
    }

    fn repeat(self, count: usize) -> Self {
        match self {
            DepthEstimate::Exact(n) => DepthEstimate::Exact(n.saturating_mul(count)),
            DepthEstimate::AtLeast(n) => DepthEstimate::AtLeast(n.saturating_mul(count)),
        }
    }

    fn branch(a: Self, b: Self) -> Self {
        match (a, b) {
            (DepthEstimate::Exact(a), DepthEstimate::Exact(b)) => DepthEstimate::Exact(a.max(b)),
            (a, b) => DepthEstimate::AtLeast(a.lower_bound().max(b.lower_bound())),
        }
    }
}

impl std::fmt::Display for DepthEstimate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DepthEstimate::Exact(n) => write!(f, "{}", n),
            DepthEstimate::AtLeast(n) => write!(f, ">={}", n),
        }
    }
}

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
        self.depth_estimate().lower_bound()
    }

    pub fn depth_estimate(&self) -> DepthEstimate {
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

fn depth_stmts(stmts: &[HirStmt]) -> DepthEstimate {
    stmts
        .iter()
        .map(depth_stmt)
        .fold(DepthEstimate::Exact(0), DepthEstimate::add)
}

fn depth_stmt(stmt: &HirStmt) -> DepthEstimate {
    match stmt {
        HirStmt::Ast(_) => DepthEstimate::Exact(0),
        HirStmt::Circuit(dag) => DepthEstimate::Exact(dag.depth()),
        HirStmt::If {
            condition,
            then_body,
            else_body,
        } => match eval_bool(condition) {
            Some(true) => depth_stmts(then_body),
            Some(false) => else_body
                .as_deref()
                .map_or(DepthEstimate::Exact(0), depth_stmts),
            None => {
                let then_depth = depth_stmts(then_body);
                let else_depth = else_body
                    .as_deref()
                    .map_or(DepthEstimate::Exact(0), depth_stmts);
                DepthEstimate::branch(then_depth, else_depth)
            }
        },
        HirStmt::For { range, body, .. } => {
            let body_depth = depth_stmts(body);
            match range_trip_count(range) {
                Some(count) => body_depth.repeat(count),
                None => DepthEstimate::AtLeast(0),
            }
        }
        HirStmt::While { condition, body } => match eval_bool(condition) {
            Some(false) => DepthEstimate::Exact(0),
            Some(true) => DepthEstimate::AtLeast(depth_stmts(body).lower_bound()),
            None => DepthEstimate::AtLeast(0),
        },
    }
}

fn range_trip_count(range: &ast::ForRange) -> Option<usize> {
    let start = eval_int(&range.start)?;
    let end = eval_int(&range.end)?;
    let step = match &range.step {
        Some(step) => eval_int(step)?,
        None => 1,
    };

    if step == 0 {
        return None;
    }

    let distance = if step > 0 {
        if start > end {
            return Some(0);
        }
        end.checked_sub(start)?
    } else {
        if start < end {
            return Some(0);
        }
        start.checked_sub(end)?
    };

    let step_abs = step.checked_abs()?;
    let intervals = distance.checked_div(step_abs)?;
    intervals.checked_add(1)?.try_into().ok()
}

fn eval_bool(expr: &ast::Expr) -> Option<bool> {
    match expr {
        ast::Expr::BoolLit(value, _) => Some(*value),
        ast::Expr::Compare { op, lhs, rhs, .. } => {
            if let (Some(lhs), Some(rhs)) = (eval_int(lhs), eval_int(rhs)) {
                return Some(match op {
                    ast::CompareOp::Eq => lhs == rhs,
                    ast::CompareOp::Ne => lhs != rhs,
                    ast::CompareOp::Lt => lhs < rhs,
                    ast::CompareOp::Le => lhs <= rhs,
                    ast::CompareOp::Gt => lhs > rhs,
                    ast::CompareOp::Ge => lhs >= rhs,
                });
            }
            None
        }
        _ => None,
    }
}

fn eval_int(expr: &ast::Expr) -> Option<i128> {
    match expr {
        ast::Expr::IntLit(value, _) => Some((*value).into()),
        ast::Expr::Neg(inner, _) => eval_int(inner)?.checked_neg(),
        ast::Expr::BinOp { op, lhs, rhs, .. } => {
            let lhs = eval_int(lhs)?;
            let rhs = eval_int(rhs)?;
            match op {
                ast::BinOp::Add => lhs.checked_add(rhs),
                ast::BinOp::Sub => lhs.checked_sub(rhs),
                ast::BinOp::Mul => lhs.checked_mul(rhs),
                ast::BinOp::Div => {
                    if rhs == 0 {
                        None
                    } else {
                        lhs.checked_div(rhs)
                    }
                }
                ast::BinOp::Pow => {
                    let exp: u32 = rhs.try_into().ok()?;
                    lhs.checked_pow(exp)
                }
            }
        }
        _ => None,
    }
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
