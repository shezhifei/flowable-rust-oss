# Java / Rust Differential Verification

This directory contains black-box contract fixtures that are executed by both
Flowable Java and Flowable Rust. A fixture is considered compatible only when
the normalized JSON produced by both engines is equal.

## HTTP contract batch

The Java runner is an isolated Maven application pinned to Flowable Java
`8.0.0`, matching the sibling Java checkout used as the implementation
baseline. It does not modify or compile test helpers into the Java repository.

### Adding a new domain

Prefer **operations-scripted** cases so neither the Java runner nor Rust
HTTP helpers need new branches:

```json
{
  "id": "my_case",
  "observeVariables": ["x"],
  "operations": [
    { "op": "deploy", "bpmn": "my.bpmn20.xml" },
    { "op": "start", "variables": { "x": 1 } },
    { "op": "completeTask", "taskDefinitionKey": "review" },
    { "op": "snapshot" }
  ]
}
```

Supported operations: `deploy`, `start`, `completeTask`, `setVariable`,
`setVariableLocal`, `trigger`, `signalEvent`, `claimTask`, `delegateTask`,
`resolveTask`, `executeJobs`, `httpStub`, `snapshot`.

Fixture root fields (`fixedClockMillis`, `automaticLockOwner`, tenants,
`observeVariables`, …) are read by both engines. Per-case
`observeVariables` / `observe` override the root defaults.

Complex HTTP lifecycle modes (`automaticAsyncRetry`, `unlockOwnedJobs`, …)
remain specialized `execution` modes.

The shared inputs are:

- `fixtures/http/cases.json`
- `fixtures/http/request_response.bpmn20.xml`
- `fixtures/http/handled_status.bpmn20.xml`
- `fixtures/http/fail_status.bpmn20.xml`
- `fixtures/http/ignore_exception.bpmn20.xml`
- `fixtures/http/uncaught_handled_status.bpmn20.xml`
- `fixtures/http/async_retry.bpmn20.xml`
- `fixtures/http/automatic_async_retry.bpmn20.xml`
- `fixtures/http/unrecoverable.bpmn20.xml`
- `fixtures/http/nested_unrecoverable.bpmn20.xml`
- `fixtures/http/cancel_job.bpmn20.xml`
- `fixtures/http/smoke_user_task.bpmn20.xml`

Run the real Java-vs-Rust comparison from the Rust workspace root:

```powershell
cargo test -p flowable-engine --test java_http_differential_test -- --ignored --nocapture
```

By default the test locates the Java Maven wrapper at
`../flowable-engine/mvnw.cmd`. To use another Java checkout:

```powershell
$env:FLOWABLE_JAVA_ENGINE_ROOT = 'C:\path\to\flowable-engine'
cargo test -p flowable-engine --test java_http_differential_test -- --ignored --nocapture
```

Normalized diagnostic output is written under `target/differential/`:

- `java-http.json`
- `rust-http.json`

The test is explicitly ignored during ordinary Cargo runs because it requires
a Java checkout, a JDK, Maven dependency resolution, and local process/network
access. It must be run explicitly before claiming parity for the covered HTTP
cases. A missing Java environment is an error during the explicit run; the
runner does not silently skip the comparison.

The Rust-only `httpResult` projection is intentionally not removed or forced
into the Java JSON schema. Java-visible variables are compared as the mandatory
contract, while Rust extensions remain additive and are protected by the
existing Rust extension tests.

The async fixtures additionally compare:

- external HTTP request count;
- per-attempt success/failure;
- executable, timer, consumed, and dead-letter Job state;
- retries and due-date presence;
- persisted Job error message;
- `JOB_EXECUTION_FAILURE`, `JOB_MOVED_TO_DEADLETTER`,
  `ENTITY_UPDATED`, `JOB_RETRIES_DECREMENTED`, and
  `JOB_EXECUTION_SUCCESS` ordering;
- direct dead-letter behavior for an unrecoverable response-handler error;
- nested unrecoverable-cause classification while preserving the outer
  caller-visible and persisted error message, the typed inner cause, and both
  messages in persisted error details;
- automatic async-executor acquisition, lock visibility, retry-timer
  persistence, logical-clock advancement, and automatic second-attempt
  consumption.

For the manual `ManagementService.executeJob` path used by these fixtures, an
unrecoverable failure emits `JOB_EXECUTION_FAILURE`,
`JOB_MOVED_TO_DEADLETTER`, `ENTITY_UPDATED`, then
`JOB_RETRIES_DECREMENTED`. Automatic async-executor acquisition has a distinct
failure path and must be verified by a separate fixture rather than assuming
the same event order.

The automatic async-retry fixture runs in an isolated process engine with a
fixed logical clock, fixed executor lock owner, 5000-millisecond async and
timer locks, and 25-millisecond acquisition polling. A two-stage gated HTTP
server pauses each request after it arrives, allowing the fixture to inspect
the acquired executable Job before releasing the response. The first response
is HTTP 500; after the `R2/PT10S` retry timer becomes visible, the fixture
advances logical time by 10001 milliseconds. It then observes the automatically
acquired second attempt before releasing HTTP 200 and waiting for the `review`
task.

Only relative `retryDelayMillis` and `lockDurationMillis` values are emitted.
Absolute timestamps, generated Job identifiers, and other random values are
excluded. Wall-clock deadlines are deadlock guards only: observable behavior
is coordinated through request-arrived and allow-response latches plus
condition polling, not scheduling sleeps.

The synchronous failure fixtures additionally compare:

- caller-visible `failStatusCodes` errors and transaction rollback;
- `ignoreException` suppression of ordinary HTTP status failures while
  retaining response variables and the Java `ErrorMessage` variable;
- non-suppression of `handleStatusCodes` BPMN errors;
- the exact caller-visible message for an uncaught BPMN error;
- process-instance and active-task state after success or rollback.

The cancellation fixtures compare immediate and transaction lifecycle delivery
for `JOB_CANCELED`, including fatal `COMMITTING` and fatal `COMMITTED`
listeners. Flowable Java emits `ROLLINGBACK` and `ROLLED_BACK` notifications
after a fatal `COMMITTED` listener even though the original database commit has
already completed. The Rust runner therefore verifies both the lifecycle event
sequence and the final persisted Job state, so post-commit notifications cannot
be mistaken for an actual rollback.
