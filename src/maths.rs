//! Usage turned into % of the unit account's week.

use crate::api::{Usage, Window};
use jiff::Timestamp;

const DAY: f64 = 86400.0;

/// The weekly multiple of Pro for a `rateLimitTier`, and whether it was recognised.
pub fn tier_multiple(tier: Option<&str>) -> (f64, bool) {
    match tier {
        Some(t) if t.ends_with("max_20x") => (10.0, true),
        Some(t) if t.ends_with("max_5x") => (5.0, true),
        _ => (1.0, false),
    }
}

#[derive(Debug, Clone)]
pub struct Metrics {
    /// What can be spent before hitting a limit.
    pub now: f64,
    /// What is left of the week.
    pub week: f64,
    /// The whole week, `100 × weight`.
    pub capacity: f64,
    /// The daily rate that uses the week up before it resets.
    pub day: f64,
    pub days_to_reset: f64,
    /// When the limit that sets `now` starts over.
    pub binding_reset: Option<Timestamp>,
}

/// Percent left in a window; one that has already reset counts as unused.
fn left(w: &Window, at: Timestamp) -> f64 {
    match w.resets_at {
        Some(r) if r <= at => 100.0,
        _ => 100.0 - w.utilization,
    }
}

pub fn metrics(usage: &Usage, weight: f64, five_hour_share: f64, at: Timestamp) -> Metrics {
    let seven = usage.seven_day.as_ref();
    let week_left = seven.map_or(100.0, |w| left(w, at));
    let week = week_left * weight;
    let reset = seven.and_then(|w| w.resets_at);
    let days_to_reset = reset.map_or(7.0, |r| (r.as_second() - at.as_second()) as f64 / DAY);
    let day = week / days_to_reset.max(1.0);

    let (now, binding_reset) = match usage.five_hour.as_ref() {
        Some(w) => {
            let five = left(w, at) * five_hour_share * weight;
            if five < week {
                (five, w.resets_at)
            } else {
                (week, reset)
            }
        }
        None => (week, reset),
    };

    Metrics {
        now,
        week,
        capacity: 100.0 * weight,
        day,
        days_to_reset,
        binding_reset,
    }
}

#[derive(Debug, Clone)]
pub struct Totals {
    pub now: f64,
    pub week: f64,
    pub capacity: f64,
    pub day: f64,
}

pub fn totals(ms: &[Metrics]) -> Totals {
    Totals {
        now: ms.iter().map(|m| m.now).sum(),
        week: ms.iter().map(|m| m.week).sum(),
        capacity: ms.iter().map(|m| m.capacity).sum(),
        day: ms.iter().map(|m| m.day).sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Timestamp {
        "2026-11-23T12:00:00Z".parse().unwrap()
    }

    fn win(used: f64, secs: i64) -> Window {
        Window {
            utilization: used,
            resets_at: Some(Timestamp::from_second(at().as_second() + secs).unwrap()),
        }
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }

    fn team() -> Metrics {
        let u = Usage {
            five_hour: Some(win(6.0, 3600)),
            seven_day: Some(win(94.0, (1.9 * DAY) as i64)),
        };
        metrics(&u, 1.0, 0.18, at())
    }

    fn ultra() -> Metrics {
        let u = Usage {
            five_hour: None,
            seven_day: Some(win(0.0, 7 * 86400)),
        };
        metrics(&u, 1.6, 0.18, at())
    }

    #[test]
    fn team_account_low_week() {
        let m = team();
        close(m.week, 6.0);
        close(m.now, 6.0);
        close(m.day, 3.158);
    }

    #[test]
    fn ultra_without_five_hour() {
        let m = ultra();
        close(m.now, 160.0);
        close(m.week, 160.0);
        close(m.day, 22.857);
        close(m.capacity, 160.0);
    }

    #[test]
    fn five_hour_binds() {
        let u = Usage {
            five_hour: Some(win(90.0, 3600)),
            seven_day: Some(win(0.0, 7 * 86400)),
        };
        let m = metrics(&u, 1.0, 0.18, at());
        close(m.now, 1.8);
        assert_eq!(m.binding_reset, u.five_hour.unwrap().resets_at);
    }

    #[test]
    fn last_day_counts_whole() {
        let u = Usage {
            five_hour: None,
            seven_day: Some(win(90.0, 7200)),
        };
        close(metrics(&u, 1.0, 0.18, at()).day, 10.0);
    }

    #[test]
    fn past_reset_counts_as_unused() {
        let u = Usage {
            five_hour: None,
            seven_day: Some(win(94.0, -3600)),
        };
        close(metrics(&u, 1.0, 0.18, at()).week, 100.0);
    }

    #[test]
    fn totals_sum() {
        let t = totals(&[team(), ultra()]);
        close(t.week, 166.0);
        close(t.capacity, 260.0);
    }

    #[test]
    fn tiers() {
        assert_eq!(tier_multiple(Some("default_claude_max_5x")), (5.0, true));
        assert_eq!(tier_multiple(Some("default_claude_max_20x")), (10.0, true));
        assert_eq!(tier_multiple(None), (1.0, false));
    }

    #[test]
    fn parses_api_json() {
        let u: Usage = serde_json::from_str(
            r#"{"five_hour":{"utilization":6.0,"resets_at":"2026-11-23T15:00:00Z"},"seven_day":null}"#,
        )
        .unwrap();
        close(u.five_hour.unwrap().utilization, 6.0);
        assert!(u.seven_day.is_none());
    }
}
