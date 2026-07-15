//! GitHub REST API client: plain HTTPS requests, no gh CLI dependency.

use crate::date::Day;
use serde_json::Value;
use std::io::Write;
use std::time::Duration;

const API: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("rust-star-history/", env!("CARGO_PKG_VERSION"));
const STAR_ACCEPT: &str = "application/vnd.github.star+json";
const JSON_ACCEPT: &str = "application/vnd.github+json";
/// The stargazers endpoint only serves the first 400 pages (40,000 stars).
const API_PAGE_CAP: u64 = 400;

pub struct Client {
    agent: ureq::Agent,
    token: Option<String>,
}

/// Cumulative star history: (day, total stars at end of that day), ascending.
pub type Cumulative = Vec<(Day, u64)>;

impl Client {
    pub fn new(token: Option<String>) -> Client {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        Client { agent, token }
    }

    fn get(&self, path: &str, accept: &str) -> Result<Value, String> {
        const MAX_ATTEMPTS: u32 = 6;
        let url = format!("{API}{path}");
        for attempt in 1..=MAX_ATTEMPTS {
            let mut req = self
                .agent
                .get(&url)
                .set("User-Agent", USER_AGENT)
                .set("Accept", accept)
                .set("X-GitHub-Api-Version", "2022-11-28");
            if let Some(token) = &self.token {
                req = req.set("Authorization", &format!("Bearer {token}"));
            }
            match req.call() {
                Ok(resp) => {
                    let text = resp
                        .into_string()
                        .map_err(|e| format!("failed reading response body: {e}"))?;
                    return serde_json::from_str(&text)
                        .map_err(|e| format!("GitHub returned invalid JSON: {e}"));
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let retry_after = resp
                        .header("retry-after")
                        .and_then(|v| v.parse::<u64>().ok());
                    let body = resp.into_string().unwrap_or_default();
                    // Retry server errors and rate limits (GitHub's burst
                    // buckets 403 transiently even with quota remaining), but
                    // fail fast on real auth/permission errors.
                    if (code >= 500 || is_rate_limited(code, &body)) && attempt < MAX_ATTEMPTS {
                        let delay = retry_after.unwrap_or(1 << attempt).min(60);
                        eprintln!(
                            "\n  GitHub API {code} on attempt {attempt}/{MAX_ATTEMPTS}, retrying in {delay}s..."
                        );
                        std::thread::sleep(Duration::from_secs(delay));
                        continue;
                    }
                    return Err(friendly_http_error(code, &body, path, self.token.is_some()));
                }
                Err(e) if attempt < MAX_ATTEMPTS => {
                    eprintln!(
                        "  transient error ({e}), retrying in {}s...",
                        1u64 << attempt
                    );
                    std::thread::sleep(Duration::from_secs(1 << attempt));
                }
                Err(e) => return Err(format!("request to {url} failed: {e}")),
            }
        }
        unreachable!("retry loop always returns")
    }

    /// Number of stargazers reported by the repo endpoint.
    pub fn stargazer_count(&self, repo: &str) -> Result<u64, String> {
        let info = self.get(&format!("/repos/{repo}"), JSON_ACCEPT)?;
        info["stargazers_count"]
            .as_u64()
            .ok_or_else(|| "repo response missing stargazers_count".to_string())
    }

    fn stargazer_page(&self, repo: &str, page: u64) -> Result<Vec<Day>, String> {
        let path = format!("/repos/{repo}/stargazers?per_page=100&page={page}");
        let value = self.get(&path, STAR_ACCEPT)?;
        let items = value
            .as_array()
            .ok_or_else(|| "stargazers response was not an array".to_string())?;
        items
            .iter()
            .map(|item| {
                item["starred_at"]
                    .as_str()
                    .and_then(Day::parse)
                    .ok_or_else(|| "stargazer entry missing starred_at timestamp".to_string())
            })
            .collect()
    }

    /// Fetch star history as cumulative (day, count) pairs.
    ///
    /// Repos needing at most `max_pages` requests are fetched exactly. Bigger
    /// ones are sampled: an evenly spaced subset of pages is fetched, and the
    /// cumulative count at each star is reconstructed from its page offset
    /// (page N starts at star (N-1)*100), the same approach star-history.com
    /// uses.
    pub fn fetch_cumulative(&self, repo: &str, max_pages: u64) -> Result<Cumulative, String> {
        let count = self.stargazer_count(repo)?;
        if count == 0 {
            return Err(format!("no stargazers found for {repo}"));
        }
        let total_pages = count.div_ceil(100).min(API_PAGE_CAP);
        let sampled = total_pages > max_pages;
        let pages: Vec<u64> = if sampled {
            sample_pages(total_pages, max_pages)
        } else {
            (1..=total_pages).collect()
        };

        eprintln!(
            "Fetching stargazers for {repo}: {count} stars, {} of {total_pages} pages{}",
            pages.len(),
            if sampled { " (sampled)" } else { "" }
        );

        let mut anchors: Vec<(Day, u64)> = Vec::new();
        for (i, &page) in pages.iter().enumerate() {
            eprint!("\r  page {}/{}", i + 1, pages.len());
            std::io::stderr().flush().ok();
            let days = self.stargazer_page(repo, page)?;
            if days.is_empty() {
                break;
            }
            let base = (page - 1) * 100;
            anchors.extend(
                days.into_iter()
                    .enumerate()
                    .map(|(idx, day)| (day, base + idx as u64 + 1)),
            );
        }
        eprintln!();

        if anchors.is_empty() {
            return Err(format!("no stargazers found for {repo}"));
        }

        // Stars past the API's 40k cap are unreachable; pin the curve's end to
        // the true total so the chart doesn't understate the count.
        if count > API_PAGE_CAP * 100 {
            let last_day = anchors.last().unwrap().0;
            anchors.push((last_day, count));
        }

        Ok(collapse(anchors))
    }
}

/// Evenly spaced page numbers in [1, total], always including both endpoints.
fn sample_pages(total: u64, wanted: u64) -> Vec<u64> {
    let wanted = wanted.clamp(2, total);
    let mut pages: Vec<u64> = (0..wanted)
        .map(|i| 1 + (i * (total - 1)) / (wanted - 1))
        .collect();
    pages.dedup();
    pages
}

/// Reduce (day, cumulative) anchors to one monotonically increasing entry per day.
fn collapse(mut anchors: Vec<(Day, u64)>) -> Cumulative {
    anchors.sort();
    let mut out: Cumulative = Vec::new();
    let mut running = 0u64;
    for (day, cum) in anchors {
        let cum = cum.max(running);
        running = cum;
        match out.last_mut() {
            Some(last) if last.0 == day => last.1 = cum,
            _ => out.push((day, cum)),
        }
    }
    out
}

/// Rate-limit 403/429s are transient and worth retrying; permission 403s are not.
fn is_rate_limited(code: u16, body: &str) -> bool {
    if code != 403 && code != 429 {
        return false;
    }
    let b = body.to_lowercase();
    b.contains("rate limit") || b.contains("secondary") || b.contains("abuse")
}

fn friendly_http_error(code: u16, body: &str, path: &str, had_token: bool) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect());
    let hint = if code == 403
        && message
            .to_lowercase()
            .contains("permission to view the stargazers")
    {
        "GitHub's 2026 API change restricts the stargazers list to tokens of users with access \
         to the repo (owner/collaborator). Pass such a token via --token or GITHUB_TOKEN. In \
         GitHub Actions, the automatic workflow token cannot read stargazers — set the action's \
         `token` input to a PAT stored as a secret."
    } else {
        match code {
            401 => {
                if had_token {
                    "The token was rejected — check that it is valid and not expired."
                } else {
                    "Authentication is required since GitHub's 2026 API change. Pass a token via \
                     --token or GITHUB_TOKEN (it must belong to a user with access to the repo)."
                }
            }
            403 | 429 => {
                if had_token {
                    "Rate limited or access denied (still failing after retries). Your token may \
                     lack access to this repo's stargazers."
                } else {
                    "Rate limited or access denied (still failing after retries). Pass a token \
                     via --token or GITHUB_TOKEN."
                }
            }
            404 => {
                "Repo not found. Check the owner/name spelling; private repos need a token with \
                 repo access."
            }
            _ => "",
        }
    };
    format!("GitHub API error {code} on {path}: {message}\n{hint}")
        .trim_end()
        .to_string()
}

/// Resolve a token: explicit flag, then env vars, then `gh auth token` if the
/// gh CLI happens to be installed (optional convenience, never required).
pub fn resolve_token(explicit: Option<String>) -> Option<String> {
    if let Some(t) = explicit {
        return Some(t);
    }
    for var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            if !t.trim().is_empty() {
                eprintln!("Using token from ${var}");
                return Some(t.trim().to_string());
            }
        }
    }
    if let Ok(out) = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
    {
        if out.status.success() {
            let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !t.is_empty() {
                eprintln!("Using token from gh CLI");
                return Some(t);
            }
        }
    }
    eprintln!(
        "No token found; trying unauthenticated (likely to fail — GitHub's 2026 API change \
         requires a token from a user with access to the repo)"
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Day {
        Day::parse(s).unwrap()
    }

    #[test]
    fn rate_limit_errors_are_retryable_but_permission_errors_are_not() {
        assert!(is_rate_limited(
            403,
            r#"{"message":"API rate limit exceeded for user ID 1"}"#
        ));
        assert!(is_rate_limited(
            429,
            r#"{"message":"You have exceeded a secondary rate limit"}"#
        ));
        assert!(!is_rate_limited(
            403,
            r#"{"message":"You do not have permission to view the stargazers of this repository"}"#
        ));
        assert!(!is_rate_limited(200, "rate limit"));
    }

    #[test]
    fn permission_error_explains_2026_api_change() {
        let msg = friendly_http_error(
            403,
            r#"{"message":"You do not have permission to view the stargazers of this repository"}"#,
            "/repos/o/r/stargazers",
            true,
        );
        assert!(msg.contains("2026 API change"));
        assert!(msg.contains("`token` input"));
    }

    #[test]
    fn sample_pages_covers_endpoints() {
        assert_eq!(sample_pages(10, 20), (1..=10).collect::<Vec<_>>());
        let s = sample_pages(400, 30);
        assert_eq!(*s.first().unwrap(), 1);
        assert_eq!(*s.last().unwrap(), 400);
        assert!(s.len() <= 30);
        assert!(s.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn collapse_dedupes_days_and_stays_monotonic() {
        let cum = collapse(vec![
            (d("2025-01-02"), 3),
            (d("2025-01-01"), 1),
            (d("2025-01-02"), 5),
            (d("2025-01-03"), 4), // sampled anchor below running max
        ]);
        assert_eq!(
            cum,
            vec![
                (d("2025-01-01"), 1),
                (d("2025-01-02"), 5),
                (d("2025-01-03"), 5)
            ]
        );
    }
}
