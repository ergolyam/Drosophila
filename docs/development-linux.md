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
> It is also possible to build the project in `nix-shell` using `shell.nix`.

The binary is `target/release/drosophila`. For debug logs, run `cargo run --locked -- --debug`.

## TUN access

TUN uses `pkexec` on demand and requires a PolicyKit authentication agent. The GUI remains unprivileged; the worker retains only `CAP_NET_ADMIN`.

Run the downloaded binary directly as a regular desktop user; no additional files or installation steps are required. When TUN is selected, Drosophila requests administrator authorization through `pkexec`.

## Flatpak build

```bash
flatpak-builder --user --install --force-clean build-dir flatpak/io.github.ergolyam.Drosophila.yml
flatpak run io.github.ergolyam.Drosophila
```
