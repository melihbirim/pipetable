# Contributing

Thanks for looking at Pipetable. Small, focused PRs are the fastest to review and merge.

## Setup

```sh
cargo build
cargo test
```

Rust stable, edition 2021. No other setup needed — DuckDB is bundled.

## Before opening a PR

- Keep the diff scoped to the issue. If you spot something unrelated, open a separate issue instead of bundling it in.
- `cargo test` passes.
- One issue = one PR. Reference it in the description (`Closes #N`).

## Picking an issue

Look for [`good first issue`](https://github.com/melihbirim/pipetable/labels/good%20first%20issue) — each one has suggested files, acceptance criteria, and points at the exact code involved. If the issue is unclear or you think the suggested approach is wrong, comment before starting — cheaper than a wasted PR.

## Code style

- Match the existing style in the file you're editing over any external style guide.
- No new dependencies without a good reason — check `Cargo.toml` first, most things you need are probably already a dependency.
- Comments only where the *why* isn't obvious from the code.

## Commit messages

One line, plain description of the change (e.g. `fix: stale token on upload modal`). No AI attribution trailers.

## Questions

Open an issue, or email hello@pipetable.com.
