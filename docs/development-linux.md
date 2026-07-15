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

Install the tool that assigns the capability:

```bash
# Debian
sudo apt install libcap2-bin

# Fedora
sudo dnf install libcap

# Arch Linux
sudo pacman -S libcap

# Alpine Linux
sudo apk add libcap-utils
```

Install the binary before assigning the capability. Do not run the capability-enabled copy from `/tmp`.

```bash
sudo install -Dm0755 /path/to/drosophila /usr/local/bin/drosophila
sudo setcap cap_net_admin=ep /usr/local/bin/drosophila
getcap /usr/local/bin/drosophila
```

Run `/usr/local/bin/drosophila`.

## Flatpak build

```bash
flatpak-builder --user --install --force-clean build-dir flatpak/io.github.ergolyam.Drosophila.yml
flatpak run io.github.ergolyam.Drosophila
```
