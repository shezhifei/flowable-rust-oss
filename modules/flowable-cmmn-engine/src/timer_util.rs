//! ISO-8601 duration / date / repetition (R-cycle) resolution for the CMMN
//! timerEventListener chain (P117).
//!
//! The CMMN engine cannot depend on `flowable-engine`, where the BPMN timer
//! machinery lives (`time_source.rs`), so this is a lean self-contained port of
//! the semantics needed by `TimerEventListenerActivityBehaviour.java` and the
//! `DueDateBusinessCalendar` / `CycleBusinessCalendar` / `DurationHelper` chain:
//!
//! - duration `PT…` / `P…` → due = now + period/duration
//!   (`DueDateBusinessCalendar.java:31-52`)
//! - absolute date → due = that instant (`DateUtil.parseDate` fallback)
//! - `R…` repetition → first due after now, then a stored repeat expression
//!   `R<count>/<start>/<period>` drives rescheduling
//!   (`CycleBusinessCalendar.java:29-51`, `DurationHelper.java`)
//!
//! P117 scope left cron out of timer-event-listener resolution. P127 reuses the
//! same Quartz-style cron helper for automatic history-cleanup scheduling
//! (`historyCleaningTimeCycleConfig = "0 0 1 * * ?"`), matching Java
//! `CycleBusinessCalendar` for non-R values.

use chrono::{
    DateTime, Datelike, Days, Duration as ChronoDuration, Months, Timelike, TimeZone, Utc,
};

/// Parsed ISO-8601 duration split into calendar (months/days) and clock (seconds)
/// components, mirroring java.time `Period` + `Duration` (DueDateBusinessCalendar.java:34-48).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerDuration {
    pub months: i32,
    pub days: i64,
    pub seconds: i64,
}

impl TimerDuration {
    pub fn is_zero(&self) -> bool {
        self.months == 0 && self.days == 0 && self.seconds == 0
    }

    /// `date + period + duration` (Java `calculateTime.plus(period).plus(duration)`).
    /// Returns `None` when the calendar addition overflows/underflows chrono.
    pub fn add_to(&self, date: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut result = date;
        if self.months != 0 {
            result = result.checked_add_months(Months::new(self.months.unsigned_abs()))?;
        }
        if self.days != 0 {
            result = result.checked_add_days(Days::new(self.days.unsigned_abs()))?;
        }
        result.checked_add_signed(chrono::Duration::seconds(self.seconds))
    }

}

/// Parse an ISO-8601 duration (`PnYnMnDTnHnMnS`). Java `Duration.parse`/`Period.parse`
/// accept fractional seconds; we keep integer components plus a fractional-seconds
/// tail on the `S` field.
pub fn parse_iso8601_duration_parts(expression: &str) -> Option<TimerDuration> {
    let expression = expression.trim();
    if !expression.starts_with('P') {
        return None;
    }
    let (date_part, time_part) = match expression.find('T') {
        Some(index) => (&expression[1..index], Some(&expression[index + 1..])),
        None => (&expression[1..], None),
    };

    let mut months = 0i32;
    let mut days = 0i64;
    let mut seconds = 0i64;

    // Date part: nY / nM / nD
    let mut rest = date_part;
    while !rest.is_empty() {
        let (number, unit) = split_number_unit(rest)?;
        rest = &rest[number.len() + 1..];
        let value = number_token(number)?;
        match unit {
            'Y' => months = months.checked_add(value as i32 * 12)?,
            'M' => months = months.checked_add(value as i32)?,
            'D' => days = days.checked_add(value)?,
            _ => return None,
        }
    }

    // Time part: nH / nM / nS
    if let Some(time_part) = time_part {
        let mut rest = time_part;
        while !rest.is_empty() {
            let (number, unit) = split_number_unit(rest)?;
            rest = &rest[number.len() + 1..];
            let value = number_token(number)?;
            match unit {
                'H' => seconds = seconds.checked_add(value * 3600)?,
                'M' => seconds = seconds.checked_add(value * 60)?,
                'S' => seconds = seconds.checked_add(value)?,
                _ => return None,
            }
        }
    }

    Some(TimerDuration {
        months,
        days,
        seconds,
    })
}

/// Split a leading `[digits]` (possibly decimal) followed by a single unit letter.
/// Returns the raw numeric token and the unit char.
fn split_number_unit(segment: &str) -> Option<(&str, char)> {
    let split = segment
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(segment.len());
    if split == 0 {
        return None;
    }
    let unit = segment[split..].chars().next()?;
    Some((&segment[..split], unit))
}

fn number_token(token: &str) -> Option<i64> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    // Fractional components are truncated to the nearest second for `S`; any other
    // fractional unit is rejected (Java Period/Duration reject those too).
    if let Some(whole) = token.split('.').next() {
        whole.parse::<i64>().ok()
    } else {
        token.parse::<i64>().ok()
    }
}

/// Parse an ISO-8601 date/date-time. Java `DateUtil.parseDate`; deviations: a
/// timezone-less value is interpreted as UTC (Java uses the system default zone),
/// and a bare date `YYYY-MM-DD` is treated as start-of-day.
pub fn parse_date_time(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }
    // "YYYY-MM-DDTHH:MM[:SS]" without offset → UTC.
    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M") {
        return Some(Utc.from_utc_datetime(&parsed));
    }
    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Some(Utc.from_utc_datetime(&parsed));
    }
    // Bare date → start of day.
    if let Ok(parsed) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&parsed.and_hms_opt(0, 0, 0)?));
    }
    None
}

fn is_duration_token(token: &str) -> bool {
    token.starts_with('P')
}

/// Parsed `R…` repetition expression.
struct ParsedCycle {
    /// Remaining fires after the current one (`None` for infinite `R`).
    remaining: Option<i64>,
    /// Explicit start anchor; `None` means "now" (DurationHelper.java:89-90).
    start: Option<DateTime<Utc>>,
    /// Repeating interval.
    period: Option<TimerDuration>,
    /// Optional end bound embedded in the expression.
    end: Option<DateTime<Utc>>,
    /// The part after the leading `R…` token (used to rebuild the expression).
    body_after_r: String,
}

fn parse_r_cycle(expression: &str) -> Option<ParsedCycle> {
    let expression = expression.trim();
    if !expression.starts_with('R') {
        return None;
    }
    let parts: Vec<&str> = expression.split('/').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let r_token = parts[0];
    let remaining = if r_token == "R" {
        None
    } else {
        r_token.strip_prefix('R')?.parse::<i64>().ok()
    };
    let rest = &parts[1..];
    if rest.is_empty() {
        return None;
    }
    let body_after_r = rest.join("/");

    let (start, period, end) = if is_duration_token(rest[0]) {
        // R/period  or  R/period/end
        let period = parse_iso8601_duration_parts(rest[0])?;
        let end = if rest.len() == 2 {
            Some(parse_date_time(rest[1])?)
        } else {
            None
        };
        (None, Some(period), end)
    } else {
        // R/start/period  or  R/start/end
        let start = parse_date_time(rest[0])?;
        if rest.len() < 2 {
            return None;
        }
        if is_duration_token(rest[1]) {
            (Some(start), parse_iso8601_duration_parts(rest[1]), None)
        } else {
            let end = parse_date_time(rest[1])?;
            let millis = (end - start).num_milliseconds();
            if millis <= 0 {
                return None;
            }
            let period = TimerDuration {
                months: 0,
                days: millis / 86_400_000,
                seconds: (millis / 1000) % 86_400,
            };
            (Some(start), Some(period), Some(end))
        }
    };

    Some(ParsedCycle {
        remaining,
        start,
        period,
        end,
        body_after_r,
    })
}

/// Java `TimerUtil.prepareRepeat`: for 2-part `R[n]/duration`, inject clock as anchor.
/// Leaves 3-part expressions and non-R values unchanged (TimerEventListenerActivityBehaviour.java:237-242).
pub fn prepare_repeat(cycle: &str, now: DateTime<Utc>) -> String {
    let cycle = cycle.trim();
    if !cycle.starts_with('R') {
        return cycle.to_string();
    }
    let parts: Vec<&str> = cycle.split('/').collect();
    if parts.len() == 2 && is_duration_token(parts[1]) {
        let anchor = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        return format!("{}/{}/{}", parts[0], anchor, parts[1]);
    }
    cycle.to_string()
}

/// First due time strictly after `now` for a cycle description (Java
/// `DurationHelper.getDateAfterRepeat`). Applies the embedded end bound.
fn next_due_from_cycle(parsed: &ParsedCycle, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let period = parsed.period?;
    if period.is_zero() {
        return None;
    }
    let start = parsed.start.unwrap_or(now);
    let mut current = start;
    // Java loop bound: `times + 1` iterations for a bounded repeat, unbounded for `R/`.
    let max_loops = parsed
        .remaining
        .map(|remaining| remaining.saturating_add(1))
        .unwrap_or(i64::MAX);
    let mut iterations = 0u64;
    while iterations <= max_loops as u64 {
        iterations += 1;
        let next = period.add_to(current)?;
        if next <= current {
            break;
        }
        if next > now {
            if let Some(end) = parsed.end
                && next > end
            {
                return None;
            }
            return Some(next);
        }
        current = next;
    }
    None
}

/// Compute the initial due date for a resolved timer expression (duration / date /
/// `R…` cycle / Quartz cron). `now` is the engine clock at activation time.
pub fn resolve_timer_due(expression: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let expression = expression.trim();
    if expression.is_empty() {
        return None;
    }
    if expression.starts_with('R') {
        let prepared = prepare_repeat(expression, now);
        let parsed = parse_r_cycle(&prepared)?;
        if parsed.remaining == Some(0) {
            return None;
        }
        next_due_from_cycle(&parsed, now)
    } else if expression.starts_with('P') {
        // Duration (DueDateBusinessCalendar.java:31-52): now + period + duration.
        parse_iso8601_duration_parts(expression)?.add_to(now)
    } else if looks_like_cron(expression) {
        // P127: Quartz cron (CycleBusinessCalendar.java:44-49) for history cleanup.
        next_cron_after(expression, now)
    } else {
        // Absolute date (DateUtil.parseDate fallback).
        parse_date_time(expression)
    }
}

/// After a cycle timer fires: produce the next persisted repeat expression
/// (Java `calculateRepeatValue` + `setNewRepeat`, TimerJobEntityManagerImpl.java:250-260).
/// Returns `None` when the cycle is exhausted (`R0`/`R1` after this fire).
/// Cron and infinite schedules return the expression unchanged.
pub fn next_repeat_expression(cycle: &str) -> Option<String> {
    let cycle = cycle.trim();
    if cycle.is_empty() {
        return None;
    }
    if !cycle.starts_with('R') {
        // Cron / other unbounded repeat: keep as-is (Java cron never decrements).
        return Some(cycle.to_string());
    }
    let parsed = parse_r_cycle(cycle)?;
    let next_remaining = match parsed.remaining {
        None => None,
        Some(0) | Some(1) => return None,
        Some(n) => Some(n - 1),
    };
    let body = parsed.body_after_r;
    Some(match next_remaining {
        None => format!("R/{body}"),
        Some(n) => format!("R{n}/{body}"),
    })
}

/// Compute the next fire time for a persisted repeat expression after `now`
/// (Java `CycleBusinessCalendar.resolveDuedate` on the stored repeat).
pub fn resolve_next_due(cycle: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let cycle = cycle.trim();
    if looks_like_cron(cycle) {
        return next_cron_after(cycle, now);
    }
    let parsed = parse_r_cycle(cycle)?;
    if parsed.remaining == Some(0) {
        return None;
    }
    next_due_from_cycle(&parsed, now)
}

/// Heuristic: Quartz cron has 5–7 whitespace-separated fields.
fn looks_like_cron(expression: &str) -> bool {
    let n = expression.split_whitespace().count();
    (5..=7).contains(&n)
}

/// Minimal Quartz-style cron next-fire (mirrors BPMN `time_source::next_cron_after`).
/// Fields: [seconds] minutes hours day-of-month month day-of-week [year].
/// Supports `*`, `?`, `N`, `N-M`, `*/N`, lists.
pub fn next_cron_after(expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let (sec_f, min_f, hour_f, dom_f, mon_f, dow_f) = match fields.len() {
        5 => ("0", fields[0], fields[1], fields[2], fields[3], fields[4]),
        6 => (
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
        ),
        7 => (
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
        ),
        _ => return None,
    };

    let seconds = parse_cron_field(sec_f, 0, 59)?;
    let minutes = parse_cron_field(min_f, 0, 59)?;
    let hours = parse_cron_field(hour_f, 0, 23)?;
    let days_of_month = parse_cron_field(dom_f, 1, 31)?;
    let months = parse_cron_field(mon_f, 1, 12)?;
    let days_of_week = parse_cron_field(dow_f, 0, 7)?;

    let mut cursor = after + ChronoDuration::seconds(1);
    cursor = cursor.with_nanosecond(0).unwrap_or(cursor);
    for _ in 0..(2 * 366 * 24 * 60 * 60) {
        let month = cursor.month();
        if !months.contains(&month) {
            let (y, m) = if month == 12 {
                (cursor.year() + 1, 1)
            } else {
                (cursor.year(), month + 1)
            };
            cursor = Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).single()?;
            continue;
        }
        let day = cursor.day();
        let weekday_sun0 = cursor.weekday().num_days_from_sunday();
        let quartz_dow = weekday_sun0 + 1;
        let dow_ok = days_of_week.contains(&weekday_sun0)
            || days_of_week.contains(&quartz_dow)
            || (weekday_sun0 == 6 && days_of_week.contains(&7));
        let dom_is_any = dom_f == "*" || dom_f == "?";
        let dow_is_any = dow_f == "*" || dow_f == "?";
        let day_ok = if dom_is_any && dow_is_any {
            true
        } else if dom_is_any {
            dow_ok
        } else if dow_is_any {
            days_of_month.contains(&day)
        } else {
            days_of_month.contains(&day) || dow_ok
        };
        if !day_ok {
            cursor = (cursor + ChronoDuration::days(1))
                .with_hour(0)
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .unwrap_or(cursor + ChronoDuration::days(1));
            continue;
        }
        if !hours.contains(&cursor.hour()) {
            cursor = (cursor + ChronoDuration::hours(1))
                .with_minute(0)
                .and_then(|d| d.with_second(0))
                .unwrap_or(cursor + ChronoDuration::hours(1));
            continue;
        }
        if !minutes.contains(&cursor.minute()) {
            cursor = (cursor + ChronoDuration::minutes(1))
                .with_second(0)
                .unwrap_or(cursor + ChronoDuration::minutes(1));
            continue;
        }
        if !seconds.contains(&cursor.second()) {
            cursor += ChronoDuration::seconds(1);
            continue;
        }
        return Some(cursor);
    }
    None
}

fn parse_cron_field(field: &str, min: u32, max: u32) -> Option<Vec<u32>> {
    let field = field.trim();
    if field == "*" || field == "?" {
        return Some((min..=max).collect());
    }
    let mut values = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((range, step_s)) = part.split_once('/') {
            let step: u32 = step_s.parse().ok()?;
            if step == 0 {
                return None;
            }
            let (start, end) = if range == "*" {
                (min, max)
            } else if let Some((a, b)) = range.split_once('-') {
                (a.parse().ok()?, b.parse().ok()?)
            } else {
                let s: u32 = range.parse().ok()?;
                (s, max)
            };
            let mut v = start;
            while v <= end {
                if v >= min && v <= max {
                    values.push(v);
                }
                v = v.saturating_add(step);
            }
        } else if let Some((a, b)) = part.split_once('-') {
            let start: u32 = a.parse().ok()?;
            let end: u32 = b.parse().ok()?;
            for v in start..=end {
                if v >= min && v <= max {
                    values.push(v);
                }
            }
        } else {
            let v: u32 = part.parse().ok()?;
            if v >= min && v <= max {
                values.push(v);
            }
        }
    }
    if values.is_empty() {
        None
    } else {
        values.sort_unstable();
        values.dedup();
        Some(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_durations() {
        let d = parse_iso8601_duration_parts("PT1H").expect("PT1H");
        assert_eq!(d.seconds, 3600);
        let d = parse_iso8601_duration_parts("P3D").expect("P3D");
        assert_eq!(d.days, 3);
        let d = parse_iso8601_duration_parts("P1DT2H").expect("P1DT2H");
        assert_eq!(d.days, 1);
        assert_eq!(d.seconds, 7200);
        let d = parse_iso8601_duration_parts("P1Y2M3DT4H5M6S").expect("P1Y2M3DT4H5M6S");
        assert_eq!(d.months, 14);
        assert_eq!(d.days, 3);
        assert_eq!(d.seconds, 4 * 3600 + 5 * 60 + 6);
    }

    #[test]
    fn resolves_duration_due() {
        let now = Utc.with_ymd_and_hms(2026, 8, 4, 12, 0, 0).unwrap();
        let due = resolve_timer_due("PT1H", now).expect("due");
        assert_eq!(due, now + chrono::Duration::hours(1));
    }

    #[test]
    fn resolves_absolute_date_due() {
        let now = Utc.with_ymd_and_hms(2026, 8, 4, 12, 0, 0).unwrap();
        let due = resolve_timer_due("2026-08-05T10:00:00Z", now).expect("due");
        assert_eq!(due.to_rfc3339(), "2026-08-05T10:00:00+00:00");
    }

    #[test]
    fn prepares_and_resolves_repeat() {
        let now = Utc.with_ymd_and_hms(2026, 8, 4, 12, 0, 0).unwrap();
        let prepared = prepare_repeat("R/PT20S", now);
        assert_eq!(prepared, "R/2026-08-04T12:00:00.000Z/PT20S");

        let first = resolve_timer_due("R/PT20S", now).expect("first due");
        assert_eq!(first, now + chrono::Duration::seconds(20));

        let next_expr = next_repeat_expression(&prepared).expect("next expr");
        assert_eq!(next_expr, "R/2026-08-04T12:00:00.000Z/PT20S");

        let next = resolve_next_due(&next_expr, now + chrono::Duration::seconds(25))
            .expect("next due");
        assert_eq!(next, now + chrono::Duration::seconds(40));
    }

    #[test]
    fn bounded_repeat_decrements_and_exhausts() {
        let now = Utc.with_ymd_and_hms(2026, 8, 4, 12, 0, 0).unwrap();
        let first = resolve_timer_due("R3/PT20S", now).expect("first due");
        assert_eq!(first, now + chrono::Duration::seconds(20));

        let prepared = prepare_repeat("R3/PT20S", now);
        assert_eq!(prepared, "R3/2026-08-04T12:00:00.000Z/PT20S");

        let next_expr = next_repeat_expression(&prepared).expect("next expr");
        assert_eq!(next_expr, "R2/2026-08-04T12:00:00.000Z/PT20S");
        // Exhaust after the final decrement.
        let exhausted = next_repeat_expression("R1/2026-08-04T12:00:00.000Z/PT20S");
        assert_eq!(exhausted, None);
    }

    #[test]
    fn repeat_with_end_bound_stops() {
        let now = Utc.with_ymd_and_hms(2026, 8, 4, 12, 0, 0).unwrap();
        let end = "2026-08-04T12:00:25Z";
        let expr = format!("R/PT20S/{end}");
        let first = resolve_timer_due(&expr, now).expect("first due");
        assert_eq!(first, now + chrono::Duration::seconds(20));
        // Second fire (now+25s) would land at +40s > end → no more fires.
        let next = resolve_next_due(&prepare_repeat(&expr, now), now + chrono::Duration::seconds(25));
        assert_eq!(next, None);
    }
}
