# Java 8 error handling parity verification — Implementation Plan

**Goal:** Complete the existing Rust/Java Flowable 8 error handling comparison and fix every verified inappropriate unwrap or error fallback, without treating a passing lint check as proof of semantic parity.

**Architecture:** Preserve the existing converter, engine, persistence, and REST boundaries. Java exceptions correspond to Rust typed errors propagated through command/transaction boundaries; valid nulls and documented recovery paths remain successful values. Interpreter control flow must be represented independently of execution errors.

**Tech Stack:** Rust workspace (36 crates), Java Flowable 8 source baseline, Cargo/Clippy, GitNexus CLI on Windows.

## Current evidence and scope

- Source baseline: Rust `6d7a8dc`, Java `C:/flowable/flowable-engine-8`.
- The September 29 final report is not completion evidence: it leaves listener and CMMN semantics unresolved.
- Fresh Clippy with `--force-warn` overrides local allowances and finds 45 benchmark unwraps, one benchmark expect, four library expects, and a Tokio macro-generated expect. The strict lib+bins check fails on the benchmark expect. A Tokio-generated expect is not an explicit source call.
- GitNexus incremental refresh produced invalid Function FTS data and mismatched symbol identities. Repair failed; full rebuild is in progress. Do not infer safety from empty results.
- The scripting interpreter uses `ExecutionError("__RETURN__:...")` as return control flow. `call_user_function` tests `contains` and deserializes with `unwrap_or(Null)`, so an actual error containing the marker becomes a successful null. This is a verified new candidate, not covered by earlier bare-unwrap checks.

## Execution sequence

1. Rebuild and validate the Rust GitNexus index; bind every graph query explicitly. Run impact for each existing symbol before editing and record callers/risk. Use source searches to resolve empty/UNKNOWN walks.
2. Add a regression proving genuine script failures containing the return marker stay errors at the interpreter and BPMN command boundary. Include positive return tests for nested blocks/loops and structured values.
3. Replace the return-marker encoding and JSON fallback with typed `ControlFlow` inside the evaluator. Keep `Result` exclusively for execution failures; preserve the public engine API. Confirm that command failure rolls back state.
4. Continue the full inventory: review every production unwrap/expect allowance and fallible fallback, compare Java source, and distinguish absence/defaulting from failed evaluation/I/O. Audit outstanding listener, CMMN, historical migration, and REST findings instead of silently excluding them.
5. Run affected regression tests, then workspace tests and production Clippy. Capture exact commands, current source identity, and exit codes. Tests/benchmark assertions and macro expansions must be reported separately from service code.
6. Update the parity report and completion ledger from current evidence. Run GitNexus detect-changes for the actual Rust checkout; partial/truncated results remain unresolved. Do not mark the goal complete while a requirement or finding lacks proof.

## First fix: files and validation

- Modify `modules/flowable-engine/src/scripting/evaluator.rs`: `execute`, `execute_statement`, `execute_block`, `call_user_function`; remove `return_signal_value` after impact analysis.
- Add `modules/flowable-engine/tests/script_error_propagation_test.rs`.
- Java evidence: `ScriptingEngines.evaluate` and `ScriptTaskActivityBehavior.safelyExecuteScript` propagate script failures independently of successful script results.
- Red/green command: `cargo test -p flowable-engine --test script_error_propagation_test`.
- Related tests: `cargo test -p flowable-engine --test script_task_semantics_test --test p1_1_write_result_and_t4_delegate_result_test` plus existing scripting interpreter suites identified by Cargo.

## Completion status

All steps are done: see `C:/flowable/docs/parity/rust-unwrap-vs-java8-misalignment.md` §12.9 (R6-1 … R6-8, keep-list, out-of-scope, verification). Full `cargo test -j 4 --workspace --no-fail-fast`: 3941 passed / 1 failed / 16 ignored. The single failure is a flaky SQLite table-lock race in `async_executor_auto_activate_runs_without_manual_start`. It reproduces on unmodified `HEAD` (1 in 40 runs), so it predates R6 and is tracked separately.
