# Multi-DB Engine Contract Tests

CI recipe for PostgreSQL and MySQL backend contract suites on `flowable-engine` and
`flowable-persistence`. Default CI without a live remote DB is safe: engine
integration tests **skip gracefully** when the backend is unreachable.

## Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `FLOWABLE_TEST_POSTGRES_URL` | `postgres://postgres:postgres@localhost:5432/flowable_test` | PostgreSQL connection URL |
| `FLOWABLE_TEST_MYSQL_URL` | `mysql://flowable:flowable@localhost:3306/flowable_test` | MySQL connection URL |

The engine creates/migrates its own tables on first
`ProcessEngine::build_with_config`, so bootstrap only has to create the database
and the login role.

> The credentials below are throwaway values for a local test database and match
> the defaults in the table above. Do not reuse them anywhere else.

PostgreSQL (as a superuser; neither statement supports `IF NOT EXISTS`, so
re-running reports an "already exists" error you can ignore). The default URL
above connects as the `postgres` superuser, in which case only the first line is
needed — create the `flowable` role only if you point
`FLOWABLE_TEST_POSTGRES_URL` at it instead:

```sql
CREATE DATABASE flowable_test;
-- Only if you connect as `flowable` rather than as the superuser:
CREATE ROLE flowable LOGIN PASSWORD 'flowable';
GRANT ALL PRIVILEGES ON DATABASE flowable_test TO flowable;
-- then, connected to flowable_test:
GRANT ALL ON SCHEMA public TO flowable;
```

MySQL:

```sql
CREATE DATABASE IF NOT EXISTS flowable_test;
CREATE USER IF NOT EXISTS 'flowable'@'%' IDENTIFIED BY 'flowable';
GRANT ALL PRIVILEGES ON flowable_test.* TO 'flowable'@'%';
FLUSH PRIVILEGES;
```

## Prerequisites

### PostgreSQL

```powershell
# Example: local postgres with default superuser
# Ensure database exists:
$env:PGPASSWORD = "postgres"
psql -h localhost -U postgres -c "CREATE DATABASE flowable_test;" 2>$null

$env:FLOWABLE_TEST_POSTGRES_URL = "postgres://postgres:postgres@localhost:5432/flowable_test"
```

### MySQL

```powershell
# Apply the MySQL bootstrap SQL above first, then:
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

## Exit criteria

- Postgres engine suite green for deploy / start / complete user task / timer job presence / history presence.
- MySQL engine suite present with the same skip-if-unavailable contract (smoke at minimum when MySQL is provisioned).
- This runbook documents exact cargo commands and env vars for CI.
