mod ast;
mod lexer;
mod parser;
mod span;

use ariadne::{Color, Label, Report, ReportKind, Source};
use parser::Parser;

fn compile(name: &str, source: &str) {
    println!("── {} ──", name);

    // Report lex errors first.
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

    let mut parser = Parser::new(source);
    match parser.parse() {
        Ok(program) => {
            println!("Parsed successfully — version {}", program.version);
            println!("{} statement(s):", program.statements.len());
            for stmt in &program.statements {
                println!("  {:?}", stmt);
            }
        }
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
        }
    }
    println!();
}

fn main() {
    compile(
        "bell.qasm",
        r#"OPENQASM 3.0;
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
c = measure q;
"#,
    );

    compile(
        "bad_semicolon.qasm",
        r#"OPENQASM 3.0;
qubit[2] q
bit[2] c;
"#,
    );

    compile(
        "bad_token.qasm",
        r#"OPENQASM 3.0;
qubit # q;
"#,
    );
}
