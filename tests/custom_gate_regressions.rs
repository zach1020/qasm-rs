use qasm_rs::{codegen, compile_source, inline, parser::Parser, sema, CompileOptions};

#[test]
fn optimizer_preserves_custom_gates_in_every_cancellation_position() {
    let cases = [
        ("gate s q { h q; }", "z q; s q; z q;", 3),
        ("gate sx q { h q; }", "x q; sx q; x q;", 3),
        ("gate sdg q { h q; }", "s q; sdg q;", 2),
        ("gate s q { h q; }", "s q; sdg q;", 2),
        ("gate sdg q { h q; }", "s q; z q; sdg q;", 3),
    ];
    for (definition, circuit, gates) in cases {
        let source = format!("OPENQASM 3.0; {definition} qubit q; {circuit}");
        let output = compile_source(&source, CompileOptions::default()).unwrap();
        assert_eq!(output.gates_removed, 0, "{source}");
        assert_eq!(output.hir.gate_count(), gates, "{source}");
    }
}

#[test]
fn optimizer_still_cancels_standard_gates_with_unrelated_custom_definition() {
    let source = "OPENQASM 3.0; gate custom q { h q; } qubit q; z q; s q; z q;";
    let output = compile_source(source, CompileOptions::default()).unwrap();
    assert_eq!(output.gates_removed, 2);
    assert_eq!(output.hir.gate_count(), 1);
}

fn inline_and_check(source: &str) -> String {
    let program = Parser::new(source).parse().unwrap();
    assert!(sema::analyze(&program)
        .iter()
        .all(|diagnostic| !matches!(diagnostic.severity, sema::Severity::Error)));
    let inlined = inline::inline_gate_definitions(&program).unwrap();
    let output = codegen::emit(&inlined);
    compile_source(&output, CompileOptions::default()).unwrap();
    output
}

#[test]
fn inlining_retains_transitive_dependencies_of_modified_calls() {
    let definitions = "gate leaf q { h q; } gate middle q { leaf q; } gate outer q { middle q; } gate unused q { x q; }";
    for call in ["inv @ outer q;", "if (true) { inv @ outer q; }"] {
        let source = format!("OPENQASM 3.0; {definitions} qubit q; {call}");
        let output = inline_and_check(&source);
        for gate in ["leaf", "middle", "outer"] {
            assert!(output.contains(&format!("gate {gate} ")), "{output}");
        }
        assert!(!output.contains("gate unused"));
    }
}

#[test]
fn inlining_removes_unreferenced_definitions_after_unmodified_calls() {
    let output = inline_and_check(
        "OPENQASM 3.0; gate inner q { h q; } gate outer q { inner q; } qubit q; outer q;",
    );
    assert!(!output.contains("gate "));
    assert!(output.contains("h q;"));
}

#[test]
fn collecting_retained_recursive_gate_dependencies_terminates() {
    let source =
        "OPENQASM 3.0; gate recursive q { inv @ recursive q; } qubit q; inv @ recursive q;";
    let output = inline_and_check(source);
    assert!(output.contains("gate recursive"));
}
