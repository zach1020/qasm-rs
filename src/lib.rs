pub mod ast;
pub mod codegen;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod opt;
pub mod parser;
pub mod sema;
pub mod span;

use span::Span;

#[derive(Debug, Clone, Copy)]
pub struct CompileOptions {
    pub optimize: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self { optimize: true }
    }
}

pub struct CompileOutput {
    pub qasm: String,
    pub dag: ir::CircuitDAG,
    pub diagnostics: Vec<sema::Diagnostic>,
    pub gates_removed: usize,
}

#[derive(Debug)]
pub enum CompileError {
    Lex(Vec<Span>),
    Parse(parser::ParseError),
    Semantic(Vec<sema::Diagnostic>),
    Lower(lower::LowerError),
}

pub fn compile_source(
    source: &str,
    options: CompileOptions,
) -> Result<CompileOutput, CompileError> {
    let (_, lex_errors) = lexer::lex(source);
    if !lex_errors.is_empty() {
        return Err(CompileError::Lex(lex_errors));
    }

    let mut parser = parser::Parser::new(source);
    let program = parser.parse().map_err(CompileError::Parse)?;

    let diagnostics = sema::analyze(&program);
    let has_errors = diagnostics
        .iter()
        .any(|d| matches!(d.severity, sema::Severity::Error));
    if has_errors {
        return Err(CompileError::Semantic(diagnostics));
    }

    let mut dag = lower::lower(&program).map_err(CompileError::Lower)?;
    let gates_removed = if options.optimize {
        opt::cancel_inverses(&mut dag).gates_removed
    } else {
        0
    };

    let qasm = dag.emit_qasm();
    Ok(CompileOutput {
        qasm,
        dag,
        diagnostics,
        gates_removed,
    })
}
