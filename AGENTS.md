# Repository Guidelines

## Project Structure & Module Organization
- `src/main.rs` hosts the CLI entry point, service clients, and shared helpers; split additions into focused modules.
- `Cargo.toml` defines the binary crate (`dist`) and dependencies; update feature flags here when adding APIs.
- `README.md` covers setup and environment variables; mirror schema updates there.
- Create additional modules under `src/` (e.g. `src/services/bluesky.rs`) as code grows; place tests inline with `#[cfg(test)]` or in `tests/`.

## Build, Test, and Development Commands
- `cargo check` quickly validates the code compiles before pushing.
- `cargo fmt --all` applies rustfmt across the workspace; run before committing to keep diffs minimal.
- `cargo clippy --all-targets --all-features` surfaces lints; fix or use targeted `#[allow]` sparingly.
- `cargo test` runs unit and integration tests; add focused integration tests in `tests/` for CLI flows.
- `cargo run -- --help` confirms argument parsing; `cargo run -- "Test post" --no-bsky` is a quick smoketest.

## Coding Style & Naming Conventions
- Rust 2024 edition with 4-space indentation; rely on rustfmt defaults for layout and imports.
- Use `snake_case` for modules/functions, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for environment variables and constants.
- Keep async helpers small; share request builders for testability.
- Document Clap options with `///` comments so `--help` output stays descriptive.

## Testing Guidelines
- Add integration tests in `tests/` (e.g. `tests/crosspost_cli.rs`) when changing CLI behavior or concurrency.
- Prefer doctests or unit tests near helpers for parsing, serialization, or env loading.
- Avoid live network calls in automated tests; stub responses or gate manual checks behind `--no-*` flags.
- Describe manual verification steps (services exercised, env used) in the PR body.

## Commit & Pull Request Guidelines
- Write imperative commit subjects (`Add nostr relay fallback`) and keep them under ~72 characters.
- Structure commits around a single concern; include context in the body when behavior changes.
- PRs should summarize impact, link issues, and list validation commands (`cargo fmt`, `cargo test`, manual runs).
- Attach screenshots or log excerpts only when they clarify new user-facing output—never include tokens or handles.

## Configuration & Security Notes
- Store credentials in `.env`; add new variables to `.env.example` or the README but never to history.
- Ensure new services document required scopes and rate limits; highlight secrets needed for local runs.
- Rotate or revoke app passwords used during development after merges touching authentication flows.
