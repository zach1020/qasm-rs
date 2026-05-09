use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use ariadne::{Color, Label, Report, ReportKind, Source};
use clap::Parser;
use qasm_rs::{compile_source, CompileError, CompileOptions};

#[derive(Debug, Parser)]
#[command(
    name = "qasm-rs",
    version,
    about = "Compile and optimize an OpenQASM 3 circuit"
)]
struct Args {
    /// Input OpenQASM 3 source file.
    input: PathBuf,

    /// Write emitted OpenQASM to this file instead of stdout.
    #[arg(short, long, value_name = "PATH")]
    emit: Option<PathBuf>,

    /// Skip optimization passes.
    #[arg(long)]
    no_optimize: bool,

    /// Print compiler statistics to stderr.
    #[arg(long)]
    stats: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let source_name = args.input.display().to_string();

    let source = match fs::read_to_string(&args.input) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("qasm-rs: failed to read {}: {}", source_name, err);
            return ExitCode::FAILURE;
        }
    };

    let options = CompileOptions {
        optimize: !args.no_optimize,
    };

    let output = match compile_source(&source, options) {
        Ok(output) => output,
        Err(err) => {
            render_compile_error(&source_name, &source, err);
            return ExitCode::FAILURE;
        }
    };

    for diagnostic in output
        .diagnostics
        .iter()
        .filter(|d| matches!(d.severity, qasm_rs::sema::Severity::Warning))
    {
        render_sema_diagnostic(&source_name, &source, diagnostic);
    }

    if args.stats {
        eprintln!(
            "qasm-rs: qubits={}, bits={}, gates={}, depth={}, gates_removed={}",
            output.hir.num_qubits,
            output.hir.num_bits,
            output.hir.gate_count(),
            output.hir.depth(),
            output.gates_removed
        );
    }

    if let Some(path) = args.emit {
        if let Err(err) = fs::write(&path, output.qasm) {
            eprintln!("qasm-rs: failed to write {}: {}", path.display(), err);
            return ExitCode::FAILURE;
        }
    } else {
        print!("{}", output.qasm);
    }

    ExitCode::SUCCESS
}

fn render_compile_error(file_name: &str, source: &str, err: CompileError) {
    match err {
        CompileError::Lex(spans) => {
            for span in spans {
                Report::build(ReportKind::Error, file_name, span.start)
                    .with_message("unexpected character")
                    .with_label(
                        Label::new((file_name, span))
                            .with_message("this character is not valid in OpenQASM 3")
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint((file_name, Source::from(source)))
                    .unwrap();
            }
        }
        CompileError::Parse(err) => {
            Report::build(ReportKind::Error, file_name, err.span.start)
                .with_message(&err.message)
                .with_label(
                    Label::new((file_name, err.span.clone()))
                        .with_message(&err.message)
                        .with_color(Color::Red),
                )
                .finish()
                .eprint((file_name, Source::from(source)))
                .unwrap();
        }
        CompileError::Semantic(diagnostics) => {
            for diagnostic in diagnostics {
                render_sema_diagnostic(file_name, source, &diagnostic);
            }
        }
        CompileError::Lower(err) => {
            eprintln!("qasm-rs: {}", err);
        }
    }
}

fn render_sema_diagnostic(file_name: &str, source: &str, diagnostic: &qasm_rs::sema::Diagnostic) {
    let (kind, color) = match diagnostic.severity {
        qasm_rs::sema::Severity::Error => (ReportKind::Error, Color::Red),
        qasm_rs::sema::Severity::Warning => (ReportKind::Warning, Color::Yellow),
    };

    let mut report = Report::build(kind, file_name, diagnostic.span.start)
        .with_message(&diagnostic.message)
        .with_label(
            Label::new((file_name, diagnostic.span.clone()))
                .with_message(&diagnostic.message)
                .with_color(color),
        );

    if let Some((note, note_span)) = &diagnostic.secondary {
        report = report.with_label(
            Label::new((file_name, note_span.clone()))
                .with_message(note)
                .with_color(Color::Blue),
        );
    }

    report
        .finish()
        .eprint((file_name, Source::from(source)))
        .unwrap();
}
