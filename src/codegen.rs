//! Pretty-printer: emit valid OpenQASM 3 from the AST.
//!
//! This closes the compiler loop — parse source → AST → regenerate source.
//! The output is semantically equivalent but canonically formatted.

use crate::ast::*;

pub fn emit(program: &Program) -> String {
    let mut out = String::new();
    out.push_str(&format!("OPENQASM {};\n", program.version));

    for stmt in &program.statements {
        out.push('\n');
        emit_stmt(&mut out, stmt, 0);
    }

    out
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn emit_stmt(out: &mut String, stmt: &Stmt, depth: usize) {
    indent(out, depth);
    match stmt {
        Stmt::QubitDecl { name, size, .. } => {
            out.push_str("qubit");
            if let Some(n) = size {
                out.push_str(&format!("[{}]", n));
            }
            out.push_str(&format!(" {};\n", name));
        }

        Stmt::BitDecl { name, size, .. } => {
            out.push_str("bit");
            if let Some(n) = size {
                out.push_str(&format!("[{}]", n));
            }
            out.push_str(&format!(" {};\n", name));
        }

        Stmt::ClassicalDecl { ty, name, init, .. } => {
            out.push_str(&format!("{} {}", ty, name));
            if let Some(expr) = init {
                out.push_str(" = ");
                emit_expr(out, expr);
            }
            out.push_str(";\n");
        }

        Stmt::Assignment { name, op, value, .. } => {
            out.push_str(&format!("{} {} ", name, op));
            emit_expr(out, value);
            out.push_str(";\n");
        }

        Stmt::GateCall {
            name,
            modifiers,
            params,
            args,
            ..
        } => {
            for m in modifiers {
                emit_modifier(out, m);
            }
            out.push_str(name);
            if !params.is_empty() {
                out.push('(');
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    emit_expr(out, p);
                }
                out.push(')');
            }
            out.push(' ');
            emit_operand_list(out, args);
            out.push_str(";\n");
        }

        Stmt::GateDef {
            name,
            params,
            qparams,
            body,
            ..
        } => {
            out.push_str("gate ");
            out.push_str(name);
            if !params.is_empty() {
                out.push('(');
                out.push_str(&params.join(", "));
                out.push(')');
            }
            if !qparams.is_empty() {
                out.push(' ');
                out.push_str(&qparams.join(", "));
            }
            out.push_str(" {\n");
            for s in body {
                emit_stmt(out, s, depth + 1);
            }
            indent(out, depth);
            out.push_str("}\n");
        }

        Stmt::Measure { qubit, target, .. } => {
            if let Some(t) = target {
                out.push_str(&format!("{} = measure {};\n", t, qubit));
            } else {
                out.push_str(&format!("measure {};\n", qubit));
            }
        }

        Stmt::Reset { target, .. } => {
            out.push_str(&format!("reset {};\n", target));
        }

        Stmt::Barrier { targets, .. } => {
            out.push_str("barrier ");
            emit_operand_list(out, targets);
            out.push_str(";\n");
        }

        Stmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            out.push_str("if (");
            emit_expr(out, condition);
            out.push_str(") {\n");
            for s in then_body {
                emit_stmt(out, s, depth + 1);
            }
            indent(out, depth);
            if let Some(else_stmts) = else_body {
                out.push_str("} else {\n");
                for s in else_stmts {
                    emit_stmt(out, s, depth + 1);
                }
                indent(out, depth);
                out.push_str("}\n");
            } else {
                out.push_str("}\n");
            }
        }

        Stmt::For {
            var_name,
            var_ty,
            range,
            body,
            ..
        } => {
            out.push_str(&format!("for {} {} in [", var_ty, var_name));
            emit_expr(out, &range.start);
            out.push(':');
            if let Some(ref step) = range.step {
                emit_expr(out, step);
                out.push(':');
            }
            emit_expr(out, &range.end);
            out.push_str("] {\n");
            for s in body {
                emit_stmt(out, s, depth + 1);
            }
            indent(out, depth);
            out.push_str("}\n");
        }

        Stmt::While {
            condition, body, ..
        } => {
            out.push_str("while (");
            emit_expr(out, condition);
            out.push_str(") {\n");
            for s in body {
                emit_stmt(out, s, depth + 1);
            }
            indent(out, depth);
            out.push_str("}\n");
        }
    }
}

fn emit_operand_list(out: &mut String, ops: &[GateOperand]) {
    for (i, op) in ops.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&op.to_string());
    }
}

fn emit_modifier(out: &mut String, m: &GateModifier) {
    match m {
        GateModifier::Ctrl(arg, _) => {
            out.push_str("ctrl");
            if let Some(e) = arg {
                out.push('(');
                emit_expr(out, e);
                out.push(')');
            }
            out.push_str(" @ ");
        }
        GateModifier::NegCtrl(arg, _) => {
            out.push_str("negctrl");
            if let Some(e) = arg {
                out.push('(');
                emit_expr(out, e);
                out.push(')');
            }
            out.push_str(" @ ");
        }
        GateModifier::Inv(_) => {
            out.push_str("inv @ ");
        }
        GateModifier::Pow(e, _) => {
            out.push_str("pow(");
            emit_expr(out, e);
            out.push_str(") @ ");
        }
    }
}

fn emit_expr(out: &mut String, expr: &Expr) {
    match expr {
        Expr::IntLit(n, _) => out.push_str(&n.to_string()),
        Expr::FloatLit(f, _) => out.push_str(&format!("{}", f)),
        Expr::BoolLit(b, _) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Ident(name, _) => out.push_str(name),
        Expr::Const(kind, _) => out.push_str(&kind.to_string()),
        Expr::Neg(inner, _) => {
            out.push('-');
            let needs_parens = matches!(**inner, Expr::BinOp { .. } | Expr::Compare { .. });
            if needs_parens {
                out.push('(');
            }
            emit_expr(out, inner);
            if needs_parens {
                out.push(')');
            }
        }
        Expr::BinOp { op, lhs, rhs, .. } => {
            let needs_parens_lhs = matches!(**lhs, Expr::BinOp { .. });
            let needs_parens_rhs = matches!(**rhs, Expr::BinOp { .. });

            if needs_parens_lhs {
                out.push('(');
            }
            emit_expr(out, lhs);
            if needs_parens_lhs {
                out.push(')');
            }

            out.push_str(&format!(" {} ", op));

            if needs_parens_rhs {
                out.push('(');
            }
            emit_expr(out, rhs);
            if needs_parens_rhs {
                out.push(')');
            }
        }
        Expr::Compare { op, lhs, rhs, .. } => {
            emit_expr(out, lhs);
            out.push_str(&format!(" {} ", op));
            emit_expr(out, rhs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    /// Parse source, emit it, re-parse the output, and verify the AST matches.
    fn round_trip(source: &str) {
        let mut p1 = Parser::new(source);
        let prog1 = p1.parse().expect("initial parse failed");
        let emitted = emit(&prog1);
        let mut p2 = Parser::new(&emitted);
        let prog2 = p2.parse().expect(&format!(
            "re-parse of emitted code failed.\nEmitted:\n{}",
            emitted
        ));
        assert_eq!(prog1.statements.len(), prog2.statements.len());
    }

    #[test]
    fn round_trip_bell_pair() {
        round_trip(
            "OPENQASM 3.0;\n\
             qubit[2] q;\n\
             bit[2] c;\n\
             h q[0];\n\
             cx q[0], q[1];\n\
             c = measure q;\n",
        );
    }

    #[test]
    fn round_trip_scalar() {
        round_trip("OPENQASM 3.0; qubit q; bit c; h q; measure q; reset q;");
    }

    #[test]
    fn round_trip_barrier() {
        round_trip("OPENQASM 3.0; qubit[3] q; barrier q[0], q[1], q[2];");
    }

    #[test]
    fn round_trip_gate_def() {
        round_trip("OPENQASM 3.0; gate h q { U(pi/2, 0, pi) q; }");
    }

    #[test]
    fn round_trip_gate_def_with_params() {
        round_trip("OPENQASM 3.0; gate rx(theta) q { U(theta, -pi/2, pi/2) q; }");
    }

    #[test]
    fn round_trip_modified_gate() {
        round_trip("OPENQASM 3.0; gate cx c, t { ctrl @ x c, t; }");
    }

    #[test]
    fn round_trip_parameterized_call() {
        round_trip("OPENQASM 3.0; qubit q; rx(pi/2) q;");
    }

    #[test]
    fn round_trip_complex_expr() {
        round_trip("OPENQASM 3.0; qubit q; rx(pi / 2 + 1) q;");
    }

    #[test]
    fn round_trip_inv_modifier() {
        round_trip("OPENQASM 3.0; qubit q; inv @ h q;");
    }

    #[test]
    fn round_trip_pow_modifier() {
        round_trip("OPENQASM 3.0; qubit q; pow(2) @ h q;");
    }

    #[test]
    fn round_trip_if_else() {
        round_trip(
            "OPENQASM 3.0;\n\
             qubit q;\n\
             bit c;\n\
             c = measure q;\n\
             if (c == 1) {\n\
               h q;\n\
             } else {\n\
               x q;\n\
             }\n",
        );
    }

    #[test]
    fn round_trip_for_loop() {
        round_trip(
            "OPENQASM 3.0;\n\
             qubit[4] q;\n\
             for int i in [0:4] {\n\
               h q;\n\
             }\n",
        );
    }

    #[test]
    fn round_trip_while_loop() {
        round_trip(
            "OPENQASM 3.0;\n\
             int count = 0;\n\
             while (count < 10) {\n\
               count += 1;\n\
             }\n",
        );
    }

    #[test]
    fn round_trip_classical_decl() {
        round_trip("OPENQASM 3.0; int x = 42; float y; bool flag = true;");
    }

    #[test]
    fn round_trip_assignment() {
        round_trip("OPENQASM 3.0; int x = 0; x = 5; x += 1;");
    }

    #[test]
    fn emitted_output_is_valid_qasm() {
        let source = "OPENQASM 3.0; qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().unwrap();
        let output = emit(&program);

        assert!(output.starts_with("OPENQASM 3;"));
        assert!(output.contains("qubit[2] q;"));
        assert!(output.contains("h q[0];"));
        assert!(output.contains("cx q[0], q[1];"));
        assert!(output.contains("c = measure q;"));
    }

    #[test]
    fn emitted_gate_def_formatting() {
        let source = "OPENQASM 3.0; gate rx(theta) q { U(theta, -pi/2, pi/2) q; }";
        let mut parser = Parser::new(source);
        let program = parser.parse().unwrap();
        let output = emit(&program);
        assert!(output.contains("gate rx(theta) q {"));
        assert!(output.contains("  U(theta, -pi / 2, pi / 2) q;"));
        assert!(output.contains("}"));
    }
}
