# Drosophila

Drosophila is a desktop application for running, configuring and monitoring a [Yggdrasil-ng](https://github.com/Revertron/Yggdrasil-ng) node.

## Features

- Start and stop a Yggdrasil-ng node
- View the IPv6 address, subnet and peer status
- Add, remove and discover peers
- Edit or generate a private key
- Use a local SOCKS5 proxy in Flatpak
- Use a TUN interface on Linux and Windows
- Request TUN privileges on demand through PolicyKit or UAC while keeping the GUI unprivileged

## Flatpak

```bash
flatpak remote-add --user Drosophila https://ergolyam.github.io/Drosophila/ergolyam.flatpakrepo
flatpak install --user Drosophila io.github.ergolyam.Drosophila
```

The SOCKS5 proxy listens on `127.0.0.1:1080` by default.

## Windows

Download the Windows installer from the [releases page](https://github.com/ergolyam/Drosophila/releases) and run it. Configuration is stored in the application directory.

## Build

- [Linux](docs/development-linux.md)
- [Windows](docs/development-windows.md)

## Screenshots

| Main page | Settings |
|---|---|
| ![Main page screenshot](.github/docs/main.png) | ![Settings screenshot](.github/docs/settings.png) |

## License

Drosophila is licensed under GPL-3.0-or-later. Yggdrasil-ng is licensed under MPL-2.0.
