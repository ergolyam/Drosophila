# Linux development

## Native build

Requirements: the Nix package manager. Alpine Linux can use Rust 1.92 or newer, GTK 4.12 or newer and libadwaita 1.6 or newer instead to produce a musl binary.

### Install dependencies

#### Debian 13 or newer

```bash
sudo apt update
sudo apt install nix-setup-systemd
sudo adduser "$USER" nix-users
```

Log out and back in after adding your user to `nix-users`.

#### Fedora

```bash
sudo dnf install nix nix-daemon
sudo systemctl enable --now nix-daemon
```

#### Arch Linux

```bash
sudo pacman -S nix
sudo systemctl enable --now nix-daemon
```

#### Alpine Linux

For a native musl build:

```bash
sudo apk add cargo libadwaita-dev
```

For a glibc build with Nix:

```bash
sudo apk add nix
sudo addgroup "$USER" nix
sudo rc-update add nix-daemon
sudo rc-service nix-daemon start
```

Log out and back in after adding your user to `nix`.

### Build from the repository root

```bash
nix-shell --pure --run 'cargo build --release --locked'
```

On Alpine Linux, build directly with Cargo to produce a musl binary:

```bash
cargo build --release --locked
```

The glibc binary produced on Alpine Linux must be run from the Nix environment:

```bash
nix-shell --run './target/release/drosophila'
```

The binary is `target/release/drosophila`. For debug logs, run `nix-shell --run 'cargo run --locked -- --debug'`, or `cargo run --locked -- --debug` for a native Alpine Linux build.

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
