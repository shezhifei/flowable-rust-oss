//! Differential verification for timer/job lifecycle basics.
//!
//! Fixture-driven via `differential/fixtures/timers/`.
//! Requires JDK 17+ and the Flowable Java Maven wrapper.
//!
//! Dual-node exclusive-job locking (P48) is intentionally out of scope here
//! (marked M difficulty in the plan); record as remaining work.

mod differential;

use differential::{run_differential_suite, run_rust_operations_case};

#[test]
#[ignore = "requires the sibling Flowable Java checkout and its Maven wrapper"]
fn flowable_java_and_rust_match_timer_contract_fixtures() {
    run_differential_suite("differential/fixtures/timers", "timers", |dir, fixture, case| {
        run_rust_operations_case(dir, fixture, case)
    });
}
