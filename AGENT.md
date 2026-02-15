# Agent Directives

- Read `docs/SPEC.md` for the full technical specification.
- Read `docs/WORKLOG.md` for known issues, architecture notes, and planned work.
- Update `docs/WORKLOG.md` and this file after completing work to keep them current.
- Run `cargo test --workspace` after making changes to verify nothing is broken (182 tests across 8 suites).
- Server integration tests use `tower::oneshot()` with in-memory SQLite — see `crates/conclave-server/tests/api_tests.rs` for the pattern.
- Client MLS tests use `tempfile::TempDir` for isolated crypto state — see the test module in `crates/conclave-lib/src/mls.rs`.
- Shared client logic (API, MLS, config, state, store, commands) lives in `conclave-lib`. Both `conclave-cli` (TUI) and `conclave-gui` (iced) depend on it.
