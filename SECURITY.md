# Security policy

Report suspected vulnerabilities using this fork's
[private vulnerability reporting form](https://github.com/Sogni-AI/sogni-client-rs/security/advisories/new).
Do not include API keys, access tokens, refresh tokens, wallet
passwords, signed permits, private media URLs, or generated private media in a
report, test fixture, commit, log, or screenshot.

The SDK treats all client-provided billing, authorization, entitlement, and
safety values as untrusted hints. The service remains authoritative for those
decisions. Credential-bearing types redact their `Debug` output, but callers
must also avoid serializing or logging their own request objects and
environment variables.

This is a community-origin alpha fork. Fixes are developed on this fork's
current main branch and released as new alpha tags; it has no long-term
support commitment for older tags. Include the exact Git tag or commit,
Rust version, platform, minimal reproduction, and sanitized error details.
