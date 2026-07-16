//! rust-star-history: generate a star-history SVG chart for any GitHub repo.
//!
//! Self-hosted alternative to star-history.com. Talks to the GitHub REST API
//! over plain HTTPS, no gh CLI required (though `gh auth token` is used as a
//! token fallback when available).

mod chart;
mod date;
mod github;
mod store;

use clap::Parser;
use std::path::Path;
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
  rust-star-history --repo owner/a --fetch-only         # update cached data, no chart
  rust-star-history --repo owner/a,owner/b --offline owner/a   # owner/a from cache

Fetched data is cached per repo in .rust-star-history/ (see --data-dir), so
repeat runs only fetch pages added since last time. --no-cache bypasses the
cache, --update-cache rebuilds it from scratch (this also happens automatically
every 28 days), and --offline charts straight from it without contacting
GitHub — handy when different repos need different tokens.

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

    /// Directory where fetched star data is cached (created on first use)
    #[arg(long, default_value = ".rust-star-history")]
    data_dir: String,

    /// Bypass the data cache entirely: fetch fresh, read and write nothing
    #[arg(long, conflicts_with_all = ["offline", "fetch_only", "update_cache"])]
    no_cache: bool,

    /// Chart from cached data without contacting GitHub; bare --offline
    /// applies to all repos, or pass specific repos (repeat/comma-separate)
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    offline: Option<Vec<String>>,

    /// Fetch and cache star data only, without generating charts
    #[arg(long)]
    fetch_only: bool,

    /// Discard cached data for the given repos and refetch from scratch
    /// (otherwise the cache updates incrementally, with an automatic full
    /// refresh every 28 days)
    #[arg(long)]
    update_cache: bool,
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

    if let Some(list) = &args.offline {
        for repo in list {
            if !args.repo.contains(repo) {
                return Err(format!(
                    "--offline '{repo}' is not one of the --repo values"
                ));
            }
        }
    }
    let offline = |repo: &String| {
        args.offline
            .as_ref()
            .is_some_and(|list| list.is_empty() || list.contains(repo))
    };

    let data_dir = Path::new(&args.data_dir);
    let today = date::Day::today();
    let client = if args.repo.iter().all(offline) {
        None // everything comes from the data dir; no token needed
    } else {
        Some(github::Client::new(github::resolve_token(
            args.token.clone(),
        )))
    };

    let mut fetched: Vec<(String, github::Cumulative)> = Vec::new();
    for repo in &args.repo {
        let cached = if args.no_cache {
            None
        } else {
            store::load(data_dir, repo)
        };

        let cum = if offline(repo) {
            let entry = cached.ok_or_else(|| {
                format!(
                    "--offline: no cached data for {repo} in {}; fetch it first \
                     (e.g. rust-star-history --repo {repo} --fetch-only)",
                    data_dir.display()
                )
            })?;
            eprintln!(
                "Using cached data for {repo}: {} stars as of {}",
                entry.stargazers_count,
                entry.updated_at.iso()
            );
            entry.anchors
        } else {
            let client = client.as_ref().expect("client exists for online repos");
            let max_pages = args.max_pages.max(2);

            // Try an incremental update of the cache; fall back to a full
            // fetch when there is no usable cache, it's due for its periodic
            // full refresh, or net unstars made incremental unsafe.
            let mut full_fetch_at = today;
            let mut prior_sampled = false;
            let incremental = match &cached {
                None => None,
                Some(_) if args.update_cache => None,
                Some(entry)
                    if today.to_epoch_days() - entry.full_fetch_at.to_epoch_days()
                        >= store::FULL_REFRESH_DAYS =>
                {
                    eprintln!(
                        "Cached data for {repo} last fully fetched {}; doing the periodic full refresh",
                        entry.full_fetch_at.iso()
                    );
                    None
                }
                Some(entry) => {
                    let data = client.fetch_incremental(
                        repo,
                        &entry.anchors,
                        entry.stargazers_count,
                        max_pages,
                    )?;
                    if data.is_none() {
                        eprintln!(
                            "Star count for {repo} went down since the last fetch; refetching from scratch"
                        );
                    } else {
                        full_fetch_at = entry.full_fetch_at;
                        prior_sampled = entry.sampled;
                    }
                    data
                }
            };
            let data = match incremental {
                Some(data) => data,
                None => client.fetch_full(repo, max_pages)?,
            };

            if !args.no_cache {
                store::save(
                    data_dir,
                    &store::Entry {
                        repo: repo.clone(),
                        full_fetch_at,
                        updated_at: today,
                        stargazers_count: data.stargazers_count,
                        sampled: prior_sampled || data.sampled,
                        anchors: data.cum.clone(),
                    },
                )?;
            }
            data.cum
        };
        fetched.push((repo.clone(), cum));
    }

    if args.fetch_only {
        println!("Fetched star data into {}:", args.data_dir);
        for (repo, cum) in &fetched {
            let (first, last) = (cum[0].0, cum.last().unwrap().0);
            println!(
                "  {repo}: {} stars, {} – {}",
                cum.last().unwrap().1,
                first.month_day(),
                last.month_day()
            );
        }
        return Ok(());
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
