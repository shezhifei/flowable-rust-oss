# Running the Flowable UI Apps

From a fresh source checkout to all four web apps usable against the Rust
server. Apps and configuration knobs are summarized in the
[README UI chapter](../README.md#ui-apps); endpoint coverage against the Java
6.8 UI apps is audited in `docs/plans/ui-migration-coverage.md` (sibling
checkout).

## 1. Build the modeler frontend

The legacy idm/admin/task bundles ship prebuilt under `ui/legacy/`; only the
self-developed modeler needs a build:

```sh
cd ui/modeler
npm install
npm run build        # emits ui/modeler/dist
```

Requires Node.js (18+). Other useful scripts: `npm run dev` (Vite dev server),
`npm test` (vitest), `npm run lint`, `npm run test:e2e` (regenerates render
fixtures, builds with `--mode e2e`, then runs Playwright — the e2e build is
what mounts the test harness, so a plain `npm run build` dist makes the
fixture specs fail with "Modeler E2E harness is unavailable").

## 2. Start the server with a bootstrap admin

No default user exists. Create the first admin at boot:

```sh
FLOWABLE_BOOTSTRAP_CREATE_DEFAULT_ADMIN=true \
FLOWABLE_BOOTSTRAP_ADMIN_USER_ID=admin \
FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD=<pick-one> \
cargo run -p flowable-rest
```

The server binds `FLOWABLE_SERVER_BIND_ADDRESS` (default `0.0.0.0:8080`-style;
see the printed `Listening on …` line). Static roots resolve automatically
(`ui/legacy` and `ui/modeler/dist` relative to the working directory — run
from the repo root, or set `FLOWABLE_UI_STATIC_DIR` /
`FLOWABLE_MODELER_STATIC_DIR`).

## 3. Open the apps

| App | URL |
|---|---|
| Task (landing) | `http://localhost:8080/` |
| IDM (users/groups/privileges) | `http://localhost:8080/idm/` |
| Admin (engine monitoring) | `http://localhost:8080/admin/` |
| Modeler (BPMN/DMN/form editors) | `http://localhost:8080/modeler-app/` |

Sign in with the bootstrap account. The admin app requires the `access-admin`
privilege (granted via IDM → privileges); IDM administration requires
`access-idm`.

For development without sign-in, start the server with
`FLOWABLE_UI_AUTH_MODE=disabled` — every request then runs as
`FLOWABLE_UI_DEV_USER` (default `admin`) with all app privileges. Do not use
this outside local development.

## 4. Point the admin app at the engine

The admin app proxies engine REST calls through server-configs. In the
single-binary deployment the defaults already match; overrides:

- `FLOWABLE_UI_ENGINE_HOST` / `FLOWABLE_UI_ENGINE_PORT` /
  `FLOWABLE_UI_ENGINE_USER` / `FLOWABLE_UI_ENGINE_PASSWORD` — connection
  defaults.
- `FLOWABLE_UI_SERVER_CONFIG_PATH` — where durable edits made in the admin UI
  are stored (JSON file).

## 5. Database backends

Default storage is SQLite (file paths via `FLOWABLE_PROCESS_DATABASE_PATH` and
siblings, see `flowable-platform-bootstrap`).

To run the whole server on PostgreSQL or MySQL instead, build with the matching
feature and point `FLOWABLE_DATABASE_URL` at the instance — one URL selects the
backend for the process engine and the DMN/CMMN/App engines together:

```powershell
$env:FLOWABLE_DATABASE_URL = "mysql://user:pass@localhost:3306/flowable"
cargo run -p flowable-rest --features mysql
```

```powershell
$env:FLOWABLE_DATABASE_URL = "postgres://user:pass@localhost:5432/flowable"
cargo run -p flowable-rest --features postgres
```

The URL scheme decides the kind (`mysql://`, `postgres://` / `postgresql://`,
`:memory:`, otherwise SQLite). Without the variable every engine keeps its
existing SQLite path, so the default binary is unchanged.

PostgreSQL/MySQL contract suites follow [multi-db-test.md](multi-db-test.md)
(`FLOWABLE_TEST_POSTGRES_URL` / `FLOWABLE_TEST_MYSQL_URL`). All of them skip
gracefully — and pass — when the database is unreachable, so a default
`cargo test` never depends on one:

- Whole-server PostgreSQL boot:
  `cargo test -p flowable-rest --features postgres --test postgres_server_boot_test`.
  Column metadata goes through `DbSession::table_columns` (SQLite
  `PRAGMA table_info`, Postgres/MySQL `information_schema.columns`).
- UI-level PostgreSQL smoke tests:
  `cargo test -p flowable-ui-rest --features postgres` (idm 4 + admin/task/modeler
  5).
- Whole-server MySQL boot:
  `cargo test -p flowable-rest --features mysql --test mysql_server_boot_test`.
- UI-level MySQL smoke tests:
  `cargo test -p flowable-ui-rest --features mysql --test ui_mysql_smoke_test`
  (the idm 4: login, token round-trip, logout, user CRUD).
- MySQL live smoke is **unrun** — the adaptation (bootstrap URL parsing plus the
  `--features mysql` chain through bootstrap and ui-rest) is in place and the
  suites exist, but no local instance was available, so they have only been
  observed skipping. Set `FLOWABLE_TEST_MYSQL_URL` to a live instance to
  actually exercise them. Budget ~60s for the first probe against a dead
  address: the MySQL pool's acquire timeout is fixed in `sqlx_executor.rs` and
  `busy_timeout_ms` does not shorten it.

## 6. Known deliberate deviations

Indexed with rationale in `docs/plans/ui-migration-coverage.md` §6 (sibling
checkout). Headlines: CMMN designer and DMN DRD graphical editing are not
reimplemented; the app-definition editor is JSON-text only; model history
versioning is not implemented; the Oryx editor protocol (stencil-sets, legacy
editor JSON) is replaced by the self-developed `bpmn-json` / `dmn-json` /
`form-json` protocol.
