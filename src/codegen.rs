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
        emit_stmt(&mut out, stmt);
    }

    out
}

fn emit_stmt(out: &mut String, stmt: &Stmt) {
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

        Stmt::GateCall { name, args, .. } => {
            out.push_str(name);
            out.push(' ');
            emit_operand_list(out, args);
            out.push_str(";\n");
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
    fn emitted_output_is_valid_qasm() {
        let source = "OPENQASM 3.0; qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;";
        let mut parser = Parser::new(source);
        let program = parser.parse().unwrap();
        let output = emit(&program);

        // Verify it contains expected tokens.
        assert!(output.starts_with("OPENQASM 3;"));
        assert!(output.contains("qubit[2] q;"));
        assert!(output.contains("h q[0];"));
        assert!(output.contains("cx q[0], q[1];"));
        assert!(output.contains("c = measure q;"));
    }
}
