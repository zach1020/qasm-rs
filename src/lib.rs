//! OpenQASM 3 parsing, semantic analysis, circuit lowering, optimization, and
//! emission.
//!
//! Use [`compile_source`] for in-memory programs. File-based clients can call
//! [`include::load_with_includes`] before compilation to resolve local include
//! files recursively.

pub mod ast;
pub mod codegen;
pub mod hir;
pub mod include;
pub mod inline;
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
    /// Run the default, semantics-preserving optimization pipeline.
    pub optimize: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self { optimize: true }
    }
}

pub struct CompileOutput {
    /// Canonically emitted OpenQASM.
    pub qasm: String,
    /// Lowered program with circuit DAG regions.
    pub hir: hir::HirProgram,
    /// Non-fatal diagnostics, currently warnings.
    pub diagnostics: Vec<sema::Diagnostic>,
    /// Number of gates removed by optimization.
    pub gates_removed: usize,
}

#[derive(Debug)]
pub enum CompileError {
    /// Invalid source bytes and their spans.
    Lex(Vec<Span>),
    /// One or more syntax errors recovered from the source.
    Parse(Vec<parser::ParseError>),
    /// Semantic or linearity errors.
    Semantic(Vec<sema::Diagnostic>),
    /// Failure while lowering a semantically valid AST.
    Lower(lower::LowerError),
}

/// Compile OpenQASM source through parsing, semantic analysis, HIR lowering,
/// optional optimization, and canonical emission.
pub fn compile_source(
    source: &str,
    options: CompileOptions,
) -> Result<CompileOutput, CompileError> {
    let (_, lex_errors) = lexer::lex(source);
    if !lex_errors.is_empty() {
        return Err(CompileError::Lex(lex_errors));
    }

    let mut parser = parser::Parser::new(source);
    let program = parser.parse_recovering().map_err(CompileError::Parse)?;

    let diagnostics = sema::analyze(&program);
    let has_errors = diagnostics
        .iter()
        .any(|d| matches!(d.severity, sema::Severity::Error));
    if has_errors {
        return Err(CompileError::Semantic(diagnostics));
    }

    let mut hir = lower::lower_hir(&program).map_err(CompileError::Lower)?;
    let gates_removed = if options.optimize {
        opt::optimize_hir(&mut hir).gates_removed
    } else {
        0
    };

    let qasm = hir.emit_qasm();
    Ok(CompileOutput {
        qasm,
        hir,
        diagnostics,
        gates_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_source_preserves_control_flow() {
        let source =
            "OPENQASM 3.0; qubit q; bool flag = true; if (flag == true) { h q; } else { x q; }";
        let output = compile_source(source, CompileOptions { optimize: false })
            .expect("control flow should lower to HIR");

        assert!(output.qasm.contains("if (flag == true)"));
        assert!(output.qasm.contains("h q;"));
        assert!(output.qasm.contains("x q;"));
        assert_eq!(output.hir.gate_count(), 2);
    }

    #[test]
    fn compile_source_optimizes_nested_circuit_regions() {
        let source = "OPENQASM 3.0; qubit q; if (true) { h q; h q; }";
        let output = compile_source(source, CompileOptions { optimize: true })
            .expect("nested circuit region should optimize");

        assert!(output.qasm.contains("if (true)"));
        assert!(!output.qasm.contains("h q;"));
        assert_eq!(output.gates_removed, 2);
        assert_eq!(output.hir.gate_count(), 0);
    }
}
