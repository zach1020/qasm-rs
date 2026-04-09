mod ast;
mod lexer;
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

    if !has_errors && diagnostics.is_empty() {
        println!("✓ no errors");
    } else if !has_errors {
        println!("✓ no errors ({} warning(s))", diagnostics.len());
    }

    println!("  {} statement(s) parsed", program.statements.len());
    println!();
}

fn main() {
    // Valid program.
    compile(
        "bell.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         bit[2] c;\n\
         h q[0];\n\
         cx q[0], q[1];\n\
         c = measure q;\n",
    );

    // Use after measurement — no-cloning violation.
    compile(
        "use_after_measure.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         bit[2] c;\n\
         h q[0];\n\
         c = measure q;\n\
         cx q[0], q[1];\n",
    );

    // Reset clears measured state.
    compile(
        "reset_ok.qasm",
        "OPENQASM 3.0;\n\
         qubit q;\n\
         bit c;\n\
         h q;\n\
         measure q;\n\
         reset q;\n\
         h q;\n",
    );

    // Undeclared + out of bounds.
    compile(
        "bad_refs.qasm",
        "OPENQASM 3.0;\n\
         qubit[2] q;\n\
         h r[0];\n\
         cx q[0], q[5];\n",
    );

    // Kind mismatch: gate on a bit.
    compile(
        "kind_mismatch.qasm",
        "OPENQASM 3.0;\n\
         bit c;\n\
         h c;\n",
    );
}