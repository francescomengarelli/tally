//! What tally prints: the startup notice, the status line and the summary table.

use jiff::{Timestamp, tz::TimeZone};

use crate::api::Window;
use crate::commands::{Row, Snapshot};
use crate::maths::{self, Pace};
use crate::usage::Status;

const DIM: &str = "\x1b[90m";
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

/// Colour unless the viewer asked for none (https://no-color.org).
pub fn colour(to_terminal: bool) -> bool {
    to_terminal && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

fn paint(on: bool, code: Option<&str>, text: &str) -> String {
    match code {
        Some(code) if on => format!("{code}{text}{RESET}"),
        _ => text.to_string(),
    }
}

/// Green above steady (spend it before it resets), red below (go easy).
fn pace_colour(day: f64, steady: f64) -> Option<&'static str> {
    match maths::pace(day, steady) {
        Pace::Ahead => Some(GREEN),
        Pace::Even => None,
        Pace::Behind => Some(RED),
    }
}

/// Red when there is not enough for a session, yellow when it is tight.
fn now_colour(now: f64, min_session: f64) -> Option<&'static str> {
    if now < min_session {
        Some(RED)
    } else if now < 2.0 * min_session {
        Some(YELLOW)
    } else {
        Some(GREEN)
    }
}

/// By the share of the week that is left.
fn week_colour(week: f64, capacity: f64) -> Option<&'static str> {
    let share = if capacity > 0.0 { week / capacity } else { 0.0 };
    if share < 0.15 {
        Some(RED)
    } else if share < 0.35 {
        Some(YELLOW)
    } else {
        Some(GREEN)
    }
}

fn pct(x: f64) -> String {
    format!("{x:.0}%")
}

/// Labels dim, numbers in their colour.
struct Line {
    on: bool,
    out: String,
}

impl Line {
    fn new(on: bool) -> Self {
        Line {
            on,
            out: String::new(),
        }
    }
    fn dim(mut self, text: &str) -> Self {
        self.out.push_str(&paint(self.on, Some(DIM), text));
        self
    }
    fn val(mut self, text: &str, code: Option<&str>) -> Self {
        self.out.push_str(&paint(self.on, code, text));
        self
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
    let mut line = Line::new(color)
        .dim(" claude: now ")
        .val(&pct(t.now), now_colour(t.now, s.min_session))
        .dim(" · week ")
        .val(&pct(t.week), week_colour(t.week, t.capacity))
        .dim(&format!(" left (of {}) · ", pct(t.capacity)))
        .val(
            &format!("{}/day", pct(t.day)),
            pace_colour(t.day, t.steady()),
        )
        .dim(&format!(" ({} steady)", pct(t.steady())));
    if out > 0 {
        line = line
            .dim(" · ")
            .val(&format!("{out} to log in"), Some(YELLOW));
    }
    if let Some(p) = &s.pick {
        line = line
            .dim("  →  ")
            .val(&format!("cl ({})", p.name), Some(BOLD));
    }
    Some(line.out)
}

/// One line for Claude Code's status line, from the totals.
pub fn statusline(account: &str, s: &Snapshot, color: bool) -> String {
    let t = &s.totals;
    Line::new(color)
        .val(account, Some(BOLD))
        .dim(" · now ")
        .val(&pct(t.now), now_colour(t.now, s.min_session))
        .dim(" · week ")
        .val(&pct(t.week), week_colour(t.week, t.capacity))
        .dim(" · ")
        .val(
            &format!("{}/day", pct(t.day)),
            pace_colour(t.day, t.steady()),
        )
        .dim(&format!(" ({})", pct(t.steady())))
        .out
}

/// Percent used in a window; one that has already reset counts as unused.
fn used(w: Option<&Window>, at: Timestamp) -> String {
    match w {
        None => "-".into(),
        Some(w) if w.resets_at.is_some_and(|r| r <= at) => "0%".into(),
        Some(w) => pct(w.utilization),
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

type Cell = (String, Option<&'static str>);

fn plain(text: impl Into<String>) -> Cell {
    (text.into(), None)
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
        "steady",
    ];
    // a row is its cells, plus a message that takes the place of the numbers
    let mut rows: Vec<(Vec<Cell>, Option<Cell>)> = Vec::new();
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
                        (label, if *stale { Some(DIM) } else { None }),
                        plain(plan(r)),
                        plain(used(usage.five_hour.as_ref(), at)),
                        plain(used(usage.seven_day.as_ref(), at)),
                        plain(local(reset)),
                        (pct(m.now), now_colour(m.now, s.min_session)),
                        (pct(m.week), week_colour(m.week, m.capacity)),
                        (pct(m.day), pace_colour(m.day, m.steady())),
                        (pct(m.steady()), Some(DIM)),
                    ],
                    None,
                ));
            }
            (status, _) => {
                let msg = match status {
                    Status::NotLoggedIn => (format!("not logged in → tally login {name}"), YELLOW),
                    Status::LoginExpired => (format!("login expired → tally login {name}"), RED),
                    _ => ("unavailable".into(), DIM),
                };
                rows.push((
                    vec![plain(name.clone()), plain(plan(r))],
                    Some((msg.0, Some(msg.1))),
                ));
            }
        }
    }
    let t = &s.totals;
    rows.push((
        vec![
            (String::from("total"), Some(BOLD)),
            plain(""),
            plain(""),
            plain(""),
            plain(""),
            (pct(t.now), now_colour(t.now, s.min_session)),
            (
                format!("{} of {}", pct(t.week), pct(t.capacity)),
                week_colour(t.week, t.capacity),
            ),
            (pct(t.day), pace_colour(t.day, t.steady())),
            (pct(t.steady()), Some(DIM)),
        ],
        None,
    ));

    let widths: Vec<usize> = (0..header.len())
        .map(|i| {
            rows.iter()
                .filter(|(_, m)| m.is_none() || i < 2)
                .filter_map(|(c, _)| c.get(i).map(|x| x.0.chars().count()))
                .chain([header[i].chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[Cell], msg: Option<&Cell>| {
        let last = cells.len() + usize::from(msg.is_some());
        let mut parts: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, (text, code))| {
                // the last cell isn't padded, so lines don't end in spaces
                let text = if i + 1 == last {
                    text.clone()
                } else {
                    format!("{text:<w$}", w = widths[i])
                };
                paint(color, *code, &text)
            })
            .collect();
        if let Some((m, code)) = msg {
            parts.push(paint(color, *code, m));
        }
        parts.join("  ").trim_end().to_string()
    };
    let head: Vec<Cell> = header.iter().map(|h| (h.to_string(), Some(DIM))).collect();
    let mut out = vec![line(&head, None)];
    out.extend(rows.iter().map(|(c, m)| line(c, m.as_ref())));
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
            min_session: 3.0,
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
            " claude: now 166% · week 166% left (of 260%) · 26%/day (37% steady)  →  cl (personal)"
        );
    }

    #[test]
    fn notice_colours_the_prefix_dim_and_the_command_bold() {
        let n = notice(&two(), true).unwrap();
        assert!(n.starts_with("\x1b[90m claude: now \x1b[0m"));
        assert!(n.ends_with("\x1b[1mcl (personal)\x1b[0m"));
    }

    #[test]
    fn colours_follow_the_numbers() {
        // 26%/day against 37% steady: the week went early, so red
        let n = notice(&two(), true).unwrap();
        assert!(n.contains("\x1b[31m26%/day\x1b[0m"));
        assert!(n.contains("\x1b[32m166%\x1b[0m"));
        // the fresh week alone: 23%/day, steady 23% → no colour
        let mut s = two();
        s.rows.remove(0);
        let ms: Vec<maths::Metrics> = s.rows.iter().filter_map(|r| r.metrics.clone()).collect();
        s.totals = maths::totals(&ms);
        let n = notice(&s, true).unwrap();
        assert!(n.contains("· \x1b[0m23%/day"));
    }

    #[test]
    fn spend_soon_is_green_and_tight_is_red() {
        assert_eq!(pace_colour(72.0, 18.0), Some(GREEN));
        assert_eq!(pace_colour(18.0, 18.0), None);
        assert_eq!(pace_colour(2.0, 18.0), Some(RED));
        assert_eq!(now_colour(2.0, 3.0), Some(RED));
        assert_eq!(now_colour(5.0, 3.0), Some(YELLOW));
        assert_eq!(now_colour(6.0, 3.0), Some(GREEN));
        assert_eq!(week_colour(10.0, 100.0), Some(RED));
        assert_eq!(week_colour(30.0, 100.0), Some(YELLOW));
        assert_eq!(week_colour(35.0, 100.0), Some(GREEN));
    }

    #[test]
    fn no_colour_without_a_terminal() {
        assert!(!colour(false));
    }

    #[test]
    fn notice_counts_logins() {
        let mut s = two();
        s.rows.push(gone("work", Status::NotLoggedIn));
        assert!(
            notice(&s, false)
                .unwrap()
                .contains("(37% steady) · 1 to log in  →")
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
            statusline("personal", &two(), false),
            "personal · now 166% · week 166% · 26%/day (37%)"
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
        assert!(lines[0].ends_with("steady"));
        assert!(lines[2].ends_with("23%"));
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
