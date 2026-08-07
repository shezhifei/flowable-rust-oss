# flowable-rust

An independent Rust reimplementation of the [Flowable](https://github.com/flowable/flowable-engine)
BPMN, CMMN, DMN and Event Registry engines, plus a REST API layer modeled on the
Flowable REST API.

> **Disclaimer:** This project is not affiliated with, endorsed by, or sponsored
> by Flowable AG. "Flowable" is a trademark of Flowable AG. The name is used
> here only to describe compatibility.

## Status

Work in progress. The codebase passes **3255 tests (0 failures)** across the
ten main crates and is behavior-aligned with the Flowable Java engines on the
covered surface. Alignment was driven file-by-file against the Java sources
(every behavioral rule cites the corresponding Java file and line number in
code comments).

## Security hardening (deviations from Java defaults)

A pre-release security audit hardened several dangerous Java-compatible
defaults. Each is an intentional deviation, documented at the implementation
site:

- No default `admin/admin` bootstrap user; startup refuses a blank/default
  password. Privileged REST writes (deployments, `/idm`, `/management`,
  `/cmmn-management`) require an admin from `FLOWABLE_REST_ADMIN_USERS`.
- Shell service tasks are disabled by default (opt-in via engine config).
- Outbound HTTP (HTTP service tasks, event-registry REST channels) denies
  private/loopback/link-local targets by default (SSRF guard); escape hatches
  via `allow_private_networks` / `allowed_private_hosts`.
- Multipart uploads, request bodies and ZIP extraction have cumulative size /
  entry-count limits; expression evaluation has a recursion-depth cap.
- SQL `LIKE` in-memory matching uses a single shared O(pattern × value)
  implementation with a 512-character input cap (`flowable-engine-common`,
  every crate delegates to it); HTTP 500 responses no longer echo internal
  error details.
- `/metrics` requires authentication; `FLOWABLE_REST_AUTH_MODE=disabled`
  refuses to bind non-loopback addresses.

Known deliberate deviations are documented in code comments next to the
implementation (search for `Java ` citations and `P1xx` markers). Notable ones:

- Expression language: read-only JUEL dialect; expression-based variable writes
  are not modeled.
- CMMN historic queries: parameters without a persisted data source return
  HTTP 400 rather than silently no-op'ing (see `docs/runbooks/` and code).
- Event listener / lifecycle listener extension points use a registry of named
  handlers instead of Java class loading / Spring beans.

## Workspace layout

| Crate | Contents |
|---|---|
| `flowable-engine` | BPMN engine (runtime, persistence wiring, jobs, history, mail, task service) |
| `flowable-bpmn-converter` / `flowable-bpmn-model` | BPMN 2.0 XML parsing and model |
| `flowable-cmmn-engine` / `flowable-cmmn-converter` / `flowable-cmmn-model` | CMMN 1.1 engine, XML parsing and model |
| `flowable-dmn-engine` / `flowable-dmn-converter` / `flowable-dmn-model` | DMN engine (FEEL subset), XML parsing and model |
| `flowable-event-registry-service` / `-converter` / `-model` | Event registry (channel/event definitions, consumers) |
| `flowable-rest` | REST API (axum), modeled on the Flowable REST endpoints |
| `flowable-persistence` | Storage abstraction (SQLite in-memory/file; MySQL/PostgreSQL backends) |
| `flowable-app-*`, `flowable-form-service`, `flowable-identity-service`, `flowable-content-service`, `flowable-history-service`, `flowable-http-service`, `flowable-mail-service`, `flowable-task-service`, `flowable-variable-service`, `flowable-image-generator`, `flowable-bpmn-layout`, `flowable-cmmn-image-generator`, `flowable-dmn-image-generator`, `flowable-engine-common`, `flowable-platform-bootstrap` | Supporting services and helpers |

## Build and test

Requires a recent stable Rust toolchain.

```sh
cargo test --workspace
```

Some integration tests exercise MySQL / PostgreSQL backends and are gated on
environment variables (see `docs/runbooks/multi-db-test.md`); they are skipped
or use defaults when the variables are absent.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
