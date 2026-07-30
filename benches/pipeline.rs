use std::hint::black_box;
use std::time::Instant;

use qasm_rs::{compile_source, CompileOptions};

fn main() {
    let source = r#"
        OPENQASM 3.0;
        qubit[4] q;
        bit[4] c;
        h q;
        cx q[0], q[1];
        cx q[2], q[3];
        for int i in [0:9] {
            rz(pi / 2) q[0];
            rz(-pi / 2) q[0];
        }
        c = measure q;
    "#;
    let iterations = 1_000;
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(
            compile_source(black_box(source), CompileOptions::default())
                .expect("benchmark input should compile"),
        );
    }
    let elapsed = started.elapsed();
    println!(
        "pipeline: {} iterations in {:?} ({:?}/iteration)",
        iterations,
        elapsed,
        elapsed / iterations
    );
}
