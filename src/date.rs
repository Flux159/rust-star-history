//! Minimal calendar helpers so we don't need a date/time crate.

pub const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A calendar day parsed from the leading `YYYY-MM-DD` of an ISO-8601 timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Day {
    pub fn parse(s: &str) -> Option<Day> {
        let b = s.as_bytes();
        if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let year: i32 = s.get(0..4)?.parse().ok()?;
        let month: u32 = s.get(5..7)?.parse().ok()?;
        let day: u32 = s.get(8..10)?.parse().ok()?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(Day { year, month, day })
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil` algorithm).
    pub fn to_epoch_days(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let d = i64::from(self.day);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// Inverse of `to_epoch_days` (Hinnant's `civil_from_days`).
    pub fn from_epoch_days(days: i64) -> Day {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        Day {
            year: (if m <= 2 { y + 1 } else { y }) as i32,
            month: m,
            day: d,
        }
    }

    /// Current UTC date from the system clock.
    pub fn today() -> Day {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Day::from_epoch_days((secs / 86_400) as i64)
    }

    /// "2026-07-15"
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// "Jul 2026"
    pub fn month_year(self) -> String {
        format!("{} {}", MONTHS[(self.month - 1) as usize], self.year)
    }

    /// "Jul 15"
    pub fn month_day(self) -> String {
        format!("{} {:02}", MONTHS[(self.month - 1) as usize], self.day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_timestamps() {
        let d = Day::parse("2024-12-08T18:23:11Z").unwrap();
        assert_eq!((d.year, d.month, d.day), (2024, 12, 8));
        assert!(Day::parse("garbage").is_none());
    }

    #[test]
    fn epoch_days_match_known_values() {
        assert_eq!(Day::parse("1970-01-01").unwrap().to_epoch_days(), 0);
        assert_eq!(Day::parse("2000-03-01").unwrap().to_epoch_days(), 11017);
        assert_eq!(Day::parse("2026-07-15").unwrap().to_epoch_days(), 20649);
    }

    #[test]
    fn from_epoch_days_inverts_to_epoch_days() {
        for days in (-40_000..40_000).step_by(97) {
            assert_eq!(Day::from_epoch_days(days).to_epoch_days(), days);
        }
        let d = Day::from_epoch_days(20_649);
        assert_eq!((d.year, d.month, d.day), (2026, 7, 15));
        assert_eq!(d.iso(), "2026-07-15");
    }

    #[test]
    fn formats_labels() {
        let d = Day::parse("2025-03-05").unwrap();
        assert_eq!(d.month_year(), "Mar 2025");
        assert_eq!(d.month_day(), "Mar 05");
    }
}
