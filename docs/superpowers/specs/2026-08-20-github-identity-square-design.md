# GitHub identity square

Approved layout: a small bordered cell at the top-right of the main TUI, showing the signed-in GitHub avatar, display name, and `@login`.

## Behavior

- Daemon calls `GET /user` with the existing GitHub token (never stored in cache).
- Response fields used: `login`, `name`, `avatar_url`. Display name is `name` if non-empty, otherwise `login`.
- Avatar bytes are written under the XDG cache dir (`…/gfc/avatars/<login>`). Tokens never go there.
- `auth.status` includes optional `github: { login, name, avatar_path }` or `null`.
- TUI paints the square with `ratatui-image` (Kitty protocol when detected, half-blocks otherwise).
- Unauthenticated or failed fetch: same square, text `not signed in`, no image.

## Out of scope

GitLab/Origin identity chips, click-through to profile, live avatar refresh more often than daemon scan.
