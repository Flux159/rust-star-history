# rust-star-history

Generate self-hosted star-history SVG charts for any GitHub repository — a single static Rust binary (~2 MB), no gh CLI, no external services, no runtime dependencies.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="star-history-dark.svg">
  <img alt="Star History" src="star-history.svg">
</picture>

## Features

- **Single self-contained binary** — talks to the GitHub REST API directly over HTTPS (rustls, no OpenSSL); builds and runs on macOS, Linux, and Windows
- **Self-contained SVG output** — the chart is a static file with the Handlee font (OFL licensed) embedded as a data URI, so it renders identically everywhere GitHub renders SVGs
- **xkcd sketch aesthetic** — hand-drawn look via SVG `feTurbulence` + `feDisplacementMap` filters
- **Multi-repo comparison** — pass `--repo` more than once to plot several repos on one chart, each with its own color
- **Light & dark themes** — GitHub-dark palette, with `--both` emitting a matched pair for `<picture>`-based auto-switching
- **GitHub Action included** — publish always-fresh charts to a `star-history` branch on a schedule
- **Scales to big repos** — adaptive y-axis ticks, thinned month labels, and evenly-sampled page fetching for repos with tens of thousands of stars
- **Flexible auth** — works unauthenticated on public repos (60 req/hour); accepts a token via `--token`, `$GITHUB_TOKEN`, `$GH_TOKEN`, or (if installed) `gh auth token`

## Install

```sh
cargo install --git https://github.com/Flux159/rust-star-history
# or from a checkout:
cargo install --path .
# or just build a release binary (~2 MB):
cargo build --release   # → target/release/rust-star-history
```

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

## GitHub Action: automated, always-fresh charts

The bundled action regenerates your charts on a schedule and force-pushes them to a dedicated `star-history` branch — a consistent, predictable location in every repo that uses it, with no commit noise on `main`.

**1.** Add `.github/workflows/star-history.yml` to your repo:

```yaml
name: Star History

on:
  schedule:
    - cron: '17 3 * * 1' # weekly
  workflow_dispatch:      # allows manual runs from the Actions tab

permissions:
  contents: write         # required to push the star-history branch

jobs:
  star-history:
    runs-on: ubuntu-latest
    steps:
      - uses: Flux159/rust-star-history@main
```

That's it — the default `${{ github.token }}` is used automatically; no PAT or secrets setup is needed for charting the repo the workflow runs in. Run it once manually (Actions tab → Star History → Run workflow) to create the branch.

**2.** After the first run, the branch `star-history` contains `star-history.svg` and `star-history-dark.svg`. Embed them in your README via raw URLs (replace `OWNER/REPO`):

```html
<a href="https://github.com/Flux159/rust-star-history">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/OWNER/REPO/star-history/star-history-dark.svg">
    <img alt="Star History" src="https://raw.githubusercontent.com/OWNER/REPO/star-history/star-history.svg">
  </picture>
</a>
```

### Action inputs

All inputs are optional:

| Input | Default | Description |
|---|---|---|
| `repos` | the current repo | Repo(s) to chart, comma-separated for a comparison chart |
| `token` | `${{ github.token }}` | Token for API calls and pushing the branch. Needed explicitly only when charting *other* repos beyond the API's unauthenticated reach or pushing elsewhere |
| `branch` | `star-history` | Branch the SVGs are published to |
| `title` | `Star History` | Chart title |
| `colors` | built-in palette | Comma-separated hex colors, one per repo |
| `width` / `height` | `800` / `533` | Chart dimensions |
| `push` | `true` | Set `false` to only generate the SVGs into the workspace (e.g. to commit them yourself) |

Example — comparison chart with custom colors:

```yaml
      - uses: Flux159/rust-star-history@main
        with:
          repos: Flux159/mcp-server-kubernetes,Flux159/mcp-chat
          colors: '#dd4528,#28a9dd'
```

The action compiles the CLI with the runner's preinstalled Rust toolchain on first use (about a minute); the rest of the run is a few seconds.

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

## Font license

The bundled Handlee font subset is licensed under the [SIL Open Font License](assets/HANDLEE-LICENSE).

## License

MIT
