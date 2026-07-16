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

/// Result of a stargazer fetch: the cumulative curve plus the metadata the
/// data store records alongside it.
pub struct StarData {
    pub cum: Cumulative,
    pub stargazers_count: u64,
    /// Whether this fetch skipped pages (page sampling) rather than reading
    /// every page it covered.
    pub sampled: bool,
}

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
        // Debug aid: log every request (including retries) actually sent.
        let trace = std::env::var_os("STAR_HISTORY_TRACE").is_some();
        for attempt in 1..=MAX_ATTEMPTS {
            if trace {
                eprintln!("TRACE GET {path}");
            }
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

    /// Fetch star history from scratch.
    ///
    /// Repos needing at most `max_pages` requests are fetched exactly. Bigger
    /// ones are sampled: an evenly spaced subset of pages is fetched, and the
    /// cumulative count at each star is reconstructed from its page offset
    /// (page N starts at star (N-1)*100), the same approach star-history.com
    /// uses.
    pub fn fetch_full(&self, repo: &str, max_pages: u64) -> Result<StarData, String> {
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

        let anchors = self.fetch_pages(repo, &pages)?;
        Ok(StarData {
            cum: finalize(repo, anchors, count)?,
            stargazers_count: count,
            sampled,
        })
    }

    /// Extend cached anchors by fetching only pages at and past the cached
    /// end (the page holding the last cached star may have been partial, so
    /// it is re-fetched; `collapse` dedupes the overlap).
    ///
    /// Returns Ok(None) when the cache can't be extended safely — net unstars
    /// shift every page offset — and the caller should fall back to a full
    /// fetch.
    pub fn fetch_incremental(
        &self,
        repo: &str,
        cached: &Cumulative,
        cached_count: u64,
        max_pages: u64,
    ) -> Result<Option<StarData>, String> {
        let cached_final = match cached.last() {
            Some(&(_, n)) if n > 0 => n,
            _ => return Ok(None),
        };
        let count = self.stargazer_count(repo)?;
        if count < cached_final {
            return Ok(None);
        }
        // Compare against the count seen last time, not the last anchor: the
        // list can yield slightly fewer entries than stargazers_count (e.g.
        // deleted accounts), which would otherwise refetch the last page on
        // every run.
        if count == cached_count || count == cached_final {
            eprintln!("Stargazers for {repo}: {count} stars, unchanged since last fetch (cached)");
            return Ok(Some(StarData {
                cum: cached.clone(),
                stargazers_count: count,
                sampled: false,
            }));
        }

        let total_pages = count.div_ceil(100).min(API_PAGE_CAP);
        let (pages, sampled) = tail_pages(cached_final, total_pages, max_pages);
        eprintln!(
            "Fetching stargazers for {repo}: {count} stars, {cached_final} cached, {} new page(s){}",
            pages.len(),
            if sampled { " (sampled)" } else { "" }
        );

        let mut anchors = cached.clone();
        anchors.extend(self.fetch_pages(repo, &pages)?);
        Ok(Some(StarData {
            cum: finalize(repo, anchors, count)?,
            stargazers_count: count,
            sampled,
        }))
    }

    fn fetch_pages(&self, repo: &str, pages: &[u64]) -> Result<Vec<(Day, u64)>, String> {
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
        Ok(anchors)
    }
}

/// Pin the curve's endpoint past the API's 40k cap and collapse to daily anchors.
fn finalize(repo: &str, mut anchors: Vec<(Day, u64)>, count: u64) -> Result<Cumulative, String> {
    if anchors.is_empty() {
        return Err(format!("no stargazers found for {repo}"));
    }
    // Stars past the API's 40k cap are unreachable; pin the curve's end to
    // the true total so the chart doesn't understate the count.
    if count > API_PAGE_CAP * 100 {
        let last_day = anchors.iter().map(|a| a.0).max().unwrap();
        anchors.push((last_day, count));
    }
    Ok(collapse(anchors))
}

/// Pages needed to extend a cache whose last anchor is star number
/// `cached_final`: from the page containing that star through the last page,
/// sampled (second return) when the span exceeds `max_pages`.
fn tail_pages(cached_final: u64, total_pages: u64, max_pages: u64) -> (Vec<u64>, bool) {
    let first = (cached_final / 100 + 1).min(total_pages);
    let span = total_pages - first + 1;
    if span > max_pages {
        let pages = sample_pages(span, max_pages)
            .into_iter()
            .map(|p| p + first - 1)
            .collect();
        (pages, true)
    } else {
        ((first..=total_pages).collect(), false)
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
         GitHub Actions, the automatic workflow token cannot read stargazers; set the action's \
         `token` input to a PAT stored as a secret."
    } else {
        match code {
            401 => {
                if had_token {
                    "The token was rejected; check that it is valid and not expired."
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
        "No token found; trying unauthenticated (likely to fail: GitHub's 2026 API change \
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
    fn tail_pages_resumes_from_the_partial_last_page() {
        // 337 stars cached → page 4 (stars 301+) onward, re-covering the overlap
        assert_eq!(tail_pages(337, 10, 100), ((4..=10).collect(), false));
        // exactly on a page boundary → next page only
        assert_eq!(tail_pages(300, 4, 100), (vec![4], false));
        // cached end at/past the API cap → re-fetch just the last page
        assert_eq!(tail_pages(45_000, 400, 100), (vec![400], false));
        // oversized tail gets sampled within [first, total]
        let (pages, sampled) = tail_pages(1_000, 400, 10);
        assert!(sampled);
        assert!(pages.len() <= 10);
        assert_eq!(*pages.first().unwrap(), 11);
        assert_eq!(*pages.last().unwrap(), 400);
        assert!(pages.windows(2).all(|w| w[0] < w[1]));
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
