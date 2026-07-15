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
    fn formats_labels() {
        let d = Day::parse("2025-03-05").unwrap();
        assert_eq!(d.month_year(), "Mar 2025");
        assert_eq!(d.month_day(), "Mar 05");
    }
}
