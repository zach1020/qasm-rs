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
    Const(ClassicalType),
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

#[derive(Debug, Clone)]
struct FunctionSig {
    params: Vec<ClassicalType>,
    return_type: ClassicalType,
    decl_span: Span,
}

fn standard_gate_signature(name: &str) -> Option<(usize, usize)> {
    match name.to_ascii_lowercase().as_str() {
        "u" => Some((3, 1)),
        "gphase" => Some((1, 0)),
        "id" | "x" | "y" | "z" | "h" | "s" | "sdg" | "t" | "tdg" | "sx" => Some((0, 1)),
        "p" | "rx" | "ry" | "rz" => Some((1, 1)),
        "cx" | "cnot" | "cy" | "cz" | "ch" | "swap" => Some((0, 2)),
        "cp" | "crx" | "cry" | "crz" => Some((1, 2)),
        "ccx" | "cswap" => Some((0, 3)),
        _ => None,
    }
}

fn modifier_control_count(modifiers: &[GateModifier]) -> Option<usize> {
    let mut total = 0usize;
    for modifier in modifiers {
        match modifier {
            GateModifier::Ctrl(arg, _) | GateModifier::NegCtrl(arg, _) => {
                let count = match arg {
                    Some(expr) => eval_const_int(expr)?,
                    None => 1,
                };
                if count <= 0 {
                    return None;
                }
                total = total.checked_add(count.try_into().ok()?)?;
            }
            GateModifier::Inv(_) | GateModifier::Pow(_, _) => {}
        }
    }
    Some(total)
}

fn eval_const_int(expr: &Expr) -> Option<i128> {
    match expr {
        Expr::IntLit(value, _) => Some((*value).into()),
        Expr::Neg(inner, _) => eval_const_int(inner)?.checked_neg(),
        Expr::BinOp { op, lhs, rhs, .. } => {
            let lhs = eval_const_int(lhs)?;
            let rhs = eval_const_int(rhs)?;
            match op {
                BinOp::Add => lhs.checked_add(rhs),
                BinOp::Sub => lhs.checked_sub(rhs),
                BinOp::Mul => lhs.checked_mul(rhs),
                BinOp::Div => lhs.checked_div(rhs),
                BinOp::Pow => {
                    let exponent: u32 = rhs.try_into().ok()?;
                    lhs.checked_pow(exponent)
                }
            }
        }
        _ => None,
    }
}

fn type_name(ty: ClassicalType) -> &'static str {
    match ty {
        ClassicalType::Int => "int",
        ClassicalType::UInt => "uint",
        ClassicalType::Float => "float",
        ClassicalType::Angle => "angle",
        ClassicalType::Bool => "bool",
    }
}

fn is_numeric(ty: ClassicalType) -> bool {
    matches!(
        ty,
        ClassicalType::Int | ClassicalType::UInt | ClassicalType::Float | ClassicalType::Angle
    )
}

fn assignable(expected: ClassicalType, actual: ClassicalType) -> bool {
    expected == actual
        || (expected == ClassicalType::UInt && actual == ClassicalType::Int)
        || (expected == ClassicalType::Angle && actual == ClassicalType::Float)
        || (matches!(expected, ClassicalType::Float | ClassicalType::Angle)
            && matches!(actual, ClassicalType::Int | ClassicalType::UInt))
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
    functions: HashMap<String, FunctionSig>,
    /// Tracks (register_name, index) → span of measurement.
    measured: HashMap<(String, Option<u64>), Span>,
    diags: Vec<Diagnostic>,
}

impl SemaContext {
    fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            gates: HashMap::new(),
            functions: HashMap::new(),
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
            let ok = matches!(
                (&expected, &sym.kind),
                (SymbolKind::Qubit, SymbolKind::Qubit)
                    | (SymbolKind::Qubit, SymbolKind::GateQubit)
                    | (SymbolKind::Bit, SymbolKind::Bit)
            );
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
                        ClassicalType::UInt => "uint",
                        ClassicalType::Float => "float",
                        ClassicalType::Angle => "angle",
                        ClassicalType::Bool => "bool",
                    },
                    SymbolKind::Const(t) => match t {
                        ClassicalType::Int => "const int",
                        ClassicalType::UInt => "const uint",
                        ClassicalType::Float => "const float",
                        ClassicalType::Angle => "const angle",
                        ClassicalType::Bool => "const bool",
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
        if sym.kind == SymbolKind::GateQubit && (op.index.is_some() || op.slice.is_some()) {
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
        if let Some((start, end)) = op.slice {
            match sym.size {
                Some(size) if start <= end && end < size => {}
                Some(size) => self.diags.push(Diagnostic::error_with_note(
                    format!(
                        "slice {}:{} is out of bounds for `{}` (size {})",
                        start, end, op.name, size
                    ),
                    op.span.clone(),
                    format!("`{}` declared with size {} here", op.name, size),
                    sym.decl_span.clone(),
                )),
                None => self.diags.push(Diagnostic::error(
                    format!("cannot slice scalar `{}`", op.name),
                    op.span.clone(),
                )),
            }
        }
    }

    fn operand_width(&self, op: &GateOperand) -> Option<u64> {
        let symbol = self.symbols.get(&op.name)?;
        if let Some((start, end)) = op.slice {
            end.checked_sub(start)?.checked_add(1)
        } else if op.index.is_some() {
            Some(1)
        } else {
            Some(symbol.size.unwrap_or(1))
        }
    }

    fn check_use_after_measure(&mut self, op: &GateOperand, use_span: &Span) {
        if let Some((start, end)) = op.slice {
            for index in start..=end {
                if let Some(measure_span) = self.lookup_measured(&op.name, Some(index)) {
                    self.diags.push(Diagnostic::error_with_note(
                        format!("use of qubit `{}[{}]` after measurement", op.name, index),
                        use_span.clone(),
                        "qubit was measured here",
                        measure_span,
                    ));
                    return;
                }
            }
            return;
        }
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
        if let Some((start, end)) = qubit.slice {
            for index in start..=end {
                self.measured
                    .insert((qubit.name.clone(), Some(index)), span.clone());
            }
            return;
        }
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
        if let Some((start, end)) = target.slice {
            for index in start..=end {
                self.measured.remove(&(target.name.clone(), Some(index)));
            }
            return;
        }
        self.measured.remove(&(target.name.clone(), target.index));
        if target.index.is_none() {
            self.measured.retain(|(name, _), _| name != &target.name);
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
            Stmt::Include { .. } => {}

            Stmt::QubitDecl { name, size, span } => {
                self.declare(name, SymbolKind::Qubit, *size, span);
            }

            Stmt::BitDecl { name, size, span } => {
                self.declare(name, SymbolKind::Bit, *size, span);
            }

            Stmt::ClassicalDecl {
                qualifier,
                ty,
                name,
                init,
                span,
            } => {
                let kind = if matches!(qualifier, Some(ClassicalQualifier::Const)) {
                    SymbolKind::Const(*ty)
                } else {
                    SymbolKind::Classical(*ty)
                };
                self.declare(name, kind, None, span);
                if matches!(qualifier, Some(ClassicalQualifier::Const)) && init.is_none() {
                    self.diags.push(Diagnostic::error(
                        format!("const `{}` requires an initializer", name),
                        span.clone(),
                    ));
                }
                if matches!(qualifier, Some(ClassicalQualifier::Input)) && init.is_some() {
                    self.diags.push(Diagnostic::error(
                        format!("input `{}` cannot have an initializer", name),
                        span.clone(),
                    ));
                }
                if let Some(expr) = init {
                    if let Some(actual) = self.check_expr(expr) {
                        if !assignable(*ty, actual) {
                            self.diags.push(Diagnostic::error(
                                format!(
                                    "cannot initialize {} `{}` with {} expression",
                                    type_name(*ty),
                                    name,
                                    type_name(actual)
                                ),
                                expr.span().clone(),
                            ));
                        }
                    }
                }
            }

            Stmt::Assignment {
                name,
                op,
                value,
                span,
            } => {
                let target = self.symbols.get(name).cloned();
                let actual = self.check_expr(value);
                match target {
                    None => self.diags.push(Diagnostic::error(
                        format!("`{}` is not declared", name),
                        span.clone(),
                    )),
                    Some(Symbol {
                        kind: SymbolKind::Classical(expected),
                        ..
                    }) => {
                        if let Some(actual) = actual {
                            if !assignable(expected, actual) {
                                self.diags.push(Diagnostic::error(
                                    format!(
                                        "cannot assign {} expression to {} `{}`",
                                        type_name(actual),
                                        type_name(expected),
                                        name
                                    ),
                                    value.span().clone(),
                                ));
                            }
                            if !matches!(op, AssignOp::Assign)
                                && (!is_numeric(expected) || !is_numeric(actual))
                            {
                                self.diags.push(Diagnostic::error(
                                    "compound assignment requires numeric operands",
                                    span.clone(),
                                ));
                            }
                        }
                    }
                    Some(Symbol {
                        kind: SymbolKind::Const(_),
                        ..
                    }) => self.diags.push(Diagnostic::error(
                        format!("cannot assign to const `{}`", name),
                        span.clone(),
                    )),
                    Some(_) => self.diags.push(Diagnostic::error(
                        format!("`{}` is not an assignable classical variable", name),
                        span.clone(),
                    )),
                }
            }

            Stmt::GateCall {
                name,
                modifiers,
                params,
                args,
                span,
            } => {
                // Gate arity check.
                let signature = self
                    .gates
                    .get(name)
                    .map(|sig| {
                        (
                            sig.param_count,
                            sig.qubit_count,
                            Some(sig.decl_span.clone()),
                        )
                    })
                    .or_else(|| {
                        standard_gate_signature(name).map(|(params, qubits)| (params, qubits, None))
                    });
                if let Some((param_count, qubit_count, decl_span)) = signature {
                    if params.len() != param_count {
                        let message = format!(
                            "gate `{}` expects {} parameter(s), got {}",
                            name,
                            param_count,
                            params.len()
                        );
                        if let Some(decl_span) = decl_span.clone() {
                            self.diags.push(Diagnostic::error_with_note(
                                message,
                                span.clone(),
                                format!("`{}` defined here", name),
                                decl_span,
                            ));
                        } else {
                            self.diags.push(Diagnostic::error(message, span.clone()));
                        }
                    }
                    let expected_qubits =
                        qubit_count + modifier_control_count(modifiers).unwrap_or_default();
                    if args.len() != expected_qubits {
                        let message = format!(
                            "gate `{}` expects {} qubit(s) after modifiers, got {}",
                            name,
                            expected_qubits,
                            args.len()
                        );
                        if let Some(decl_span) = decl_span {
                            self.diags.push(Diagnostic::error_with_note(
                                message,
                                span.clone(),
                                format!("`{}` defined here", name),
                                decl_span,
                            ));
                        } else {
                            self.diags.push(Diagnostic::error(message, span.clone()));
                        }
                    }
                } else {
                    self.diags.push(Diagnostic::error(
                        format!("gate `{}` is not defined", name),
                        span.clone(),
                    ));
                }

                self.check_modifiers(modifiers);
                for p in params {
                    if let Some(ty) = self.check_expr(p) {
                        if !is_numeric(ty) {
                            self.diags.push(Diagnostic::error(
                                "gate parameters must be numeric",
                                p.span().clone(),
                            ));
                        }
                    }
                }
                for op in args {
                    self.check_operand(op, Some(SymbolKind::Qubit));
                    self.check_use_after_measure(op, span);
                }
                let register_widths: Vec<u64> = args
                    .iter()
                    .filter_map(|operand| self.operand_width(operand))
                    .filter(|width| *width > 1)
                    .collect();
                if let Some(first) = register_widths.first() {
                    if register_widths.iter().any(|width| width != first) {
                        self.diags.push(Diagnostic::error(
                            "gate register operands must have the same length for broadcasting",
                            span.clone(),
                        ));
                    }
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

            Stmt::FunctionDef {
                name,
                params,
                return_type,
                body,
                span,
            } => {
                if let Some(previous) = self.functions.get(name) {
                    self.diags.push(Diagnostic::error_with_note(
                        format!("function `{}` is already defined", name),
                        span.clone(),
                        "first defined here",
                        previous.decl_span.clone(),
                    ));
                } else {
                    self.functions.insert(
                        name.clone(),
                        FunctionSig {
                            params: params.iter().map(|(ty, _)| *ty).collect(),
                            return_type: *return_type,
                            decl_span: span.clone(),
                        },
                    );
                }
                self.symbols.push_scope();
                for (ty, param_name) in params {
                    self.symbols.insert(
                        param_name.clone(),
                        Symbol {
                            kind: SymbolKind::Classical(*ty),
                            size: None,
                            decl_span: span.clone(),
                        },
                    );
                }
                if let Some(actual) = self.check_expr(body) {
                    if !assignable(*return_type, actual) {
                        self.diags.push(Diagnostic::error(
                            format!(
                                "function `{}` returns {}, but its body has type {}",
                                name,
                                type_name(*return_type),
                                type_name(actual)
                            ),
                            body.span().clone(),
                        ));
                    }
                }
                self.symbols.pop_scope();
            }

            Stmt::Measure {
                qubit,
                target,
                span,
            } => {
                self.check_operand(qubit, Some(SymbolKind::Qubit));
                if let Some(t) = target {
                    self.check_operand(t, Some(SymbolKind::Bit));
                    if let (Some(source_width), Some(target_width)) =
                        (self.operand_width(qubit), self.operand_width(t))
                    {
                        if source_width != target_width {
                            self.diags.push(Diagnostic::error_with_note(
                                format!(
                                    "measurement source has {} qubit(s), but target has {} bit(s)",
                                    source_width, target_width
                                ),
                                span.clone(),
                                "measurement target declared here",
                                t.span.clone(),
                            ));
                        }
                    }
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

            Stmt::Delay {
                duration,
                targets,
                span,
            } => {
                if let Some(ty) = self.check_expr(duration) {
                    if !is_numeric(ty) {
                        self.diags.push(Diagnostic::error(
                            "delay duration must be numeric",
                            duration.span().clone(),
                        ));
                    }
                }
                for operand in targets {
                    self.check_operand(operand, Some(SymbolKind::Qubit));
                    self.check_use_after_measure(operand, span);
                }
            }

            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.require_type(condition, ClassicalType::Bool, "if condition");

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
                if *var_ty != ClassicalType::Int {
                    self.diags.push(Diagnostic::error(
                        "for-loop variable must have type int",
                        span.clone(),
                    ));
                }
                self.require_type(&range.start, ClassicalType::Int, "for-loop range bound");
                self.require_type(&range.end, ClassicalType::Int, "for-loop range bound");
                if let Some(ref step) = range.step {
                    self.require_type(step, ClassicalType::Int, "for-loop range step");
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
                self.require_type(condition, ClassicalType::Bool, "while condition");

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

    fn check_modifiers(&mut self, modifiers: &[GateModifier]) {
        for modifier in modifiers {
            match modifier {
                GateModifier::Ctrl(arg, span) | GateModifier::NegCtrl(arg, span) => {
                    if let Some(arg) = arg {
                        self.require_type(arg, ClassicalType::Int, "control modifier argument");
                        if !matches!(eval_const_int(arg), Some(value) if value > 0) {
                            self.diags.push(Diagnostic::error(
                                "control count must be a positive compile-time integer",
                                span.clone(),
                            ));
                        }
                    }
                }
                GateModifier::Pow(expr, _) => {
                    if let Some(ty) = self.check_expr(expr) {
                        if !is_numeric(ty) {
                            self.diags.push(Diagnostic::error(
                                "power modifier argument must be numeric",
                                expr.span().clone(),
                            ));
                        }
                    }
                }
                GateModifier::Inv(_) => {}
            }
        }
    }

    fn require_type(&mut self, expr: &Expr, expected: ClassicalType, context: &str) {
        if let Some(actual) = self.check_expr(expr) {
            if !assignable(expected, actual) {
                self.diags.push(Diagnostic::error(
                    format!(
                        "{} must have type {}, found {}",
                        context,
                        type_name(expected),
                        type_name(actual)
                    ),
                    expr.span().clone(),
                ));
            }
        }
    }

    /// Validate an expression and return its inferred classical type.
    fn check_expr(&mut self, expr: &Expr) -> Option<ClassicalType> {
        match expr {
            Expr::Ident(name, span) => match self.symbols.get(name).map(|symbol| symbol.kind) {
                Some(SymbolKind::Classical(ty)) => Some(ty),
                Some(SymbolKind::Const(ty)) => Some(ty),
                Some(SymbolKind::Bit) => Some(ClassicalType::Int),
                Some(SymbolKind::GateParam) => Some(ClassicalType::Float),
                Some(_) => {
                    self.diags.push(Diagnostic::error(
                        format!("`{}` cannot be used as a classical expression", name),
                        span.clone(),
                    ));
                    None
                }
                None => {
                    self.diags.push(Diagnostic::error(
                        format!("`{}` is not declared", name),
                        span.clone(),
                    ));
                    None
                }
            },
            Expr::Index { name, index, span } => {
                let Some(symbol) = self.symbols.get(name).cloned() else {
                    self.diags.push(Diagnostic::error(
                        format!("`{}` is not declared", name),
                        span.clone(),
                    ));
                    return None;
                };
                match symbol.kind {
                    SymbolKind::Bit => match symbol.size {
                        Some(size) if *index < size => Some(ClassicalType::Int),
                        Some(size) => {
                            self.diags.push(Diagnostic::error(
                                format!(
                                    "index {} is out of bounds for `{}` (size {})",
                                    index, name, size
                                ),
                                span.clone(),
                            ));
                            None
                        }
                        None => {
                            self.diags.push(Diagnostic::error(
                                format!("cannot index scalar bit `{}`", name),
                                span.clone(),
                            ));
                            None
                        }
                    },
                    _ => {
                        self.diags.push(Diagnostic::error(
                            format!("`{}` is not an indexable classical register", name),
                            span.clone(),
                        ));
                        None
                    }
                }
            }
            Expr::Call { name, args, span } => {
                let Some(signature) = self.functions.get(name).cloned() else {
                    self.diags.push(Diagnostic::error(
                        format!("function `{}` is not defined", name),
                        span.clone(),
                    ));
                    for arg in args {
                        self.check_expr(arg);
                    }
                    return None;
                };
                if args.len() != signature.params.len() {
                    self.diags.push(Diagnostic::error_with_note(
                        format!(
                            "function `{}` expects {} argument(s), got {}",
                            name,
                            signature.params.len(),
                            args.len()
                        ),
                        span.clone(),
                        "function defined here",
                        signature.decl_span.clone(),
                    ));
                }
                for (index, arg) in args.iter().enumerate() {
                    let actual = self.check_expr(arg);
                    if let (Some(expected), Some(actual)) = (signature.params.get(index), actual) {
                        if !assignable(*expected, actual) {
                            self.diags.push(Diagnostic::error(
                                format!(
                                    "argument {} to `{}` must be {}, found {}",
                                    index + 1,
                                    name,
                                    type_name(*expected),
                                    type_name(actual)
                                ),
                                arg.span().clone(),
                            ));
                        }
                    }
                }
                Some(signature.return_type)
            }
            Expr::IntLit(..) => Some(ClassicalType::Int),
            Expr::FloatLit(..) | Expr::Const(..) => Some(ClassicalType::Float),
            Expr::BoolLit(..) => Some(ClassicalType::Bool),
            Expr::Neg(inner, span) => match self.check_expr(inner) {
                Some(ty) if is_numeric(ty) => Some(ty),
                Some(_) => {
                    self.diags.push(Diagnostic::error(
                        "unary negation requires a numeric operand",
                        span.clone(),
                    ));
                    None
                }
                None => None,
            },
            Expr::BinOp { lhs, rhs, span, .. } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                match (lhs_ty, rhs_ty) {
                    (Some(lhs_ty), Some(rhs_ty)) if is_numeric(lhs_ty) && is_numeric(rhs_ty) => {
                        if lhs_ty == ClassicalType::Angle || rhs_ty == ClassicalType::Angle {
                            Some(ClassicalType::Angle)
                        } else if lhs_ty == ClassicalType::Float || rhs_ty == ClassicalType::Float {
                            Some(ClassicalType::Float)
                        } else if lhs_ty == ClassicalType::Int || rhs_ty == ClassicalType::Int {
                            Some(ClassicalType::Int)
                        } else {
                            Some(ClassicalType::UInt)
                        }
                    }
                    (Some(_), Some(_)) => {
                        self.diags.push(Diagnostic::error(
                            "arithmetic operators require numeric operands",
                            span.clone(),
                        ));
                        None
                    }
                    _ => None,
                }
            }
            Expr::Compare { lhs, rhs, span, .. } => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                match (lhs_ty, rhs_ty) {
                    (Some(lhs_ty), Some(rhs_ty))
                        if lhs_ty == rhs_ty || (is_numeric(lhs_ty) && is_numeric(rhs_ty)) =>
                    {
                        Some(ClassicalType::Bool)
                    }
                    (Some(_), Some(_)) => {
                        self.diags.push(Diagnostic::error(
                            "comparison operands have incompatible types",
                            span.clone(),
                        ));
                        None
                    }
                    _ => None,
                }
            }
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
        let diags = analyze_source("OPENQASM 3.0; qubit[2] q; bit[2] c; c = measure q; h q[0];");
        assert_eq!(errors(&diags).len(), 1);
        assert!(errors(&diags)[0].message.contains("after measurement"));
    }

    #[test]
    fn reset_clears_measured() {
        let diags = analyze_source("OPENQASM 3.0; qubit q; bit c; measure q; reset q; h q;");
        assert!(
            errors(&diags).is_empty(),
            "reset should clear measured state: {:?}",
            diags
        );
    }

    #[test]
    fn use_after_measure_partial_reset() {
        // Measure whole register, reset only q[0], use q[1] → error.
        let diags =
            analyze_source("OPENQASM 3.0; qubit[2] q; bit c; measure q; reset q[0]; h q[1];");
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
            errs.iter()
                .any(|d| d.message.contains("not declared") && d.message.contains("theta")),
            "theta should not be in scope: {:?}",
            errs
        );
    }

    #[test]
    fn duplicate_gate_def() {
        let diags = analyze_source("OPENQASM 3.0; gate h q { } gate h q { }");
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("already defined")),
            "expected duplicate gate error: {:?}",
            errs
        );
    }

    #[test]
    fn classical_decl_and_assignment() {
        let diags = analyze_source("OPENQASM 3.0; int x = 42; x = 10; x += 1;");
        assert!(errors(&diags).is_empty(), "expected no errors: {:?}", diags);
    }

    #[test]
    fn rejects_classical_type_mismatches() {
        let diags = analyze_source("OPENQASM 3.0; int x = true; bool flag = 3; x = false;");
        assert_eq!(errors(&diags).len(), 3);
        assert!(errors(&diags)
            .iter()
            .all(|diag| diag.message.contains("cannot")));
    }

    #[test]
    fn rejects_non_boolean_conditions() {
        let diags = analyze_source("OPENQASM 3.0; int x = 1; if (x) { x = 2; }");
        assert!(errors(&diags)
            .iter()
            .any(|diag| diag.message.contains("if condition must have type bool")));
    }

    #[test]
    fn rejects_unknown_gate() {
        let diags = analyze_source("OPENQASM 3.0; qubit q; totally_unknown q;");
        assert!(errors(&diags)
            .iter()
            .any(|diag| diag.message.contains("is not defined")));
    }

    #[test]
    fn validates_control_modifier_count() {
        let valid = analyze_source("OPENQASM 3.0; qubit[3] q; ctrl(1 + 1) @ x q[0], q[1], q[2];");
        assert!(errors(&valid).is_empty(), "{valid:?}");

        let invalid = analyze_source("OPENQASM 3.0; qubit q; ctrl(0) @ x q;");
        assert!(errors(&invalid)
            .iter()
            .any(|diag| diag.message.contains("positive compile-time integer")));
    }

    #[test]
    fn rejects_mismatched_measurement_widths_during_sema() {
        let diags = analyze_source("OPENQASM 3.0; qubit[2] q; bit c; c = measure q;");
        assert!(errors(&diags)
            .iter()
            .any(|diag| diag.message.contains("measurement source has 2")));
    }

    #[test]
    fn rejects_mismatched_broadcast_widths_during_sema() {
        let diags = analyze_source("OPENQASM 3.0; qubit[2] a; qubit[3] b; cx a, b;");
        assert!(errors(&diags)
            .iter()
            .any(|diag| diag.message.contains("same length")));
    }

    #[test]
    fn validates_qualified_declarations_and_indexed_expressions() {
        let valid = analyze_source(
            "OPENQASM 3.0; const int shots = 100; input float theta; bit[2] c; bool flag = c[1] == 1;",
        );
        assert!(errors(&valid).is_empty(), "{valid:?}");

        let invalid = analyze_source(
            "OPENQASM 3.0; const int shots; shots = 2; bit[1] c; bool b = c[2] == 1;",
        );
        assert!(errors(&invalid).len() >= 3, "{invalid:?}");
    }

    #[test]
    fn validates_expression_functions() {
        let valid = analyze_source(
            "OPENQASM 3.0; def twice(int x) -> int { return x * 2; } int y = twice(3);",
        );
        assert!(errors(&valid).is_empty(), "{valid:?}");

        let invalid =
            analyze_source("OPENQASM 3.0; def bad(bool x) -> int { return x; } int y = bad(3);");
        assert!(errors(&invalid).len() >= 2, "{invalid:?}");
    }

    #[test]
    fn validates_uint_and_angle_types() {
        let diags = analyze_source(
            "OPENQASM 3.0; uint count = 2; angle theta = pi / 2; qubit q; rz(theta) q;",
        );
        assert!(errors(&diags).is_empty(), "{diags:?}");
    }

    #[test]
    fn validates_quantum_operand_slices() {
        let valid = analyze_source("OPENQASM 3.0; qubit[4] q; h q[1:3];");
        assert!(errors(&valid).is_empty(), "{valid:?}");
        let invalid = analyze_source("OPENQASM 3.0; qubit[2] q; h q[1:3];");
        assert!(errors(&invalid)
            .iter()
            .any(|diag| diag.message.contains("slice 1:3")));
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
        let diags = analyze_source("OPENQASM 3.0; qubit[4] q; for int i in [0:4] { h q; } i = 5;");
        // `i` assignment after loop should fail — not in scope, and not declared
        // as classical. We expect an error about `i`.
        let errs = errors(&diags);
        assert!(
            errs.iter()
                .any(|d| d.message.contains("not declared") || d.message.contains("`i`")),
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
        let diags = analyze_source("OPENQASM 3.0; int x = y + 1;");
        let errs = errors(&diags);
        assert!(
            errs.iter().any(|d| d.message.contains("`y`")),
            "expected undeclared `y`: {:?}",
            errs
        );
    }
}
