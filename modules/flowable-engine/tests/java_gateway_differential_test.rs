//! Differential verification for gateway join / exclusive-select semantics.
//!
//! Fixture-driven via `differential/fixtures/gateways/` operations scripts.
//! Requires JDK 17+ and the Flowable Java Maven wrapper.

mod differential;

use differential::{run_differential_suite, run_rust_operations_case};

#[test]
#[ignore = "requires the sibling Flowable Java checkout and its Maven wrapper"]
fn flowable_java_and_rust_match_gateway_contract_fixtures() {
    run_differential_suite("differential/fixtures/gateways", "gateways", |dir, fixture, case| {
        run_rust_operations_case(dir, fixture, case)
    });
}
