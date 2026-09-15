# Roadmap

Public, auditable plan. Core remains deterministic: local Git health does not
depend on accounts, telemetry, or AI.

## Done in 0.1.0 (MVP)

1. Discovery and monitoring of local repositories (target: 50)
2. Dirty, conflicted, diverged, stale, and CI detection with evidence + timestamps
3. GitHub and self-hosted GitLab metadata/CI adapters
4. Cursor Origin generic Git adapter; app CI path present but not claimed at parity
5. Offline inventory and timestamped cache (`presented_as: cached` when stale)
6. systemd user unit (`gfc daemon install`) and `gfc daemon run`
7. TUI settings write the same XDG `config.toml`
8. Signed webhook verification (GitHub HMAC, GitLab token, Origin Ed25519)
9. Capability-restricted Wasmtime host (summarizer world off by default)
10. Packaging sketches: Nix flake, musl CI, x86_64 and aarch64
11. MIT license

## Next

- Demonstrate Origin app-installation CI on a live workspace, then claim parity
- WASM sample plugin compiled to `wasm32-wasip2`
- AUR / Fedora copr packages
- Optional `gitoxide` status backend if the 3s bench needs it without index writes
