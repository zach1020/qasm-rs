# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows semantic versioning.

## [Unreleased]

### Added

- Recursive local include resolution with cycle detection.
- Qualified `const`, `input`, and `output` declarations.
- `uint` and `angle` numeric types.
- Indexed bit expressions and inclusive quantum-register slices.
- Numeric delay statements.
- Single-expression classical functions and definition inlining.
- User-defined gate inlining.
- Recovering parser API for multiple syntax diagnostics.
- Register-width diagnostics during semantic analysis.
- Commuting inverse cancellation and inverse-rotation peepholes.
- Opt-in decomposition to the `{rz, sx, cx}` basis.
- CLI, conformance, generated round-trip, and include-resolution tests.
- CI, benchmark harness, and tag-based release automation.

### Fixed

- Invalid cancellation of repeated phase gates.
- Loss of non-literal control-modifier counts.
- Incorrect lowering of register gate broadcasts.
- Acceptance of unknown gates and classical type mismatches.

[Unreleased]: https://github.com/zach1020/qasm-rs/compare/v0.1.0...HEAD
