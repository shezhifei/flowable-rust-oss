# Multi-DB Engine Contract Tests

CI recipe for PostgreSQL and MySQL backend contract suites on `flowable-engine` and
`flowable-persistence`. Default CI without a live remote DB is safe: engine
integration tests **skip gracefully** when the backend is unreachable.

## Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `FLOWABLE_TEST_POSTGRES_URL` | `postgres://postgres:postgres@localhost:5432/flowable_test` | PostgreSQL connection URL |
| `FLOWABLE_TEST_MYSQL_URL` | `mysql://flowable:flowable@localhost:3306/flowable_test` | MySQL connection URL |

Schema bootstrap helpers (repo root / parent workspace):

- `setup_postgres.sql` — creates `flowable_test` DB and `flowable` role (if using that user)
- `setup_mysql.sql` — creates `flowable_test` DB and `flowable` user

The engine creates/migrates its own tables on first `ProcessEngine::build_with_config`.

## Prerequisites

### PostgreSQL

```powershell
# Example: local postgres with default superuser
# Ensure database exists:
$env:PGPASSWORD = "postgres"
psql -h localhost -U postgres -c "CREATE DATABASE flowable_test;" 2>$null
# Or apply parent setup script if present:
# psql -h localhost -U postgres -f setup_postgres.sql

$env:FLOWABLE_TEST_POSTGRES_URL = "postgres://postgres:postgres@localhost:5432/flowable_test"
```

### MySQL

```powershell
# mysql -u root -p < setup_mysql.sql
$env:FLOWABLE_TEST_MYSQL_URL = "mysql://flowable:flowable@localhost:3306/flowable_test"
```

## Cargo commands

Run from the `flowable-rust` workspace root.

### PostgreSQL — engine contract (deploy / start / complete / timer / history)

```powershell
$env:FLOWABLE_TEST_POSTGRES_URL = "postgres://postgres:postgres@localhost:5432/flowable_test"
cargo test -p flowable-engine --features postgres --test postgres_engine_integration_test -- --nocapture
```

Covered cases (skip if DB unreachable):

- `postgres_deploy_and_query_resources`
- `postgres_delete_deployment_removes_process_definitions`
- `postgres_dual_write_populates_normalized_act_tables`
- `postgres_runtime_state_persists_after_start`
- `postgres_repeated_deployment_increments_version`
- `postgres_deploy_start_complete_user_task`
- `postgres_history_present_after_complete`
- `postgres_timer_intermediate_catch_creates_timer_job`

### PostgreSQL — persistence smoke

```powershell
$env:FLOWABLE_TEST_POSTGRES_URL = "postgres://postgres:postgres@localhost:5432/flowable_test"
cargo test -p flowable-persistence --features postgres --test smoke_test_postgres -- --nocapture
```

### MySQL — engine contract

```powershell
$env:FLOWABLE_TEST_MYSQL_URL = "mysql://flowable:flowable@localhost:3306/flowable_test"
cargo test -p flowable-engine --features mysql --test mysql_engine_integration_test -- --nocapture
```

Covered cases (skip if DB unreachable): same set as Postgres, `mysql_*` prefix.

### MySQL — persistence smoke

```powershell
$env:FLOWABLE_TEST_MYSQL_URL = "mysql://flowable:flowable@localhost:3306/flowable_test"
cargo test -p flowable-persistence --features mysql --test smoke_test_mysql -- --nocapture
```

### Full multi-backend gate (both DBs available)

```powershell
$env:FLOWABLE_TEST_POSTGRES_URL = "postgres://postgres:postgres@localhost:5432/flowable_test"
$env:FLOWABLE_TEST_MYSQL_URL = "mysql://flowable:flowable@localhost:3306/flowable_test"

cargo test -p flowable-persistence --features postgres --test smoke_test_postgres -- --nocapture
cargo test -p flowable-engine --features postgres --test postgres_engine_integration_test -- --nocapture

cargo test -p flowable-persistence --features mysql --test smoke_test_mysql -- --nocapture
cargo test -p flowable-engine --features mysql --test mysql_engine_integration_test -- --nocapture
```

## CI notes

1. **Default PR CI (no remote DB):**
   - Do **not** enable `--features postgres` / `--features mysql` for default jobs, **or**
   - Enable features but rely on engine integration skip-if-unreachable (tests return `ok` after `eprintln!` skip).
   - Persistence smoke tests currently **fail hard** if the DB is down; only schedule them on multi-DB jobs.

2. **Multi-DB certification job (recommended):**
   - Provision Postgres (and MySQL if in scope).
   - Export `FLOWABLE_TEST_POSTGRES_URL` / `FLOWABLE_TEST_MYSQL_URL`.
   - Run the four cargo commands above; treat failures as blockers.

3. **Serialisation:** engine tests take a process-wide mutex per backend so shared DB keyspace is not raced.

4. **Idempotency:** process keys are UUID-suffixed; safe to re-run against a shared `flowable_test` database.

## Bash equivalents

```bash
export FLOWABLE_TEST_POSTGRES_URL="postgres://postgres:postgres@localhost:5432/flowable_test"
cargo test -p flowable-engine --features postgres --test postgres_engine_integration_test -- --nocapture

export FLOWABLE_TEST_MYSQL_URL="mysql://flowable:flowable@localhost:3306/flowable_test"
cargo test -p flowable-engine --features mysql --test mysql_engine_integration_test -- --nocapture
```

## Exit criteria (M78)

- Postgres engine suite green for deploy / start / complete user task / timer job presence / history presence.
- MySQL engine suite present with the same skip-if-unavailable contract (smoke at minimum when MySQL is provisioned).
- This runbook documents exact cargo commands and env vars for CI.
