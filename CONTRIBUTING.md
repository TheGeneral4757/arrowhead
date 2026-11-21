# Contributing

## Workflow
- One milestone at a time (see [roadmap](docs/specs/11-roadmap.md)). One branch and one PR per milestone; parallel work inside a milestone goes on short task branches merged into it.
- Specs first. If a change disagrees with a spec, update the spec in the same PR.
- Unverified game facts live in [open questions](docs/specs/12-open-questions.md). Do not hard-code one without a measurement or a config value.

## Commits
Conventional commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `ci:`, `chore:`), scoped by crate where useful: `feat(core): fit scroll speed with Theil-Sen`. release-please builds the changelog from them.

## Checks (once code exists)
| Command | From | Must pass |
|---|---|---|
| `cargo fmt --all --check` | root | yes |
| `cargo clippy --workspace --all-targets -- -D warnings` | root | yes |
| `cargo test --workspace` | root | yes |
| `npm run lint && npm run build` | `ui/` | yes |

Recorded clips are in git LFS. Run `git lfs pull` before the clip regression tests.

## Style
- Straight quotes and plain hyphens in code and docs.
- Comments only for the why.
- No allocation, locking or I/O in the scan and scheduler hot paths.
