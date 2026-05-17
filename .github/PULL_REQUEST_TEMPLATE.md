## Summary

Describe the change in 1-3 sentences.

## Motivation

Why is this change needed?

## Test Plan

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all-targets`
- [ ] `cargo sqlx prepare` and commit `.sqlx/` if any `sqlx::query!` changed

## Screenshots

If the change affects the UI, add before/after screenshots or a short GIF.
