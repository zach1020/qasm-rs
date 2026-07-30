//! Inlining of user-defined OpenQASM gates.

use std::collections::HashMap;

use crate::ast::{Expr, GateModifier, GateOperand, Program, Stmt};

#[derive(Debug)]
pub struct InlineError {
    pub message: String,
}

impl std::fmt::Display for InlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inlining error: {}", self.message)
    }
}

impl std::error::Error for InlineError {}

/// Inline unmodified calls to user-defined gates and remove their definitions.
///
/// Calls with modifiers are preserved because applying a modifier to an entire
/// composite gate requires modifier-specific decomposition.
pub fn inline_gate_definitions(program: &Program) -> Result<Program, InlineError> {
    let definitions: HashMap<String, Stmt> = program
        .statements
        .iter()
        .filter_map(|statement| match statement {
            Stmt::GateDef { name, .. } => Some((name.clone(), statement.clone())),
            _ => None,
        })
        .collect();
    let mut stack = Vec::new();
    let mut statements = inline_statements(&program.statements, &definitions, &mut stack)?;
    let mut still_referenced = std::collections::HashSet::new();
    for statement in statements
        .iter()
        .filter(|statement| !matches!(statement, Stmt::GateDef { .. }))
    {
        collect_gate_calls(statement, &definitions, &mut still_referenced);
    }
    statements.retain(|statement| match statement {
        Stmt::GateDef { name, .. } => still_referenced.contains(name),
        _ => true,
    });
    Ok(Program {
        version: program.version.clone(),
        statements,
    })
}

fn collect_gate_calls(
    statement: &Stmt,
    definitions: &HashMap<String, Stmt>,
    calls: &mut std::collections::HashSet<String>,
) {
    match statement {
        Stmt::GateCall { name, .. } if definitions.contains_key(name) => {
            calls.insert(name.clone());
        }
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            for statement in then_body {
                collect_gate_calls(statement, definitions, calls);
            }
            if let Some(else_body) = else_body {
                for statement in else_body {
                    collect_gate_calls(statement, definitions, calls);
                }
            }
        }
        Stmt::For { body, .. } | Stmt::While { body, .. } => {
            for statement in body {
                collect_gate_calls(statement, definitions, calls);
            }
        }
        _ => {}
    }
}

/// Inline user-defined gates and single-expression classical functions.
pub fn inline_definitions(program: &Program) -> Result<Program, InlineError> {
    let mut program = inline_gate_definitions(program)?;
    let functions: HashMap<String, (Vec<String>, Expr)> = program
        .statements
        .iter()
        .filter_map(|statement| match statement {
            Stmt::FunctionDef {
                name, params, body, ..
            } => Some((
                name.clone(),
                (
                    params.iter().map(|(_, name)| name.clone()).collect(),
                    body.clone(),
                ),
            )),
            _ => None,
        })
        .collect();
    let mut stack = Vec::new();
    for statement in &mut program.statements {
        inline_functions_in_statement(statement, &functions, &mut stack)?;
    }
    program
        .statements
        .retain(|statement| !matches!(statement, Stmt::FunctionDef { .. }));
    Ok(program)
}

fn inline_functions_in_statement(
    statement: &mut Stmt,
    definitions: &HashMap<String, (Vec<String>, Expr)>,
    stack: &mut Vec<String>,
) -> Result<(), InlineError> {
    match statement {
        Stmt::ClassicalDecl { init, .. } => {
            if let Some(init) = init {
                inline_function_expr(init, definitions, stack)?;
            }
        }
        Stmt::Assignment { value, .. } => {
            inline_function_expr(value, definitions, stack)?;
        }
        Stmt::GateCall {
            modifiers, params, ..
        } => {
            for modifier in modifiers {
                match modifier {
                    GateModifier::Ctrl(Some(expr), _)
                    | GateModifier::NegCtrl(Some(expr), _)
                    | GateModifier::Pow(expr, _) => {
                        inline_function_expr(expr, definitions, stack)?;
                    }
                    GateModifier::Ctrl(None, _)
                    | GateModifier::NegCtrl(None, _)
                    | GateModifier::Inv(_) => {}
                }
            }
            for param in params {
                inline_function_expr(param, definitions, stack)?;
            }
        }
        Stmt::GateDef { body, .. } => {
            for statement in body {
                inline_functions_in_statement(statement, definitions, stack)?;
            }
        }
        Stmt::FunctionDef { body, .. } => {
            inline_function_expr(body, definitions, stack)?;
        }
        Stmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            inline_function_expr(condition, definitions, stack)?;
            for statement in then_body {
                inline_functions_in_statement(statement, definitions, stack)?;
            }
            if let Some(else_body) = else_body {
                for statement in else_body {
                    inline_functions_in_statement(statement, definitions, stack)?;
                }
            }
        }
        Stmt::For { range, body, .. } => {
            inline_function_expr(&mut range.start, definitions, stack)?;
            inline_function_expr(&mut range.end, definitions, stack)?;
            if let Some(step) = &mut range.step {
                inline_function_expr(step, definitions, stack)?;
            }
            for statement in body {
                inline_functions_in_statement(statement, definitions, stack)?;
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            inline_function_expr(condition, definitions, stack)?;
            for statement in body {
                inline_functions_in_statement(statement, definitions, stack)?;
            }
        }
        Stmt::Include { .. }
        | Stmt::QubitDecl { .. }
        | Stmt::BitDecl { .. }
        | Stmt::Measure { .. }
        | Stmt::Reset { .. }
        | Stmt::Barrier { .. } => {}
        Stmt::Delay { duration, .. } => {
            inline_function_expr(duration, definitions, stack)?;
        }
    }
    Ok(())
}

fn inline_function_expr(
    expr: &mut Expr,
    definitions: &HashMap<String, (Vec<String>, Expr)>,
    stack: &mut Vec<String>,
) -> Result<(), InlineError> {
    match expr {
        Expr::Neg(inner, _) => inline_function_expr(inner, definitions, stack)?,
        Expr::BinOp { lhs, rhs, .. } | Expr::Compare { lhs, rhs, .. } => {
            inline_function_expr(lhs, definitions, stack)?;
            inline_function_expr(rhs, definitions, stack)?;
        }
        Expr::Call { name, args, .. } => {
            for arg in args.iter_mut() {
                inline_function_expr(arg, definitions, stack)?;
            }
            if let Some((params, body)) = definitions.get(name) {
                if stack.contains(name) {
                    return Err(InlineError {
                        message: format!("recursive function definition involving `{}`", name),
                    });
                }
                if params.len() != args.len() {
                    return Err(InlineError {
                        message: format!("arity mismatch while inlining function `{}`", name),
                    });
                }
                let arguments: HashMap<&str, &Expr> =
                    params.iter().map(String::as_str).zip(args.iter()).collect();
                let mut replacement = substitute_expr(body, &arguments);
                stack.push(name.clone());
                inline_function_expr(&mut replacement, definitions, stack)?;
                stack.pop();
                *expr = replacement;
            }
        }
        Expr::IntLit(..)
        | Expr::FloatLit(..)
        | Expr::BoolLit(..)
        | Expr::Ident(..)
        | Expr::Index { .. }
        | Expr::Const(..) => {}
    }
    Ok(())
}

fn inline_statements(
    statements: &[Stmt],
    definitions: &HashMap<String, Stmt>,
    stack: &mut Vec<String>,
) -> Result<Vec<Stmt>, InlineError> {
    let mut output = Vec::new();
    for statement in statements {
        match statement {
            Stmt::GateCall {
                name,
                modifiers,
                params,
                args,
                ..
            } if modifiers.is_empty() && definitions.contains_key(name) => {
                if stack.contains(name) {
                    return Err(InlineError {
                        message: format!("recursive gate definition involving `{}`", name),
                    });
                }
                let Stmt::GateDef {
                    params: formal_params,
                    qparams,
                    body,
                    ..
                } = &definitions[name]
                else {
                    unreachable!()
                };
                if formal_params.len() != params.len() || qparams.len() != args.len() {
                    return Err(InlineError {
                        message: format!("arity mismatch while inlining gate `{}`", name),
                    });
                }
                let param_map: HashMap<&str, &Expr> = formal_params
                    .iter()
                    .map(String::as_str)
                    .zip(params.iter())
                    .collect();
                let qubit_map: HashMap<&str, &GateOperand> = qparams
                    .iter()
                    .map(String::as_str)
                    .zip(args.iter())
                    .collect();
                let substituted: Vec<Stmt> = body
                    .iter()
                    .map(|body_statement| {
                        substitute_statement(body_statement, &param_map, &qubit_map)
                    })
                    .collect();
                stack.push(name.clone());
                output.extend(inline_statements(&substituted, definitions, stack)?);
                stack.pop();
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                span,
            } => output.push(Stmt::If {
                condition: condition.clone(),
                then_body: inline_statements(then_body, definitions, stack)?,
                else_body: else_body
                    .as_deref()
                    .map(|body| inline_statements(body, definitions, stack))
                    .transpose()?,
                span: span.clone(),
            }),
            Stmt::For {
                var_name,
                var_ty,
                range,
                body,
                span,
            } => output.push(Stmt::For {
                var_name: var_name.clone(),
                var_ty: *var_ty,
                range: range.clone(),
                body: inline_statements(body, definitions, stack)?,
                span: span.clone(),
            }),
            Stmt::While {
                condition,
                body,
                span,
            } => output.push(Stmt::While {
                condition: condition.clone(),
                body: inline_statements(body, definitions, stack)?,
                span: span.clone(),
            }),
            _ => output.push(statement.clone()),
        }
    }
    Ok(output)
}

fn substitute_statement(
    statement: &Stmt,
    params: &HashMap<&str, &Expr>,
    qubits: &HashMap<&str, &GateOperand>,
) -> Stmt {
    match statement {
        Stmt::GateCall {
            name,
            modifiers,
            params: arguments,
            args,
            span,
        } => Stmt::GateCall {
            name: name.clone(),
            modifiers: modifiers
                .iter()
                .map(|modifier| substitute_modifier(modifier, params))
                .collect(),
            params: arguments
                .iter()
                .map(|expr| substitute_expr(expr, params))
                .collect(),
            args: args
                .iter()
                .map(|operand| {
                    qubits
                        .get(operand.name.as_str())
                        .map(|actual| (*actual).clone())
                        .unwrap_or_else(|| operand.clone())
                })
                .collect(),
            span: span.clone(),
        },
        _ => statement.clone(),
    }
}

fn substitute_modifier(modifier: &GateModifier, params: &HashMap<&str, &Expr>) -> GateModifier {
    match modifier {
        GateModifier::Ctrl(arg, span) => GateModifier::Ctrl(
            arg.as_ref().map(|expr| substitute_expr(expr, params)),
            span.clone(),
        ),
        GateModifier::NegCtrl(arg, span) => GateModifier::NegCtrl(
            arg.as_ref().map(|expr| substitute_expr(expr, params)),
            span.clone(),
        ),
        GateModifier::Inv(span) => GateModifier::Inv(span.clone()),
        GateModifier::Pow(expr, span) => {
            GateModifier::Pow(substitute_expr(expr, params), span.clone())
        }
    }
}

fn substitute_expr(expr: &Expr, params: &HashMap<&str, &Expr>) -> Expr {
    match expr {
        Expr::Ident(name, _) if params.contains_key(name.as_str()) => params[name.as_str()].clone(),
        Expr::Neg(inner, span) => Expr::Neg(Box::new(substitute_expr(inner, params)), span.clone()),
        Expr::BinOp { op, lhs, rhs, span } => Expr::BinOp {
            op: *op,
            lhs: Box::new(substitute_expr(lhs, params)),
            rhs: Box::new(substitute_expr(rhs, params)),
            span: span.clone(),
        },
        Expr::Compare { op, lhs, rhs, span } => Expr::Compare {
            op: *op,
            lhs: Box::new(substitute_expr(lhs, params)),
            rhs: Box::new(substitute_expr(rhs, params)),
            span: span.clone(),
        },
        Expr::Call { name, args, span } => Expr::Call {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_expr(arg, params))
                .collect(),
            span: span.clone(),
        },
        _ => expr.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{codegen, parser::Parser};

    #[test]
    fn inlines_parameterized_nested_gate_calls() {
        let mut parser = Parser::new(
            "OPENQASM 3.0; gate turn(theta) q { rz(theta) q; } gate twice(phi) q { turn(phi) q; turn(phi) q; } qubit q; twice(pi / 2) q;",
        );
        let program = parser.parse().unwrap();
        let inlined = inline_gate_definitions(&program).unwrap();
        let emitted = codegen::emit(&inlined);
        assert!(!emitted.contains("gate turn"));
        assert!(!emitted.contains("twice("));
        assert_eq!(emitted.matches("rz(").count(), 2);
        assert!(emitted.contains("pi / 2"));
    }

    #[test]
    fn inlines_expression_functions() {
        let mut parser = Parser::new(
            "OPENQASM 3.0; def twice(int x) -> int { return x * 2; } int y = twice(3);",
        );
        let program = parser.parse().unwrap();
        let inlined = inline_definitions(&program).unwrap();
        let emitted = codegen::emit(&inlined);
        assert!(!emitted.contains("def twice"));
        assert!(!emitted.contains("twice(3)"));
        assert!(emitted.contains("3 * 2"));
    }

    #[test]
    fn retains_definition_for_modified_composite_call() {
        let mut parser =
            Parser::new("OPENQASM 3.0; gate custom q { x q; } qubit q; inv @ custom q;");
        let program = parser.parse().unwrap();
        let inlined = inline_gate_definitions(&program).unwrap();
        let emitted = codegen::emit(&inlined);
        assert!(emitted.contains("gate custom"));
        assert!(emitted.contains("inv @ custom q;"));
    }
}
