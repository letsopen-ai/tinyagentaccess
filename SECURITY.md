# Security

## Report privately

Please use [GitHub private vulnerability reporting](https://github.com/letsopen-ai/tinyagentaccess/security/advisories/new).
If the form is unavailable, email [git@cnie.xyz](mailto:git@cnie.xyz)
(maintainer [@niechen](https://github.com/niechen)). Do not file public exploit
issues or disclose sensitive details in public discussions.

Include the version or commit, OS and desktop environment, reproduction steps,
impact, and a sanitized example. Remove credentials, identifying information,
and private screen content. Security support targets the latest source revision;
there is no guaranteed response time.

## Current boundaries

This server has **no authentication**. It binds loopback by default;
do not expose it to a network or the public internet. Wildcard binds are rejected
and concrete non-loopback binds emit a warning. Host/Origin checks reduce browser
and DNS-rebinding risks but are not authentication and do not authenticate local
processes. Forwarded subprocesses inherit the server environment. Screenshots
and optional local replay may contain sensitive data; keep their storage private.
See [permissions](docs/permissions.md) and [design](docs/design.md).

After publication, maintainers should enable private vulnerability reporting if
the form is missing and require approval for external contributors' fork workflow
runs. All CI jobs use standard GitHub-hosted runners, including macOS and
Windows, with read-only tokens and no secrets. Fork PRs are eligible for all CI
jobs, subject to external contributor approval. Approved fork runs use the fork's
workflow files, so human review remains an important control.
