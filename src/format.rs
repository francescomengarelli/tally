//! What tally prints: the startup notice, the status line and the summary table.

use jiff::{Timestamp, tz::TimeZone};

use crate::api::Window;
use crate::commands::{Row, Snapshot};
use crate::maths;
use crate::usage::Status;

const DIM: &str = "\x1b[90m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

fn paint(on: bool, code: &str, text: &str) -> String {
    if on {
        format!("{code}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn ready(s: &Snapshot) -> bool {
    s.rows
        .iter()
        .any(|r| matches!(r.status, Status::Ready { .. }))
}

/// One line for shell startup; `None` when there is nothing to show.
pub fn notice(s: &Snapshot, color: bool) -> Option<String> {
    if !ready(s) {
        return None;
    }
    let t = &s.totals;
    let out = s
        .rows
        .iter()
        .filter(|r| matches!(r.status, Status::NotLoggedIn | Status::LoginExpired))
        .count();
    let mut head = format!(
        " claude: now {:.0}% · week {:.0}% left (of {:.0}%) · {:.0}%/day",
        t.now, t.week, t.capacity, t.day
    );
    if out > 0 {
        head.push_str(&format!(" · {out} to log in"));
    }
    Some(match &s.pick {
        Some(p) => format!(
            "{}{}",
            paint(color, DIM, &format!("{head}  →  ")),
            paint(color, BOLD, &format!("cl ({})", p.name))
        ),
        None => paint(color, DIM, &head),
    })
}

/// One line for Claude Code's status line, from the totals.
pub fn statusline(account: &str, s: &Snapshot) -> String {
    let t = &s.totals;
    format!(
        "{account} · now {:.0}% · week {:.0}% · {:.0}%/day",
        t.now, t.week, t.day
    )
}

/// Percent used in a window; one that has already reset counts as unused.
fn used(w: Option<&Window>, at: Timestamp) -> String {
    match w {
        None => "-".into(),
        Some(w) if w.resets_at.is_some_and(|r| r <= at) => "0%".into(),
        Some(w) => format!("{:.0}%", w.utilization),
    }
}

fn local(t: Option<Timestamp>) -> String {
    t.map_or("-".into(), |t| {
        t.to_zoned(TimeZone::system())
            .strftime("%a %H:%M")
            .to_string()
    })
}

fn plan(row: &Row) -> String {
    format!("{}x{}", row.plan, if row.plan_known { "" } else { "?" })
}

/// One row per account and a totals row, columns aligned.
pub fn table(s: &Snapshot, color: bool) -> String {
    let at = Timestamp::now();
    let header = [
        "account",
        "plan",
        "5h used",
        "week used",
        "week resets",
        "now",
        "week left",
        "/day",
    ];
    // a row is its cells, plus a message that takes the place of the numbers
    let mut rows: Vec<(Vec<String>, Option<String>)> = Vec::new();
    for r in &s.rows {
        let name = &r.account.name;
        match (&r.status, &r.metrics) {
            (Status::Ready { usage, stale, .. }, Some(m)) => {
                let label = if *stale {
                    format!("{name} (stale)")
                } else {
                    name.clone()
                };
                let reset = usage
                    .seven_day
                    .as_ref()
                    .and_then(|w| maths::next_week_reset(w, at));
                rows.push((
                    vec![
                        label,
                        plan(r),
                        used(usage.five_hour.as_ref(), at),
                        used(usage.seven_day.as_ref(), at),
                        local(reset),
                        format!("{:.0}%", m.now),
                        format!("{:.0}%", m.week),
                        format!("{:.0}%", m.day),
                    ],
                    None,
                ));
            }
            (status, _) => {
                let msg = match status {
                    Status::NotLoggedIn => format!("not logged in → tally login {name}"),
                    Status::LoginExpired => format!("login expired → tally login {name}"),
                    _ => "unavailable".into(),
                };
                rows.push((vec![name.clone(), plan(r)], Some(msg)));
            }
        }
    }
    let t = &s.totals;
    rows.push((
        vec![
            "total".into(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            format!("{:.0}%", t.now),
            format!("{:.0}% of {:.0}%", t.week, t.capacity),
            format!("{:.0}%", t.day),
        ],
        None,
    ));

    let widths: Vec<usize> = (0..header.len())
        .map(|i| {
            rows.iter()
                .filter(|(_, m)| m.is_none() || i < 2)
                .filter_map(|(c, _)| c.get(i).map(|x| x.chars().count()))
                .chain([header[i].chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[String], msg: Option<&str>| {
        let mut parts: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect();
        if let Some(m) = msg {
            parts.push(paint(color, DIM, m));
        }
        parts.join("  ").trim_end().to_string()
    };
    let head: Vec<String> = header.iter().map(|h| h.to_string()).collect();
    let mut out = vec![paint(color, DIM, &line(&head, None))];
    out.extend(rows.iter().map(|(c, m)| line(c, m.as_deref())));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Usage;
    use crate::config::Account;
    use crate::pick::{Pick, Rule};

    const DAY: f64 = 86400.0;

    fn win(used: f64, secs: i64, at: Timestamp) -> Window {
        Window {
            utilization: used,
            resets_at: Some(Timestamp::from_second(at.as_second() + secs).unwrap()),
        }
    }

    fn account(name: &str) -> Account {
        Account {
            name: name.into(),
            dir: format!("/x/{name}").into(),
            plan: None,
            is_main: false,
        }
    }

    fn ready_row(name: &str, plan: f64, weight: f64, usage: Usage, stale: bool) -> Row {
        let at = Timestamp::now();
        Row {
            account: account(name),
            plan,
            plan_known: true,
            metrics: Some(maths::metrics(&usage, weight, 0.18, at)),
            status: Status::Ready {
                usage,
                fetched_at: at,
                stale,
            },
        }
    }

    fn gone(name: &str, status: Status) -> Row {
        Row {
            account: account(name),
            status,
            plan: 1.0,
            plan_known: false,
            metrics: None,
        }
    }

    fn snap(rows: Vec<Row>, pick: Option<&str>) -> Snapshot {
        let ms: Vec<maths::Metrics> = rows.iter().filter_map(|r| r.metrics.clone()).collect();
        Snapshot {
            totals: maths::totals(&ms),
            rows,
            pick: pick.map(|n| Pick {
                name: n.into(),
                rule: Rule::MostUrgent,
            }),
        }
    }

    fn two() -> Snapshot {
        let at = Timestamp::now();
        let team = Usage {
            five_hour: Some(win(6.0, 3600, at)),
            seven_day: Some(win(94.0, (1.9 * DAY) as i64, at)),
        };
        let ultra = Usage {
            five_hour: None,
            seven_day: Some(win(0.0, 7 * 86400, at)),
        };
        snap(
            vec![
                ready_row("acme", 6.25, 1.0, team, false),
                ready_row("personal", 10.0, 1.6, ultra, false),
            ],
            Some("personal"),
        )
    }

    #[test]
    fn notice_line_exact() {
        assert_eq!(
            notice(&two(), false).unwrap(),
            " claude: now 166% · week 166% left (of 260%) · 26%/day  →  cl (personal)"
        );
    }

    #[test]
    fn notice_colours_the_prefix_dim_and_the_command_bold() {
        let n = notice(&two(), true).unwrap();
        assert!(n.starts_with("\x1b[90m claude:"));
        assert!(n.ends_with("\x1b[1mcl (personal)\x1b[0m"));
    }

    #[test]
    fn notice_counts_logins() {
        let mut s = two();
        s.rows.push(gone("work", Status::NotLoggedIn));
        assert!(
            notice(&s, false)
                .unwrap()
                .contains("26%/day · 1 to log in  →")
        );
    }

    #[test]
    fn notice_none_without_data() {
        let s = snap(
            vec![
                gone("a", Status::NotLoggedIn),
                gone("b", Status::Unavailable),
            ],
            None,
        );
        assert_eq!(notice(&s, false), None);
    }

    #[test]
    fn statusline_exact() {
        assert_eq!(
            statusline("personal", &two()),
            "personal · now 166% · week 166% · 26%/day"
        );
    }

    #[test]
    fn table_has_total_and_states() {
        let mut s = two();
        s.rows[0] = {
            let at = Timestamp::now();
            let u = Usage {
                five_hour: Some(win(6.0, 3600, at)),
                seven_day: Some(win(94.0, (1.9 * DAY) as i64, at)),
            };
            ready_row("acme", 6.25, 1.0, u, true)
        };
        s.rows.push(gone("a", Status::NotLoggedIn));
        s.rows.push(gone("b", Status::LoginExpired));
        s.rows.push(gone("c", Status::Unavailable));
        let ms: Vec<maths::Metrics> = s.rows.iter().filter_map(|r| r.metrics.clone()).collect();
        s.totals = maths::totals(&ms);
        let t = table(&s, false);
        let lines: Vec<&str> = t.lines().collect();
        assert!(lines[0].starts_with("account"));
        assert!(lines[0].contains("week resets"));
        assert!(lines[1].starts_with("acme (stale)"));
        assert!(lines[1].contains("6.25x"));
        assert!(lines[2].contains("10x"));
        assert!(t.contains("not logged in → tally login a"));
        assert!(t.contains("login expired → tally login b"));
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("c ") && l.ends_with("unavailable"))
        );
        let last = lines.last().unwrap();
        assert!(last.starts_with("total"));
        assert!(last.contains("166% of 260%"));
    }

    #[test]
    fn guessed_plan_gets_a_question_mark() {
        let r = gone("a", Status::Unavailable);
        assert_eq!(plan(&r), "1x?");
    }
}
