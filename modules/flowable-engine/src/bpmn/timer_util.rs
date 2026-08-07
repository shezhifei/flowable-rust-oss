//! Timer expression evaluation and schedule resolution (Java `TimerUtil` parity).
//!
//! Before scheduling a timer job/subscription, `timeDate` / `timeDuration` /
//! `timeCycle` / `endDate` are evaluated as UEL expressions against the
//! execution variable scope. Evaluation failure or an unresolvable due date
//! raises [`FlowableError`] so the surrounding command rolls back (Java
//! `TimerUtil.createTimerEntity` lines 152–155, 188–191, 218–221).
//!
//! EL evaluation happens **before** P16 `prepare_repeat` / cycle anchoring.

use crate::el::expression::{Expression, SimpleExpression};
use crate::engine::business_calendar::{
    BusinessCalendarRegistry, CYCLE_CALENDAR_NAME, DUE_DATE_CALENDAR_NAME, DURATION_CALENDAR_NAME,
};
use crate::error::FlowableError;
use crate::runtime::execution::Execution;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Resolved timer fields ready for persistence on a job or subscription.
#[derive(Debug, Clone)]
pub struct ResolvedTimerSchedule {
    pub time_date: Option<String>,
    pub time_duration: Option<String>,
    pub time_cycle: Option<String>,
    pub end_date: Option<String>,
    pub due_time: Option<i64>,
    /// Raw `businessCalendarName` / `<calendar>` text exactly as modelled —
    /// literal or `${…}`. ADR-2: the expression is persisted, never the name it
    /// happened to resolve to, so repeat and reschedule re-evaluate it.
    pub calendar_name: Option<String>,
}

/// Default retries for newly created timer jobs
/// (Java `asyncExecutorNumberOfRetries`, default 3).
pub fn default_timer_retries(command_context: &crate::interceptor::command_context::CommandContext) -> Option<i32> {
    Some(
        command_context
            .config
            .async_executor
            .number_of_retries
            .max(0),
    )
}

/// Evaluate a single timer field: literals pass through; `${...}` is UEL.
pub fn evaluate_timer_field(
    raw: &str,
    execution: &Execution,
    field_name: &str,
) -> Result<String, FlowableError> {
    evaluate_timer_field_value(raw, execution, field_name).map(|field| field.text)
}

/// An evaluated timer field plus, when the EL value was *already* an instant
/// rather than a description, that instant.
///
/// Java hands `java.util.Date` / `Instant` / `LocalDateTime` results straight to
/// the timer entity and only calls `businessCalendar.resolveDuedate` for the
/// `String` case (`TimerUtil.java:162-195`). Rust mirrors that: an instant-valued
/// expression never reaches a custom calendar's parser.
#[derive(Debug, Clone)]
struct EvaluatedTimerField {
    text: String,
    instant: Option<DateTime<Utc>>,
}

fn evaluate_timer_field_value(
    raw: &str,
    execution: &Execution,
    field_name: &str,
) -> Result<EvaluatedTimerField, FlowableError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FlowableError::ExecutionError(format!(
            "Timer {field_name} was empty"
        )));
    }

    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        let value = SimpleExpression::new(trimmed.to_string())
            .get_value(execution)
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Timer {field_name} expression '{trimmed}' could not be evaluated"
                ))
            })?;
        timer_value_to_field(&value, field_name)
    } else {
        Ok(EvaluatedTimerField {
            text: trimmed.to_string(),
            instant: None,
        })
    }
}

fn timer_value_to_field(
    value: &Value,
    field_name: &str,
) -> Result<EvaluatedTimerField, FlowableError> {
    match value {
        Value::Null => Err(FlowableError::ExecutionError(format!(
            "Timer {field_name} expression resolved to null"
        ))),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Err(FlowableError::ExecutionError(format!(
                    "Timer {field_name} expression resolved to an empty string"
                )))
            } else {
                Ok(EvaluatedTimerField {
                    text: trimmed.to_string(),
                    instant: None,
                })
            }
        }
        // Java accepts Date/Instant/LocalDate/LocalDateTime; numbers are treated
        // as epoch milliseconds when large enough, otherwise decimal text.
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                // Epoch millis heuristic (≥ ~2001-09-09 in ms).
                if i.abs() >= 1_000_000_000_000 {
                    let dt = DateTime::<Utc>::from_timestamp_millis(i).ok_or_else(|| {
                        FlowableError::ExecutionError(format!(
                            "Timer {field_name} expression resolved to an invalid epoch millis"
                        ))
                    })?;
                    return Ok(EvaluatedTimerField {
                        text: dt.to_rfc3339(),
                        instant: Some(dt),
                    });
                }
                return Ok(EvaluatedTimerField {
                    text: i.to_string(),
                    instant: None,
                });
            }
            Ok(EvaluatedTimerField {
                text: n.to_string(),
                instant: None,
            })
        }
        Value::Bool(b) => Ok(EvaluatedTimerField {
            text: b.to_string(),
            instant: None,
        }),
        Value::Array(_) | Value::Object(_) => Err(FlowableError::ExecutionError(format!(
            "Timer {field_name} was not configured with a valid duration/time \
             (expected String, Date/Instant, or Number)"
        ))),
    }
}

fn evaluate_optional_field(
    raw: Option<&String>,
    execution: &Execution,
    field_name: &str,
) -> Result<Option<EvaluatedTimerField>, FlowableError> {
    match raw {
        Some(v) if !v.trim().is_empty() => {
            Ok(Some(evaluate_timer_field_value(v, execution, field_name)?))
        }
        _ => Ok(None),
    }
}

/// Which modelled field supplies the due-date description, and therefore which
/// built-in calendar Java selects before any `calendarName` override
/// (`TimerUtil.java:130-144`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerFieldKind {
    Date,
    Cycle,
    Duration,
}

impl TimerFieldKind {
    fn default_calendar_name(self) -> &'static str {
        match self {
            Self::Date => DUE_DATE_CALENDAR_NAME,
            Self::Cycle => CYCLE_CALENDAR_NAME,
            Self::Duration => DURATION_CALENDAR_NAME,
        }
    }
}

/// Resolve the calendar a timer must use.
///
/// The raw `calendarName` is evaluated exactly like any other timer field
/// (literal or `${…}`), then looked up in the engine-local registry. An unknown
/// name is a hard error so the surrounding command rolls back — Java
/// `MapBusinessCalendarManager.getBusinessCalendar` throws for the same reason
/// and there is deliberately no fallback (plan §0 non-goals).
pub fn resolve_business_calendar(
    kind_default_name: &str,
    raw_calendar_name: Option<&String>,
    execution: &Execution,
    calendars: &BusinessCalendarRegistry,
) -> Result<
    (
        String,
        std::sync::Arc<dyn crate::engine::business_calendar::BusinessCalendar>,
    ),
    FlowableError,
> {
    let name = match raw_calendar_name {
        Some(raw) if !raw.trim().is_empty() => {
            evaluate_timer_field(raw, execution, "calendarName")?
        }
        _ => kind_default_name.to_string(),
    };
    let calendar = calendars.require(&name)?;
    Ok((name, calendar))
}

/// Java `TimerUtil.calculateMaxIterationsValue`: a counted cycle (`R5/…`)
/// bounds the schedule at 5 iterations; `R/…` and cron expressions are
/// unbounded (Java `Integer.MAX_VALUE` → `None` here).
fn max_iterations_from_cycle(cycle: &str) -> Option<u32> {
    let count = cycle.trim().strip_prefix('R')?.split('/').next()?;
    if count.is_empty() {
        return None;
    }
    count.parse().ok()
}

/// Evaluate EL on all timer fields, resolve the business calendar, then prepare
/// cycle / compute due (Java `TimerUtil.createTimerEntity` + P16 `prepare_repeat`
/// ordering).
///
/// `calendar_name` is the *raw* modelled `businessCalendarName` / `<calendar>`
/// text. It is evaluated here to pick the calendar and returned unevaluated for
/// persistence (ADR-2).
pub fn resolve_timer_schedule(
    time_date: Option<&String>,
    time_duration: Option<&String>,
    time_cycle: Option<&String>,
    end_date: Option<&String>,
    calendar_name: Option<&String>,
    execution: &Execution,
    calendars: &BusinessCalendarRegistry,
    now: DateTime<Utc>,
) -> Result<ResolvedTimerSchedule, FlowableError> {
    // Empty `<timerEventDefinition />` (no timeDate/timeDuration/timeCycle) is a
    // hard configuration error: Java TimerUtil.java:152-155 throws
    // "Timer needs configuration (either timeDate, timeCycle or timeDuration is
    // needed)" when no expression could be built, rolling back the command.
    // Silently inserting a never-firing `due_time = None` job would stall the
    // process instead.
    let has_any = time_date
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        || time_duration.map(|s| !s.trim().is_empty()).unwrap_or(false)
        || time_cycle.map(|s| !s.trim().is_empty()).unwrap_or(false);
    if !has_any {
        return Err(FlowableError::ExecutionError(
            "Timer needs configuration (either timeDate, timeCycle or timeDuration is needed)"
                .to_string(),
        ));
    }

    let eval_date = evaluate_optional_field(time_date, execution, "timeDate")?;
    let eval_duration = evaluate_optional_field(time_duration, execution, "timeDuration")?;
    let eval_cycle = evaluate_optional_field(time_cycle, execution, "timeCycle")?;
    let eval_end = evaluate_optional_field(end_date, execution, "endDate")?;

    // Java precedence for which field supplies the description and therefore the
    // default calendar: timeDate → timeCycle → timeDuration (TimerUtil.java:130-144).
    let (kind, description) = if let Some(field) = eval_date.as_ref() {
        (TimerFieldKind::Date, field)
    } else if let Some(field) = eval_cycle.as_ref() {
        (TimerFieldKind::Cycle, field)
    } else if let Some(field) = eval_duration.as_ref() {
        (TimerFieldKind::Duration, field)
    } else {
        // `has_any` guaranteed one raw field, but EL may legitimately have been
        // configured on a field that evaluated away; treat as unconfigured.
        return Err(FlowableError::ExecutionError(
            "Timer needs configuration (either timeDate, timeCycle or timeDuration is needed)"
                .to_string(),
        ));
    };

    let (_resolved_calendar_name, calendar) = resolve_business_calendar(
        kind.default_calendar_name(),
        calendar_name,
        execution,
        calendars,
    )?;

    // Java `TimerUtil.calculateMaxIterationsValue` runs only for a modelled
    // timeCycle; every other kind has no iteration bound.
    let max_iterations = if kind == TimerFieldKind::Cycle {
        max_iterations_from_cycle(&description.text)
    } else {
        None
    };

    // An already-instant EL value keeps existing Rust instant semantics and is
    // never handed to a calendar parser.
    //
    // Calendar hard errors (`Err`) propagate verbatim and roll back the
    // command; `Ok(None)` is the calendar's legitimate "no fire" answer
    // (e.g. an already-exhausted cycle-embedded end bound) and falls through
    // to the generic no-due-date rejection below.
    let due_time = match description.instant {
        Some(instant) => Some(instant.timestamp_millis()),
        None => match calendar.resolve_due_date(&description.text, now, max_iterations)? {
            Some(due) => {
                // The modelled `endDate` attribute is not visible to
                // `resolveDuedate`, so the calendar validates the candidate
                // against it separately. This reproduces the previous
                // `schedule_cycle(cycle, end_date, now)` rejection. Cycle-embedded
                // end bounds (`R/PT1H/<end>`) are already applied inside the
                // calendar. Java never applies `endDate` to timeDate/timeDuration
                // at creation, so neither does Rust.
                //
                // A non-instant endDate string is resolved by the *selected*
                // calendar (Java TimerUtil `businessCalendar.resolveEndDate`);
                // a resolve failure is a hard error, never a silently
                // dropped bound.
                let end_instant = match eval_end.as_ref() {
                    Some(field) => Some(match field.instant {
                        Some(instant) => instant,
                        None => calendar.resolve_end_date(&field.text, now)?,
                    }),
                    None => None,
                }
                .filter(|_| kind == TimerFieldKind::Cycle);
                if calendar.validate_due_date(
                    &description.text,
                    max_iterations,
                    end_instant,
                    due,
                )? {
                    Some(due.timestamp_millis())
                } else {
                    None
                }
            }
            None => None,
        },
    };

    // Keep a cycle text even when the calendar cannot produce a due yet
    // (mirrors previous `.0.or_else(|| timer_def.time_cycle.clone())` behaviour
    // for prepared vs raw), but never keep an unevaluated `${...}` expression.
    // Java applies `prepareRepeat` whenever timeCycle is modelled, independent of
    // which calendar resolved the due date (TimerUtil.java:224-237).
    let time_cycle_out = eval_cycle
        .as_ref()
        .map(|field| crate::engine::time_source::prepare_repeat(&field.text, now));

    if due_time.is_none() {
        let due_hint = description.text.as_str();
        return Err(FlowableError::ExecutionError(format!(
            "Due date could not be determined for timer job {due_hint}"
        )));
    }

    Ok(ResolvedTimerSchedule {
        time_date: eval_date.map(|field| field.text),
        time_duration: eval_duration.map(|field| field.text),
        time_cycle: time_cycle_out,
        end_date: eval_end.map(|field| field.text),
        due_time,
        calendar_name: calendar_name
            .map(|raw| raw.trim())
            .filter(|raw| !raw.is_empty())
            .map(str::to_string),
    })
}

/// Deploy-time start event variant: same EL rules and the same hard error for
/// an empty timer definition (Java TimerUtil.java:152-155); deployment aborts.
#[allow(clippy::too_many_arguments)]
pub fn resolve_timer_schedule_for_start(
    time_date: Option<&String>,
    time_duration: Option<&String>,
    time_cycle: Option<&String>,
    end_date: Option<&String>,
    calendar_name: Option<&String>,
    execution: &Execution,
    calendars: &BusinessCalendarRegistry,
    now: DateTime<Utc>,
) -> Result<ResolvedTimerSchedule, FlowableError> {
    resolve_timer_schedule(
        time_date,
        time_duration,
        time_cycle,
        end_date,
        calendar_name,
        execution,
        calendars,
        now,
    )
}

/// After a repeating timer fires: prepare the next cycle expression, re-evaluate
/// the raw `calendar_name`, resolve the business calendar, calculate the next due,
/// and validate it (Java `TimerJobEntityManagerImpl.createAndCalculateNextTimer`).
///
/// Returns:
/// - `Ok(None)` only for legitimate exhaustion (`R0`/`R1`), an endDate
///   rejection, or the calendar's own `Ok(None)` "no further fire" answer;
/// - `Err` when the calendar name is unknown or the calendar fails hard — the
///   error propagates so the command rolls back and the job stays in place;
/// - `Ok(Some(schedule))` with the decremented cycle text and calendar-computed due.
///
/// Production call sites must use this instead of
/// [`crate::engine::time_source::reschedule_cycle_after_fire`] so custom calendars
/// participate in every repeat (P64 Task 3 / ADR-2).
pub fn resolve_next_timer_schedule(
    cycle: &str,
    end_date: Option<&str>,
    raw_calendar_name: Option<&String>,
    execution: &Execution,
    calendars: &BusinessCalendarRegistry,
    now: DateTime<Utc>,
) -> Result<Option<crate::engine::time_source::CycleSchedule>, FlowableError> {
    let Some(next_expr) = crate::engine::time_source::next_repeat_expression(cycle) else {
        return Ok(None);
    };

    // Repeats always use the cycle calendar as the kind default; a modelled
    // calendarName still overrides (DefaultJobManager.getBusinessCalendarName
    // defaults to CYCLE_TYPE then evaluates the job's calendarName expression).
    let (_resolved_name, calendar) = resolve_business_calendar(
        CYCLE_CALENDAR_NAME,
        raw_calendar_name,
        execution,
        calendars,
    )?;

    // The decremented cycle text still carries the remaining count
    // (`R2/…` → 2 left), mirroring the maxIterations Java persists on the
    // timer entity at creation.
    let max_iterations = max_iterations_from_cycle(&next_expr);

    // A calendar hard error must fail the fire command (rollback keeps the
    // job for retry); only the calendar's explicit `Ok(None)` — Java
    // `resolveDuedate` returning null — retires the repeat.
    let Some(due) = calendar.resolve_due_date(&next_expr, now, max_iterations)? else {
        return Ok(None);
    };

    // The persisted endDate text is usually an already-resolved instant; any
    // other text re-resolves through the same calendar, and a resolve failure
    // rolls the fire back instead of silently dropping the bound.
    let end_instant = match end_date {
        Some(text) => Some(match crate::engine::time_source::parse_instant(text) {
            Some(instant) => instant,
            None => calendar.resolve_end_date(text, now)?,
        }),
        None => None,
    };
    if !calendar.validate_due_date(&next_expr, max_iterations, end_instant, due)? {
        return Ok(None);
    }

    Ok(Some(crate::engine::time_source::CycleSchedule {
        cycle: next_expr,
        due_time_millis: due.timestamp_millis(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::business_calendar::BusinessCalendar;
    use chrono::Duration;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn exec_with(vars: HashMap<String, Value>) -> Execution {
        Execution {
            variables: vars,
            ..Default::default()
        }
    }

    fn calendars() -> BusinessCalendarRegistry {
        BusinessCalendarRegistry::default()
    }

    /// Always resolves `now + offset`, ignoring the description entirely, so a
    /// test can prove the *custom* calendar was the one consulted.
    #[derive(Debug)]
    struct FixedOffsetCalendar {
        offset_minutes: i64,
    }

    impl BusinessCalendar for FixedOffsetCalendar {
        fn resolve_due_date(
            &self,
            _description: &str,
            now: DateTime<Utc>,
            _max_iterations: Option<u32>,
        ) -> Result<Option<DateTime<Utc>>, FlowableError> {
            Ok(Some(now + Duration::minutes(self.offset_minutes)))
        }

        fn resolve_end_date(
            &self,
            _description: &str,
            now: DateTime<Utc>,
        ) -> Result<DateTime<Utc>, FlowableError> {
            Ok(now + Duration::days(1))
        }

        fn validate_due_date(
            &self,
            _repeat: &str,
            _max_iterations: Option<u32>,
            _end_date: Option<DateTime<Utc>>,
            _candidate: DateTime<Utc>,
        ) -> Result<bool, FlowableError> {
            Ok(true)
        }
    }

    fn with_custom(name: &str, offset_minutes: i64) -> BusinessCalendarRegistry {
        let mut registry = BusinessCalendarRegistry::default();
        registry
            .register(name, Arc::new(FixedOffsetCalendar { offset_minutes }))
            .unwrap();
        registry
    }

    #[test]
    fn literal_duration_passes_through() {
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let s = resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            None,
            None,
            &exec,
            &calendars(),
            now,
        )
        .unwrap();
        assert_eq!(s.time_duration.as_deref(), Some("PT5M"));
        assert!(s.due_time.is_some());
        assert_eq!(s.calendar_name, None);
    }

    #[test]
    fn expression_duration_evaluates() {
        let mut vars = HashMap::new();
        vars.insert("duration".to_string(), Value::String("PT10M".to_string()));
        let exec = exec_with(vars);
        let now = Utc::now();
        let s = resolve_timer_schedule(
            None,
            Some(&"${duration}".to_string()),
            None,
            None,
            None,
            &exec,
            &calendars(),
            now,
        )
        .unwrap();
        assert_eq!(s.time_duration.as_deref(), Some("PT10M"));
        assert!(s.due_time.is_some());
    }

    #[test]
    fn missing_variable_is_hard_error() {
        let exec = exec_with(HashMap::new());
        let err = resolve_timer_schedule(
            None,
            Some(&"${missing}".to_string()),
            None,
            None,
            None,
            &exec,
            &calendars(),
            Utc::now(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("could not be evaluated")
                || err.to_string().contains("null"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn empty_timer_definition_is_hard_error() {
        // Java TimerUtil.java:152-155: no timeDate/timeCycle/timeDuration →
        // FlowableException "Timer needs configuration (either timeDate,
        // timeCycle or timeDuration is needed)".
        let exec = exec_with(HashMap::new());
        let err = resolve_timer_schedule(
            None,
            None,
            None,
            None,
            None,
            &exec,
            &calendars(),
            Utc::now(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(
                "Timer needs configuration (either timeDate, timeCycle or timeDuration is needed)"
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn string_literal_expression_for_start_date() {
        let exec = exec_with(HashMap::new());
        let s = resolve_timer_schedule(
            Some(&"${'2036-11-14T11:12:22Z'}".to_string()),
            None,
            None,
            None,
            None,
            &exec,
            &calendars(),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(s.time_date.as_deref(), Some("2036-11-14T11:12:22Z"));
        assert!(s.due_time.is_some());
    }

    #[test]
    fn end_date_expression_evaluates() {
        let mut vars = HashMap::new();
        vars.insert(
            "EndDateForBoundary".to_string(),
            Value::String("2030-01-01T00:00:00Z".to_string()),
        );
        let exec = exec_with(vars);
        let s = resolve_timer_schedule(
            None,
            None,
            Some(&"R5/PT1H".to_string()),
            Some(&"${EndDateForBoundary}".to_string()),
            None,
            &exec,
            &calendars(),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(s.end_date.as_deref(), Some("2030-01-01T00:00:00Z"));
        assert!(s.time_cycle.is_some());
        assert!(s.due_time.is_some());
    }

    #[test]
    fn literal_custom_calendar_overrides_the_default_kind_calendar() {
        // TimerUtil.java:145-148 — a modelled businessCalendarName replaces the
        // kind-derived name, so `PT5M` never reaches the duration parser.
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let s = resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            None,
            Some(&"custom".to_string()),
            &exec,
            &with_custom("custom", 42),
            now,
        )
        .unwrap();
        assert_eq!(
            s.due_time,
            Some((now + Duration::minutes(42)).timestamp_millis()),
            "the custom calendar, not `duration`, must have resolved the due date"
        );
        // ADR-2: the raw expression is persisted, never the resolved name.
        assert_eq!(s.calendar_name.as_deref(), Some("custom"));
    }

    #[test]
    fn calendar_name_expression_is_evaluated_but_persisted_raw() {
        let mut vars = HashMap::new();
        vars.insert(
            "calendarSelector".to_string(),
            Value::String("custom".to_string()),
        );
        let exec = exec_with(vars);
        let now = Utc::now();
        let s = resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            None,
            Some(&"${calendarSelector}".to_string()),
            &exec,
            &with_custom("custom", 11),
            now,
        )
        .unwrap();
        assert_eq!(
            s.due_time,
            Some((now + Duration::minutes(11)).timestamp_millis())
        );
        assert_eq!(
            s.calendar_name.as_deref(),
            Some("${calendarSelector}"),
            "ADR-2: persist the expression, not the name it resolved to"
        );
    }

    #[test]
    fn unknown_calendar_name_is_a_hard_error_listing_allowed_names() {
        // MapBusinessCalendarManager.java:41-46 — no silent fallback to the
        // kind default; the command rolls back.
        let exec = exec_with(HashMap::new());
        let err = resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            None,
            Some(&"missingCalendar".to_string()),
            &exec,
            &calendars(),
            Utc::now(),
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("missingCalendar"), "unexpected error: {text}");
        assert!(
            text.contains("duration") && text.contains("dueDate") && text.contains("cycle"),
            "unexpected error: {text}"
        );
    }

    #[test]
    fn time_date_wins_over_time_cycle_for_calendar_selection() {
        // TimerUtil.java:130-144 — timeDate is checked first, so the `dueDate`
        // calendar is selected even when timeCycle is also modelled.
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let s = resolve_timer_schedule(
            Some(&"2036-11-14T11:12:22Z".to_string()),
            None,
            Some(&"R5/PT1H".to_string()),
            None,
            None,
            &exec,
            &calendars(),
            now,
        )
        .unwrap();
        let expected = crate::engine::time_source::parse_instant("2036-11-14T11:12:22Z").unwrap();
        assert_eq!(s.due_time, Some(expected.timestamp_millis()));
        // The cycle is still prepared and persisted for the repeat.
        assert!(s.time_cycle.is_some());
    }

    #[test]
    fn instant_valued_expression_bypasses_the_calendar() {
        // TimerUtil.java:162-195 — a Date/Instant EL result is used verbatim and
        // never handed to resolveDuedate, so even a hostile custom calendar
        // cannot change it.
        let mut vars = HashMap::new();
        let instant = Utc::now() + Duration::days(3);
        vars.insert(
            "when".to_string(),
            Value::Number(instant.timestamp_millis().into()),
        );
        let exec = exec_with(vars);
        let s = resolve_timer_schedule(
            Some(&"${when}".to_string()),
            None,
            None,
            None,
            Some(&"custom".to_string()),
            &exec,
            &with_custom("custom", 999),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(s.due_time, Some(instant.timestamp_millis()));
    }

    #[test]
    fn cycle_end_date_still_blocks_a_due_beyond_it() {
        // Pre-existing Rust behaviour retained: the modelled endDate attribute
        // rejects a cycle whose first fire is already past it.
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let end = (now + Duration::minutes(30)).to_rfc3339();
        let err = resolve_timer_schedule(
            None,
            None,
            Some(&"R5/PT1H".to_string()),
            Some(&end),
            None,
            &exec,
            &calendars(),
            now,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("Due date could not be determined"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn end_date_does_not_block_a_duration_timer() {
        // Java applies endDate only through the cycle path at creation time.
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let end = (now + Duration::minutes(1)).to_rfc3339();
        let s = resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            Some(&end),
            None,
            &exec,
            &calendars(),
            now,
        )
        .unwrap();
        assert_eq!(
            s.due_time,
            Some((now + Duration::minutes(5)).timestamp_millis())
        );
    }

    #[test]
    fn start_variant_shares_calendar_resolution() {
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let s = resolve_timer_schedule_for_start(
            None,
            None,
            Some(&"R/PT1H".to_string()),
            None,
            Some(&"custom".to_string()),
            &exec,
            &with_custom("custom", 15),
            now,
        )
        .unwrap();
        assert_eq!(
            s.due_time,
            Some((now + Duration::minutes(15)).timestamp_millis())
        );
        assert_eq!(s.calendar_name.as_deref(), Some("custom"));
        // prepare_repeat still anchors the persisted cycle.
        assert!(
            s.time_cycle.as_deref().unwrap().starts_with("R/"),
            "unexpected cycle: {:?}",
            s.time_cycle
        );
    }

    /// Fails `resolve_due_date` outright, or returns `Ok(None)`, so tests can
    /// prove the distinction between a hard error and "no further fire".
    #[derive(Debug)]
    struct BrokenCalendar {
        message: &'static str,
        /// `true` → `Ok(None)` instead of `Err`.
        soft_none: bool,
    }

    impl BusinessCalendar for BrokenCalendar {
        fn resolve_due_date(
            &self,
            _description: &str,
            _now: DateTime<Utc>,
            _max_iterations: Option<u32>,
        ) -> Result<Option<DateTime<Utc>>, FlowableError> {
            if self.soft_none {
                Ok(None)
            } else {
                Err(FlowableError::ExecutionError(self.message.to_string()))
            }
        }

        fn resolve_end_date(
            &self,
            _description: &str,
            now: DateTime<Utc>,
        ) -> Result<DateTime<Utc>, FlowableError> {
            Ok(now)
        }

        fn validate_due_date(
            &self,
            _repeat: &str,
            _max_iterations: Option<u32>,
            _end_date: Option<DateTime<Utc>>,
            _candidate: DateTime<Utc>,
        ) -> Result<bool, FlowableError> {
            Ok(true)
        }
    }

    fn with_broken(name: &str, message: &'static str, soft_none: bool) -> BusinessCalendarRegistry {
        let mut registry = BusinessCalendarRegistry::default();
        registry
            .register(name, Arc::new(BrokenCalendar { message, soft_none }))
            .unwrap();
        registry
    }

    #[test]
    fn initial_calendar_error_propagates_verbatim() {
        // The original calendar error must reach the caller (rollback), not be
        // replaced by the generic "Due date could not be determined" message.
        let exec = exec_with(HashMap::new());
        let err = resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            None,
            Some(&"broken".to_string()),
            &exec,
            &with_broken("broken", "shift roster service unavailable", false),
            Utc::now(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("shift roster service unavailable"),
            "unexpected error: {err}"
        );
        assert!(
            !err.to_string().contains("Due date could not be determined"),
            "original calendar error must not be masked: {err}"
        );
    }

    #[test]
    fn repeat_calendar_error_propagates_instead_of_retiring_the_timer() {
        // Contract: `Err` from the calendar must fail the fire command so the
        // job rolls back and retries — it must never be read as exhaustion.
        let exec = exec_with(HashMap::new());
        let err = resolve_next_timer_schedule(
            "R3/PT10M",
            None,
            Some(&"broken".to_string()),
            &exec,
            &with_broken("broken", "transient dependency failure", false),
            Utc::now(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("transient dependency failure"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn repeat_calendar_ok_none_retires_the_timer_legitimately() {
        let exec = exec_with(HashMap::new());
        let next = resolve_next_timer_schedule(
            "R3/PT10M",
            None,
            Some(&"soft".to_string()),
            &exec,
            &with_broken("soft", "", true),
            Utc::now(),
        )
        .unwrap();
        assert!(next.is_none(), "calendar Ok(None) is a legitimate stop");
    }

    #[test]
    fn max_iterations_from_cycle_matches_java_calculate_max_iterations_value() {
        assert_eq!(max_iterations_from_cycle("R5/PT1H"), Some(5));
        assert_eq!(
            max_iterations_from_cycle(" R12/PT1H/2030-01-01T00:00:00Z "),
            Some(12)
        );
        assert_eq!(
            max_iterations_from_cycle("R/PT1H"),
            None,
            "uncounted repeat is unbounded"
        );
        assert_eq!(
            max_iterations_from_cycle("0 0 * * * ?"),
            None,
            "cron is unbounded"
        );
    }

    /// Records the `max_iterations` and end bound each trait call receives so
    /// tests can prove the production wiring; `end_offset_minutes: None`
    /// hard-fails `resolve_end_date`.
    #[derive(Debug)]
    struct CapturingCalendar {
        due_offset_minutes: i64,
        end_offset_minutes: Option<i64>,
        resolve_max: std::sync::Mutex<Vec<Option<u32>>>,
        validate_seen: std::sync::Mutex<Vec<(Option<u32>, Option<DateTime<Utc>>)>>,
    }

    impl CapturingCalendar {
        fn new(due_offset_minutes: i64, end_offset_minutes: Option<i64>) -> Self {
            Self {
                due_offset_minutes,
                end_offset_minutes,
                resolve_max: std::sync::Mutex::new(Vec::new()),
                validate_seen: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl BusinessCalendar for CapturingCalendar {
        fn resolve_due_date(
            &self,
            _description: &str,
            now: DateTime<Utc>,
            max_iterations: Option<u32>,
        ) -> Result<Option<DateTime<Utc>>, FlowableError> {
            self.resolve_max.lock().unwrap().push(max_iterations);
            Ok(Some(now + Duration::minutes(self.due_offset_minutes)))
        }

        fn resolve_end_date(
            &self,
            description: &str,
            now: DateTime<Utc>,
        ) -> Result<DateTime<Utc>, FlowableError> {
            match self.end_offset_minutes {
                Some(offset) => Ok(now + Duration::minutes(offset)),
                None => Err(FlowableError::ExecutionError(format!(
                    "shift roster cannot resolve end '{description}'"
                ))),
            }
        }

        fn validate_due_date(
            &self,
            _repeat: &str,
            max_iterations: Option<u32>,
            end_date: Option<DateTime<Utc>>,
            candidate: DateTime<Utc>,
        ) -> Result<bool, FlowableError> {
            self.validate_seen
                .lock()
                .unwrap()
                .push((max_iterations, end_date));
            if let Some(end) = end_date {
                return Ok(candidate <= end);
            }
            Ok(true)
        }
    }

    fn with_capturing(
        name: &str,
        calendar: Arc<CapturingCalendar>,
    ) -> BusinessCalendarRegistry {
        let mut registry = BusinessCalendarRegistry::default();
        registry.register(name, calendar).unwrap();
        registry
    }

    #[test]
    fn cycle_creation_passes_the_iteration_bound_to_the_calendar() {
        // Java TimerUtil hands calculateMaxIterationsValue(timeCycle) to both
        // resolveDuedate and validateDuedate; Rust must not pin it to None.
        let calendar = Arc::new(CapturingCalendar::new(30, Some(600)));
        let exec = exec_with(HashMap::new());
        resolve_timer_schedule(
            None,
            None,
            Some(&"R5/PT10M".to_string()),
            None,
            Some(&"capture".to_string()),
            &exec,
            &with_capturing("capture", Arc::clone(&calendar)),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(*calendar.resolve_max.lock().unwrap(), vec![Some(5)]);
        assert_eq!(calendar.validate_seen.lock().unwrap()[0].0, Some(5));
    }

    #[test]
    fn non_cycle_creation_has_no_iteration_bound() {
        let calendar = Arc::new(CapturingCalendar::new(30, Some(600)));
        let exec = exec_with(HashMap::new());
        resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            None,
            Some(&"capture".to_string()),
            &exec,
            &with_capturing("capture", Arc::clone(&calendar)),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(*calendar.resolve_max.lock().unwrap(), vec![None]);
    }

    #[test]
    fn non_instant_end_date_is_resolved_by_the_selected_calendar() {
        // Java TimerUtil: a String endDate goes through the *selected*
        // calendar's resolveEndDate, never a hardwired instant parser.
        let calendar = Arc::new(CapturingCalendar::new(30, Some(600)));
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let s = resolve_timer_schedule(
            None,
            None,
            Some(&"R5/PT10M".to_string()),
            Some(&"shift-close".to_string()),
            Some(&"capture".to_string()),
            &exec,
            &with_capturing("capture", Arc::clone(&calendar)),
            now,
        )
        .unwrap();
        assert_eq!(s.end_date.as_deref(), Some("shift-close"));
        assert_eq!(
            calendar.validate_seen.lock().unwrap()[0].1,
            Some(now + Duration::minutes(600)),
            "validate must see the calendar-resolved end bound"
        );
    }

    #[test]
    fn end_date_resolve_error_propagates_at_creation() {
        let calendar = Arc::new(CapturingCalendar::new(30, None));
        let exec = exec_with(HashMap::new());
        let err = resolve_timer_schedule(
            None,
            None,
            Some(&"R5/PT10M".to_string()),
            Some(&"shift-close".to_string()),
            Some(&"capture".to_string()),
            &exec,
            &with_capturing("capture", Arc::clone(&calendar)),
            Utc::now(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("shift roster cannot resolve end 'shift-close'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn unparseable_end_date_is_a_hard_error_with_built_in_calendars() {
        // Previously silently dropped: `parse_instant` failed and the bound
        // vanished. Java resolveEndDate throws instead.
        let exec = exec_with(HashMap::new());
        let err = resolve_timer_schedule(
            None,
            Some(&"PT5M".to_string()),
            None,
            Some(&"not-a-date".to_string()),
            None,
            &exec,
            &calendars(),
            Utc::now(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("could not resolve end date from 'not-a-date'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn repeat_passes_the_decremented_iteration_bound_and_calendar_end_date() {
        let calendar = Arc::new(CapturingCalendar::new(30, Some(600)));
        let exec = exec_with(HashMap::new());
        let now = Utc::now();
        let next = resolve_next_timer_schedule(
            "R3/PT10M",
            Some("shift-close"),
            Some(&"capture".to_string()),
            &exec,
            &with_capturing("capture", Arc::clone(&calendar)),
            now,
        )
        .unwrap()
        .expect("within the calendar-resolved end bound");
        assert!(next.cycle.starts_with("R2/"), "unexpected cycle: {}", next.cycle);
        assert_eq!(
            *calendar.resolve_max.lock().unwrap(),
            vec![Some(2)],
            "the repeat bound is the remaining count"
        );
        assert_eq!(
            calendar.validate_seen.lock().unwrap()[0],
            (Some(2), Some(now + Duration::minutes(600))),
            "validate must see the remaining count and the calendar-resolved end"
        );
    }

    #[test]
    fn repeat_end_date_resolve_error_propagates() {
        let calendar = Arc::new(CapturingCalendar::new(30, None));
        let exec = exec_with(HashMap::new());
        let err = resolve_next_timer_schedule(
            "R3/PT10M",
            Some("shift-close"),
            Some(&"capture".to_string()),
            &exec,
            &with_capturing("capture", Arc::clone(&calendar)),
            Utc::now(),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("shift roster cannot resolve end 'shift-close'"),
            "unexpected error: {err}"
        );
    }
}
