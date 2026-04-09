mod ast;
mod codegen;
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

    if has_errors {
        println!("  ✗ {} error(s) — codegen skipped\n", diagnostics.len());
        return;
    }

    // 4. Codegen — emit canonical QASM.
    let output = codegen::emit(&program);
    println!("  ✓ emitted QASM:\n");
    for line in output.lines() {
        println!("    {}", line);
    }
    println!();
}

fn main() {
    // Valid Bell pair — full pipeline.
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

    // Reset clears measured state — should pass.
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
}
