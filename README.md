# Git Forge Cockpit (`gfc`)

Local-first, keyboard-driven Linux TUI that inventories Git repositories and
shows which ones need attention across local Git, GitHub, self-hosted GitLab,
and Cursor Origin.

No accounts, telemetry, or AI are required for core triage. The complete local
inventory works offline. Cached remote data is timestamped and is **never**
presented as current when it is stale.

## Quick start

```sh
gfc config --init
# edit ~/.config/gfc/config.toml  (see config.example.toml)
gfc daemon install    # optional systemd --user unit
gfc daemon run        # or run manually
gfc                   # TUI
gfc scan --json       # one-shot inventory
```

## Keys

| Key | Action |
| --- | --- |
| j/k | move |
| / | filter |
| s | sort |
| a | attention-only |
| e/t/b/g | editor / terminal / browser / lazygit |
| S | settings (writes the same TOML file) |
| ? | help |
| q | quit |

## Layout

- `gfc` — TUI (default) + `scan` / `daemon` / `config` subcommands
- Daemon owns Git, providers, webhooks, SQLite metadata cache
- Unix-socket JSON-RPC at `$XDG_RUNTIME_DIR/gfc/daemon.sock`
- Cache at `~/.local/share/gfc/cache.sqlite` (metadata only, never tokens)

## Providers

See [docs/providers.md](docs/providers.md). GitHub and GitLab have full
metadata/CI parity. Origin ships a generic Git adapter; check-run CI requires
validated Origin app credentials.

## License

MIT
