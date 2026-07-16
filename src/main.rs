//! rust-star-history: generate a star-history SVG chart for any GitHub repo.
//!
//! Self-hosted alternative to star-history.com. Talks to the GitHub REST API
//! over plain HTTPS, no gh CLI required (though `gh auth token` is used as a
//! token fallback when available).

mod chart;
mod date;
mod github;

use clap::Parser;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    version,
    about = "Generate a star-history SVG chart for any GitHub repo. No gh CLI required.",
    after_help = "\
Examples:
  rust-star-history --repo Flux159/mcp-server-kubernetes
  rust-star-history --repo owner/name --output chart.svg --color '#0066cc'
  rust-star-history --repo owner/a --repo owner/b       # comparison chart
  rust-star-history --repo owner/a,owner/b --both       # same, plus dark variant

Embed in README.md (auto light/dark switching):
  <picture>
    <source media=\"(prefers-color-scheme: dark)\" srcset=\"star-history-dark.svg\">
    <img alt=\"Star History\" src=\"star-history.svg\">
  </picture>

Token lookup order: --token, $GITHUB_TOKEN, $GH_TOKEN, `gh auth token`.
Since GitHub's 2026 API change, the stargazers list requires a token from a
user with access to the repo (owner/collaborator); unauthenticated requests
and GitHub Actions workflow tokens are rejected."
)]
struct Args {
    /// GitHub repo in owner/name format; repeat (or comma-separate) to compare
    #[arg(long, required = true, value_delimiter = ',')]
    repo: Vec<String>,

    /// Output SVG file path
    #[arg(long, default_value = "star-history.svg")]
    output: String,

    /// Line color(s) as hex, matched to repos in order (default: built-in palette)
    #[arg(long, value_delimiter = ',')]
    color: Vec<String>,

    /// Chart title
    #[arg(long, default_value = "Star History")]
    title: String,

    /// Chart width in px
    #[arg(long, default_value_t = 800)]
    width: u32,

    /// Chart height in px
    #[arg(long, default_value_t = 533)]
    height: u32,

    /// Use dark theme (GitHub-dark-like palette)
    #[arg(long)]
    dark: bool,

    /// Write both themes: OUTPUT plus a -dark variant
    #[arg(long, conflicts_with = "dark")]
    both: bool,

    /// GitHub token (falls back to $GITHUB_TOKEN, $GH_TOKEN, then `gh auth token`)
    #[arg(long)]
    token: Option<String>,

    /// Max stargazer pages per repo (100 stars each); bigger repos are sampled
    #[arg(long, default_value_t = 100)]
    max_pages: u64,
}

fn normalize_color(c: &str) -> String {
    if c.starts_with('#') {
        c.to_string()
    } else {
        format!("#{c}")
    }
}

fn run(args: Args) -> Result<(), String> {
    for repo in &args.repo {
        if repo.matches('/').count() != 1 {
            return Err(format!("--repo '{repo}' must be in 'owner/name' format"));
        }
    }

    let token = github::resolve_token(args.token.clone());
    let client = github::Client::new(token);
    let mut fetched: Vec<(String, github::Cumulative)> = Vec::new();
    for repo in &args.repo {
        let cum = client.fetch_cumulative(repo, args.max_pages.max(2))?;
        fetched.push((repo.clone(), cum));
    }

    let colors: Vec<String> = (0..fetched.len())
        .map(|i| {
            args.color
                .get(i)
                .map(|c| normalize_color(c))
                .unwrap_or_else(|| chart::SERIES_COLORS[i % chart::SERIES_COLORS.len()].to_string())
        })
        .collect();
    let series: Vec<chart::Series> = fetched
        .iter()
        .zip(&colors)
        .map(|((repo, cum), color)| chart::Series { repo, color, cum })
        .collect();

    let mut outputs = vec![(args.output.clone(), args.dark)];
    if args.both {
        let dark_path = match args.output.rsplit_once('.') {
            Some((stem, ext)) => format!("{stem}-dark.{ext}"),
            None => format!("{}-dark", args.output),
        };
        outputs.push((dark_path, true));
    }

    for (path, dark) in outputs {
        let svg = chart::generate_svg(
            &series,
            &chart::Options {
                title: &args.title,
                width: args.width,
                height: args.height,
                dark,
            },
        );
        std::fs::write(&path, &svg).map_err(|e| format!("failed writing {path}: {e}"))?;
        println!("Generated {path}:");
        for (repo, cum) in &fetched {
            let (first, last) = (cum[0].0, cum.last().unwrap().0);
            println!(
                "  {repo}: {} stars, {} – {}",
                cum.last().unwrap().1,
                first.month_day(),
                last.month_day()
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ERROR: {e}");
            ExitCode::FAILURE
        }
    }
}
