# CLAUDE.md

Guidance for AI coding agents working in this repository.

## What this is

A Rust CLI that generates self-hosted star-history SVG charts for GitHub repos (an alternative to star-history.com), plus a composite GitHub Action that publishes those charts to a `star-history` branch on a schedule. Core constraints: a single static binary with no runtime dependencies (plain HTTPS to the GitHub REST API via rustls — deliberately **no gh CLI dependency**), and fully self-contained SVG output (font embedded as a data URI).

## Commands

```sh
cargo test                                    # unit tests
cargo fmt --check                             # CI enforces formatting
cargo clippy --all-targets -- -D warnings     # CI enforces zero warnings
cargo build --release                         # ~2 MB binary (lto + strip)
```

Regenerate the showcase charts after any renderer change (requires network + a token in `GITHUB_TOKEN`/`GH_TOKEN`):

```sh
cargo run --release -- --repo Flux159/mcp-server-kubernetes --both --output assets/star-history.svg
cargo run --release -- --repo Flux159/mcp-server-kubernetes --repo Flux159/mcp-chat --both --output assets/star-history-compare.svg
```

Validate output with `xmllint --noout assets/*.svg`.

## Architecture

- `src/main.rs` — clap CLI. Token resolution order: `--token` → `$GITHUB_TOKEN` → `$GH_TOKEN` → `gh auth token` (optional convenience) → unauthenticated. `--repo` is repeatable/comma-separated for comparison charts; `--both` writes a `-dark`-suffixed second file.
- `src/github.rs` — API client (ureq/rustls). Fetches `starred_at` timestamps from `/repos/{repo}/stargazers` with the `application/vnd.github.star+json` accept header, 100/page. Repos needing more than `--max-pages` requests are **sampled**: evenly spaced pages, cumulative counts reconstructed from page offsets. The API serves at most 400 pages (40K stars); beyond that the curve endpoint is pinned to the repo's true `stargazers_count`. `collapse()` reduces anchors to one monotonic entry per day.
- `src/chart.rs` — SVG renderer. xkcd sketch aesthetic via `feTurbulence` + `feDisplacementMap`; Handlee font subset (OFL) embedded at compile time from `assets/handlee-subset.woff2` via `include_bytes!` and emitted as a base64 data URI. Light + GitHub-dark palettes. Y-axis: adaptive "nice" tick steps with ~8% headroom above the tallest line (keeps the end-count label clear of the curve). Curves are downsampled to ≤64 points before the Catmull-Rom spline so daily jitter doesn't make the line jagged. Single-repo charts get a gradient area fill; multi-repo charts are lines-only, cycling through `SERIES_COLORS`.
- `src/date.rs` — hand-rolled civil-date math (Hinnant's `days_from_civil`). Deliberate: **no chrono/time dependency**. Total dependency budget is ureq, serde_json, clap, base64 — think hard before adding more.

## Conventions and gotchas

- **Font rendering contexts**: the embedded data-URI font works in `<img>`/README embeds and local viewing, but `raw.githubusercontent.com` serves a `default-src 'none'; sandbox` CSP that blocks it when an SVG is opened as a top-level document. That's why `FONT_FAMILY` has a deliberate fallback stack (Comic Sans MS etc.) — don't "simplify" it away, and don't reference external font URLs (self-contained SVG is a hard requirement).
- The four SVGs in `assets/` are committed showcase artifacts referenced by the README — regenerate them whenever renderer output changes, in the same commit.
- **2026 stargazers API restriction** (verified empirically): the stargazers *list* endpoint requires a token from a user with access to the repo. Owner PAT → 200; unauthenticated → 401; GitHub Actions automatic workflow token → 403 "You do not have permission to view the stargazers" (even for the workflow's own repo, and even though the same token can read `stargazers_count`). This is why the action's `token` input must be a PAT secret while `push-token` can stay the automatic token.
- GitHub API quirk observed in practice: `/repos/*` requests can 403 with "rate limit exceeded" for a few minutes even when `/rate_limit` shows quota remaining (separate anti-scraping bucket). The client retries rate-limit 403/429s and 5xxs with exponential backoff (6 attempts, `Retry-After`-aware, capped at 60s/attempt) but fails fast on permission errors — keep that distinction.
- Tests are pure-function unit tests plus SVG structure assertions (`generates_wellformed_svg_with_expected_elements`). Keep output `xmllint`-valid; all user-supplied text goes through `esc()`.
- SVG text uses attribute-quoted strings — font names with spaces need single quotes inside the double-quoted attribute.

## Release process

Push a version tag and `cd.yml` does everything:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

The workflow runs tests, syncs `Cargo.toml`/`Cargo.lock` to the tag with `cargo set-version` (committing the bump back to main and force-moving the tag onto it), creates a GitHub release with generated notes, then builds and uploads binaries for four targets (Linux x86_64/arm64, macOS arm64, Windows x86_64). Intel mac is intentionally not built — source install covers it.

`action.yml` (the reusable composite action) downloads the release asset matching the runner's platform — pinned to the action's own `@vX.Y.Z` ref, else latest — and falls back to `cargo install --git` only when no asset exists. `install.sh` is the curl-based installer served from main via raw.githubusercontent.

## Workflows

- `.github/workflows/ci.yml` — fmt, clippy `-D warnings`, tests, release build on pushes/PRs.
- `.github/workflows/cd.yml` — the tag-triggered release pipeline described above.
- `.github/workflows/star-history.yml` — self-charts this repo daily using the action; doubles as the canonical usage example.
