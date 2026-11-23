//! Which account to spend next: what expires first, where a session can run.

use crate::maths::Metrics;

pub struct Candidate<'a> {
    pub name: &'a str,
    pub metrics: Option<Metrics>,
    pub is_unit: bool,
}

/// What decided the pick.
#[derive(Debug, PartialEq)]
pub enum Rule {
    MostUrgent,
    NoneEligible,
    NoData,
}

#[derive(Debug, PartialEq)]
pub struct Pick {
    pub name: String,
    pub rule: Rule,
}

pub fn pick(cands: &[Candidate], min_session: f64) -> Option<Pick> {
    let rated: Vec<(&str, &Metrics)> = cands
        .iter()
        .filter_map(|c| c.metrics.as_ref().map(|m| (c.name, m)))
        .collect();
    let pick = |name: &str, rule| {
        Some(Pick {
            name: name.to_string(),
            rule,
        })
    };

    if rated.is_empty() {
        let c = cands.iter().find(|c| c.is_unit).or(cands.first())?;
        return pick(c.name, Rule::NoData);
    }

    // Strict comparisons keep the first candidate on a full tie.
    let urgent = rated
        .iter()
        .filter(|(_, m)| m.now >= min_session)
        .reduce(|best, c| {
            let (b, m) = (best.1, c.1);
            if m.day > b.day || (m.day == b.day && m.now > b.now) {
                c
            } else {
                best
            }
        });
    if let Some((name, _)) = urgent {
        return pick(name, Rule::MostUrgent);
    }

    let roomiest = rated
        .iter()
        .reduce(|best, c| if c.1.now > best.1.now { c } else { best })?;
    pick(roomiest.0, Rule::NoneEligible)
}

/// Why the account in use should give way to the picked one.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Switch {
    /// The current one has no room for a session; the picked one has.
    Needed,
    /// Both have room, but the picked one has much more to spend per day.
    Better,
}

/// How many times the current day rate the picked one needs to be worth a
/// switch, which costs re-reading the session on the new account.
const BETTER_BY: f64 = 1.5;

pub fn switch(current: &Metrics, picked: &Metrics, min_session: f64) -> Option<Switch> {
    if picked.now < min_session {
        None
    } else if current.now < min_session {
        Some(Switch::Needed)
    } else if picked.day > current.day && picked.day >= BETTER_BY * current.day {
        Some(Switch::Better)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(now: f64, day: f64) -> Option<Metrics> {
        Some(Metrics {
            now,
            week: 100.0,
            capacity: 100.0,
            day,
            days_to_reset: 3.0,
            binding_reset: None,
        })
    }

    fn cand(name: &str, metrics: Option<Metrics>, is_unit: bool) -> Candidate<'_> {
        Candidate {
            name,
            metrics,
            is_unit,
        }
    }

    fn picked(cands: &[Candidate], min: f64) -> (String, Rule) {
        let p = pick(cands, min).unwrap();
        (p.name, p.rule)
    }

    #[test]
    fn urgent_beats_roomy() {
        let c = [
            cand("a", m(10.0, 30.0), false),
            cand("b", m(100.0, 14.0), false),
        ];
        assert_eq!(picked(&c, 3.0), ("a".into(), Rule::MostUrgent));
    }

    #[test]
    fn ineligible_skipped() {
        let c = [
            cand("a", m(2.0, 50.0), false),
            cand("b", m(20.0, 10.0), false),
        ];
        assert_eq!(picked(&c, 3.0), ("b".into(), Rule::MostUrgent));
    }

    #[test]
    fn tie_goes_to_more_now() {
        let c = [
            cand("a", m(5.0, 20.0), false),
            cand("b", m(9.0, 20.0), false),
        ];
        assert_eq!(picked(&c, 3.0).0, "b");
    }

    #[test]
    fn none_eligible_takes_most_now() {
        let c = [
            cand("a", m(1.0, 50.0), false),
            cand("b", m(2.0, 10.0), false),
        ];
        assert_eq!(picked(&c, 3.0), ("b".into(), Rule::NoneEligible));
    }

    #[test]
    fn none_eligible_tie_keeps_first() {
        let c = [cand("a", m(2.0, 1.0), false), cand("b", m(2.0, 9.0), false)];
        assert_eq!(picked(&c, 3.0), ("a".into(), Rule::NoneEligible));
    }

    #[test]
    fn no_data_takes_unit() {
        let c = [cand("a", None, false), cand("u", None, true)];
        assert_eq!(picked(&c, 3.0), ("u".into(), Rule::NoData));
    }

    #[test]
    fn no_data_without_unit_takes_first() {
        let c = [cand("a", None, false), cand("b", None, false)];
        assert_eq!(picked(&c, 3.0), ("a".into(), Rule::NoData));
    }

    #[test]
    fn some_data_but_none_eligible_ignores_missing() {
        let c = [cand("u", None, true), cand("b", m(1.0, 5.0), false)];
        assert_eq!(picked(&c, 3.0), ("b".into(), Rule::NoneEligible));
    }

    #[test]
    fn missing_data_is_not_eligible() {
        let c = [cand("u", None, true), cand("b", m(5.0, 5.0), false)];
        assert_eq!(picked(&c, 3.0), ("b".into(), Rule::MostUrgent));
    }

    #[test]
    fn min_session_boundary_is_eligible() {
        let c = [
            cand("a", m(3.0, 50.0), false),
            cand("b", m(20.0, 10.0), false),
        ];
        assert_eq!(picked(&c, 3.0), ("a".into(), Rule::MostUrgent));
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(pick(&[], 3.0), None);
    }

    fn sw(cur: (f64, f64), to: (f64, f64)) -> Option<Switch> {
        switch(&m(cur.0, cur.1).unwrap(), &m(to.0, to.1).unwrap(), 3.0)
    }

    #[test]
    fn switch_needed_when_current_has_no_room() {
        assert_eq!(sw((2.0, 50.0), (20.0, 10.0)), Some(Switch::Needed));
    }

    #[test]
    fn no_switch_when_picked_has_no_room_either() {
        assert_eq!(sw((1.0, 5.0), (2.0, 10.0)), None);
    }

    #[test]
    fn switch_better_from_one_and_a_half_times_the_day() {
        assert_eq!(sw((20.0, 10.0), (20.0, 15.0)), Some(Switch::Better));
        assert_eq!(sw((20.0, 10.0), (20.0, 14.0)), None);
    }

    #[test]
    fn no_switch_from_an_empty_week_to_another() {
        assert_eq!(sw((20.0, 0.0), (20.0, 0.0)), None);
    }
}
