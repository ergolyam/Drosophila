# Windows build

## Requirements

- Windows 10 or newer
- MSYS2
- 7-Zip

Open an **MSYS2 MINGW64** shell and install:

```bash
pacman -Syu
pacman -S --needed \
  mingw-w64-x86_64-libadwaita \
  mingw-w64-x86_64-pkgconf \
  mingw-w64-x86_64-rust
```

## Build

```bash
cargo build --release --locked
```

Download [Wintun 0.14.1](https://www.wintun.net/), then copy `wintun/bin/amd64/wintun.dll` next to `target/release/drosophila.exe`.

The binary is `target/release/drosophila.exe`.

## Modes

- System Proxy updates the current user's proxy settings without UAC.
- Proxy exposes a local HTTP/SOCKS5 endpoint.
- TUN uses Wintun and requests UAC elevation.

Configuration is stored next to `Drosophila.exe`. Run the binary with `--debug` from PowerShell or `cmd.exe` for logs.
