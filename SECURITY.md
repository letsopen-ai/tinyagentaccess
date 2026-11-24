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

Every `/mcp` request requires `Authorization: Bearer <token>`. The token is a
random value in an owner-only file (`0600`, in a `0700` directory; owner-only ACL on
Windows). The observer (`/observe*`) uses a separate secret, delivered in a URL
fragment, in addition to the browser checks. There is no no-auth mode, no TLS, no
OAuth, and one role. The server binds loopback by default; do not expose it to a
network or the public internet. Wildcard binds are rejected, and a concrete
non-loopback bind needs `--allow-non-loopback` and still warns; use an SSH tunnel for
remote clients. Host/Origin checks reduce browser and DNS-rebinding risks but are not
authentication.

Residual risk: the token file only separates OS users. Any process running as your
user, including the authorized agent's own shell and any child that inherits an
exported `TAA_TOKEN`, can read the token and drive the desktop; a fully compromised user
account defeats it. Do not commit tokens or client configs that contain them (a
literal `--header` may be written into `.mcp.json`). Rotate with
`tinyagentaccess rotate-token`; stopping a session does not revoke the token. `TAA_TOKEN` is the documented
client variable; the server removes it from forwarded subprocess environments, which
otherwise inherit the server environment. Children running as your user can still read
the 0600 token file. The agent token and observer secret must be distinct files and
values; startup rejects a shared path, link, or equal value.
Screenshots and optional local replay may contain sensitive data; keep their storage
private. The client header setups are documented from vendor docs and were not tested
against live clients.
See [permissions](docs/permissions.md) and [design](docs/design.md).

After publication, maintainers should enable private vulnerability reporting if
the form is missing and require approval for external contributors' fork workflow
runs. All CI jobs use standard GitHub-hosted runners, including macOS and
Windows, with read-only tokens and no secrets. Fork PRs are eligible for all CI
jobs, subject to external contributor approval. Approved fork runs use the fork's
workflow files, so human review remains an important control.
