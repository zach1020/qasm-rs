# Architecture

`qasm-rs` separates source-language structure from circuit optimization:

1. `lexer` produces span-carrying tokens.
2. `parser` builds the AST; `parse_recovering` can collect independent errors.
3. `sema` resolves symbols, infers classical expression types, validates
   register widths and gate/function signatures, and tracks measured qubits.
4. `lower` preserves control flow in HIR and represents straight-line quantum
   regions as circuit DAGs.
5. `opt` performs local graph rewrites. Basis decomposition is opt-in because
   it changes the emitted gate vocabulary.
6. `codegen` emits canonical OpenQASM from AST, HIR, or circuit regions.

Local include files are expanded by `include::load_with_includes` before
parsing. `stdgates.inc` remains symbolic because its supported signatures are
known to semantic analysis.

The legacy `lower` function is a compatibility alias for
`lower_straight_line`. New callers compiling complete programs should use
`lower_hir` or the top-level `compile_source` API.

Gate and function inlining are explicit transformations in `inline`; they are
not enabled by default, preserving source-level definitions unless requested.
