# Linux build

## Native build

Requirements: Rust 1.92 or newer, GTK 4.12 or newer and libadwaita 1.6 or newer.

### Debian 13 or newer

```bash
sudo apt update
sudo apt install --no-install-recommends ca-certificates curl gcc libadwaita-1-dev
```

Install the toolchain from [rustup.rs](https://rustup.rs/) if the packaged version is older than 1.92. Debian 12 has GTK and libadwaita versions that are too old for this build.

### Fedora

```bash
sudo dnf install --setopt=install_weak_deps=False cargo libadwaita-devel
```

### Arch Linux

```bash
sudo pacman -S rust libadwaita pkgconf
```

### Alpine Linux

```bash
sudo apk add cargo libadwaita-dev
```

Build from the repository root:

```bash
cargo build --release --locked
```

The binary is `target/release/drosophila`.

Run `cargo run --locked -- --debug` from a terminal to enable debug-level application and
GTK/GLib logging.

## Podman build

```bash
mkdir -p /tmp/drosophila-linux-build

podman run --rm \
  --volume "$PWD:/src:ro,Z" \
  --volume "/tmp/drosophila-linux-build:/out:Z" \
  --workdir /src \
  docker.io/library/rust:1.97.0-slim-trixie \
  bash -euxo pipefail -c '
    apt-get update
    DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
      libadwaita-1-dev
    CARGO_TARGET_DIR=/tmp/drosophila-target cargo build --release --locked
    install -Dm0755 /tmp/drosophila-target/release/drosophila /out/drosophila
  '
```

The binary is `/tmp/drosophila-linux-build/drosophila`. On Debian 13, install its runtime libraries with:

```bash
sudo apt install libadwaita-1-0
```

## TUN access

TUN mode uses PolicyKit on demand. The GTK application remains unprivileged; when TUN is enabled it launches the same executable in a restricted worker mode through `pkexec`. After authorization, the worker drops the root user ID and every capability except `CAP_NET_ADMIN`. No file capability is applied to the executable.

Install the binary and its PolicyKit action:

```bash
sudo install -Dm0755 /path/to/drosophila /usr/local/bin/drosophila
sudo install -Dm0644 \
  xdg/io.github.ergolyam.Drosophila.policy \
  /usr/share/polkit-1/actions/io.github.ergolyam.Drosophila.policy
```

Run `/usr/local/bin/drosophila` as the desktop user. A running PolicyKit authentication agent and `pkexec` are required. Development builds at other paths still use `pkexec`'s standard action; the installed policy provides the application-specific prompt for `/usr/bin/drosophila` and `/usr/local/bin/drosophila`.

Flatpak is built with `--no-default-features`, so the Yggdrasil TUN adapter, PolicyKit worker and Linux capability dependencies are omitted entirely. System Proxy is the default and uses direct dconf access to update the current user's GNOME proxy settings while the node is running. Plain Proxy mode exposes the same local HTTP/SOCKS5 endpoint without changing GNOME settings. System Proxy restores the previous settings on stop and recovers them on the next launch after an unclean exit.

## Flatpak build

```bash
flatpak-builder --user --install --force-clean build-dir flatpak/io.github.ergolyam.Drosophila.yml
flatpak run io.github.ergolyam.Drosophila
```
