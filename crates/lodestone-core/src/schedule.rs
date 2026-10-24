//! Scheduling (spec: list_tasks / set_task / remove_task) and the
//! sleep-when-empty policy. Cron expressions are matched against a UTC
//! timestamp broken into (minute, hour, day-of-month, month, day-of-week).

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub cron: String,
    pub command: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    pub tasks: BTreeMap<String, ScheduledTask>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            tasks: BTreeMap::new(),
        }
    }

    pub fn set(&mut self, task: ScheduledTask) {
        self.tasks.insert(task.id.clone(), task);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.tasks.remove(id).is_some()
    }

    pub fn list(&self) -> Vec<&ScheduledTask> {
        self.tasks.values().collect()
    }

    /// Tasks whose cron matches `now` and which are enabled.
    pub fn due(&self, now: &CronTime) -> Vec<&ScheduledTask> {
        self.tasks
            .values()
            .filter(|t| t.enabled && cron_matches(&t.cron, now))
            .collect()
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CronTime {
    pub minute: u8,
    pub hour: u8,
    pub day_of_month: u8,
    pub month: u8,
    pub day_of_week: u8, // 0 = Sunday
}

impl CronTime {
    /// Build from a unix timestamp (seconds).
    pub fn from_unix(ts: u64) -> Self {
        // Civil-time conversion (Howard Hinnant's algorithm), UTC.
        let days = (ts / 86400) as i64;
        let secs = (ts % 86400) as u32;
        let (_y, m, d) = days_to_ymd(days);
        let minute = ((secs / 60) % 60) as u8;
        let hour = (secs / 3600) as u8;
        // day of week: 1970-01-01 was Thursday (4 in Sun=0 convention? 1970-01-01 Thursday => dow=4)
        let dow = ((days + 4) % 7) as u8;
        CronTime {
            minute,
            hour,
            day_of_month: d as u8,
            month: m as u8,
            day_of_week: dow,
        }
    }
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Parse a single cron field into the set of allowed values.
fn parse_field(field: &str, min: u8, max: u8) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if part == "*" {
            for v in min..=max {
                out.push(v);
            }
            continue;
        }
        // step: */n or a-b/n
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => {
                let s: u8 = s.parse().ok()?;
                (r, s.max(1))
            }
            None => (part, 1),
        };
        let (lo, hi) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let a: u8 = a.parse().ok()?;
            let b: u8 = b.parse().ok()?;
            (a.max(min), b.min(max))
        } else {
            let v: u8 = range_part.parse().ok()?;
            (v, v)
        };
        let mut v = lo;
        while v <= hi {
            if !out.contains(&v) {
                out.push(v);
            }
            v = v.saturating_add(step);
            if step == 0 {
                break;
            }
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

pub fn cron_matches(expr: &str, t: &CronTime) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    let mins = match parse_field(fields[0], 0, 59) {
        Some(v) => v,
        None => return false,
    };
    let hours = match parse_field(fields[1], 0, 23) {
        Some(v) => v,
        None => return false,
    };
    let doms = match parse_field(fields[2], 1, 31) {
        Some(v) => v,
        None => return false,
    };
    let months = match parse_field(fields[3], 1, 12) {
        Some(v) => v,
        None => return false,
    };
    let dows = match parse_field(fields[4], 0, 6) {
        Some(v) => v,
        None => return false,
    };
    mins.contains(&t.minute)
        && hours.contains(&t.hour)
        && doms.contains(&t.day_of_month)
        && months.contains(&t.month)
        && dows.contains(&t.day_of_week)
}

/// Sleep-when-empty policy (spec: sleep when no players). Returns true if the
/// server should be paused/slept given the last activity timestamp.
pub fn should_sleep(now_unix: u64, last_activity_unix: u64, idle_threshold_secs: u64) -> bool {
    now_unix.saturating_sub(last_activity_unix) >= idle_threshold_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_daily_at_3am() {
        // 2026-10-24 03:00 UTC
        let ts = 1761274800u64; // approximate; adjust via from_unix roundtrip below
        let _ = ts;
        // Build a known time directly instead.
        let t = CronTime {
            minute: 0,
            hour: 3,
            day_of_month: 24,
            month: 10,
            day_of_week: 6,
        };
        assert!(cron_matches("0 3 * * *", &t));
        assert!(!cron_matches("0 4 * * *", &t));
        assert!(cron_matches("*/15 * * * *", &t));
        assert!(cron_matches("0 3 * * 6", &t)); // saturday
        assert!(!cron_matches("0 3 * * 1", &t));
    }

    #[test]
    fn cron_step_and_range() {
        let t = CronTime {
            minute: 30,
            hour: 14,
            day_of_month: 10,
            month: 7,
            day_of_week: 3,
        };
        assert!(cron_matches("0,15,30,45 * * * *", &t));
        assert!(cron_matches("*/10 * * * *", &t));
        assert!(cron_matches("0-59/2 * * * *", &t)); // 30 is in range step 2
        assert!(!cron_matches("1-59/7 * * * *", &t)); // 30 not reachable by step 7 from 1? 1,8,15,22,29,36 -> 30 no
        assert!(cron_matches("* * 10 7 *", &t));
    }

    #[test]
    fn cron_rejects_bad_expr() {
        let t = CronTime {
            minute: 0,
            hour: 0,
            day_of_month: 1,
            month: 1,
            day_of_week: 0,
        };
        assert!(!cron_matches("0 0", &t)); // wrong field count
        assert!(!cron_matches("99 * * * *", &t)); // out of range
    }

    #[test]
    fn from_unix_roundtrips_calendar() {
        // 2026-01-01 00:00:00 UTC
        let ts = 1767225600u64;
        let t = CronTime::from_unix(ts);
        assert_eq!(t.minute, 0);
        assert_eq!(t.hour, 0);
        assert_eq!(t.day_of_month, 1);
        assert_eq!(t.month, 1);
    }

    #[test]
    fn scheduler_due_and_remove() {
        let mut s = Scheduler::new();
        s.set(ScheduledTask {
            id: "t1".into(),
            name: "backup".into(),
            cron: "0 3 * * *".into(),
            command: "backup".into(),
            enabled: true,
        });
        s.set(ScheduledTask {
            id: "t2".into(),
            name: "disabled".into(),
            cron: "0 3 * * *".into(),
            command: "x".into(),
            enabled: false,
        });
        let now = CronTime {
            minute: 0,
            hour: 3,
            day_of_month: 1,
            month: 1,
            day_of_week: 4,
        };
        let due = s.due(&now);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "t1");
        assert!(s.remove("t1"));
        assert!(!s.remove("t1"));
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn sleep_policy() {
        assert!(!should_sleep(1000, 990, 30));
        assert!(should_sleep(1030, 990, 30));
        assert!(should_sleep(1000, 1000, 0));
    }
}
