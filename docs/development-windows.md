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

The release action in `.github/actions/build-windows/action.yml` builds a 7-Zip self-extracting installer. The GTK application runs with the current user's token. Windows displays a UAC prompt only when TUN is enabled, then launches the same `Drosophila.exe` in a non-GUI worker mode for Wintun operations. Configuration is stored next to `Drosophila.exe`.
