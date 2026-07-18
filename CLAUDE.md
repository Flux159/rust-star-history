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

Set `STAR_HISTORY_TRACE=1` to log every HTTP request the client actually sends (including retries) — useful for verifying cache behavior.

## Architecture

- `src/main.rs` — clap CLI. Token resolution order: `--token` → `$GITHUB_TOKEN` → `$GH_TOKEN` → `gh auth token` (optional convenience) → unauthenticated. `--repo` is repeatable/comma-separated for comparison charts; `--both` writes a `-dark`-suffixed second file. Cache flags: `--data-dir` (default `.rust-star-history`), `--no-cache`, `--update-cache` (force full refetch), `--fetch-only` (no chart), `--offline` (serve repo(s) from the store, no network — bare flag means all repos, and an all-offline run skips token resolution entirely).
- `src/github.rs` — API client (ureq/rustls). Fetches `starred_at` timestamps from `/repos/{repo}/stargazers` with the `application/vnd.github.star+json` accept header, 100/page. Repos needing more than `--max-pages` requests are **sampled**: evenly spaced pages, cumulative counts reconstructed from page offsets. The API serves at most 400 pages (40K stars); beyond that the curve endpoint is pinned to the repo's true `stargazers_count`. `collapse()` reduces anchors to one monotonic entry per day. `fetch_full` builds the curve from scratch; `fetch_incremental` extends cached anchors by fetching only pages at/past the cached end (`tail_pages`) and returns `Ok(None)` when the star count went down — net unstars shift page offsets, so the caller falls back to a full fetch.
- `src/store.rs` — per-repo JSON cache under the data dir (`owner__name.json`; owner names can't contain `_`, so the separator is unambiguous). Stores collapsed anchors plus `full_fetch_at`/`updated_at`/`stargazers_count`/`sampled` and a format `version` — bump `VERSION` on any format change (mismatches are silently refetched). Entries whose last full fetch is ≥ `FULL_REFRESH_DAYS` (28) old get a full refetch to shed drift from unstar-then-star sequences that incremental updates can't see.
- `src/chart.rs` — SVG renderer. xkcd sketch aesthetic via `feTurbulence` + `feDisplacementMap`; Handlee font subset (OFL) embedded at compile time from `assets/handlee-subset.woff2` via `include_bytes!` and emitted as a base64 data URI. Light + GitHub-dark palettes. Y-axis: adaptive "nice" tick steps with ~8% headroom above the tallest line (keeps the end-count label clear of the curve). Curves are downsampled to ≤64 points before the Catmull-Rom spline so daily jitter doesn't make the line jagged. Single-repo charts get a gradient area fill; multi-repo charts are lines-only, cycling through `SERIES_COLORS`.
- `src/date.rs` — hand-rolled civil-date math (Hinnant's `days_from_civil`). Deliberate: **no chrono/time dependency**. Total dependency budget is ureq, serde_json, clap, base64 — think hard before adding more.

## Conventions and gotchas

- **Font rendering contexts**: the embedded data-URI font works in `<img>`/README embeds and local viewing, but `raw.githubusercontent.com` serves a `default-src 'none'; sandbox` CSP that blocks it when an SVG is opened as a top-level document. That's why `FONT_FAMILY` has a deliberate fallback stack (Comic Sans MS etc.) — don't "simplify" it away, and don't reference external font URLs (self-contained SVG is a hard requirement).
- The four SVGs in `assets/` are committed showcase artifacts referenced by the README — regenerate them whenever renderer output changes, in the same commit.
- **2026 stargazers API restriction** (verified empirically): the stargazers *list* endpoint requires a token from a user with access to the repo. Owner PAT → 200; unauthenticated → 401; GitHub Actions automatic workflow token → 403 "You do not have permission to view the stargazers" (even for the workflow's own repo, and even though the same token can read `stargazers_count`). This is why the action's `token` input must be a PAT secret while `push-token` can stay the automatic token.
- GitHub API quirk observed in practice: `/repos/*` requests can 403 with "rate limit exceeded" for a few minutes even when `/rate_limit` shows quota remaining (separate anti-scraping bucket). The client retries rate-limit 403/429s and 5xxs with exponential backoff (6 attempts, `Retry-After`-aware, capped at 60s/attempt) but fails fast on permission errors — keep that distinction.
- Tests are pure-function unit tests plus SVG structure assertions (`generates_wellformed_svg_with_expected_elements`). Keep output `xmllint`-valid; all user-supplied text goes through `esc()`. Store tests exercise `render`/`parse` directly rather than touching the filesystem.
- The action carries `.rust-star-history` between runs with `actions/cache` (`cache` input, default on): the key embeds `github.run_id` so each run saves an updated cache, and `restore-keys` restores the newest previous one. Cache keys can't contain commas — the repos list is flattened with `tr` first.
- SVG text uses attribute-quoted strings — font names with spaces need single quotes inside the double-quoted attribute.

## Release process

Push a version tag and `cd.yml` does everything:

```sh
git tag v1.0.1 && git push origin v1.0.1
```

The workflow runs tests, syncs `Cargo.toml`/`Cargo.lock` to the tag with `cargo set-version` (committing the bump back to main and force-moving the tag onto it), creates a GitHub release with generated notes, then builds and uploads binaries for four targets (Linux x86_64/arm64, macOS arm64, Windows x86_64). Intel mac is intentionally not built — source install covers it.

`action.yml` (the reusable composite action) downloads the release asset matching the runner's platform via the direct `github.com/…/releases/download/…` CDN URL (pinned to the action's own `@vX.Y.Z` ref, else `releases/latest/download`) — deliberately not the REST API, which needs a token and can hit rate limits (a fine-grained PAT once burned its quota and 403'd the lookup). A failed download is a hard error; there is no build-from-source fallback. `install.sh` is the curl-based installer served from main via raw.githubusercontent.

## Workflows

- `.github/workflows/ci.yml` — fmt, clippy `-D warnings`, tests, release build on pushes/PRs.
- `.github/workflows/cd.yml` — the tag-triggered release pipeline described above.
- `.github/workflows/star-history.yml` — self-charts this repo daily using the action; doubles as the canonical usage example.
