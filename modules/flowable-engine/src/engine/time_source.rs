use chrono::{DateTime, Datelike, Duration as ChronoDuration, TimeZone, Timelike, Utc};
use std::panic::{RefUnwindSafe, UnwindSafe};
use std::sync::Mutex;

pub trait TimeSource: Send + Sync + UnwindSafe + RefUnwindSafe {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemTimeSource;

impl TimeSource for SystemTimeSource {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct TestTimeSource {
    time: Mutex<DateTime<Utc>>,
}

impl TestTimeSource {
    pub fn new(time: DateTime<Utc>) -> Self {
        Self {
            time: Mutex::new(time),
        }
    }

    pub fn set_time(&self, time: DateTime<Utc>) {
        *self.time.lock().unwrap() = time;
    }

    pub fn advance_time(&self, duration_millis: i64) {
        let mut time = self.time.lock().unwrap();
        *time += ChronoDuration::milliseconds(duration_millis);
    }
}

impl TimeSource for TestTimeSource {
    fn now(&self) -> DateTime<Utc> {
        *self.time.lock().unwrap()
    }
}

// ── ISO-8601 duration ──────────────────────────────────────────────────────

/// Parsed ISO-8601 duration components (e.g. `P1Y2M3DT4H5M6S`, `P2W`, `PT10S`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IsoDuration {
    pub years: i32,
    pub months: i32,
    pub weeks: i32,
    pub days: i32,
    pub hours: i32,
    pub minutes: i32,
    pub seconds: i32,
    pub millis: i32,
}

impl IsoDuration {
    pub fn is_zero(&self) -> bool {
        self.years == 0
            && self.months == 0
            && self.weeks == 0
            && self.days == 0
            && self.hours == 0
            && self.minutes == 0
            && self.seconds == 0
            && self.millis == 0
    }

    /// Add this duration to `dt` using real calendar months/years (Java `Duration.addTo`).
    pub fn add_to(&self, dt: DateTime<Utc>) -> DateTime<Utc> {
        let mut year = dt.year() + self.years;
        let mut month0 = dt.month0() as i32 + self.months;
        while month0 >= 12 {
            year += 1;
            month0 -= 12;
        }
        while month0 < 0 {
            year -= 1;
            month0 += 12;
        }
        let month = (month0 + 1) as u32;
        // Clamp day to last day of target month (calendar-safe).
        let max_day = days_in_month(year, month);
        let day = dt.day().min(max_day);
        let base = Utc
            .with_ymd_and_hms(year, month, day, dt.hour(), dt.minute(), dt.second())
            .single()
            .unwrap_or(dt)
            + ChronoDuration::nanoseconds(dt.nanosecond() as i64);

        base + ChronoDuration::weeks(self.weeks as i64)
            + ChronoDuration::days(self.days as i64)
            + ChronoDuration::hours(self.hours as i64)
            + ChronoDuration::minutes(self.minutes as i64)
            + ChronoDuration::seconds(self.seconds as i64)
            + ChronoDuration::milliseconds(self.millis as i64)
    }

    /// Approximate total milliseconds (months=30d, years=365d). Prefer `add_to` for scheduling.
    pub fn to_approx_millis(&self) -> i64 {
        let days = self.years as i64 * 365
            + self.months as i64 * 30
            + self.weeks as i64 * 7
            + self.days as i64;
        days * 24 * 60 * 60 * 1000
            + self.hours as i64 * 60 * 60 * 1000
            + self.minutes as i64 * 60 * 1000
            + self.seconds as i64 * 1000
            + self.millis as i64
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Parse an ISO-8601 duration string (`P…` / `PT…`). Returns `None` if not a duration.
pub fn parse_iso8601_duration_parts(duration: &str) -> Option<IsoDuration> {
    let dur_str = duration.trim();
    if !dur_str.starts_with('P') {
        return None;
    }

    let mut chars = dur_str.chars().peekable();
    chars.next(); // Skip 'P'

    let mut result = IsoDuration::default();
    let mut is_time = false;
    let mut current_num = String::new();
    let mut saw_designator = false;

    for c in chars {
        if c == 'T' {
            is_time = true;
            current_num.clear();
            continue;
        }
        if c == '.' || c == ',' {
            // Fractional seconds: keep a decimal marker so the S designator can
            // split whole/frac into seconds + millis.
            if !current_num.is_empty() && !current_num.contains('.') {
                current_num.push('.');
            }
            continue;
        }

        if c.is_ascii_digit() {
            current_num.push(c);
            continue;
        }

        if current_num.is_empty() {
            return None;
        }

        // Fractional seconds: "1.5" → seconds=1, millis=500
        let (num_i, millis_extra) = if let Some((whole, frac)) = current_num.split_once('.') {
            let whole_v: i32 = whole.parse().unwrap_or(0);
            let mut frac_digits = frac.to_string();
            while frac_digits.len() < 3 {
                frac_digits.push('0');
            }
            frac_digits.truncate(3);
            let millis_v: i32 = frac_digits.parse().unwrap_or(0);
            (whole_v, millis_v)
        } else {
            (current_num.parse().unwrap_or(0), 0)
        };
        current_num.clear();
        saw_designator = true;

        match c {
            'Y' if !is_time => result.years += num_i,
            'M' if !is_time => result.months += num_i,
            'W' if !is_time => result.weeks += num_i,
            'D' if !is_time => result.days += num_i,
            'H' if is_time => result.hours += num_i,
            'M' if is_time => result.minutes += num_i,
            'S' if is_time => {
                result.seconds += num_i;
                result.millis += millis_extra;
            }
            _ => return None,
        }
    }

    if !saw_designator {
        return None;
    }
    Some(result)
}

/// Parse ISO-8601 duration to approximate milliseconds.
/// Handles repeating prefixes (`R3/PT10H`, `R/PT5S`) by extracting the duration segment.
/// Supports weeks (`P2W` = 14 days). Returns `None` for unparsable input (not zero).
pub fn parse_iso8601_duration(duration: &str) -> Option<i64> {
    let mut dur_str = duration.trim();
    if dur_str.is_empty() {
        return None;
    }

    // Strip repeating cycle prefix: R, R3, R/… → duration part
    if dur_str.starts_with('R') {
        let parts: Vec<&str> = dur_str.split('/').collect();
        // Find first duration segment starting with P
        let period = parts.iter().find(|p| p.starts_with('P'))?;
        dur_str = period;
    }

    let parts = parse_iso8601_duration_parts(dur_str)?;
    Some(parts.to_approx_millis())
}

// ── Cycle expressions (Java DurationHelper + CycleBusinessCalendar) ────────

#[derive(Debug, Clone)]
struct ParsedCycle {
    /// `None` = infinite (R without count). `Some(n)` = remaining fires including next.
    remaining: Option<u32>,
    /// Anchor start for period addition. `None` means "use clock now as start".
    start: Option<DateTime<Utc>>,
    period: Option<IsoDuration>,
    /// End bound embedded in cycle (`R/period/end` or `R/start/end`).
    end: Option<DateTime<Utc>>,
    /// Original body after the `R[n]` prefix (without leading slash), for rebuild.
    body_after_r: String,
}

fn parse_date_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // RFC3339 / ISO-8601
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Instant without zone: treat as UTC
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0).map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));
    }
    None
}

fn is_duration_token(s: &str) -> bool {
    s.starts_with('P')
}

/// Public instant parser shared with [`crate::engine::business_calendar`].
/// Accepts RFC3339, zone-less `yyyy-MM-dd'T'HH:mm:ss[.SSS]`, and plain dates.
pub fn parse_instant(value: &str) -> Option<DateTime<Utc>> {
    parse_date_time(value)
}

/// Parse `R[n]/…` ISO cycle. Returns `None` if not an R-cycle (may be cron).
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
        None // infinite
    } else {
        let n = r_token.strip_prefix('R')?.parse::<u32>().ok()?;
        Some(n)
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
            let period = parse_iso8601_duration_parts(rest[1])?;
            (Some(start), Some(period), None)
        } else {
            let end = parse_date_time(rest[1])?;
            let millis = (end - start).num_milliseconds();
            if millis <= 0 {
                return None;
            }
            let period = IsoDuration {
                millis: (millis % 1000) as i32,
                seconds: ((millis / 1000) % 60) as i32,
                minutes: ((millis / 60_000) % 60) as i32,
                hours: ((millis / 3_600_000) % 24) as i32,
                days: (millis / 86_400_000) as i32,
                ..IsoDuration::default()
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
/// Leaves 3-part expressions and non-R values unchanged.
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

/// First due time after `now` for a cycle (or duration/date). Anchored — no drift.
fn next_due_from_cycle(parsed: &ParsedCycle, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let period = parsed.period?;
    if period.is_zero() {
        return None;
    }

    let start = parsed.start.unwrap_or(now);
    // Java getDateAfterRepeat: advance while current <= now, return first after now.
    let mut current = start;
    // Safety cap to avoid infinite loops on broken periods
    for _ in 0..100_000 {
        let next = period.add_to(current);
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

fn rebuild_cycle_expression(remaining: Option<u32>, body_after_r: &str) -> String {
    match remaining {
        None => format!("R/{}", body_after_r),
        Some(n) => format!("R{}/{}", n, body_after_r),
    }
}

/// Result of scheduling / rescheduling a timeCycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleSchedule {
    /// Prepared cycle expression (may include injected start anchor).
    pub cycle: String,
    pub due_time_millis: i64,
}

/// Outcome of resolving the next fire of a cycle description, separating a
/// legitimately finished schedule from an unparseable description so the
/// `cycle` business calendar can map them to `Ok(None)` vs a hard `Err`
/// instead of conflating both into one failure (P64 repeat contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CycleResolution {
    /// Next fire computed; carries the prepared cycle text.
    Due(CycleSchedule),
    /// `R0`, or the next fire would pass the cycle-embedded end bound —
    /// a legitimate "no next fire", not an error.
    Finished,
    /// The description is neither a valid `R…` cycle nor a cron expression.
    Unresolvable,
}

/// First fire strictly after `now` for a cycle description (`R…` or cron).
///
/// Cycle-embedded end bounds (`R/PT1H/<end>`) are applied here and report
/// [`CycleResolution::Finished`]; the modelled `endDate` attribute stays with
/// [`schedule_cycle`] / the timer layer's `validate_due_date` call.
pub fn resolve_cycle(cycle: &str, now: DateTime<Utc>) -> CycleResolution {
    let cycle = cycle.trim();
    if cycle.is_empty() {
        return CycleResolution::Unresolvable;
    }

    if cycle.starts_with('R') {
        let prepared = prepare_repeat(cycle, now);
        let Some(parsed) = parse_r_cycle(&prepared) else {
            return CycleResolution::Unresolvable;
        };
        // R0 means no fires
        if parsed.remaining == Some(0) {
            return CycleResolution::Finished;
        }
        // Strip the embedded end bound before computing the fire so a `None`
        // here always means the period itself is unusable, then re-apply it
        // to classify "past end" as Finished rather than Unresolvable.
        let embedded_end = parsed.end;
        let unbounded = ParsedCycle {
            end: None,
            ..parsed
        };
        let Some(due) = next_due_from_cycle(&unbounded, now) else {
            return CycleResolution::Unresolvable;
        };
        if !is_due_before_end(due, None, embedded_end) {
            return CycleResolution::Finished;
        }
        return CycleResolution::Due(CycleSchedule {
            cycle: prepared,
            due_time_millis: due.timestamp_millis(),
        });
    }

    // Cron fallback (Java CycleBusinessCalendar for non-R values)
    match next_cron_after(cycle, now) {
        Some(due) => CycleResolution::Due(CycleSchedule {
            cycle: cycle.to_string(),
            due_time_millis: due.timestamp_millis(),
        }),
        None => CycleResolution::Unresolvable,
    }
}

/// Compute initial due time + prepared cycle for a timeCycle value.
pub fn schedule_cycle(
    cycle: &str,
    end_date: Option<&str>,
    now: DateTime<Utc>,
) -> Option<CycleSchedule> {
    match resolve_cycle(cycle, now) {
        CycleResolution::Due(schedule) => {
            let due = DateTime::<Utc>::from_timestamp_millis(schedule.due_time_millis)?;
            if !is_due_before_end(due, end_date, None) {
                return None;
            }
            Some(schedule)
        }
        CycleResolution::Finished | CycleResolution::Unresolvable => None,
    }
}

/// After a cycle timer fires: produce the next persisted repeat expression
/// (Java `calculateRepeatValue` + `setNewRepeat`).
///
/// Returns `None` when the cycle is exhausted (`R0`/`R1` after this fire). Cron
/// and infinite `R/...` expressions are returned unchanged. Does **not** compute
/// a due date — that is the business calendar's job (P64 `resolve_next_timer_schedule`).
pub fn next_repeat_expression(cycle: &str) -> Option<String> {
    let cycle = cycle.trim();
    if cycle.is_empty() {
        return None;
    }

    if cycle.starts_with('R') {
        let parsed = parse_r_cycle(cycle)?;
        // Java calculateRepeatValue: if R has a count, decrement; 0 means stop.
        let next_remaining = match parsed.remaining {
            None => None, // infinite
            Some(0) | Some(1) => return None,
            Some(n) => Some(n - 1),
        };
        return Some(rebuild_cycle_expression(next_remaining, &parsed.body_after_r));
    }

    // Cron: expression unchanged.
    Some(cycle.to_string())
}

/// After a cycle timer fires: decrement remaining count and compute next due (anchored).
/// Returns `None` when the cycle is exhausted or next due would pass endDate.
///
/// Pure cycle-calendar path used by unit/parity tests. Production reschedule
/// routes through [`crate::bpmn::timer_util::resolve_next_timer_schedule`] so a
/// custom `businessCalendarName` can override due-date calculation.
pub fn reschedule_cycle_after_fire(
    cycle: &str,
    end_date: Option<&str>,
    now: DateTime<Utc>,
) -> Option<CycleSchedule> {
    let next_expr = next_repeat_expression(cycle)?;

    if next_expr.starts_with('R') {
        let next_parsed = parse_r_cycle(&next_expr)?;
        let due = next_due_from_cycle(&next_parsed, now)?;
        if !is_due_before_end(due, end_date, next_parsed.end) {
            return None;
        }
        return Some(CycleSchedule {
            cycle: next_expr,
            due_time_millis: due.timestamp_millis(),
        });
    }

    // Cron: expression unchanged, next occurrence after now
    let due = next_cron_after(&next_expr, now)?;
    if !is_due_before_end(due, end_date, None) {
        return None;
    }
    Some(CycleSchedule {
        cycle: next_expr,
        due_time_millis: due.timestamp_millis(),
    })
}

/// Java `isValidDate` / `validateDuedate`: due must not be after endDate.
pub fn is_due_before_end(
    due: DateTime<Utc>,
    end_date_attr: Option<&str>,
    cycle_end: Option<DateTime<Utc>>,
) -> bool {
    if let Some(end) = cycle_end
        && due > end
    {
        return false;
    }
    if let Some(end_s) = end_date_attr
        && let Some(end) = parse_date_time(end_s)
        && due > end
    {
        return false;
    }
    true
}

/// True if a due timestamp (millis) is still valid against endDate (Java fire-time check).
pub fn is_valid_due_millis(due_millis: i64, end_date: Option<&str>) -> bool {
    let Some(end_s) = end_date else {
        return true;
    };
    let Some(end) = parse_date_time(end_s) else {
        return true; // unparsed endDate (EL) — do not block (P17)
    };
    due_millis <= end.timestamp_millis()
}

// ── Cron (minimal Quartz-style, 6 or 5 fields) ─────────────────────────────
// Fields: [seconds] minutes hours day-of-month month day-of-week
// Supports: *, N, N-M, */N, N/S, lists. `?` treated as `*`.
// Month/day-of-week names not required for core parity tests.

fn next_cron_after(expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    // 5-field (unix, no seconds) or 6-field (quartz with seconds)
    let (sec_f, min_f, hour_f, dom_f, mon_f, dow_f) = match fields.len() {
        5 => ("0", fields[0], fields[1], fields[2], fields[3], fields[4]),
        6 => (
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
        ),
        7 => (
            // with optional year — ignore year field for next calc
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
        ),
        _ => return None,
    };

    let seconds = parse_cron_field(sec_f, 0, 59)?;
    let minutes = parse_cron_field(min_f, 0, 59)?;
    let hours = parse_cron_field(hour_f, 0, 23)?;
    let days_of_month = parse_cron_field(dom_f, 1, 31)?;
    let months = parse_cron_field(mon_f, 1, 12)?;
    // Quartz: 1=SUN..7=SAT; also allow 0=SUN
    let days_of_week = parse_cron_field(dow_f, 0, 7)?;

    // Search second-by-second up to ~2 years
    let mut cursor = after + ChronoDuration::seconds(1);
    cursor = cursor
        .with_nanosecond(0)
        .unwrap_or(cursor);
    for _ in 0..(2 * 366 * 24 * 60 * 60) {
        let month = cursor.month();
        if !months.contains(&month) {
            // jump to first of next month
            let (y, m) = if month == 12 {
                (cursor.year() + 1, 1)
            } else {
                (cursor.year(), month + 1)
            };
            cursor = Utc
                .with_ymd_and_hms(y, m, 1, 0, 0, 0)
                .single()?;
            continue;
        }
        let day = cursor.day();
        // chrono: 0=Sun..6=Sat; Quartz: 1=Sun..7=Sat (7 also means Sat in some dialects)
        let weekday_sun0 = cursor.weekday().num_days_from_sunday();
        let quartz_dow = weekday_sun0 + 1;
        let dow_ok = days_of_week.contains(&weekday_sun0)
            || days_of_week.contains(&quartz_dow)
            || (weekday_sun0 == 6 && days_of_week.contains(&7));
        // When DOM is * and DOW is restricted (or vice versa), either can match (simplified).
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
            while v <= end && v <= max {
                if v >= min {
                    values.push(v);
                }
                v += step;
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

// ── Public calculate_due_time ───────────────────────────────────────────────

pub fn calculate_due_time(
    time_date: Option<&String>,
    time_duration: Option<&String>,
    time_cycle: Option<&String>,
    now: DateTime<Utc>,
) -> Option<i64> {
    calculate_due_time_with_end(time_date, time_duration, time_cycle, None, now)
}

pub fn calculate_due_time_with_end(
    time_date: Option<&String>,
    time_duration: Option<&String>,
    time_cycle: Option<&String>,
    end_date: Option<&str>,
    now: DateTime<Utc>,
) -> Option<i64> {
    if let Some(date_str) = time_date
        && let Some(dt) = parse_date_time(date_str)
    {
        return Some(dt.timestamp_millis());
    }

    if let Some(dur_str) = time_duration {
        if let Some(parts) = parse_iso8601_duration_parts(dur_str) {
            return Some(parts.add_to(now).timestamp_millis());
        }
        // Also allow R-prefixed by mistake on duration field
        if let Some(millis) = parse_iso8601_duration(dur_str) {
            return Some(now.timestamp_millis() + millis);
        }
    }

    if let Some(cycle) = time_cycle {
        return schedule_cycle(cycle, end_date, now).map(|s| s.due_time_millis);
    }

    None
}

/// Prepare a timeCycle for persistence at schedule time (inject anchor) and compute due.
/// Returns `(prepared_cycle, due_millis)`.
pub fn prepare_cycle_and_due(
    time_cycle: Option<&String>,
    end_date: Option<&str>,
    now: DateTime<Utc>,
) -> (Option<String>, Option<i64>) {
    match time_cycle {
        Some(cycle) => match schedule_cycle(cycle, end_date, now) {
            Some(s) => (Some(s.cycle), Some(s.due_time_millis)),
            None => (Some(cycle.clone()), None),
        },
        None => (None, None),
    }
}

/// Resolve prepared `time_cycle` + `due_time` from a timer event definition's fields.
/// When `time_cycle` is present, injects the schedule-time anchor (Java `prepareRepeat`).
pub fn resolve_timer_fields(
    time_date: Option<&String>,
    time_duration: Option<&String>,
    time_cycle: Option<&String>,
    end_date: Option<&str>,
    now: DateTime<Utc>,
) -> (Option<String>, Option<i64>) {
    if time_cycle.is_some() {
        return prepare_cycle_and_due(time_cycle, end_date, now);
    }
    (
        None,
        calculate_due_time_with_end(time_date, time_duration, None, end_date, now),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn weeks_are_seven_days() {
        let millis = parse_iso8601_duration("P2W").expect("P2W");
        assert_eq!(millis, 2 * 7 * 24 * 60 * 60 * 1000);
        // Must not be Some(0) (old bug)
        assert!(millis > 0);
    }

    #[test]
    fn three_segment_start_period_schedules() {
        let start = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        let cycle = format!("R3/{}/PT1H", start.to_rfc3339());
        let now = start;
        let s = schedule_cycle(&cycle, None, now).expect("schedule");
        assert_eq!(s.due_time_millis, (start + ChronoDuration::hours(1)).timestamp_millis());
    }

    #[test]
    fn three_segment_period_end_schedules() {
        let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        let end = now + ChronoDuration::hours(5);
        let cycle = format!("R/PT1H/{}", end.to_rfc3339());
        let s = schedule_cycle(&cycle, None, now).expect("schedule");
        assert_eq!(s.due_time_millis, (now + ChronoDuration::hours(1)).timestamp_millis());
    }

    #[test]
    fn prepare_repeat_injects_anchor() {
        let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        let prepared = prepare_repeat("R3/PT1H", now);
        assert!(prepared.starts_with("R3/"));
        assert!(prepared.ends_with("/PT1H"));
        assert_eq!(prepared.matches('/').count(), 2);
    }

    #[test]
    fn reschedule_is_anchored_no_drift() {
        let start = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        let cycle = format!("R3/{}/PT1H", start.to_rfc3339());
        // Fire late: 1h30 after start (should have been due at +1h)
        let late_now = start + ChronoDuration::minutes(90);
        let next = reschedule_cycle_after_fire(&cycle, None, late_now).expect("reschedule");
        // Next due is start+2h = 14:00, not late_now+1h = 14:30
        assert_eq!(
            next.due_time_millis,
            (start + ChronoDuration::hours(2)).timestamp_millis()
        );
        assert!(next.cycle.starts_with("R2/"));
    }

    #[test]
    fn r2_exhausts_after_second_fire() {
        let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        let prepared = prepare_repeat("R2/PT1H", now);
        let after1 = reschedule_cycle_after_fire(&prepared, None, now + ChronoDuration::hours(1))
            .expect("first reschedule");
        assert!(after1.cycle.starts_with("R1/"));
        let after2 =
            reschedule_cycle_after_fire(&after1.cycle, None, now + ChronoDuration::hours(2));
        assert!(after2.is_none(), "R2 should exhaust after second fire");
    }

    #[test]
    fn end_date_blocks_schedule() {
        let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        let end = (now + ChronoDuration::minutes(30)).to_rfc3339();
        // First due is +1h which is after end → None
        assert!(schedule_cycle("R10/PT1H", Some(&end), now).is_none());
    }

    #[test]
    fn cron_next_fire() {
        let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
        // every hour at minute 0 second 0
        let next = next_cron_after("0 0 * * * ?", now).expect("cron");
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 4, 18, 13, 0, 0).unwrap());
    }

    #[test]
    fn months_use_calendar_add() {
        let jan31 = Utc.with_ymd_and_hms(2026, 1, 31, 12, 0, 0).unwrap();
        let parts = parse_iso8601_duration_parts("P1M").unwrap();
        let next = parts.add_to(jan31);
        // Feb has 28 days in 2026 → clamped to Feb 28
        assert_eq!(next.month(), 2);
        assert_eq!(next.day(), 28);
    }
}
