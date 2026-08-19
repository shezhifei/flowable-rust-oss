# flowable-rust

An independent Rust reimplementation of the [Flowable](https://github.com/flowable/flowable-engine)
BPMN, CMMN, DMN and Event Registry engines, plus a REST API layer modeled on the
Flowable REST API.

> **Disclaimer:** This project is not affiliated with, endorsed by, or sponsored
> by Flowable AG. "Flowable" is a trademark of Flowable AG. The name is used
> here only to describe compatibility.

## Status

Work in progress. `cargo test --workspace` passes **3605 tests (0 failures, 16
ignored)** across the 33 crates, and the port is behavior-aligned with the
Flowable Java engines on the covered surface. The ignored tests are gated on
external services rather than broken — see
[Build and test](#build-and-test). Alignment was driven file-by-file against the Java sources
(every behavioral rule cites the corresponding Java file and line number in
code comments).

## Security hardening (deviations from Java defaults)

A pre-release security audit hardened several dangerous Java-compatible
defaults. Each is an intentional deviation, documented at the implementation
site:

- Passwords are stored as argon2id digests (m=19MiB, t=2, p=1) rather than the
  Java plaintext default; verification is constant-time. Pre-existing plaintext
  rows still authenticate and are upgraded to a digest the next time the user is
  saved, so deployers upgrading an existing database should force a password
  reset to retire the plaintext values. REST user responses never echo the
  password field.
- No default `admin/admin` bootstrap user; startup refuses a blank/default
  password. Privileged REST writes (deployments, `/idm`, `/management`,
  `/cmmn-management`, `/event-registry-management`, `/app-management`,
  `/dmn-management`, `/idm-management`) require an admin from
  `FLOWABLE_REST_ADMIN_USERS`.
- Failed Basic-auth attempts are rate-limited per client IP (30 failures per
  5 minutes → HTTP 429). The key is the TCP peer address, so behind a reverse
  proxy all clients share one bucket — terminate rate limiting at the proxy in
  that topology.
- XML element nesting is capped before parsing, so hostile documents cannot
  drive a parser or converter into stack overflow: 512 levels for BPMN
  (`quick-xml`, an iterative pull parser), and 64 for CMMN/DMN, whose parser
  (`roxmltree`) recurses per element and overflows well below 512 on a debug
  build. Both caps are enforced by an iterative pre-parse scan — a post-parse
  check cannot help, because the overflow happens during parsing. CMMN/DMN also
  carry a 1M node budget. DTDs are refused (no XXE, no entity expansion).
- Shell service tasks are disabled by default (opt-in via engine config).
- Outbound HTTP (HTTP service tasks, event-registry REST channels) denies
  private/loopback/link-local targets by default (SSRF guard); escape hatches
  via `allow_private_networks` / `allowed_private_hosts`.
- Multipart uploads, request bodies and ZIP extraction have cumulative size /
  entry-count limits; expression evaluation has a recursion-depth cap.
- SQL `LIKE` in-memory matching uses a single shared O(pattern × value)
  implementation with a 512-character input cap (`flowable-engine-common::like`);
  every crate that implements `*Like` query semantics delegates to it, replacing
  earlier per-crate copies that could backtrack exponentially or allocate an
  O(n×m) matrix. Three REST filter helpers (DMN decision/deployment listing and
  app-definition listing) keep their own narrower prefix/suffix/substring
  matching: they are linear-time and carry no DoS exposure, but they are not
  SQL-LIKE-complete (`_` is literal, a mid-pattern `%` is not a wildcard).
- HTTP 500 responses no longer echo internal error details.
- `/metrics` requires authentication; `FLOWABLE_REST_AUTH_MODE=disabled`
  refuses to bind non-loopback addresses.

Known deliberate deviations are documented in code comments next to the
implementation (search for `Java ` citations and `P1xx` markers). Notable ones:

- Expression language: read-only JUEL dialect; expression-based variable writes
  are not modeled.
- CMMN historic queries: parameters without a persisted data source return
  HTTP 400 rather than silently no-op'ing (documented at the handler).
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

## UI apps

Four web apps ship with the server, mounted by `flowable-ui-rest` alongside the
engine REST API:

| App | Browser entry | REST surface | Frontend |
|---|---|---|---|
| Task | `/` | `/app/rest/**` | Legacy AngularJS bundle (`ui/legacy/task`) |
| IDM | `/idm/` | `/idm-app/**` | Legacy AngularJS bundle (`ui/legacy/idm`) |
| Admin | `/admin/` | `/admin-app/**` | Legacy AngularJS bundle (`ui/legacy/admin`) |
| Modeler | `/modeler-app/` | `/modeler-app/rest/**` | Self-developed React/TypeScript (`ui/modeler`) |

- The legacy bundles are served unmodified from `ui/legacy` (override with
  `FLOWABLE_UI_STATIC_DIR`). The modeler is a from-scratch React app — no Oryx,
  no bpmn-js — built with `cd ui/modeler && npm install && npm run build` and
  served from `ui/modeler/dist` (override with `FLOWABLE_MODELER_STATIC_DIR`).
  Its dependency allowlist lives in [ui/modeler/README.md](ui/modeler/README.md).
- Sign-in mirrors the Java form login: POST `j_username`/`j_password` to
  `/app/authentication` (session cookie, remember-me supported); `/app/logout`
  ends the session. Accounts live in the engine identity service. No default
  user is created: opt in at first boot with
  `FLOWABLE_BOOTSTRAP_CREATE_DEFAULT_ADMIN=true` plus
  `FLOWABLE_BOOTSTRAP_ADMIN_USER_ID` / `FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD`.
- `FLOWABLE_UI_AUTH_MODE=disabled` switches session enforcement off for
  development; requests then run as `FLOWABLE_UI_DEV_USER` (default `admin`).
- The admin app proxies engine REST calls per server-config; connection
  defaults come from `FLOWABLE_UI_ENGINE_HOST` / `FLOWABLE_UI_ENGINE_PORT` /
  `FLOWABLE_UI_ENGINE_USER` / `FLOWABLE_UI_ENGINE_PASSWORD`, with durable edits
  stored at `FLOWABLE_UI_SERVER_CONFIG_PATH`.
- Endpoint coverage against the Flowable 6.8 Java UI apps is audited in
  `docs/plans/ui-migration-coverage.md` (sibling checkout), and the operational
  guide is [docs/runbooks/ui-run.md](docs/runbooks/ui-run.md).

## Build and test

Requires Rust **1.85 or newer** (the workspace uses edition 2024).

```sh
cargo test --workspace
```

The default run needs no external services — SQLite is bundled. Two groups sit
outside it:

- **MySQL / PostgreSQL backend suites** are gated on `FLOWABLE_TEST_MYSQL_URL` /
  `FLOWABLE_TEST_POSTGRES_URL`; without those they fall back to defaults rather
  than failing. The whole-server boot and UI smoke suites
  (`--features mysql` / `--features postgres`) additionally skip and pass when
  the database is unreachable. MySQL live smoke has never been run against a
  real instance — the adaptation and the suites are in place, but no instance was
  available. See [docs/runbooks/multi-db-test.md](docs/runbooks/multi-db-test.md).
- **The 16 `#[ignore]`d tests** need something the repo cannot assume: the
  Java-vs-Rust differential fixtures require a Flowable Java checkout with a JDK
  and Maven (see [differential/README.md](differential/README.md)), and the
  live HTTP-client tests require outbound network access. Run them with
  `cargo test --workspace -- --ignored` once those are available.

## Security

To report a vulnerability, and for the current list of known security
limitations (authorization granularity, tenant isolation, authentication cost),
see [SECURITY.md](SECURITY.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
