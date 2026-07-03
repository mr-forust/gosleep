# Contributing

## Checks

Run these before pushing:

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --release --locked
```

## Style

- Keep the application single-binary and Rust-first.
- Keep config backwards-compatible where possible.
- Prefer explicit, small functions over hidden behavior in the TUI.
- Do not add runtime dependencies on Python, Node, or external services.

## Commits

Use concise conventional commits where practical:

```text
feat: add timer progress gauge
fix: wrap preview commands on narrow terminals
chore: release v1.1.1
```
