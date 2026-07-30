use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_qasm-rs")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn run(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("CLI should run")
}

#[test]
fn compiles_fixture_to_stdout() {
    let input = fixture("bell.qasm");
    let output = run(&[input.to_str().unwrap()]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("OPENQASM 3"));
    assert!(stdout.contains("cx q[0], q[1];"));
}

#[test]
fn reports_stats_to_stderr() {
    let input = fixture("bell.qasm");
    let output = run(&[input.to_str().unwrap(), "--stats"]);
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("qubits=2"));
    assert!(stderr.contains("gates=2"));
    assert!(stderr.contains("depth=2"));
}

#[test]
fn returns_failure_for_semantic_error() {
    let input = fixture("use_after_measure.qasm");
    let output = run(&[input.to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("after measurement"));
}

#[test]
fn emit_writes_output_file() {
    let input = fixture("bell.qasm");
    let output_path = std::env::temp_dir().join(format!(
        "qasm-rs-cli-{}-{}.qasm",
        std::process::id(),
        "emit"
    ));
    let output = run(&[
        input.to_str().unwrap(),
        "--emit",
        output_path.to_str().unwrap(),
        "--no-optimize",
    ]);
    assert!(output.status.success());
    let emitted = fs::read_to_string(&output_path).expect("emitted file should exist");
    fs::remove_file(&output_path).expect("temporary output should be removable");
    assert!(emitted.contains("include \"stdgates.inc\";"));
    assert!(emitted.contains("measure"));
}

#[test]
fn resolves_local_include_files() {
    let input = fixture("custom_include.qasm");
    let output = run(&[input.to_str().unwrap()]);
    assert!(output.status.success(), "{:?}", output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("gate custom"));
    assert!(!stdout.contains("include \"custom.inc\""));
}
