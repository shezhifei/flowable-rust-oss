# Security Policy

## Reporting a vulnerability

Please report security issues privately through GitHub's private vulnerability
reporting on this repository (**Security → Report a vulnerability**). Do not open
a public issue for an exploitable defect.

Include the affected crate and version, a description of the impact, and a
reproducer if you have one. This is a volunteer-maintained project with no
paid support contract, so please allow reasonable time for a response before
disclosing publicly.

## Supported versions

This project is pre-1.0 and work in progress. Only the latest `main` receives
fixes; there are no maintained release branches or backports.

## Hardening posture

This is an independent reimplementation, not a drop-in security-equivalent of
Flowable Java. Several Java-compatible defaults were deliberately changed
because they are unsafe in a modern deployment — password storage, bootstrap
credentials, shell task availability, outbound HTTP targets, upload limits and
XML parsing limits. Those deviations are listed in the README and documented at
each implementation site.

## Known limitations

These are understood gaps rather than undiscovered bugs. Deploy accordingly.

- **No per-resource authorization.** Authentication proves *who* a caller is,
  and a URL-prefix allowlist gates administrative writes. Beyond that, any
  authenticated caller can read and modify any process instance, task, or
  deployment. There is no per-object ownership, ACL, or candidate-group
  enforcement on the REST surface.
- **No tenant isolation at the REST boundary.** `tenantId` is a query and
  filter attribute, not a security boundary. An authenticated caller can read
  and write across tenants by passing a different `tenantId`. Do not rely on it
  to separate untrusted tenants.
- **Authenticated requests carry a password-hashing cost.** Basic auth verifies
  an argon2id digest (m=19MiB) on every request, and the brute-force lockout
  counts only *failed* attempts. A client with valid credentials can therefore
  drive significant memory and CPU use. Put a request-rate limit in front of
  the service; a session/token layer in front of Basic auth would remove the
  per-request cost entirely.
- **Rate-limit keying assumes direct connections.** The failed-auth lockout
  keys on the TCP peer address. Behind a reverse proxy every client shares one
  bucket, so lockout becomes global. Terminate rate limiting at the proxy in
  that topology.
- **Integer overflow panics rather than wrapping.** The workspace sets
  `overflow-checks = true` for release builds, departing from the Cargo default,
  because a silent wrap in a counter or byte range is worse than a loud failure.
  The consequence is that an arithmetic bug surfaces as a panic — a request
  failure, or a worker thread dying — instead of a corrupt value. If you would
  rather absorb such a bug than fail, override the profile in your own build.
- **Expression evaluation is a read-only JUEL subset**, but it is still
  attacker-reachable wherever process definitions can be deployed. Treat
  deployment permission as equivalent to code execution permission.

## Scope

In scope: the engines, converters, persistence layer and REST API in this
repository.

Out of scope: vulnerabilities in Flowable Java (report those to Flowable),
issues that require deployment permission and only affect the deploying user,
and denial of service that depends on an unlimited request rate against an
authenticated endpoint (see *Known limitations*).
