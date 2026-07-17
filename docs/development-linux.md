# Linux development

## Native build

Requirements: Rust 1.92 or newer, GTK 4.12 or newer and libadwaita 1.6 or newer.

If the packaged Rust version is older than 1.92, install Rust from [rustup.rs](https://rustup.rs/). Debian 12 has unsupported GTK and libadwaita versions.

### Install dependencies

#### Debian 13 or newer

```bash
sudo apt update
sudo apt install --no-install-recommends ca-certificates curl gcc libadwaita-1-dev
```

#### Fedora

```bash
sudo dnf install --setopt=install_weak_deps=False cargo libadwaita-devel
```

#### Arch Linux

```bash
sudo pacman -S rust libadwaita pkgconf
```

#### Alpine Linux

```bash
sudo apk add cargo libadwaita-dev
```

### Build from the repository root

```bash
cargo build --release --locked
```

The binary is `target/release/drosophila`. For debug logs, run `cargo run --locked -- --debug`.

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

The binary is `/tmp/drosophila-linux-build/drosophila`. On Debian 13, install the runtime library with:

```bash
sudo apt install libadwaita-1-0
```

## TUN access

TUN uses `pkexec` on demand and requires a PolicyKit authentication agent. The GUI remains unprivileged; the worker retains only `CAP_NET_ADMIN`.

Install the binary and its PolicyKit action:

```bash
sudo install -Dm0755 /path/to/drosophila /usr/local/bin/drosophila
sudo install -Dm0644 \
  xdg/io.github.ergolyam.Drosophila.policy \
  /usr/share/polkit-1/actions/io.github.ergolyam.Drosophila.policy
```

The policy provides the application-specific prompt for binaries installed to `/usr/bin` or `/usr/local/bin`.

## Flatpak build

Flatpak builds without the `tun` feature.

```bash
flatpak-builder --user --install --force-clean build-dir flatpak/io.github.ergolyam.Drosophila.yml
flatpak run io.github.ergolyam.Drosophila
```
