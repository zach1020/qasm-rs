mod ast;
mod codegen;
mod ir;
mod lexer;
mod lower;
mod opt;
mod parser;
mod sema;
mod span;

use ariadne::{Color, Label, Report, ReportKind, Source};
use parser::Parser;
use sema::Severity;

fn compile(name: &str, source: &str) {
    println!("── {} ──", name);

    // 1. Lex errors.
    let (_, lex_errors) = lexer::lex(source);
    for err_span in &lex_errors {
        Report::build(ReportKind::Error, name, err_span.start)
            .with_message("unexpected character")
            .with_label(
                Label::new((name, err_span.clone()))
                    .with_message("this character is not valid in OpenQASM 3")
                    .with_color(Color::Red),
            )
            .finish()
            .eprint((name, Source::from(source)))
            .unwrap();
    }

    // 2. Parse.
    let mut parser = Parser::new(source);
    let program = match parser.parse() {
        Ok(p) => p,
        Err(e) => {
            Report::build(ReportKind::Error, name, e.span.start)
                .with_message(&e.message)
                .with_label(
                    Label::new((name, e.span.clone()))
                        .with_message(&e.message)
                        .with_color(Color::Red),
                )
                .finish()
                .eprint((name, Source::from(source)))
                .unwrap();
            println!();
            return;
        }
    };

    // 3. Semantic analysis.
    let diagnostics = sema::analyze(&program);
    let has_errors = diagnostics
        .iter()
        .any(|d| matches!(d.severity, Severity::Error));

    for diag in &diagnostics {
        let kind = match diag.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };
        let color = match diag.severity {
            Severity::Error => Color::Red,
            Severity::Warning => Color::Yellow,
        };

        let mut report = Report::build(kind, name, diag.span.start)
            .with_message(&diag.message)
            .with_label(
                Label::new((name, diag.span.clone()))
                    .with_message(&diag.message)
                    .with_color(color),
            );

        if let Some((note, note_span)) = &diag.secondary {
            report = report.with_label(
                Label::new((name, note_span.clone()))
                    .with_message(note)
                    .with_color(Color::Blue),
            );
        }

        report
            .finish()
            .eprint((name, Source::from(source)))
            .unwrap();
    }

    if has_errors {
        println!("  ✗ {} error(s) — codegen skipped\n", diagnostics.len());
        return;
    }

    // 4. Lower to IR.
    let mut dag = match lower::lower(&program) {
        Ok(d) => d,
        Err(e) => {
            println!("  ✗ {}\n", e);
            return;
        }
    };

    println!("  ✓ lowered to DAG: {} qubits, {} gates, depth {}",
        dag.num_qubits, dag.gate_count(), dag.depth());
    println!("{}", dag);

    // 5. Optimize.
    let before = dag.gate_count();
    let stats = opt::cancel_inverses(&mut dag);
    if stats.gates_removed > 0 {
        println!("  ⚡ optimization: removed {} gates ({} → {})",
            stats.gates_removed, before, dag.gate_count());
        println!("{}", dag);
    } else {
        println!("  ⚡ optimization: no cancellations found");
    }

    // 6. Emit optimized QASM.
    let output = dag.emit_qasm();
    println!("  ✓ emitted QASM:\n");
    for line in output.lines() {
        println!("    {}", line);
    }
    println!();
}

fn main() {
    // 1. Bell pair — full pipeline.
    compile(
        "bell.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         bit[2] c;\n\
         h q[0];\n\
         cx q[0], q[1];\n\
         c = measure q;\n",
    );

    // 2. Redundant gates — optimization demo.
    compile(
        "redundant.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         bit[2] c;\n\
         h q[0];\n\
         x q[0];\n\
         x q[0];\n\
         cx q[0], q[1];\n\
         c = measure q;\n",
    );

    // 3. Cascading cancellation: h·x·x·h → empty.
    compile(
        "cascade.qasm",
        "OPENQASM 3.0;\n\
         qubit q;\n\
         h q;\n\
         x q;\n\
         x q;\n\
         h q;\n",
    );

    // 4. CX·CX cancellation.
    compile(
        "cx_cancel.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         h q[0];\n\
         cx q[0], q[1];\n\
         cx q[0], q[1];\n\
         h q[0];\n",
    );

    // 5. Use after measurement — error demo.
    compile(
        "use_after_measure.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         bit[2] c;\n\
         h q[0];\n\
         c = measure q;\n\
         cx q[0], q[1];\n",
    );

    // 6. Undeclared + out of bounds — error demo.
    compile(
        "bad_refs.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         h r[0];\n\
         cx q[0], q[5];\n",
    );
}
