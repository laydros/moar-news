# AGENTS.md

This file defines the working agreement for coding agents in this repository.

## Scope

- Applies to the entire repository rooted at `/Users/laydros/src/dev/moar-news`.
- If a deeper `AGENTS.md` is added later, the deeper file overrides this one for its subtree.

## Primary Goal

- Keep `moar-news` stable, simple, and fast.
- Prefer incremental, testable changes over broad refactors.

## Tech Stack

- Rust 2021
- Axum
- Askama templates + HTMX frontend behavior
- SQLx with SQLite

## Editing Rules

- Make the smallest change that fully fixes the issue.
- Preserve existing style and architecture unless a change is explicitly requested.
- Avoid introducing new dependencies unless necessary.
- Do not silently change behavior outside the requested scope.
- Never run destructive git commands unless explicitly requested.

## Validation

- Run focused tests first when possible.
- For Rust changes, prefer at least one of:
  - `cargo test <targeted-tests>`
  - `cargo test`
  - `cargo check`
- If validation cannot be run, state that clearly in the final report.

## Versioning Policy

- The crate version is in `Cargo.toml` and must stay in sync with the root package entry in `Cargo.lock`.
- On any merged code change, increment the patch version (`0.0.x`) by default.
  - Example: `0.2.3` -> `0.2.4`
- Do not bump minor (`0.x.0`) unless the user explicitly asks.
- Do not bump major (`x.0.0`) unless the user explicitly asks.
- If unsure which level to bump, ask the user before changing the version.

## Change Reporting

- Summarize exactly what changed and why.
- Include file paths for all edits.
- List validation commands run and their outcomes.
