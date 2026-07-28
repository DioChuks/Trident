# Vulnerability Triage Process

How SBOM/vulnerability findings from `.github/workflows/security-scan.yml`
(issue #311) get triaged, allowlisted, or fixed.

## What runs, and when

| Check | Tool | Scope | Runs on |
|---|---|---|---|
| SBOM | [syft](https://github.com/anchore/syft) via `anchore/sbom-action` | Each of the 3 built images (go-api, grpc-api, indexer) | push/PR to `dev`/`main`, weekly |
| Image scan | [trivy](https://github.com/aquasecurity/trivy) | Same 3 images | push/PR to `dev`/`main`, weekly |
| Rust deps | `cargo audit` (`rustsec/audit-check`) | Workspace `Cargo.lock` | push/PR to `dev`/`main`, weekly |
| Go deps | `govulncheck` | `services/api`, `sdk/go` | push/PR to `dev`/`main`, weekly |
| npm deps | `npm audit` | `explorer`, `sdk/typescript`, `sdk/react` | push/PR to `dev`/`main`, weekly |
| Python deps | `pip-audit` | `sdk/python` | push/PR to `dev`/`main`, weekly |

The weekly schedule exists so a CVE disclosed *after* a PR merged still gets
caught — not just at the moment code changes.

## Severity gate

The image scan only fails the build on **fixable** `HIGH` or `CRITICAL`
findings (`--ignore-unfixed`). An unfixable base-image CVE with no upstream
patch available would otherwise block CI indefinitely with no action anyone
could take — those are tracked (see below) rather than gated on.

Dependency audits (`cargo audit`, `govulncheck`, `npm audit --audit-level=high`,
`pip-audit`) fail on any advisory at or above the tool's own "high" severity
classification — dependencies are far easier to bump than a base image, so
there's less reason to tolerate an unfixed one.

## When a scan fails

1. **Check if a fix is available.**
   - Image: bump the base image tag/digest in the relevant `Dockerfile` (`crates/api/Dockerfile`,
     `crates/indexer/Dockerfile`, `services/api/Dockerfile`).
   - Rust: `cargo update -p <crate>` (or the minimal version bump `cargo audit fix` suggests).
   - Go: `go get <module>@<patched-version>` in the affected `go.mod`.
   - npm: `npm audit fix` (or a manual version bump if `fix` would introduce a breaking change).
   - Python: bump the pinned version in `sdk/python/pyproject.toml`.

   Fix and re-run the scan. This is always preferred over allowlisting.

2. **If no fix is available yet** (upstream hasn't shipped one), allowlist it:
   - **Image findings** → add the CVE ID to `.trivyignore` at the repo root,
     with a comment above it recording:
     - the CVE ID
     - why it's not being fixed right now (no upstream patch / not reachable
       in how we use the package / confirmed false positive)
     - the date added
     - a re-review date (suggest +90 days, sooner for anything network-reachable)
   - **Dependency findings** — each tool has its own suppression mechanism:
     - `cargo audit`: add an `[advisories.ignore]` entry to a `.cargo/audit.toml`
       (create it if it doesn't exist yet) with the same comment convention as above.
     - `govulncheck`: no first-class ignore file — if genuinely unfixable, note
       it in this doc's table below and add a `//nolint`-style comment at the
       call site referencing the CVE, or exclude the specific module version
       via `go.mod` `exclude` only if truly necessary.
     - `npm audit`: add an `overrides` entry in the affected `package.json` if
       a transitive dependency can be forced to a patched version; otherwise
       track here.
     - `pip-audit`: `pip-audit --ignore-vuln <ID>` — wire the same flag into
       the workflow step's `args` if this comes up, with a comment here
       explaining why.

3. **Open a tracking issue** for anything allowlisted, so it doesn't silently
   live in `.trivyignore` (or equivalent) forever. Link the issue in the
   allowlist comment.

## Current allowlist

_(Kept empty until something is actually allowlisted — see `.trivyignore`
for the live list. Do not pre-populate this with hypothetical entries.)_

## Re-review cadence

Everything in `.trivyignore` (or another ecosystem's suppression file) should
carry a re-review date. Check `.trivyignore` and re-run
`docs/security-triage.md`'s table above quarterly at minimum, or immediately
when notified of a new advisory for an allowlisted package.
