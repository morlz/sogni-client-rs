# Security policy

Report suspected vulnerabilities privately to Sogni rather than opening a
public issue. Do not include API keys, access tokens, refresh tokens, wallet
passwords, signed permits, private media URLs, or generated private media in a
report, test fixture, commit, log, or screenshot.

The SDK treats all client-provided billing, authorization, entitlement, and
safety values as untrusted hints. The service remains authoritative for those
decisions. Credential-bearing types redact their `Debug` output, but callers
must also avoid serializing or logging their own request objects and
environment variables.

Only currently supported crate releases receive security fixes. Include the
crate version, Rust version, platform, minimal reproduction, and sanitized
error details in a private report.
