# qasm-rs

An OpenQASM 3 compiler written in Rust with **compile-time qubit linearity enforcement**.

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
┌─────────┐   Recursive-descent parser with Pratt expression
│  Parser  │   parsing. Produces a fully span-annotated AST
└────┬─────┘   covering gate defs, modifiers, classical
     │         control flow, and parameterized expressions.
     ▼
┌─────────┐   Scoped symbol table with push/pop for gate
│  Sema    │   defs, for-loops, and block bodies.
│          │   • Symbol resolution & index bounds checking
│          │   • Gate arity validation (param + qubit count)
│          │   • Qubit linearity with conservative analysis
└────┬─────┘     through branches (if/else, for, while)
     │
     ▼
┌─────────┐   Pretty-printer emitting canonical OpenQASM 3.
│ Codegen  │   Round-trip tested: parse → emit → re-parse
└──────────┘   produces structurally identical ASTs.
```

All errors are rendered with [ariadne](https://github.com/zesterer/ariadne) for precise, colorized source diagnostics with secondary labels pointing to related locations (e.g. "first declared here", "measured here").

## Supported language features

**Quantum operations:** qubit/bit declarations (scalar and register), gate calls with parameters, gate definitions with classical and qubit parameters, gate modifiers (`ctrl`, `negctrl`, `inv`, `pow`), measurement, reset, barrier.

**Classical control flow:** `if`/`else`, `for` loops with range expressions, `while` loops.

**Classical types:** `int`, `float`, `bool` declarations with optional initializers, assignment (`=`, `+=`, `-=`).

**Expressions:** Pratt parser with correct precedence — arithmetic (`+`, `-`, `*`, `/`, `**` right-associative), comparison (`==`, `!=`, `<`, `<=`, `>`, `>=`), unary negation, parenthesization, built-in constants (`pi`, `tau`, `euler`).

**Legacy compatibility:** `qreg`/`creg` syntax is accepted and mapped to `qubit`/`bit`.

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

## Conservative linearity through branches

Linearity analysis is **conservative (sound)** through control flow. If *either* branch of an `if` measures a qubit, it's considered measured after the `if` — because the compiler can't statically resolve the classical condition. The same applies to loop bodies: any measurement inside a loop is conservatively treated as having happened.

This is directly analogous to how Rust's borrow checker handles conditional moves.

```qasm
if (x == 0) {
  c = measure q;   // only one branch measures
}
h q;                // ← ERROR: conservatively measured
```

## Other diagnostics

- **Undeclared identifiers**: `h r[0];` → *"`r` is not declared"*
- **Index out of bounds**: `qubit[2] q; h q[5];` → *"index 5 is out of bounds for `q` (size 2)"*
- **Indexing a scalar**: `qubit q; h q[0];` → *"cannot index `q` — it is a single qubit, not a register"*
- **Kind mismatch**: `bit c; h c;` → *"expected qubit, but `c` is a bit"*
- **Duplicate declarations**: `qubit q; qubit q;` → *"`q` is already declared"* with label pointing to first declaration
- **Gate arity**: `gate rx(theta) q { ... } rx(1, 2) q;` → *"gate `rx` expects 1 parameter(s), got 2"*
- **Duplicate gate definitions**: `gate h q { } gate h q { }` → *"gate `h` is already defined"*
- **Scoping**: for-loop variables are not visible after the loop

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
├── main.rs      Entry point — compiles demo programs, renders diagnostics
├── span.rs      Span type (byte-offset range) and Spanned<T> wrapper
├── lexer.rs     logos-based tokenizer producing Vec<Spanned<Token>>
├── ast.rs       Span-annotated AST types (quantum + classical)
├── parser.rs    Recursive-descent parser with Pratt expression parsing
├── sema.rs      Semantic analysis (scoped symbols + linearity checking)
└── codegen.rs   Pretty-printer / QASM emitter with round-trip tests
```

## Design decisions

**Why Rust?** The type system maps naturally onto the problem. Qubits are linear resources; Rust's ownership model is built around linear/affine types. The long-term goal is to encode qubit linearity directly into Rust's type system so the no-cloning theorem is enforced by `rustc` itself, not just by a custom analysis pass.

**Why hand-written recursive descent?** A hand-written parser gives full control over error recovery and span tracking. For a language as small as OpenQASM 3, the complexity of a parser combinator library isn't justified, and the approach produces clearer, more debuggable code. The Pratt parser for expressions gives correct precedence with minimal code.

**Conservative linearity.** The choice to use union (not intersection) at branch points makes the analysis sound — it will never allow a use-after-measure to slip through. This is the same tradeoff Rust makes: it rejects some programs that would be safe at runtime in exchange for static guarantees.

**Why ariadne?** Compiler diagnostics are a first-class feature, not an afterthought. Ariadne provides Rust-compiler-quality error rendering with minimal effort.

## Roadmap

- [ ] Function definitions (`def`)
- [ ] IR lowering — translate AST to a DAG-based circuit representation
- [ ] Circuit optimization passes (gate cancellation, commutation analysis)
- [ ] Encode qubit linearity into Rust's type system via generics

## License

MIT
