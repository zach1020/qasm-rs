//! Semantic analysis for OpenQASM 3.
//!
//! Two-pass analysis carried by a `SemaContext`:
//!   Pass 1 – Symbol resolution: duplicate declarations, undeclared names,
//!            index bounds, gate arity, classical type checking.
//!   Pass 2 – Qubit linearity: use-after-measure detection (no-cloning
//!            enforcement) with conservative analysis through branches.
//!
//! The symbol table uses a scope stack so that `for` loop variables, gate
//! definition parameters, and block-scoped names are handled correctly.

use std::collections::HashMap;

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
    Classical(ClassicalType),
    /// A parameter name inside a gate definition (angle parameter).
    GateParam,
    /// A qubit wire name inside a gate definition.
    GateQubit,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub kind: SymbolKind,
    pub size: Option<u64>,
    pub decl_span: Span,
}

/// Gate signature for arity checking.
#[derive(Debug, Clone)]
struct GateSig {
    param_count: usize,
    qubit_count: usize,
    decl_span: Span,
}

// ── Scoped symbol table ─────────────────────────────────────

struct SymbolTable {
    scopes: Vec<HashMap<String, Symbol>>,
}

impl SymbolTable {
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn insert(&mut self, name: String, sym: Symbol) {
        self.scopes.last_mut().unwrap().insert(name, sym);
    }

    fn get(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.get(name) {
                return Some(sym);
            }
        }
        None
    }

    /// Check only the current (innermost) scope for duplicates.
    fn get_current(&self, name: &str) -> Option<&Symbol> {
        self.scopes.last().unwrap().get(name)
    }
}

// ── Analysis context ────────────────────────────────────────

struct SemaContext {
    symbols: SymbolTable,
    gates: HashMap<String, GateSig>,
    /// Tracks (register_name, index) → span of measurement.
    measured: HashMap<(String, Option<u64>), Span>,
    diags: Vec<Diagnostic>,
}

impl SemaContext {
    fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            gates: HashMap::new(),
            measured: HashMap::new(),
            diags: Vec::new(),
        }
    }

    fn declare(&mut self, name: &str, kind: SymbolKind, size: Option<u64>, span: &Span) {
        if let Some(prev) = self.symbols.get_current(name) {
            self.diags.push(Diagnostic::error_with_note(
                format!("`{}` is already declared in this scope", name),
                span.clone(),
                format!("`{}` first declared here", name),
                prev.decl_span.clone(),
            ));
        } else {
            self.symbols.insert(
                name.to_string(),
                Symbol {
                    kind,
                    size,
                    decl_span: span.clone(),
                },
            );
        }
    }

    fn check_operand(&mut self, op: &GateOperand, expected_kind: Option<SymbolKind>) {
        let Some(sym) = self.symbols.get(&op.name) else {
            self.diags.push(Diagnostic::error(
                format!("`{}` is not declared", op.name),
                op.span.clone(),
            ));
            return;
        };
        let sym = sym.clone(); // clone to release borrow

        // Kind mismatch.
        if let Some(expected) = expected_kind {
            let ok = match (&expected, &sym.kind) {
                (SymbolKind::Qubit, SymbolKind::Qubit) => true,
                (SymbolKind::Qubit, SymbolKind::GateQubit) => true,
                (SymbolKind::Bit, SymbolKind::Bit) => true,
                _ => false,
            };
            if !ok {
                let expected_str = match expected {
                    SymbolKind::Qubit => "qubit",
                    SymbolKind::Bit => "bit",
                    _ => "quantum operand",
                };
                let found_str = match sym.kind {
                    SymbolKind::Qubit => "qubit",
                    SymbolKind::Bit => "bit",
                    SymbolKind::Classical(t) => match t {
                        ClassicalType::Int => "int",
                        ClassicalType::Float => "float",
                        ClassicalType::Bool => "bool",
                    },
                    SymbolKind::GateParam => "gate parameter",
                    SymbolKind::GateQubit => "gate qubit",
                };
                self.diags.push(Diagnostic::error_with_note(
                    format!(
                        "expected {}, but `{}` is a {}",
                        expected_str, op.name, found_str
                    ),
                    op.span.clone(),
                    format!("`{}` declared as {} here", op.name, found_str),
                    sym.decl_span.clone(),
                ));
            }
        }

        // Gate qubits should not be indexed.
        if sym.kind == SymbolKind::GateQubit && op.index.is_some() {
            self.diags.push(Diagnostic::error(
                format!(
                    "cannot index gate qubit `{}` — gate qubits are single wires",
                    op.name
                ),
                op.span.clone(),
            ));
            return;
        }

        // Index bounds.
        if let Some(idx) = op.index {
            match sym.size {
                Some(size) if idx >= size => {
                    self.diags.push(Diagnostic::error_with_note(
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
                    self.diags.push(Diagnostic::error_with_note(
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

    fn check_use_after_measure(&mut self, op: &GateOperand, use_span: &Span) {
        if let Some(measure_span) = self.lookup_measured(&op.name, op.index) {
            self.diags.push(Diagnostic::error_with_note(
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

    fn lookup_measured(&self, name: &str, index: Option<u64>) -> Option<Span> {
        if let Some(span) = self.measured.get(&(name.to_string(), index)) {
            return Some(span.clone());
        }
        if index.is_some() {
            if let Some(span) = self.measured.get(&(name.to_string(), None)) {
                return Some(span.clone());
            }
        }
        None
    }

    fn mark_measured(&mut self, qubit: &GateOperand, span: &Span) {
        self.measured
            .insert((qubit.name.clone(), qubit.index), span.clone());
        // If measured without index, mark all individual indices too.
        if qubit.index.is_none() {
            if let Some(sym) = self.symbols.get(&qubit.name) {
                if let Some(size) = sym.size {
                    for i in 0..size {
                        self.measured
                            .insert((qubit.name.clone(), Some(i)), span.clone());
                    }
                }
            }
        }
    }

    fn clear_measured(&mut self, target: &GateOperand) {
        self.measured
            .remove(&(target.name.clone(), target.index));
        if target.index.is_none() {
            self.measured
                .retain(|(name, _), _| name != &target.name);
        }
    }

    // ── Statement analysis ──────────────────────────────────

    fn analyze_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.analyze_stmt(stmt);
        }
    }

    fn analyze_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::QubitDecl { name, size, span } => {
                self.declare(name, SymbolKind::Qubit, *size, span);
            }

            Stmt::BitDecl { name, size, span } => {
                self.declare(name, SymbolKind::Bit, *size, span);
            }

            Stmt::ClassicalDecl {
                ty, name, init, span,
            } => {
                self.declare(name, SymbolKind::Classical(*ty), None, span);
                if let Some(expr) = init {
                    self.check_expr(expr);
                }
            }

            Stmt::Assignment { name, value, span, .. } => {
                if self.symbols.get(name).is_none() {
                    self.diags.push(Diagnostic::error(
                        format!("`{}` is not declared", name),
                        span.clone(),
                    ));
                }
                self.check_expr(value);
            }

            Stmt::GateCall {
                name,
                modifiers: _,
                params,
                args,
                span,
            } => {
                // Gate arity check.
                if let Some(sig) = self.gates.get(name).cloned() {
                    if params.len() != sig.param_count {
                        self.diags.push(Diagnostic::error_with_note(
                            format!(
                                "gate `{}` expects {} parameter(s), got {}",
                                name, sig.param_count, params.len()
                            ),
                            span.clone(),
                            format!("`{}` defined here", name),
                            sig.decl_span.clone(),
                        ));
                    }
                    if args.len() != sig.qubit_count {
                        self.diags.push(Diagnostic::error_with_note(
                            format!(
                                "gate `{}` expects {} qubit(s), got {}",
                                name, sig.qubit_count, args.len()
                            ),
                            span.clone(),
                            format!("`{}` defined here", name),
                            sig.decl_span.clone(),
                        ));
                    }
                }
                // Don't require built-in gates to be defined.

                for p in params {
                    self.check_expr(p);
                }
                for op in args {
                    self.check_operand(op, Some(SymbolKind::Qubit));
                    self.check_use_after_measure(op, span);
                }
            }

            Stmt::GateDef {
                name,
                params,
                qparams,
                body,
                span,
            } => {
                // Check for duplicate gate name.
                if self.gates.contains_key(name) {
                    self.diags.push(Diagnostic::error(
                        format!("gate `{}` is already defined", name),
                        span.clone(),
                    ));
                }
                self.gates.insert(
                    name.clone(),
                    GateSig {
                        param_count: params.len(),
                        qubit_count: qparams.len(),
                        decl_span: span.clone(),
                    },
                );

                // Analyze body in a new scope.
                self.symbols.push_scope();
                for p in params {
                    self.symbols.insert(
                        p.clone(),
                        Symbol {
                            kind: SymbolKind::GateParam,
                            size: None,
                            decl_span: span.clone(),
                        },
                    );
                }
                for q in qparams {
                    self.symbols.insert(
                        q.clone(),
                        Symbol {
                            kind: SymbolKind::GateQubit,
                            size: None,
                            decl_span: span.clone(),
                        },
                    );
                }
                self.analyze_stmts(body);
                self.symbols.pop_scope();
            }

            Stmt::Measure { qubit, target, span } => {
                self.check_operand(qubit, Some(SymbolKind::Qubit));
                if let Some(t) = target {
                    self.check_operand(t, Some(SymbolKind::Bit));
                }
                // Warn on double-measure.
                if self.lookup_measured(&qubit.name, qubit.index).is_some() {
                    self.diags.push(Diagnostic::warning(
                        format!("qubit `{}` has already been measured", qubit),
                        span.clone(),
                    ));
                }
                self.mark_measured(qubit, span);
            }

            Stmt::Reset { target, .. } => {
                self.check_operand(target, Some(SymbolKind::Qubit));
                self.clear_measured(target);
            }

            Stmt::Barrier { targets, span, .. } => {
                for op in targets {
                    self.check_operand(op, Some(SymbolKind::Qubit));
                    self.check_use_after_measure(op, span);
                }
            }

            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.check_expr(condition);

                // Conservative linearity: save measured state, analyze both
                // branches, then merge (union) — if either branch measures a
                // qubit, it's considered measured after the if.
                let measured_before = self.measured.clone();

                self.symbols.push_scope();
                self.analyze_stmts(then_body);
                self.symbols.pop_scope();
                let measured_after_then = self.measured.clone();

                if let Some(else_stmts) = else_body {
                    self.measured = measured_before.clone();
                    self.symbols.push_scope();
                    self.analyze_stmts(else_stmts);
                    self.symbols.pop_scope();
                    let measured_after_else = self.measured.clone();

                    // Union: measured if measured in either branch.
                    let mut merged = measured_after_then;
                    for (key, span) in measured_after_else {
                        merged.entry(key).or_insert(span);
                    }
                    self.measured = merged;
                } else {
                    // No else: union of before and then-branch.
                    let mut merged = measured_before;
                    for (key, span) in measured_after_then {
                        merged.entry(key).or_insert(span);
                    }
                    self.measured = merged;
                }
            }

            Stmt::For {
                var_name,
                var_ty,
                range,
                body,
                span,
            } => {
                self.check_expr(&range.start);
                self.check_expr(&range.end);
                if let Some(ref step) = range.step {
                    self.check_expr(step);
                }

                // Loop body in new scope with loop variable.
                let measured_before = self.measured.clone();
                self.symbols.push_scope();
                self.symbols.insert(
                    var_name.clone(),
                    Symbol {
                        kind: SymbolKind::Classical(*var_ty),
                        size: None,
                        decl_span: span.clone(),
                    },
                );
                self.analyze_stmts(body);
                self.symbols.pop_scope();

                // Conservative: anything measured in loop body stays measured.
                let measured_after_body = self.measured.clone();
                let mut merged = measured_before;
                for (key, span) in measured_after_body {
                    merged.entry(key).or_insert(span);
                }
                self.measured = merged;
            }

            Stmt::While {
                condition, body, ..
            } => {
                self.check_expr(condition);

                let measured_before = self.measured.clone();
                self.symbols.push_scope();
                self.analyze_stmts(body);
                self.symbols.pop_scope();

                // Conservative: anything measured in loop body stays measured.
                let measured_after_body = self.measured.clone();
                let mut merged = measured_before;
                for (key, span) in measured_after_body {
                    merged.entry(key).or_insert(span);
                }
                self.measured = merged;
            }
        }
    }

    /// Validate expressions (check that identifiers are declared).
    fn check_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(name, span) => {
                if self.symbols.get(name).is_none() {
                    self.diags.push(Diagnostic::error(
                        format!("`{}` is not declared", name),
                        span.clone(),
                    ));
                }
            }
            Expr::Neg(inner, _) => self.check_expr(inner),
            Expr::BinOp { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            Expr::Compare { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            _ => {} // literals and constants are always valid
        }
    }
}

// ── Public entry point ──────────────────────────────────────

pub fn analyze(program: &Program) -> Vec<Diagnostic> {
    let mut ctx = SemaContext::new();
    ctx.analyze_stmts(&program.statements);
    ctx.diags
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
        assert!(
            errors(&diags).is_empty(),
            "reset should clear measured state: {:?}",
            diags
        );
    }

    #[test]
    fn use_after_measure_partial_reset() {
        // Measure whole register, reset only q[0], use q[1] → error.
        let diags = analyze_source(
            "OPENQASM 3.0; qubit[2] q; bit c; measure q; reset q[0]; h q[1];",
        );
        assert_eq!(errors(&diags).len(), 1);
    }

    #[test]
    fn gate_def_arity_check() {
        let diags = analyze_source(
            "OPENQASM 3.0; gate rx(theta) q { U(theta, 0, 0) q; }\n\
             qubit q; rx(1, 2) q;",
        );
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("parameter")),
            "expected arity error: {:?}",
            errs
        );
    }

    #[test]
    fn gate_def_scope() {
        // Gate parameter `theta` should not be visible outside the gate.
        let diags = analyze_source(
            "OPENQASM 3.0; gate rx(theta) q { U(theta, 0, 0) q; }\n\
             qubit q; rx(theta) q;",
        );
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("not declared") && d.message.contains("theta")),
            "theta should not be in scope: {:?}",
            errs
        );
    }

    #[test]
    fn duplicate_gate_def() {
        let diags = analyze_source(
            "OPENQASM 3.0; gate h q { } gate h q { }",
        );
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("already defined")),
            "expected duplicate gate error: {:?}",
            errs
        );
    }

    #[test]
    fn classical_decl_and_assignment() {
        let diags = analyze_source(
            "OPENQASM 3.0; int x = 42; x = 10; x += 1;",
        );
        assert!(errors(&diags).is_empty(), "expected no errors: {:?}", diags);
    }

    #[test]
    fn undeclared_assignment() {
        let diags = analyze_source("OPENQASM 3.0; y = 5;");
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("not declared"));
    }

    #[test]
    fn for_loop_scoping() {
        // Loop variable `i` should not be visible after the loop.
        let diags = analyze_source(
            "OPENQASM 3.0; qubit[4] q; for int i in [0:4] { h q; } i = 5;",
        );
        // `i` assignment after loop should fail — not in scope, and not declared
        // as classical. We expect an error about `i`.
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("not declared") || d.message.contains("`i`")),
            "expected scoping error for `i`: {:?}",
            errs
        );
    }

    #[test]
    fn conservative_if_linearity() {
        // If one branch measures, the qubit is conservatively measured after.
        let diags = analyze_source(
            "OPENQASM 3.0; qubit q; bit c; int x = 0;\n\
             if (x == 0) { c = measure q; }\n\
             h q;",
        );
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("after measurement")),
            "expected conservative linearity error: {:?}",
            errs
        );
    }

    #[test]
    fn while_loop_linearity() {
        // Measurement inside loop body → conservatively measured after.
        let diags = analyze_source(
            "OPENQASM 3.0; qubit q; bit c; int x = 0;\n\
             while (x < 1) { c = measure q; x += 1; }\n\
             h q;",
        );
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("after measurement")),
            "expected linearity error after while: {:?}",
            errs
        );
    }

    #[test]
    fn valid_if_no_measure() {
        // If branches don't measure, qubit should remain usable.
        let diags = analyze_source(
            "OPENQASM 3.0; qubit q; int x = 0;\n\
             if (x == 0) { h q; } else { x q; }\n\
             h q;",
        );
        assert!(errors(&diags).is_empty(), "expected no errors: {:?}", diags);
    }

    #[test]
    fn expr_undeclared_ident() {
        let diags = analyze_source(
            "OPENQASM 3.0; int x = y + 1;",
        );
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("`y`")),
            "expected undeclared `y`: {:?}",
            errs
        );
    }
}
