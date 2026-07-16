//! Per-repo star-data store: JSON files under a data directory (default
//! `.rust-star-history/`) that cache fetched stargazer history between runs
//! and serve `--offline` charts.

use crate::date::Day;
use crate::github::Cumulative;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Bump when the on-disk format changes; mismatched files are refetched.
const VERSION: u64 = 1;

/// Incremental updates drift when stars are retracted without the total
/// shrinking, so entries whose last full fetch is older than this get a
/// fresh full fetch (`--update-cache` forces one immediately).
pub const FULL_REFRESH_DAYS: i64 = 28;

pub struct Entry {
    pub repo: String,
    /// Last full (non-incremental) fetch; drives the periodic full refresh.
    pub full_fetch_at: Day,
    /// Last time this entry was written after contacting the API.
    pub updated_at: Day,
    pub stargazers_count: u64,
    /// Whether any fetch that built these anchors skipped pages (sampling).
    pub sampled: bool,
    pub anchors: Cumulative,
}

/// `owner/name` → `<dir>/owner__name.json` (GitHub owner names can't contain
/// underscores, so the separator is unambiguous).
pub fn path_for(dir: &Path, repo: &str) -> PathBuf {
    dir.join(format!("{}.json", repo.replace('/', "__")))
}

pub fn load(dir: &Path, repo: &str) -> Option<Entry> {
    let path = path_for(dir, repo);
    let text = std::fs::read_to_string(&path).ok()?;
    let Some(entry) = parse(&text) else {
        eprintln!(
            "  warning: ignoring cache file {} (corrupt or older format); refetching",
            path.display()
        );
        return None;
    };
    if !entry.repo.eq_ignore_ascii_case(repo) {
        eprintln!(
            "  warning: cache file {} is for {}, not {repo}; ignoring it",
            path.display(),
            entry.repo
        );
        return None;
    }
    Some(entry)
}

pub fn save(dir: &Path, entry: &Entry) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("failed creating {}: {e}", dir.display()))?;
    let path = path_for(dir, &entry.repo);
    std::fs::write(&path, render(entry))
        .map_err(|e| format!("failed writing {}: {e}", path.display()))
}

fn render(entry: &Entry) -> String {
    let anchors: Vec<Value> = entry
        .anchors
        .iter()
        .map(|&(day, n)| json!([day.iso(), n]))
        .collect();
    json!({
        "version": VERSION,
        "repo": entry.repo,
        "full_fetch_at": entry.full_fetch_at.iso(),
        "updated_at": entry.updated_at.iso(),
        "stargazers_count": entry.stargazers_count,
        "sampled": entry.sampled,
        "anchors": anchors,
    })
    .to_string()
}

fn parse(text: &str) -> Option<Entry> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v["version"].as_u64() != Some(VERSION) {
        return None;
    }
    let anchors = v["anchors"]
        .as_array()?
        .iter()
        .map(|a| Some((Day::parse(a[0].as_str()?)?, a[1].as_u64()?)))
        .collect::<Option<Cumulative>>()?;
    if anchors.is_empty() {
        return None;
    }
    Some(Entry {
        repo: v["repo"].as_str()?.to_string(),
        full_fetch_at: Day::parse(v["full_fetch_at"].as_str()?)?,
        updated_at: Day::parse(v["updated_at"].as_str()?)?,
        stargazers_count: v["stargazers_count"].as_u64()?,
        sampled: v["sampled"].as_bool()?,
        anchors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Day {
        Day::parse(s).unwrap()
    }

    fn sample_entry() -> Entry {
        Entry {
            repo: "owner/name".to_string(),
            full_fetch_at: d("2026-06-20"),
            updated_at: d("2026-07-15"),
            stargazers_count: 42,
            sampled: true,
            anchors: vec![(d("2025-12-31"), 10), (d("2026-07-15"), 42)],
        }
    }

    #[test]
    fn render_parse_round_trips() {
        let entry = sample_entry();
        let parsed = parse(&render(&entry)).unwrap();
        assert_eq!(parsed.repo, entry.repo);
        assert_eq!(parsed.full_fetch_at, entry.full_fetch_at);
        assert_eq!(parsed.updated_at, entry.updated_at);
        assert_eq!(parsed.stargazers_count, entry.stargazers_count);
        assert_eq!(parsed.sampled, entry.sampled);
        assert_eq!(parsed.anchors, entry.anchors);
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(parse("not json").is_none());
        assert!(parse("{}").is_none());
        // wrong version
        let stale = render(&sample_entry()).replace("\"version\":1", "\"version\":99");
        assert!(parse(&stale).is_none());
        // empty anchors
        let empty = render(&Entry {
            anchors: vec![],
            ..sample_entry()
        });
        assert!(parse(&empty).is_none());
    }

    #[test]
    fn path_maps_slash_to_double_underscore() {
        assert_eq!(
            path_for(Path::new(".rust-star-history"), "owner/repo_name"),
            Path::new(".rust-star-history/owner__repo_name.json")
        );
    }
}
