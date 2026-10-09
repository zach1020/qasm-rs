# Contributing

1. Install Rust 1.85 or newer.
2. Create a focused branch.
3. Add regression tests with compiler changes.
4. Run the full local check:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo doc --no-deps
```

Parser and semantic changes should include both a valid and invalid fixture.
Optimizer changes must demonstrate the transformation and a nearby case that
must not transform.
