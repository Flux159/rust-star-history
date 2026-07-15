# rust-star-history

Generate self-hosted star-history SVG charts for any GitHub repository.

**Why this exists:** GitHub's 2026 stargazers API change broke star-history.com embeds — the charts in countless READMEs simply stopped rendering for public visitors. Instead of depending on a third-party service that can break again, this tool generates the chart as a static SVG you own: committed to your repo (or published to a branch by the bundled GitHub Action), it renders forever with no external services, no rate limits, and nothing to break.

Built in Rust with a small footprint: one static ~2 MB binary with everything embedded (even the font), talking directly to the GitHub API over HTTPS. Nothing else needs to be installed on your system — no gh CLI, no Python, no Node.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/star-history-dark.svg">
  <img alt="Star History" src="assets/star-history.svg">
</picture>

## Quickstart

Add a star-history chart to your own repo in three steps:

**1.** Create a [fine-grained PAT](https://github.com/settings/personal-access-tokens/new) with read-only access to your repo (Contents + Metadata is enough) and save it as an Actions secret named `STAR_HISTORY_TOKEN` — either in the repo UI (Settings → Secrets and variables → Actions) or with the [gh CLI](https://cli.github.com/):

```sh
gh secret set STAR_HISTORY_TOKEN -R <USERNAME>/<REPONAME>   # paste the PAT when prompted
```

Since GitHub's 2026 API change, reading the stargazers list requires a token from a user with access to the repo — the automatic workflow token isn't allowed to.

**2.** Create `.github/workflows/star-history.yml`:

```yaml
name: Star History

on:
  schedule:
    - cron: '0 5 * * *' # daily
  workflow_dispatch:     # allows manual runs from the Actions tab

permissions:
  contents: write        # lets the action push the star-history branch

jobs:
  star-history:
    runs-on: ubuntu-latest
    steps:
      - uses: Flux159/rust-star-history@v1.0.0
        with:
          token: ${{ secrets.STAR_HISTORY_TOKEN }}
```

**3.** Run it once (Actions tab → Star History → Run workflow), then put this in your `README.md`, replacing both `<USERNAME>/<REPONAME>` with your repo (e.g. `Flux159/mcp-server-kubernetes`):

```html
<a href="https://github.com/Flux159/rust-star-history">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/<USERNAME>/<REPONAME>/star-history/star-history-dark.svg">
    <img alt="Star History" src="https://raw.githubusercontent.com/<USERNAME>/<REPONAME>/star-history/star-history.svg">
  </picture>
</a>
```

The chart updates daily on a dedicated `star-history` branch (main history stays clean) and automatically matches the viewer's light/dark theme. See [GitHub Action](#github-action-automated-always-fresh-charts) below for comparison charts, custom colors, and other options.

## Features

- **Single self-contained binary** — talks to the GitHub REST API directly over HTTPS (rustls, no OpenSSL); builds and runs on macOS, Linux, and Windows
- **Self-contained SVG output** — the chart is a static file with the Handlee font (OFL licensed) embedded as a data URI, so it renders identically everywhere GitHub renders SVGs
- **xkcd sketch aesthetic** — hand-drawn look via SVG `feTurbulence` + `feDisplacementMap` filters
- **Multi-repo comparison** — pass `--repo` more than once to plot several repos on one chart, each with its own color
- **Light & dark themes** — GitHub-dark palette, with `--both` emitting a matched pair for `<picture>`-based auto-switching
- **GitHub Action included** — publish always-fresh charts to a `star-history` branch on a schedule
- **Scales to big repos** — adaptive y-axis ticks, thinned month labels, and evenly-sampled page fetching for repos with tens of thousands of stars
- **Flexible auth** — accepts a token via `--token`, `$GITHUB_TOKEN`, `$GH_TOKEN`, or (if installed) `gh auth token`; retries transient failures and rate limits with exponential backoff. (Since GitHub's 2026 API change the stargazers list requires a token from a user with access to the repo.)

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/Flux159/rust-star-history/main/install.sh | sh
```

Detects your platform and installs the prebuilt binary (~2 MB) from the latest [release](https://github.com/Flux159/rust-star-history/releases) into `~/.local/bin` (override with `INSTALL_DIR=/usr/local/bin`). Prebuilt targets: Linux x86_64/arm64, macOS Apple Silicon, and Windows (via Git Bash). Tarballs can also be downloaded straight from the releases page, or see [Install from source](#install-from-source) for other platforms.

## Usage

```sh
# Basic — writes star-history.svg
rust-star-history --repo Flux159/mcp-server-kubernetes

# Light + dark pair for README auto-switching
rust-star-history --repo owner/name --both

# Compare multiple repos on one chart
rust-star-history --repo Flux159/mcp-server-kubernetes --repo Flux159/mcp-chat --both

# Customization
rust-star-history --repo owner/name --output chart.svg --color '#0066cc' \
    --title "My Project Stars" --width 800 --height 533 --dark

# With an explicit token (private repos / higher rate limits)
rust-star-history --repo owner/name --token ghp_xxx
```

| Flag | Default | Description |
|---|---|---|
| `--repo` | (required) | Repository in `owner/name` format; repeat or comma-separate to compare |
| `--output` | `star-history.svg` | Output SVG path |
| `--color` | built-in palette | Line color(s) as hex, matched to repos in order |
| `--title` | `Star History` | Chart title |
| `--width` / `--height` | `800` / `533` | Chart dimensions in px |
| `--dark` | off | Dark theme |
| `--both` | off | Write light `OUTPUT` plus `OUTPUT-dark` variant |
| `--token` | env / gh fallback | GitHub API token |
| `--max-pages` | `100` | Max stargazer pages per repo (100 stars each); larger repos are sampled evenly |

## Examples

A single repo gets a gradient area fill:

```sh
rust-star-history --repo Flux159/mcp-server-kubernetes --both
```

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/star-history-dark.svg">
  <img alt="Star history of Flux159/mcp-server-kubernetes" src="assets/star-history.svg">
</picture>

Comparison charts draw one line per repo — pass `--repo` as many times as you like (or comma-separate); lines cycle through a built-in 8-color palette, or set your own with `--color`:

```sh
rust-star-history --repo Flux159/mcp-server-kubernetes --repo Flux159/mcp-chat --both
# three or more works too, with custom colors:
rust-star-history --repo owner/a --repo owner/b --repo owner/c --color '#dd4528,#28a9dd,#a3a948'
```

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/star-history-compare-dark.svg">
  <img alt="Star history comparison of Flux159/mcp-server-kubernetes and Flux159/mcp-chat" src="assets/star-history-compare.svg">
</picture>

## Install from source

Works on any platform with a Rust toolchain (including Intel macs):

```sh
cargo install --git https://github.com/Flux159/rust-star-history
# or from a checkout:
cargo install --path .
# or just build a release binary:
cargo build --release   # → target/release/rust-star-history
```

## GitHub Action: automated, always-fresh charts

The bundled action regenerates your charts on a schedule and force-pushes them to a dedicated `star-history` branch — a consistent, predictable location in every repo that uses it, with no commit noise on `main`. See the [Quickstart](#quickstart) above for the three-step setup; the sections below cover auth, configuration, and using the CLI without the action.

### Tokens: why a PAT is needed

Since GitHub's 2026 API change, the stargazers *list* endpoint (the one with `starred_at` timestamps) only works with a token belonging to a user who has access to the repo — typically the owner or a collaborator. Unauthenticated requests get a 401, and the workflow's automatic `${{ github.token }}` gets a 403, because it isn't a user token. That's why the `token` input should be a [fine-grained PAT](https://github.com/settings/personal-access-tokens/new) stored as an Actions secret; read-only access (Contents + Metadata) to the charted repo is enough.

Pushing the chart branch is a different story: the automatic token handles that fine, which is what the `push-token` input defaults to — so your PAT never needs write access when publishing to the same repo.

**Comparison charts**: the PAT must have access to every repo being charted (your own repos all qualify automatically for a PAT scoped to them).

**Pushing the charts to a different repo than the one the workflow runs in** — pass a `push-token` with write access (Contents) to the target repo:

```yaml
      - uses: Flux159/rust-star-history@v1.0.0
        with:
          target-repo: your-org/other-repo
          token: ${{ secrets.STAR_HISTORY_TOKEN }}
          push-token: ${{ secrets.STAR_HISTORY_PUSH_PAT }}
```

### Action inputs

All inputs are optional:

| Input | Default | Description |
|---|---|---|
| `repos` | the current repo | Repo(s) to chart, comma-separated for a comparison chart |
| `token` | `${{ github.token }}` | Token for reading stargazers — set this to a PAT secret; the default automatic token cannot list stargazers (see [Tokens](#tokens-why-a-pat-is-needed)) |
| `branch` | `star-history` | Branch the SVGs are published to |
| `target-repo` | the current repo | Repo the chart branch is pushed to (cross-repo publishing needs `push-token`) |
| `push-token` | `${{ github.token }}` | Token used only for the branch push; the automatic token works for same-repo publishing |
| `title` | `Star History` | Chart title |
| `colors` | built-in palette | Comma-separated hex colors, one per repo |
| `width` / `height` | `800` / `533` | Chart dimensions |
| `push` | `true` | Set `false` to only generate the SVGs into the workspace (e.g. to commit them yourself) |

Example — comparison chart with custom colors:

```yaml
      - uses: Flux159/rust-star-history@v1.0.0
        with:
          repos: Flux159/mcp-server-kubernetes,Flux159/mcp-chat
          colors: '#dd4528,#28a9dd'
          token: ${{ secrets.STAR_HISTORY_TOKEN }}
```

The action downloads the prebuilt binary from this repo's releases (matching the action's pinned `@vX.Y.Z` tag, or the latest release when pinned to `@main`), so runs take just a few seconds. If no release asset exists for the runner's platform it falls back to building from source with the runner's Rust toolchain (about a minute).

### Using the CLI directly in a workflow (without the action)

The CLI picks up `$GITHUB_TOKEN` from the environment automatically — export your PAT secret there (the automatic workflow token can't read stargazers; see [Tokens](#tokens-why-a-pat-is-needed)). This example commits the SVGs to the current branch instead of a separate one:

```yaml
name: Star History

on:
  schedule:
    - cron: '17 3 * * 1'
  workflow_dispatch:

permissions:
  contents: write

jobs:
  star-history:
    runs-on: ubuntu-latest
    env:
      GITHUB_TOKEN: ${{ secrets.STAR_HISTORY_TOKEN }}
    steps:
      - uses: actions/checkout@v4
      - run: curl -fsSL https://raw.githubusercontent.com/Flux159/rust-star-history/main/install.sh | sh
      - run: rust-star-history --repo "${{ github.repository }}" --both
      - run: |
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add star-history.svg star-history-dark.svg
          git diff --cached --quiet || git commit -m "chore: update star history charts"
          git push
```

## Embedding from the same branch

If you generate charts locally instead of via the action, commit the SVGs next to your README and reference them relatively:

```html
<a href="https://github.com/Flux159/rust-star-history">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="star-history-dark.svg">
    <img alt="Star History" src="star-history.svg">
  </picture>
</a>
```

The `<a>` wrapper makes the whole chart clickable. (The SVG itself contains a link on the "Made with" attribution, but GitHub renders README images through a sandboxed `<img>` tag, so links inside an embedded SVG aren't clickable there — only wrapping the image works. The plain markdown form is `[![Star History](star-history.svg)](https://github.com/Flux159/rust-star-history)`.)

## How it works

1. `GET /repos/{owner}/{name}` for the total star count
2. Pages through `GET /repos/{owner}/{name}/stargazers` with the `application/vnd.github.star+json` accept header, which includes `starred_at` timestamps (100 per page). Repos needing more than `--max-pages` requests are sampled evenly across the page range, reconstructing cumulative counts from page offsets. (The API serves at most 400 pages / 40K stars; beyond that the curve's endpoint is pinned to the true total.)
3. Buckets timestamps into cumulative daily counts, downsamples to ≤64 curve points, and renders a Catmull-Rom spline roughened with SVG displacement filters for the sketch look.

## Development

```sh
cargo test          # unit tests: date math, sampling, tick logic, SVG structure
cargo build --release
```

### Releasing

Push a version tag and CD does the rest:

```sh
git tag v1.0.1 && git push origin v1.0.1
```

The `cd.yml` workflow runs the tests, syncs `Cargo.toml`/`Cargo.lock` to the tag's version with `cargo set-version` (committing the bump back to main and moving the tag onto it), creates a GitHub release with generated notes, and builds + uploads prebuilt binaries for all four supported platforms.

## Font license

The bundled Handlee font subset is licensed under the [SIL Open Font License](assets/HANDLEE-LICENSE).

## License

MIT
