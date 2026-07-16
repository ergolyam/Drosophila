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

Native builds can select TUN, System Proxy or plain Proxy in Settings. System Proxy uses the current user's Windows internet settings and does not display a UAC prompt. Plain Proxy only exposes the local HTTP/SOCKS5 endpoint. Drosophila restores the previous system settings when System Proxy stops and recovers them on the next launch after an unclean exit.

The GTK application runs with the current user's token. Windows displays a UAC prompt only when TUN is enabled, then launches the same `Drosophila.exe` in a non-GUI worker mode for Wintun operations. Configuration is stored next to `Drosophila.exe`.
