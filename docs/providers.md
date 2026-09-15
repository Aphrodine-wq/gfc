# Provider matrix

What `gfc` **demonstrates** versus what it **discloses**. We do not claim
parity that cannot be shown in tests.

| Capability | Local Git | GitHub | Self-hosted GitLab | Cursor Origin |
| --- | --- | --- | --- | --- |
| Discover from disk | yes | yes (cloned remotes) | yes | yes (`origin.cursor.com` remotes) |
| Import from API / CLI | n/a | REST `/user/repos` | membership projects | `origin repo list` (CLI) + generic Git |
| Dirty / conflict / ahead / behind | yes | n/a (local) | n/a (local) | n/a (local) |
| Remote connectivity | `git ls-remote` | REST | REST | generic Git; REST if app token |
| CI normalization | n/a | checks + combined status | latest pipeline | **unsupported** unless Origin app credentials are configured and proven |
| Signed webhooks | n/a | HMAC-SHA256 | `X-Gitlab-Token` | Ed25519 `v1ed` (verified in unit tests) |

## Cursor Origin

Origin publishes a partner REST API (`https://api.cursor.com/v1/origin`) with
repository metadata, check runs, and signed webhooks. That API is
**app/installation** scoped:

- Namespace-wide listing is not part of the partner API
- Repositories mirrored **in** from GitHub are invisible to Origin apps
- There is no personal `gh`-style token for CI

Until a live Origin app installation is validated, the TUI labels Origin CI as
`unsupported` and uses the generic Git adapter for local health. Full metadata
and CI parity is a **documented release dependency**, not an unsupported claim.

## Credentials

Resolved in order (configurable): provider CLI (`gh`, `glab`, `origin`) →
Linux secret service (`secret-tool`) → environment variable **names** →
credential commands. Tokens are never stored in the SQLite metadata cache.
Network hosts and credential **labels** are observable in the TUI status line.
