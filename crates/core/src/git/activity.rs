//! The Git pane's activity strip: a repository's commits over the last
//! year as a grid of days, a column per week (Sunday first), each day shaded
//! by how many commits it has, like GitHub's contribution graph.

use crate::usage::time::{DAY, civil_from_days};

/// Weeks the grid spans: a year, plus the week in progress.
pub const WEEKS: usize = 53;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The `git log` arguments that print what [`parse`] reads: every ref's
/// commits but merges, author time and email.
pub const LOG_ARGS: &[&str] = &["log", "--all", "--no-merges", "--format=%at%x09%ae"];

/// One commit: author time (Unix seconds) and email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub time: i64,
    pub email: String,
}

/// [`LOG_ARGS`]' output, one line per commit; malformed lines are skipped.
pub fn parse(raw: &str) -> Vec<Stamp> {
    raw.lines()
        .filter_map(|line| {
            let (time, email) = line.split_once('\t')?;
            Some(Stamp {
                time: time.trim().parse().ok()?,
                email: email.trim().to_lowercase(),
            })
        })
        .collect()
}

/// One cell of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    /// Local days since 1970-01-01.
    pub day: i64,
    pub commits: u32,
    /// 0 (none) to 4 (the busiest days), for the shade.
    pub level: u8,
    /// After today: the cell isn't drawn.
    pub future: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    /// Oldest first; each week Sunday to Saturday. The last holds today.
    pub weeks: Vec<[Day; 7]>,
}

impl Activity {
    /// The last `weeks` weeks (all of them when fewer are kept), for a pane
    /// too narrow for the year.
    pub fn last(&self, weeks: usize) -> &[[Day; 7]] {
        &self.weeks[self.weeks.len().saturating_sub(weeks)..]
    }
}

/// Commits in `weeks`.
pub fn total(weeks: &[[Day; 7]]) -> u32 {
    weeks.iter().flatten().map(|d| d.commits).sum()
}

/// The grid ending with today's week. `offset` is the local UTC offset
/// (seconds), `now` Unix seconds. With `only` (an email), only that
/// author's commits count.
pub fn activity(stamps: &[Stamp], only: Option<&str>, offset: i64, now: i64) -> Activity {
    let today = (now + offset).div_euclid(DAY);
    let start = today - weekday(today) as i64 - (WEEKS as i64 - 1) * 7;
    let mut counts = vec![0u32; WEEKS * 7];
    for stamp in stamps {
        if only.is_some_and(|email| !stamp.email.eq_ignore_ascii_case(email)) {
            continue;
        }
        let day = (stamp.time + offset).div_euclid(DAY);
        if (start..=today).contains(&day) {
            counts[(day - start) as usize] += 1;
        }
    }
    let busiest = counts.iter().copied().max().unwrap_or(0);
    let weeks = (0..WEEKS)
        .map(|w| {
            std::array::from_fn(|d| {
                let index = w * 7 + d;
                let day = start + index as i64;
                Day {
                    day,
                    commits: counts[index],
                    level: level(counts[index], busiest),
                    future: day > today,
                }
            })
        })
        .collect();
    Activity { weeks }
}

/// A day's shade: 0 without commits, else 1–4 by its share of the busiest
/// day, so one busy day doesn't wash the rest out to the first shade.
pub fn level(commits: u32, busiest: u32) -> u8 {
    if commits == 0 || busiest == 0 {
        return 0;
    }
    (commits * 4).div_ceil(busiest).clamp(1, 4) as u8
}

/// 0 for Sunday.
pub fn weekday(day: i64) -> usize {
    // 1970-01-01 was a Thursday.
    (day + 4).rem_euclid(7) as usize
}

/// Month names over the columns of `weeks`: `(column, "Jan")` where a month
/// begins, skipping a label that would crowd the one before it.
pub fn month_labels(weeks: &[[Day; 7]]) -> Vec<(usize, &'static str)> {
    let mut labels: Vec<(usize, &'static str)> = Vec::new();
    for (column, week) in weeks.iter().enumerate() {
        let month = |day: i64| civil_from_days(day).1;
        let first = week[0].day;
        let starts = column == 0 || month(first) != month(weeks[column - 1][0].day);
        if !starts {
            continue;
        }
        labels.push((column, MONTHS[month(first) as usize - 1]));
    }
    // A label squeezed at the very start by the next one goes.
    if labels.len() > 1 && labels[1].0 - labels[0].0 < 3 {
        labels.remove(0);
    }
    labels
}

/// "Thu, Nov 20, 2026", for a cell's tooltip.
pub fn day_label(day: i64) -> String {
    let (year, month, date) = civil_from_days(day);
    format!(
        "{}, {} {date}, {year}",
        WEEKDAYS[weekday(day)],
        MONTHS[month as usize - 1]
    )
}

/// "3 commits on Thu, Nov 20, 2026".
pub fn day_summary(day: &Day) -> String {
    let count = match day.commits {
        0 => "No commits".to_string(),
        1 => "1 commit".to_string(),
        n => format!("{n} commits"),
    };
    format!("{count} on {}", day_label(day.day))
}

/// "412 commits in the last year", or "in the last 20 weeks" for fewer.
pub fn heading(commits: u32, weeks: usize) -> String {
    let count = match commits {
        1 => "1 commit".to_string(),
        n => format!("{n} commits"),
    };
    if weeks >= WEEKS {
        format!("{count} in the last year")
    } else {
        format!("{count} in the last {weeks} weeks")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::time::days_from_civil;

    fn at(y: i64, m: i64, d: i64, hour: i64) -> i64 {
        days_from_civil(y, m, d) * DAY + hour * 3600
    }

    fn stamp(time: i64, email: &str) -> Stamp {
        Stamp {
            time,
            email: email.into(),
        }
    }

    #[test]
    fn the_grid_ends_with_todays_week_and_counts_local_days() {
        // Friday 2026-11-20, 10:00 UTC; the user is at UTC−6.
        let now = at(2026, 11, 20, 10);
        let offset = -6 * 3600;
        let stamps = [
            // 02:00 UTC on the 20th is still the 19th at UTC−6.
            stamp(at(2026, 11, 20, 2), "a@x"),
            stamp(at(2026, 11, 20, 9), "a@x"),
            stamp(at(2026, 11, 20, 9), "b@x"),
            // Older than the grid.
            stamp(at(2025, 1, 1, 12), "a@x"),
        ];
        let grid = activity(&stamps, None, offset, now);
        assert_eq!(grid.weeks.len(), WEEKS);
        let last = grid.weeks.last().unwrap();
        assert_eq!(weekday(last[0].day), 0, "weeks start on Sunday");
        let today = days_from_civil(2026, 11, 20);
        let cell = |day: i64| grid.weeks.iter().flatten().find(|d| d.day == day).unwrap();
        assert_eq!(cell(today).commits, 2);
        assert_eq!(cell(today - 1).commits, 1);
        assert!(cell(today + 1).future && !cell(today).future);
        assert_eq!(total(&grid.weeks), 3);

        let mine = activity(&stamps, Some("B@X"), offset, now);
        assert_eq!(total(&mine.weeks), 1);
    }

    #[test]
    fn shades_follow_the_busiest_day() {
        assert_eq!(level(0, 9), 0);
        assert_eq!(level(1, 9), 1);
        assert_eq!(level(5, 9), 3);
        assert_eq!(level(9, 9), 4);
        assert_eq!(level(1, 1), 4);
    }

    #[test]
    fn months_are_named_where_they_begin() {
        let grid = activity(&[], None, 0, at(2026, 11, 20, 12));
        let labels = month_labels(&grid.weeks);
        let names: Vec<&str> = labels.iter().map(|(_, m)| *m).collect();
        // A year back from late November, as GitHub draws it: Nov … Nov.
        assert_eq!(names.first(), Some(&"Nov"), "{names:?}");
        assert_eq!(names.len(), 13, "{names:?}");
        assert_eq!(names.last(), Some(&"Nov"));
        assert!(labels.windows(2).all(|w| w[1].0 - w[0].0 >= 3));
    }

    #[test]
    fn log_lines_parse_and_words_read() {
        let stamps = parse("1763600000\tA@X.io\nbad\n1763600001\tb@x\n");
        assert_eq!(stamps.len(), 2);
        assert_eq!(stamps[0].email, "a@x.io");
        assert_eq!(heading(1, WEEKS), "1 commit in the last year");
        assert_eq!(heading(7, 20), "7 commits in the last 20 weeks");
        let day = Day {
            day: days_from_civil(2026, 11, 20),
            commits: 0,
            level: 0,
            future: false,
        };
        assert_eq!(day_summary(&day), "No commits on Fri, Nov 20, 2026");
    }
}
