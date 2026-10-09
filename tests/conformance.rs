use qasm_rs::parser::Parser;
use qasm_rs::{codegen, compile_source, CompileError, CompileOptions};

#[test]
fn supported_conformance_programs_compile() {
    let programs = [
        "OPENQASM 3.0; qubit[2] q; bit[2] c; h q; c = measure q;",
        "OPENQASM 3.0; const int turns = 2; input float theta; output bool done; qubit q; rz(theta) q;",
        "OPENQASM 3.0; def twice(int x) -> int { return x * 2; } int y = twice(3);",
        "OPENQASM 3.0; bit[2] c; bool flag = c[0] == 1; qubit q; if (flag) { x q; }",
        "OPENQASM 3.0; gate turn(theta) q { rz(theta) q; } qubit q; turn(pi / 2) q;",
    ];
    for source in programs {
        compile_source(source, CompileOptions::default())
            .unwrap_or_else(|error| panic!("conformance input failed: {source}\n{error:?}"));
    }
}

#[test]
fn unsupported_or_invalid_programs_fail_cleanly() {
    let programs = [
        "OPENQASM 3.0; qubit[2] q; bit c; c = measure q;",
        "OPENQASM 3.0; int value = true;",
        "OPENQASM 3.0; qubit q; misspelled q;",
        "OPENQASM 3.0; qubit[2] a; qubit[3] b; cx a, b;",
    ];
    for source in programs {
        assert!(
            compile_source(source, CompileOptions::default()).is_err(),
            "invalid input compiled: {source}"
        );
    }
}

#[test]
fn compile_reports_multiple_parse_errors() {
    let error = match compile_source("OPENQASM 3.0; qubit ; bit ;", CompileOptions::default()) {
        Ok(_) => panic!("source has two parse errors"),
        Err(error) => error,
    };
    match error {
        CompileError::Parse(errors) => assert_eq!(errors.len(), 2),
        other => panic!("expected parse errors, got {other:?}"),
    }
}

#[test]
fn generated_expression_programs_round_trip() {
    let operators = ["+", "-", "*", "**"];
    for seed in 1..=128u64 {
        let lhs = seed % 17 + 1;
        let rhs = seed.wrapping_mul(7) % 13 + 1;
        let tail = seed.wrapping_mul(11) % 9 + 1;
        let first = operators[(seed as usize) % operators.len()];
        let second = operators[((seed / 3) as usize) % operators.len()];
        let source = format!("OPENQASM 3.0; int value = ({lhs} {first} {rhs}) {second} {tail};");
        let mut parser = Parser::new(&source);
        let program = parser.parse().expect("generated source should parse");
        let emitted = codegen::emit(&program);
        let mut reparsed = Parser::new(&emitted);
        reparsed
            .parse()
            .unwrap_or_else(|error| panic!("round-trip failed for {emitted}: {error}"));
    }
}
