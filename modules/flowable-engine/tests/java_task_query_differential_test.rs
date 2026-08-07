//! Differential verification for task candidate / or() query semantics (B4b).
//!
//! Fixture-driven via `differential/fixtures/task_queries/`.
//! Requires JDK 17+ and the Flowable Java Maven wrapper.
//!
//! Covers P49/P57 parity rules:
//! - candidateUser expands group memberships
//! - candidateOrAssigned covers assigned + candidate
//! - candidate queries exclude assigned tasks by default (T4)
//! - or() block conditions are OR'd and AND'd with main criteria

mod differential;

use differential::{run_differential_suite, run_rust_operations_case};

#[test]
#[ignore = "requires the sibling Flowable Java checkout and its Maven wrapper"]
fn flowable_java_and_rust_match_task_query_contract_fixtures() {
    run_differential_suite(
        "differential/fixtures/task_queries",
        "task-queries",
        |dir, fixture, case| run_rust_operations_case(dir, fixture, case),
    );
}
