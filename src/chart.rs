//! SVG chart rendering: xkcd sketch aesthetic (feTurbulence/feDisplacementMap
//! filter) with an embedded Handlee font subset (OFL licensed), light and
//! GitHub-dark palettes, adaptive y-axis ticks, and thinned month labels so
//! charts stay readable for repos with thousands of stars.

use crate::date::Day;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

const FONT_WOFF2: &[u8] = include_bytes!("../assets/handlee-subset.woff2");
// Fallbacks cover contexts where the embedded font is blocked (e.g. the CSP
// on raw.githubusercontent.com): casual handwriting fonts commonly installed
// on macOS/Windows/Linux, before the generic keyword (which macOS maps to the
// very-serif Apple Chancery).
const FONT_FAMILY: &str =
    "Handlee, 'Comic Sans MS', 'Chalkboard SE', 'Comic Neue', 'Segoe Print', cursive";
const XKCD: &str = " filter=\"url(#xkcdify)\"";

/// Line colors assigned to series without an explicit --color, first repo red.
pub const SERIES_COLORS: [&str; 8] = [
    "#dd4528", "#28a9dd", "#f3db00", "#a3a948", "#edb92e", "#f85931", "#ce1836", "#009989",
];

/// Cap on spline control points per line; daily data beyond this is
/// downsampled so the curve stays smooth instead of tracing every day's jitter.
const MAX_CURVE_POINTS: usize = 64;

/// One repo's line: name, stroke color, and cumulative (day, stars) data.
pub struct Series<'a> {
    pub repo: &'a str,
    pub color: &'a str,
    pub cum: &'a [(Day, u64)],
}

pub struct Options<'a> {
    pub title: &'a str,
    pub width: u32,
    pub height: u32,
    pub dark: bool,
}

struct Palette {
    bg: &'static str,
    fg: &'static str,
    axis: &'static str,
    grid: &'static str,
    legend_bg: &'static str,
    legend_border: &'static str,
    dot_stroke: &'static str,
}

const LIGHT: Palette = Palette {
    bg: "#fff",
    fg: "#000",
    axis: "#222",
    grid: "#eee",
    legend_bg: "#fff",
    legend_border: "#000",
    dot_stroke: "#fff",
};

const DARK: Palette = Palette {
    bg: "#0d1117",
    fg: "#e6edf3",
    axis: "#30363d",
    grid: "#21262d",
    legend_bg: "#161b22",
    legend_border: "#30363d",
    dot_stroke: "#0d1117",
};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Catmull-Rom spline rendered as smooth SVG cubic Beziers.
fn smooth_path(pts: &[(f64, f64)]) -> String {
    if pts.len() < 2 {
        return String::new();
    }
    let mut p = format!("M {:.1},{:.1}", pts[0].0, pts[0].1);
    for i in 0..pts.len() - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(pts.len() - 1)];
        let cp1x = p1.0 + (p2.0 - p0.0) / 6.0;
        let cp1y = p1.1 + (p2.1 - p0.1) / 6.0;
        let cp2x = p2.0 - (p3.0 - p1.0) / 6.0;
        let cp2y = p2.1 - (p3.1 - p1.1) / 6.0;
        p += &format!(
            " C {cp1x:.1},{cp1y:.1} {cp2x:.1},{cp2y:.1} {:.1},{:.1}",
            p2.0, p2.1
        );
    }
    p
}

/// Keep at most `max` points, evenly spaced by index, endpoints preserved.
fn downsample(pts: Vec<(f64, f64)>, max: usize) -> Vec<(f64, f64)> {
    if pts.len() <= max {
        return pts;
    }
    (0..max)
        .map(|i| pts[i * (pts.len() - 1) / (max - 1)])
        .collect()
}

/// Format tick values: 1000 → 1K, 2500 → 2.5K.
fn fmt_count(n: u64) -> String {
    if n >= 1000 {
        let v = n as f64 / 1000.0;
        if v == v.trunc() {
            format!("{}K", v as u64)
        } else {
            format!("{v}K")
        }
    } else {
        n.to_string()
    }
}

/// Abbreviate to at most one decimal for the end-of-line count: 1458 → 1.5K.
fn fmt_short(n: u64) -> String {
    if n >= 1000 {
        let v = (n as f64 / 100.0).round() / 10.0;
        if v == v.trunc() {
            format!("{}K", v as u64)
        } else {
            format!("{v:.1}K")
        }
    } else {
        n.to_string()
    }
}

/// Smallest "nice" y-axis step that keeps the tick count at or under 8.
fn tick_step(max: u64) -> u64 {
    const STEPS: [u64; 13] = [
        25, 50, 100, 250, 500, 1000, 2500, 5000, 10_000, 25_000, 50_000, 100_000, 250_000,
    ];
    for step in STEPS {
        if max.div_ceil(step) <= 8 {
            return step;
        }
    }
    500_000
}

struct TextEl<'a> {
    x: String,
    y: String,
    content: &'a str,
    size: u32,
    weight: &'a str,
    fill: &'a str,
    anchor: Option<&'a str>,
    transform: Option<String>,
}

impl<'a> TextEl<'a> {
    fn new(x: impl ToString, y: impl ToString, content: &'a str, fill: &'a str) -> Self {
        TextEl {
            x: x.to_string(),
            y: y.to_string(),
            content,
            size: 16,
            weight: "bold",
            fill,
            anchor: None,
            transform: None,
        }
    }

    fn size(mut self, size: u32) -> Self {
        self.size = size;
        self
    }

    fn anchor(mut self, anchor: &'a str) -> Self {
        self.anchor = Some(anchor);
        self
    }

    fn transform(mut self, transform: String) -> Self {
        self.transform = Some(transform);
        self
    }

    fn render(&self) -> String {
        let mut attrs = format!(
            "x=\"{}\" y=\"{}\" fill=\"{}\" font-size=\"{}\" font-family=\"{}\" font-weight=\"{}\"",
            self.x, self.y, self.fill, self.size, FONT_FAMILY, self.weight
        );
        if let Some(a) = self.anchor {
            attrs += &format!(" text-anchor=\"{a}\"");
        }
        if let Some(t) = &self.transform {
            attrs += &format!(" transform=\"{t}\"");
        }
        format!("<text {attrs}>{}</text>", esc(self.content))
    }
}

/// Render the star history chart for one or more repos.
pub fn generate_svg(series: &[Series], opts: &Options) -> String {
    assert!(!series.is_empty(), "at least one series required");
    assert!(
        series.iter().all(|s| !s.cum.is_empty()),
        "series data must not be empty"
    );

    let (w, h) = (f64::from(opts.width), f64::from(opts.height));
    let (pad_l, pad_r, pad_t, pad_b) = (90.0, 35.0, 80.0, 70.0);
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;
    let pal = if opts.dark { &DARK } else { &LIGHT };

    // Shared domain across all series
    let first_epoch = series
        .iter()
        .map(|s| s.cum[0].0.to_epoch_days())
        .min()
        .unwrap();
    let last_epoch = series
        .iter()
        .map(|s| s.cum.last().unwrap().0.to_epoch_days())
        .max()
        .unwrap();
    let date_range = (last_epoch - first_epoch).max(1) as f64;
    let x_of = |epoch: i64| pad_l + ((epoch - first_epoch) as f64 / date_range) * plot_w;

    let max_stars_all = series
        .iter()
        .map(|s| s.cum.last().unwrap().1)
        .max()
        .unwrap();
    // ~8% headroom above the tallest line so its end-count label fits inside
    // the plot instead of colliding with the curve near the top edge
    let padded_max = max_stars_all + (max_stars_all / 12).max(1);
    let step = tick_step(padded_max);
    let y_max = padded_max.div_ceil(step).max(1) * step;
    let y_of = |count: u64| pad_t + plot_h - (count as f64 / y_max as f64) * plot_h;

    let font_b64 = BASE64.encode(FONT_WOFF2);
    // Area fill only for single-repo charts; comparisons stay lines-only
    let fill_area = series.len() == 1;

    // Legend: one row per series, sized to the longest repo name
    let char_w = 7.5;
    let max_name_len = series.iter().map(|s| s.repo.chars().count()).max().unwrap();
    let (legend_pad, swatch, swatch_gap, row_h) = (10.0, 8.0, 8.0, 22.0);
    let legend_w = max_name_len as f64 * char_w + swatch + swatch_gap + legend_pad * 2.0 + 7.0;
    let legend_h = 32.0 + (series.len() as f64 - 1.0) * row_h;
    let legend_x = pad_l + 15.0;
    let legend_y = pad_t + 10.0;
    let mut legend = format!(
        "<rect width=\"{legend_w:.0}\" height=\"{legend_h:.0}\" x=\"{legend_x}\" y=\"{legend_y}\" \
         fill=\"{}\" fill-opacity=\"0.9\" stroke=\"{}\" stroke-width=\"1.5\" rx=\"4\" ry=\"4\"{XKCD}/>",
        pal.legend_bg, pal.legend_border
    );
    for (i, s) in series.iter().enumerate() {
        let row_y = legend_y + i as f64 * row_h;
        // Repo labels link to the repo (clickable when the SVG is opened
        // directly; inert inside README <img> embeds like all SVG links)
        legend += &format!(
            "<rect width=\"{swatch}\" height=\"{swatch}\" x=\"{}\" y=\"{:.0}\" rx=\"2\" ry=\"2\" fill=\"{}\"{XKCD}/>\
             <a href=\"https://github.com/{}\" target=\"_blank\" rel=\"noopener\">{}</a>",
            legend_x + legend_pad,
            row_y + 12.0,
            s.color,
            esc(s.repo),
            TextEl::new(
                legend_x + legend_pad + swatch + swatch_gap,
                format!("{:.0}", row_y + 20.0),
                s.repo,
                pal.fg
            )
            .size(15)
            .render()
        );
    }

    // Y-axis gridlines and tick labels
    let mut y_elements: Vec<String> = Vec::new();
    let mut tick = 0;
    while tick <= y_max {
        let y_val = y_of(tick);
        if tick > 0 {
            y_elements.push(format!(
                "<line x1=\"{pad_l}\" y1=\"{y_val:.1}\" x2=\"{:.0}\" y2=\"{y_val:.1}\" \
                 stroke=\"{}\" stroke-width=\"1\"{XKCD}/>",
                w - pad_r,
                pal.grid
            ));
        }
        let label = fmt_count(tick);
        y_elements.push(
            TextEl::new(pad_l - 5.0, format!("{:.1}", y_val + 5.0), &label, pal.fg)
                .anchor("end")
                .render(),
        );
        tick += step;
    }
    let y_title_cy = pad_t + plot_h / 2.0;
    y_elements.push(
        TextEl::new(38, format!("{y_title_cy:.1}"), "GitHub Stars", pal.fg)
            .size(17)
            .anchor("middle")
            .transform(format!("rotate(-90, 38, {y_title_cy:.1})"))
            .render(),
    );

    // X-axis: calendar months across the shared domain, thinned to fit
    let first_day = series.iter().map(|s| s.cum[0].0).min().unwrap();
    let mut month_positions: Vec<(f64, Day)> = Vec::new();
    let (mut year, mut month) = (first_day.year, first_day.month);
    loop {
        let month_start = Day {
            year,
            month,
            day: 1,
        };
        let epoch = month_start.to_epoch_days().max(first_epoch);
        if epoch > last_epoch {
            break;
        }
        month_positions.push((x_of(epoch), month_start));
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    let max_labels = ((plot_w / 95.0) as usize).max(2);
    if month_positions.len() > max_labels {
        let stride = month_positions.len().div_ceil(max_labels);
        month_positions = month_positions.into_iter().step_by(stride).collect();
    }

    // Labels are centered on their month's position; drop any whose text
    // would spill past the plot's right edge.
    let half_label_w = 38.0;
    let mut x_labels: Vec<String> = Vec::new();
    for &(x, day) in &month_positions {
        if x + half_label_w > w - pad_r {
            continue;
        }
        let label = day.month_year();
        x_labels.push(
            TextEl::new(
                format!("{x:.1}"),
                format!("{:.0}", pad_t + plot_h + 25.0),
                &label,
                pal.fg,
            )
            .anchor("middle")
            .render(),
        );
    }
    x_labels.push(
        TextEl::new("50%", h - 18.0, "Date", pal.fg)
            .size(17)
            .anchor("middle")
            .render(),
    );

    let mut defs = vec![
        "  <defs>".to_string(),
        format!("    <style>@font-face{{font-family:\"Handlee\";src:url(data:font/woff2;charset=utf-8;base64,{font_b64}) format(\"woff2\")}}</style>"),
        "    <filter id=\"xkcdify\" width=\"100%\" height=\"100%\" x=\"-5\" y=\"-5\" filterUnits=\"userSpaceOnUse\">".into(),
        "      <feTurbulence baseFrequency=\".05\" result=\"noise\" type=\"fractalNoise\"/>".into(),
        "      <feDisplacementMap in=\"SourceGraphic\" in2=\"noise\" scale=\"3\" xChannelSelector=\"R\" yChannelSelector=\"G\"/>".into(),
        "    </filter>".into(),
    ];
    if fill_area {
        defs.extend([
            "    <linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">".to_string(),
            format!(
                "      <stop offset=\"0%\" stop-color=\"{}\" stop-opacity=\"0.22\"/>",
                series[0].color
            ),
            format!(
                "      <stop offset=\"100%\" stop-color=\"{}\" stop-opacity=\"0.02\"/>",
                series[0].color
            ),
            "    </linearGradient>".into(),
        ]);
    }
    defs.push("  </defs>".into());

    let mut svg: Vec<String> = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".into(),
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" width=\"100%\" height=\"auto\">", opts.width, opts.height),
    ];
    svg.extend(defs);
    svg.extend([
        format!(
            "  <rect width=\"{}\" height=\"{}\" fill=\"{}\"/>",
            opts.width, opts.height, pal.bg
        ),
        "  ".to_string()
            + &TextEl::new("50%", 30, opts.title, pal.fg)
                .size(20)
                .anchor("middle")
                .render(),
        "  ".to_string() + &legend,
    ]);
    svg.extend(y_elements.iter().map(|e| format!("  {e}")));
    svg.extend(x_labels.iter().map(|e| format!("  {e}")));
    svg.extend([
        format!(
            "  <line x1=\"{pad_l}\" y1=\"{pad_t}\" x2=\"{pad_l}\" y2=\"{:.0}\" stroke=\"{}\" stroke-width=\"2.5\"{XKCD}/>",
            pad_t + plot_h,
            pal.axis
        ),
        format!(
            "  <line x1=\"{pad_l}\" y1=\"{:.0}\" x2=\"{:.0}\" y2=\"{:.0}\" stroke=\"{}\" stroke-width=\"2.5\"{XKCD}/>",
            pad_t + plot_h,
            w - pad_r,
            pad_t + plot_h,
            pal.axis
        ),
    ]);

    for s in series {
        let points: Vec<(f64, f64)> = s
            .cum
            .iter()
            .map(|&(day, count)| (x_of(day.to_epoch_days()), y_of(count)))
            .collect();
        let points = downsample(points, MAX_CURVE_POINTS);
        let line_path = smooth_path(&points);
        let (first_pt, last_pt) = (points[0], *points.last().unwrap());

        if fill_area {
            let area_path = format!(
                "{line_path} L {:.1},{:.1} L {:.1},{:.1} Z",
                last_pt.0,
                pad_t + plot_h,
                first_pt.0,
                pad_t + plot_h
            );
            svg.push(format!("  <path d=\"{area_path}\" fill=\"url(#g)\"/>"));
        }
        svg.push(format!(
            "  <path d=\"{line_path}\" fill=\"none\" stroke=\"{}\" stroke-width=\"3\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{XKCD}/>",
            s.color
        ));
        svg.push(format!(
            "  <circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"5\" fill=\"{}\" stroke=\"{}\" stroke-width=\"2\"{XKCD}/>",
            last_pt.0, last_pt.1, s.color, pal.dot_stroke
        ));

        // End label (star count), well above the line's flat approach to the
        // dot and anchored so it never clips at the right edge
        let stars = fmt_short(s.cum.last().unwrap().1);
        let label_x = (last_pt.0 - 9.0).max(pad_l + 30.0);
        let label_y = (last_pt.1 - 18.0).max(14.0);
        svg.push(
            "  ".to_string()
                + &TextEl::new(
                    format!("{label_x:.1}"),
                    format!("{label_y:.1}"),
                    &stars,
                    s.color,
                )
                .size(14)
                .anchor("end")
                .render(),
        );
    }

    svg.extend([
        format!(
            "  <a href=\"{}\" target=\"_blank\" rel=\"noopener\"><text x=\"{:.0}\" y=\"{:.0}\" font-size=\"13\" font-family=\"{FONT_FAMILY}\" fill=\"{}\" text-anchor=\"end\">Made with Flux159/rust-star-history</text></a>",
            env!("CARGO_PKG_REPOSITORY"),
            w - pad_r - 2.0,
            h - 16.0,
            if opts.dark { "#fff" } else { "#000" }
        ),
        "</svg>".into(),
    ]);

    svg.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Day {
        Day::parse(s).unwrap()
    }

    fn sample_opts() -> Options<'static> {
        Options {
            title: "Star History",
            width: 800,
            height: 533,
            dark: false,
        }
    }

    fn sample_cum() -> Vec<(Day, u64)> {
        vec![
            (d("2024-12-08"), 3),
            (d("2025-01-15"), 40),
            (d("2025-06-01"), 120),
            (d("2026-07-01"), 480),
        ]
    }

    #[test]
    fn fmt_count_matches_reference() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1000), "1K");
        assert_eq!(fmt_count(2500), "2.5K");
        assert_eq!(fmt_count(1250), "1.25K");
    }

    #[test]
    fn fmt_short_rounds_to_one_decimal() {
        assert_eq!(fmt_short(480), "480");
        assert_eq!(fmt_short(1449), "1.4K");
        assert_eq!(fmt_short(1458), "1.5K");
        assert_eq!(fmt_short(2000), "2K");
        assert_eq!(fmt_short(38912), "38.9K");
    }

    #[test]
    fn tick_step_scales_with_star_count() {
        assert_eq!(tick_step(10), 25);
        assert_eq!(tick_step(180), 25);
        assert_eq!(tick_step(201), 50);
        assert_eq!(tick_step(1000), 250);
        assert_eq!(tick_step(38000), 5000);
    }

    #[test]
    fn smooth_path_matches_reference_shape() {
        let path = smooth_path(&[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)]);
        assert!(path.starts_with("M 0.0,0.0 C"));
        assert_eq!(path.matches(" C ").count(), 2);
        assert_eq!(smooth_path(&[(1.0, 1.0)]), "");
    }

    #[test]
    fn downsample_keeps_endpoints_and_caps_length() {
        let pts: Vec<(f64, f64)> = (0..500).map(|i| (i as f64, i as f64)).collect();
        let ds = downsample(pts.clone(), 64);
        assert_eq!(ds.len(), 64);
        assert_eq!(ds[0], pts[0]);
        assert_eq!(*ds.last().unwrap(), *pts.last().unwrap());
        assert_eq!(downsample(pts[..10].to_vec(), 64).len(), 10);
    }

    #[test]
    fn generates_wellformed_svg_with_expected_elements() {
        let cum = sample_cum();
        let series = [Series {
            repo: "owner/repo",
            color: "#dd4528",
            cum: &cum,
        }];
        let svg = generate_svg(&series, &sample_opts());
        for needle in [
            "<svg xmlns=\"http://www.w3.org/2000/svg\"",
            "font-family:\"Handlee\"",
            "data:font/woff2",
            "id=\"xkcdify\"",
            "feTurbulence",
            "feDisplacementMap",
            "owner/repo",
            "GitHub Stars",
            "Dec 2024",
            ">480</text>",
            "url(#g)",
            "</svg>",
        ] {
            assert!(svg.contains(needle), "missing {needle}");
        }
        assert_eq!(svg.matches("<text").count(), svg.matches("</text>").count());
        assert!(!svg.contains("\"\""));
    }

    #[test]
    fn multi_series_draws_both_lines_without_area_fill() {
        let cum_a = sample_cum();
        let cum_b = vec![(d("2025-05-01"), 2), (d("2026-06-01"), 90)];
        let series = [
            Series {
                repo: "owner/repo-a",
                color: "#dd4528",
                cum: &cum_a,
            },
            Series {
                repo: "owner/repo-b",
                color: "#28a9dd",
                cum: &cum_b,
            },
        ];
        let svg = generate_svg(&series, &sample_opts());
        assert!(svg.contains("owner/repo-a"));
        assert!(svg.contains("owner/repo-b"));
        assert!(svg.contains("<a href=\"https://github.com/owner/repo-a\""));
        assert!(svg.contains("<a href=\"https://github.com/owner/repo-b\""));
        assert!(svg.contains("#28a9dd"));
        assert!(
            !svg.contains("url(#g)"),
            "comparison charts should not fill areas"
        );
        assert_eq!(svg.matches("<circle").count(), 2);
        assert!(svg.contains(">480</text>"));
        assert!(svg.contains(">90</text>"));
    }

    #[test]
    fn dark_theme_uses_github_dark_palette() {
        let cum = vec![(d("2025-01-01"), 1), (d("2025-02-01"), 10)];
        let series = [Series {
            repo: "owner/repo",
            color: "#dd4528",
            cum: &cum,
        }];
        let mut opts = sample_opts();
        opts.dark = true;
        let svg = generate_svg(&series, &opts);
        assert!(svg.contains("#0d1117"));
        assert!(svg.contains("#e6edf3"));
    }

    #[test]
    fn escapes_xml_in_repo_and_title() {
        let cum = vec![(d("2025-01-01"), 1), (d("2025-02-01"), 10)];
        let series = [Series {
            repo: "owner/repo",
            color: "#dd4528",
            cum: &cum,
        }];
        let mut opts = sample_opts();
        opts.title = "Stars <& Beyond>";
        let svg = generate_svg(&series, &opts);
        assert!(svg.contains("Stars &lt;&amp; Beyond&gt;"));
    }
}
