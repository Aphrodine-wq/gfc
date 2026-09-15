# Packaging

Reproducible builds, no telemetry, MIT.

## NixOS / Nix

```sh
nix build
nix run
nix develop
```

The flake builds `gfc` for `x86_64-linux` and `aarch64-linux`.

## Immutable / atomic desktops (Silverblue, BlendOS, immutable Arch)

Prefer the musl static binary from CI (`x86_64-unknown-linux-musl` /
`aarch64-unknown-linux-musl`) and install to `~/.local/bin`. XDG paths are
used exclusively (`~/.config/gfc`, `~/.local/share/gfc`,
`$XDG_RUNTIME_DIR/gfc`). No FHS writes besides the optional systemd user unit
in `~/.config/systemd/user/gfc.service`.

```sh
gfc daemon install
systemctl --user enable --now gfc.service
```

## GNU targets

`cargo build --release -p gfc` produces a dynamically linked binary against
glibc. CI also cross-builds `aarch64-unknown-linux-gnu`.

## Lockfile

`Cargo.lock` is committed. CI runs `--locked`.
