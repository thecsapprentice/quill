# Security policy

Quill's central promise is that a server operator can never read writing
sessions. Reports that threaten that promise are the highest priority.

## Reporting a vulnerability

Please **do not open a public issue**. Report privately through GitHub's
[private vulnerability reporting](https://github.com/thecsapprentice/quill/security/advisories/new)
for this repository. Include steps to reproduce and the affected commit.

We aim to acknowledge reports within 7 days. Quill is pre-release software:
there are no supported releases yet, so fixes land on `main`.

## Especially in scope

- Any path by which a server, or anyone with its database or keys, could
  recover session plaintext or client key material.
- Any server-side dependency on MLS, HPKE or client crates (see
  `scripts/check-server-deps.sh`).
- Leaks that link a user's pseudonyms to each other or to a real identity.
- Authentication flaws, including account or username enumeration.
- Memory-safety issues (all project crates forbid `unsafe`).

## Out of scope

- The known, documented metadata a home server sees by design (who is in a
  session and when messages flow).
- Issues in dependencies already tracked by RustSec; we track those through
  `cargo deny` and `cargo audit` in CI.
