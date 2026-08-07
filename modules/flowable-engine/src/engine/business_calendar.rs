//! Engine-local business calendars (Java `BusinessCalendar` /
//! `MapBusinessCalendarManager` parity).
//!
//! Java resolves a timer's due date through a named calendar looked up in the
//! engine configuration's `BusinessCalendarManager`
//! (`TimerUtil.java:126-157`). The name comes from the timer's field kind
//! (`timeDate` → `dueDate`, `timeCycle` → `cycle`, `timeDuration` →
//! `duration`) and is overridden by `<timerEventDefinition businessCalendarName>`
//! / `<calendar>` when present.
//!
//! ADR-1: the registry is owned by [`crate::service::config::ProcessEngineConfiguration`],
//! populated before engine construction, never process-global, and never
//! serialized. Two engines may hold different implementations under the same
//! name without leaking into each other.
//!
//! Built-in calendars delegate to the pure schedule functions in
//! [`crate::engine::time_source`]; no ISO/cron parser is duplicated here.

use crate::engine::time_source;
use crate::error::FlowableError;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Java `DueDateBusinessCalendar.NAME`.
pub const DUE_DATE_CALENDAR_NAME: &str = "dueDate";
/// Java `DurationBusinessCalendar.NAME`.
pub const DURATION_CALENDAR_NAME: &str = "duration";
/// Java `CycleBusinessCalendar.NAME`.
pub const CYCLE_CALENDAR_NAME: &str = "cycle";

/// Resolves timer descriptions into instants (Java `BusinessCalendar`).
///
/// `now` is supplied by the caller rather than read from a clock so that
/// engine time sources stay authoritative and implementations remain pure.
///
/// `max_iterations` mirrors Java's `resolveDuedate(String, int)` overload. The
/// built-in calendars do not need it — their catch-up loop is bounded inside
/// [`crate::engine::time_source`] — but custom calendars receive it verbatim.
///
/// `resolve_due_date` mirrors Java's nullable return: `Ok(Some(_))` is the next
/// fire, `Ok(None)` means the schedule legitimately produces no further fire
/// (Java `resolveDuedate` → `null`, e.g. an exhausted cycle-embedded end
/// bound), and `Err` is a hard failure that must propagate and roll back the
/// surrounding command — it is never a soft "no due date" signal.
pub trait BusinessCalendar: fmt::Debug + Send + Sync + 'static {
    fn resolve_due_date(
        &self,
        description: &str,
        now: DateTime<Utc>,
        max_iterations: Option<u32>,
    ) -> Result<Option<DateTime<Utc>>, FlowableError>;

    fn resolve_end_date(
        &self,
        description: &str,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, FlowableError>;

    fn validate_due_date(
        &self,
        repeat: &str,
        max_iterations: Option<u32>,
        end_date: Option<DateTime<Utc>>,
        candidate: DateTime<Utc>,
    ) -> Result<bool, FlowableError>;
}

fn unresolvable(calendar: &str, description: &str) -> FlowableError {
    FlowableError::ExecutionError(format!(
        "Business calendar '{calendar}' could not resolve due date from '{description}'"
    ))
}

fn parse_end_date(calendar: &str, description: &str) -> Result<DateTime<Utc>, FlowableError> {
    time_source::parse_instant(description).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Business calendar '{calendar}' could not resolve end date from '{description}'"
        ))
    })
}

/// Shared endDate guard: Java `validateDuedate` rejects a candidate past endDate.
fn candidate_within_end(end_date: Option<DateTime<Utc>>, candidate: DateTime<Utc>) -> bool {
    time_source::is_due_before_end(candidate, None, end_date)
}

/// `dueDate` — the description is an absolute instant (Java `DueDateBusinessCalendar`).
#[derive(Debug, Default)]
pub struct DueDateBusinessCalendar;

impl BusinessCalendar for DueDateBusinessCalendar {
    fn resolve_due_date(
        &self,
        description: &str,
        now: DateTime<Utc>,
        _max_iterations: Option<u32>,
    ) -> Result<Option<DateTime<Utc>>, FlowableError> {
        time_source::calculate_due_time_with_end(
            Some(&description.to_string()),
            None,
            None,
            None,
            now,
        )
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(Some)
        .ok_or_else(|| unresolvable(DUE_DATE_CALENDAR_NAME, description))
    }

    fn resolve_end_date(
        &self,
        description: &str,
        _now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, FlowableError> {
        parse_end_date(DUE_DATE_CALENDAR_NAME, description)
    }

    fn validate_due_date(
        &self,
        _repeat: &str,
        _max_iterations: Option<u32>,
        end_date: Option<DateTime<Utc>>,
        candidate: DateTime<Utc>,
    ) -> Result<bool, FlowableError> {
        Ok(candidate_within_end(end_date, candidate))
    }
}

/// `duration` — the description is an ISO-8601 duration added to `now`
/// (Java `DurationBusinessCalendar`).
#[derive(Debug, Default)]
pub struct DurationBusinessCalendar;

impl BusinessCalendar for DurationBusinessCalendar {
    fn resolve_due_date(
        &self,
        description: &str,
        now: DateTime<Utc>,
        _max_iterations: Option<u32>,
    ) -> Result<Option<DateTime<Utc>>, FlowableError> {
        time_source::calculate_due_time_with_end(
            None,
            Some(&description.to_string()),
            None,
            None,
            now,
        )
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(Some)
        .ok_or_else(|| unresolvable(DURATION_CALENDAR_NAME, description))
    }

    fn resolve_end_date(
        &self,
        description: &str,
        _now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, FlowableError> {
        parse_end_date(DURATION_CALENDAR_NAME, description)
    }

    fn validate_due_date(
        &self,
        _repeat: &str,
        _max_iterations: Option<u32>,
        end_date: Option<DateTime<Utc>>,
        candidate: DateTime<Utc>,
    ) -> Result<bool, FlowableError> {
        Ok(candidate_within_end(end_date, candidate))
    }
}

/// `cycle` — the description is an `R[n]/…` repeating expression or a cron
/// expression (Java `CycleBusinessCalendar`).
///
/// The returned instant is the first fire time strictly after `now`. Anchor
/// injection (Java `prepareRepeat`) stays with the caller, exactly as in
/// `TimerUtil.createTimerEntity`, so the persisted repeat text is owned by the
/// timer layer rather than by the calendar.
#[derive(Debug, Default)]
pub struct CycleBusinessCalendar;

impl BusinessCalendar for CycleBusinessCalendar {
    fn resolve_due_date(
        &self,
        description: &str,
        now: DateTime<Utc>,
        _max_iterations: Option<u32>,
    ) -> Result<Option<DateTime<Utc>>, FlowableError> {
        // Java parity: an exhausted/end-bounded cycle is `null` (no next fire),
        // an unparseable description is a thrown exception. Conflating both
        // into `Err` made the repeat path retire timers on hard errors.
        match time_source::resolve_cycle(description, now) {
            time_source::CycleResolution::Due(schedule) => Ok(
                DateTime::<Utc>::from_timestamp_millis(schedule.due_time_millis),
            ),
            time_source::CycleResolution::Finished => Ok(None),
            time_source::CycleResolution::Unresolvable => {
                Err(unresolvable(CYCLE_CALENDAR_NAME, description))
            }
        }
    }

    fn resolve_end_date(
        &self,
        description: &str,
        _now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, FlowableError> {
        parse_end_date(CYCLE_CALENDAR_NAME, description)
    }

    fn validate_due_date(
        &self,
        _repeat: &str,
        _max_iterations: Option<u32>,
        end_date: Option<DateTime<Utc>>,
        candidate: DateTime<Utc>,
    ) -> Result<bool, FlowableError> {
        Ok(candidate_within_end(end_date, candidate))
    }
}

/// Engine-local name → calendar map (Java `MapBusinessCalendarManager`).
///
/// Mutation is configuration-time (`&mut self`); lookups clone an `Arc` and
/// release the map before any calendar code runs, so no lock is ever held
/// across user code.
#[derive(Clone)]
pub struct BusinessCalendarRegistry {
    calendars: BTreeMap<String, Arc<dyn BusinessCalendar>>,
}

impl fmt::Debug for BusinessCalendarRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BusinessCalendarRegistry")
            .field("names", &self.names())
            .finish()
    }
}

impl Default for BusinessCalendarRegistry {
    /// Seeds Java's three built-in calendars.
    fn default() -> Self {
        let mut registry = Self::empty();
        registry.replace(DUE_DATE_CALENDAR_NAME, Arc::new(DueDateBusinessCalendar));
        registry.replace(DURATION_CALENDAR_NAME, Arc::new(DurationBusinessCalendar));
        registry.replace(CYCLE_CALENDAR_NAME, Arc::new(CycleBusinessCalendar));
        registry
    }
}

impl BusinessCalendarRegistry {
    /// A registry with no calendars at all. Timer creation against it always
    /// fails with the allowed-names error; use [`Default`] for engine wiring.
    pub fn empty() -> Self {
        Self {
            calendars: BTreeMap::new(),
        }
    }

    /// Register a calendar under a name not already taken.
    ///
    /// Java's `addBusinessCalendar` silently overwrites. Rust rejects the
    /// duplicate so a host cannot shadow a built-in by accident; use
    /// [`Self::replace`] to override deliberately.
    pub fn register(
        &mut self,
        name: &str,
        calendar: Arc<dyn BusinessCalendar>,
    ) -> Result<(), FlowableError> {
        if self.calendars.contains_key(name) {
            return Err(FlowableError::BadRequest(format!(
                "Business calendar '{name}' is already registered; use replace() to override it"
            )));
        }
        self.calendars.insert(name.to_string(), calendar);
        Ok(())
    }

    /// Register or deliberately override a calendar, including a built-in.
    pub fn replace(&mut self, name: &str, calendar: Arc<dyn BusinessCalendar>) {
        self.calendars.insert(name.to_string(), calendar);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn BusinessCalendar>> {
        self.calendars.get(name).cloned()
    }

    /// Java `MapBusinessCalendarManager.getBusinessCalendar`: an unknown name is
    /// a hard error listing the allowed calendars. There is no fallback.
    pub fn require(&self, name: &str) -> Result<Arc<dyn BusinessCalendar>, FlowableError> {
        self.get(name).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Requested business calendar {name} does not exist. Allowed calendars are [{}].",
                self.names().join(", ")
            ))
        })
    }

    /// Registered names in deterministic (sorted) order.
    pub fn names(&self) -> Vec<String> {
        self.calendars.keys().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.calendars.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap()
    }

    #[test]
    fn empty_registry_lists_no_allowed_calendars() {
        let registry = BusinessCalendarRegistry::empty();
        assert!(registry.is_empty());
        let err = registry.require(DURATION_CALENDAR_NAME).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn cycle_calendar_handles_cron() {
        let registry = BusinessCalendarRegistry::default();
        let due = registry
            .require(CYCLE_CALENDAR_NAME)
            .unwrap()
            .resolve_due_date("0 0 * * * ?", now(), None)
            .unwrap()
            .expect("cron always has a next occurrence");
        assert_eq!(due, now() + Duration::hours(1));
    }

    #[test]
    fn cycle_calendar_reports_exhausted_end_bound_as_none_not_error() {
        // `Ok(None)` is Java's `resolveDuedate == null`: the repeat path may
        // legitimately retire the timer, but must never do so on `Err`.
        let registry = BusinessCalendarRegistry::default();
        let calendar = registry.require(CYCLE_CALENDAR_NAME).unwrap();
        let due = calendar
            .resolve_due_date("R5/PT1H/2020-01-01T00:00:00Z", now(), None)
            .unwrap();
        assert_eq!(due, None, "past embedded end bound is not an error");
        let err = calendar
            .resolve_due_date("definitely-not-a-cycle !!", now(), None)
            .unwrap_err();
        assert!(err.to_string().contains("definitely-not-a-cycle"));
    }

    #[test]
    fn due_date_calendar_rejects_a_duration() {
        let registry = BusinessCalendarRegistry::default();
        let err = registry
            .require(DUE_DATE_CALENDAR_NAME)
            .unwrap()
            .resolve_due_date("PT5M", now(), None)
            .unwrap_err();
        assert!(err.to_string().contains("PT5M"));
    }
}
