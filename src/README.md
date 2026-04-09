# qasm-rs

An OpenQASM 3.0 compiler written in Rust with **compile-time qubit linearity enforcement**.

## Why this exists

Quantum programs have a constraint that classical programs don't: qubits cannot be copied ([no-cloning theorem](https://en.wikipedia.org/wiki/No-cloning_theorem)), and measurement irreversibly collapses quantum state. Most quantum toolchains only catch these errors at simulation time or on hardware. `qasm-rs` catches them at compile time by treating qubits as **linear resources** — tracked through the program and statically verified before any code is emitted.

This is the same idea behind Rust's ownership system applied to quantum state: use-after-measure is the quantum analog of use-after-free.

## Compiler pipeline

```
Source (.qasm)
  │
  ▼
┌─────────┐   logos-based tokenizer with byte-offset spans
│  Lexer   │   for every token; invalid bytes collected as
└────┬─────┘   lex errors with source locations
     │
     ▼
┌─────────┐   Recursive-descent parser producing a fully
│  Parser  │   span-annotated AST. Supports qubit/bit decls,
└────┬─────┘   gate calls, measure, reset, barrier.
     │
     ▼
┌─────────┐   Pass 1: Symbol table — duplicate decls,
│  Sema    │          undeclared names, index bounds,
│          │          qubit/bit kind checking.
│          │   Pass 2: Linearity — tracks measured qubits,
└────┬─────┘          rejects use-after-measure, respects reset.
     │
     ▼
┌─────────┐   Pretty-printer emitting canonical OpenQASM 3.
│ Codegen  │   Round-trip tested: parse → emit → re-parse
└──────────┘   produces structurally identical ASTs.
```

All errors are rendered with [ariadne](https://github.com/zesterer/ariadne) for precise, colorized source diagnostics with secondary labels pointing to related locations (e.g. "first declared here", "measured here").

## Example: use-after-measure detection

```qasm
OPENQASM 3.0;
qubit[2] q;
bit[2] c;
h q[0];
c = measure q;
cx q[0], q[1];   // ← ERROR: qubit state collapsed
```

```
Error: use of qubit `q[0]` after measurement
   ╭─[example.qasm:5:1]
 4 │ c = measure q;
   │ ────────────── qubit was measured here
 5 │ cx q[0], q[1];
   │ ^^^^^^^^^^^^^^ use of qubit `q[0]` after measurement
   ╰───
```

The fix is an explicit `reset`, which re-prepares |0⟩:

```qasm
c = measure q;
reset q;          // clears measured state
cx q[0], q[1];    // ← OK
```

## Other diagnostics

- **Undeclared identifiers**: `h r[0];` → *"`r` is not declared"*
- **Index out of bounds**: `qubit[2] q; h q[5];` → *"index 5 is out of bounds for `q` (size 2)"*
- **Indexing a scalar**: `qubit q; h q[0];` → *"cannot index `q` — it is a single qubit, not a register"*
- **Kind mismatch**: `bit c; h c;` → *"expected qubit, but `c` is a bit"*
- **Duplicate declarations**: `qubit q; qubit q;` → *"`q` is already declared"* with label pointing to first declaration

## Building

```bash
cargo build
cargo test
cargo run
```

Requires Rust 1.70+.

## Architecture

```
src/
├── main.rs      Entry point — runs all pipeline stages, renders diagnostics
├── span.rs      Span type (byte-offset range) and Spanned<T> wrapper
├── lexer.rs     logos-based tokenizer producing Vec<Spanned<Token>>
├── ast.rs       Span-annotated AST types
├── parser.rs    Recursive-descent parser
├── sema.rs      Semantic analysis (symbol table + linearity checking)
└── codegen.rs   Pretty-printer / QASM emitter
```

## Design decisions

**Why Rust?** The type system maps naturally onto the problem. Qubits are linear resources; Rust's ownership model is built around linear/affine types. The long-term goal is to encode qubit linearity directly into Rust's type system so the no-cloning theorem is enforced by `rustc` itself, not just by a custom analysis pass.

**Why not chumsky/LALR?** A hand-written recursive-descent parser gives full control over error recovery and span tracking. For a language as small as OpenQASM 3, the complexity of a parser combinator library isn't justified, and the Dragon Book approach produces clearer, more debuggable code.

**Why ariadne?** Compiler diagnostics are a first-class feature, not an afterthought. Ariadne provides Rust-compiler-quality error rendering with minimal effort.

## Roadmap

- [ ] Gate definitions (`gate h q { ... }`)
- [ ] Classical expressions and control flow (`if`, `for`, `while`)
- [ ] Function definitions (`def`)
- [ ] Type system for classical values (`int`, `float`, `bool`)
- [ ] IR lowering (translate AST to a flat circuit representation)
- [ ] Encode qubit linearity into Rust's type system via generics
- [ ] Circuit optimization passes (gate cancellation, commutation)

## License

MIT
