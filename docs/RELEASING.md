# Releasing

1. Update `CHANGELOG.md` and the version in `Cargo.toml`.
2. Run `cargo package` and the checks in `CONTRIBUTING.md`.
3. Commit the release preparation.
4. Tag the commit as `vX.Y.Z` and push the tag.

The release workflow verifies the tag, creates a GitHub release, and publishes
to crates.io when the `CARGO_REGISTRY_TOKEN` repository secret is configured.
