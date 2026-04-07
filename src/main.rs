mod ast;
mod lexer;
mod parser;

use parser::Parser;

fn main() {
    let source = r#"
    OPENQASM 3.0;
    qubit[2] q;
    bit[2] c;
    h q[0];
    cx q[0], q[1];
    c = measure q;
    "#;

    println!("=== qasm-rs: OpenQASM 3 Compiler ===\n");
    println!("Source:");
    println!("{}", source);

    let mut parser = Parser::new(source);
    match parser.parse() {
        Ok(program) => {
            println!("Parsed successfully!");
            println!("Version: {}", program.version);
            println!("Statements:");
            for stmt in &program.statements {
                println!("  {:?}", stmt);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}