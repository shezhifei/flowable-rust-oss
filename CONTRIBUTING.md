# Contributing

Thanks for your interest. This project is an independent Rust reimplementation
of the Flowable engines, so most contributions are about matching Java behavior
precisely rather than designing new behavior.

## Prerequisites

- Rust 1.85 or newer (every crate is edition 2024).
- No other toolchain is needed for the default test suite; SQLite is bundled.

## Before you open a pull request

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

The default suite must be green, and your change should not add new compiler or
clippy warnings.

Be aware of the starting point: clippy currently exits successfully but emits
roughly 170 warnings across ~34 lints, mostly `field_reassign_with_default` and
style or complexity lints, plus a handful of dead-code warnings from the
compiler. None are correctness lints. Cleaning some up is welcome as its own
commit; the ask for a feature PR is just that the count not grow because of it.

Formatting is rustfmt, but scope it to what you touch:

```sh
cargo fmt -p <the-crate-you-changed>
```

The tree is not currently uniformly rustfmt-clean — the crates moved to edition
2024, whose style edition reorders `use` groups, and that reformat has not been
applied. A blanket `cargo fmt --all` therefore rewrites files your change does
not touch, which makes review harder; please don't include that in a feature PR.
(On Windows, `cargo fmt --all` also tends to fail outright with OS error 206,
"filename or extension too long", because the workspace has 33 crates — the
per-package form above avoids it.)

## Behavior alignment convention

Where the port implements a rule that exists in Flowable Java, the code cites
the Java source that establishes it, as a comment at the implementation site:

```rust
// Java DefaultCorrelationKeyGenerator.generateKey(): MD5 over sorted keys
```

If you add or change behavior that has a Java counterpart, add or update the
citation. If the Java behavior is genuinely ambiguous, say so in the comment
rather than picking silently.

Deliberate divergences from Java are marked the same way and explained at the
site — see the deviation list in [README.md](README.md). Security-motivated
divergences are additionally summarized in [SECURITY.md](SECURITY.md). Do not
introduce a silent divergence: either cite Java, or document why you are
departing from it.

## Tests

- Unit and integration tests live next to the crate they cover; the default
  `cargo test --workspace` run must cover any new behavior.
- Tests that need external services are gated, not skipped silently:
  - MySQL / PostgreSQL backend suites are gated on `FLOWABLE_TEST_MYSQL_URL` /
    `FLOWABLE_TEST_POSTGRES_URL` — see
    [docs/runbooks/multi-db-test.md](docs/runbooks/multi-db-test.md).
  - Java-vs-Rust differential tests are `#[ignore]` by default because they
    require a Java checkout, a JDK and Maven — see
    [differential/README.md](differential/README.md).
- Bug fixes should come with a regression test that fails before the fix.

## Reporting bugs

Please include the BPMN/CMMN/DMN definition (minimized if possible), the
sequence of API calls, what Flowable Java does, and what this port does
instead. A behavioral difference from Java is a valid bug report on its own.

## Security issues

Do not open a public issue for a vulnerability. Follow the process in
[SECURITY.md](SECURITY.md).

## License

Contributions are accepted under the Apache License 2.0, the same license as
the project. See [LICENSE](LICENSE).
