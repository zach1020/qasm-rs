//! Semantic analysis for OpenQASM 3.
//!
//! Pass 1 – Symbol resolution: duplicate declarations, undeclared names, index bounds.
//! Pass 2 – Qubit linearity: use-after-measure detection (no-cloning enforcement).

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::span::Span;

// ── Diagnostics ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    /// Optional secondary label (e.g. "first declared here").
    pub secondary: Option<(String, Span)>,
}

impl Diagnostic {
    fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
            secondary: None,
        }
    }

    fn error_with_note(
        message: impl Into<String>,
        span: Span,
        note: impl Into<String>,
        note_span: Span,
    ) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
            secondary: Some((note.into(), note_span)),
        }
    }

    fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span,
            secondary: None,
        }
    }
}

// ── Symbol table ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymbolKind {
    Qubit,
    Bit,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub size: Option<u64>,
    pub decl_span: Span,
}

// ── Analysis ────────────────────────────────────────────────

pub fn analyze(program: &Program) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut symbols: HashMap<String, Symbol> = HashMap::new();

    // Pass 1: collect declarations & validate references.
    for stmt in &program.statements {
        match stmt {
            Stmt::QubitDecl { name, size, span } => {
                declare(&mut symbols, &mut diags, name, SymbolKind::Qubit, *size, span);
            }
            Stmt::BitDecl { name, size, span } => {
                declare(&mut symbols, &mut diags, name, SymbolKind::Bit, *size, span);
            }
            Stmt::GateCall { args, .. } => {
                for op in args {
                    check_operand(&symbols, &mut diags, op, Some(SymbolKind::Qubit));
                }
            }
            Stmt::Measure { qubit, target, .. } => {
                check_operand(&symbols, &mut diags, qubit, Some(SymbolKind::Qubit));
                if let Some(t) = target {
                    check_operand(&symbols, &mut diags, t, Some(SymbolKind::Bit));
                }
            }
            Stmt::Reset { target, .. } => {
                check_operand(&symbols, &mut diags, target, Some(SymbolKind::Qubit));
            }
            Stmt::Barrier { targets, .. } => {
                for op in targets {
                    check_operand(&symbols, &mut diags, op, Some(SymbolKind::Qubit));
                }
            }
        }
    }

    // Pass 2: qubit linearity — use-after-measure.
    check_linearity(program, &symbols, &mut diags);

    diags
}

/// Register a declaration, erroring on duplicates.
fn declare(
    symbols: &mut HashMap<String, Symbol>,
    diags: &mut Vec<Diagnostic>,
    name: &str,
    kind: SymbolKind,
    size: Option<u64>,
    span: &Span,
) {
    if let Some(prev) = symbols.get(name) {
        diags.push(Diagnostic::error_with_note(
            format!("`{}` is already declared", name),
            span.clone(),
            format!("`{}` first declared here", name),
            prev.decl_span.clone(),
        ));
    } else {
        symbols.insert(
            name.to_string(),
            Symbol {
                kind,
                size,
                decl_span: span.clone(),
            },
        );
    }
}

/// Check that an operand references a declared symbol with valid index and correct kind.
fn check_operand(
    symbols: &HashMap<String, Symbol>,
    diags: &mut Vec<Diagnostic>,
    op: &GateOperand,
    expected_kind: Option<SymbolKind>,
) {
    let Some(sym) = symbols.get(&op.name) else {
        diags.push(Diagnostic::error(
            format!("`{}` is not declared", op.name),
            op.span.clone(),
        ));
        return;
    };

    // Kind mismatch.
    if let Some(expected) = expected_kind {
        if sym.kind != expected {
            let expected_str = match expected {
                SymbolKind::Qubit => "qubit",
                SymbolKind::Bit => "bit",
            };
            let found_str = match sym.kind {
                SymbolKind::Qubit => "qubit",
                SymbolKind::Bit => "bit",
            };
            diags.push(Diagnostic::error_with_note(
                format!("expected {}, but `{}` is a {}", expected_str, op.name, found_str),
                op.span.clone(),
                format!("`{}` declared as {} here", op.name, found_str),
                sym.decl_span.clone(),
            ));
        }
    }

    // Index bounds.
    if let Some(idx) = op.index {
        match sym.size {
            Some(size) if idx >= size => {
                diags.push(Diagnostic::error_with_note(
                    format!(
                        "index {} is out of bounds for `{}` (size {})",
                        idx, op.name, size
                    ),
                    op.span.clone(),
                    format!("`{}` declared with size {} here", op.name, size),
                    sym.decl_span.clone(),
                ));
            }
            None => {
                diags.push(Diagnostic::error_with_note(
                    format!(
                        "cannot index `{}` — it is a single qubit/bit, not a register",
                        op.name
                    ),
                    op.span.clone(),
                    format!("`{}` declared without a size here", op.name),
                    sym.decl_span.clone(),
                ));
            }
            _ => {} // in bounds
        }
    }
}

// ── Pass 2: qubit linearity (no-cloning) ────────────────────
//
// In quantum mechanics, qubits cannot be copied (no-cloning theorem) and
// measurement collapses a qubit's state irreversibly.  A qubit used after
// measurement is either a bug or requires an explicit `reset`.
//
// We track which individual qubit indices have been measured.  Any
// subsequent gate or barrier referencing a measured qubit is an error
// unless an intervening `reset` clears it.

fn check_linearity(
    program: &Program,
    symbols: &HashMap<String, Symbol>,
    diags: &mut Vec<Diagnostic>,
) {
    // Tracks (register_name, index) → span of the measure that consumed it.
    // index = None means the entire register was measured.
    let mut measured: HashMap<(String, Option<u64>), Span> = HashMap::new();

    for stmt in &program.statements {
        match stmt {
            Stmt::Measure { qubit, span, .. } => {
                // First check if this qubit is already measured (double measure).
                if let Some(prev_span) = lookup_measured(&measured, &qubit.name, qubit.index) {
                    diags.push(Diagnostic::warning(
                        format!("qubit `{}` has already been measured", qubit),
                        span.clone(),
                    ));
                }
                // Mark as measured.
                measured.insert(
                    (qubit.name.clone(), qubit.index),
                    span.clone(),
                );
                // If measured without index, mark all individual indices too.
                if qubit.index.is_none() {
                    if let Some(sym) = symbols.get(&qubit.name) {
                        if let Some(size) = sym.size {
                            for i in 0..size {
                                measured.insert(
                                    (qubit.name.clone(), Some(i)),
                                    span.clone(),
                                );
                            }
                        }
                    }
                }
            }

            Stmt::Reset { target, .. } => {
                // Reset restores a qubit to |0⟩, clearing measured state.
                measured.remove(&(target.name.clone(), target.index));
                if target.index.is_none() {
                    // Reset entire register.
                    measured.retain(|(name, _), _| name != &target.name);
                }
            }

            Stmt::GateCall { args, span, .. } => {
                for op in args {
                    check_use_after_measure(&measured, diags, op, span);
                }
            }

            Stmt::Barrier { targets, span, .. } => {
                for op in targets {
                    check_use_after_measure(&measured, diags, op, span);
                }
            }

            _ => {}
        }
    }
}

/// Check if a qubit (by name and optional index) has been measured.
fn lookup_measured(
    measured: &HashMap<(String, Option<u64>), Span>,
    name: &str,
    index: Option<u64>,
) -> Option<Span> {
    // Exact match.
    if let Some(span) = measured.get(&(name.to_string(), index)) {
        return Some(span.clone());
    }
    // If checking a specific index, also check if the whole register was measured.
    if index.is_some() {
        if let Some(span) = measured.get(&(name.to_string(), None)) {
            return Some(span.clone());
        }
    }
    None
}

fn check_use_after_measure(
    measured: &HashMap<(String, Option<u64>), Span>,
    diags: &mut Vec<Diagnostic>,
    op: &GateOperand,
    use_span: &Span,
) {
    if let Some(measure_span) = lookup_measured(measured, &op.name, op.index) {
        diags.push(Diagnostic::error_with_note(
            format!(
                "use of qubit `{}` after measurement — \
                 qubit state has collapsed and cannot be used in a gate \
                 without an explicit `reset`",
                op
            ),
            use_span.clone(),
            "qubit was measured here".to_string(),
            measure_span,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn analyze_source(source: &str) -> Vec<Diagnostic> {
        let mut parser = Parser::new(source);
        let program = parser.parse().expect("should parse");
        analyze(&program)
    }

    fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
        diags
            .iter()
            .filter(|d| matches!(d.severity, Severity::Error))
            .collect()
    }

    #[test]
    fn valid_bell_pair() {
        let diags = analyze_source(
            "OPENQASM 3.0; qubit[2] q; bit[2] c; h q[0]; cx q[0], q[1]; c = measure q;",
        );
        assert!(errors(&diags).is_empty(), "expected no errors: {:?}", diags);
    }

    #[test]
    fn undeclared_qubit() {
        let diags = analyze_source("OPENQASM 3.0; h q[0];");
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("not declared"));
    }

    #[test]
    fn duplicate_declaration() {
        let diags = analyze_source("OPENQASM 3.0; qubit q; qubit q;");
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("already declared"));
    }

    #[test]
    fn index_out_of_bounds() {
        let diags = analyze_source("OPENQASM 3.0; qubit[2] q; h q[5];");
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("out of bounds"));
    }

    #[test]
    fn index_on_scalar() {
        let diags = analyze_source("OPENQASM 3.0; qubit q; h q[0];");
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("cannot index"));
    }

    #[test]
    fn kind_mismatch() {
        let diags = analyze_source("OPENQASM 3.0; bit c; h c;");
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("expected qubit"));
    }

    #[test]
    fn use_after_measure() {
        let diags = analyze_source(
            "OPENQASM 3.0; qubit[2] q; bit[2] c; c = measure q; h q[0];",
        );
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("after measurement"));
    }

    #[test]
    fn reset_clears_measured() {
        let diags = analyze_source(
            "OPENQASM 3.0; qubit q; bit c; measure q; reset q; h q;",
        );
        assert!(errors(&diags).is_empty(), "reset should clear measured state: {:?}", diags);
    }

    #[test]
    fn use_after_measure_single_index() {
        // Measure q[0] but use q[1] — should be fine.
        let diags = analyze_source(
            "OPENQASM 3.0; qubit[2] q; bit c; measure q; reset q[0]; h q[1];",
        );
        // q[1] was measured (whole-register measure) and not reset → error.
        assert_eq!(errors(&diags).len(), 1);
    }
}