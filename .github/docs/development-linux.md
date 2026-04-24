# Linux development

These steps clone **Drosophila**, build the required Yggdrasil binaries and run the UI locally. Tested on Fedora 42; adjust package names for your distribution.

## 1. Clone the repository

```bash
git clone https://github.com/ergolyam/Drosophila.git
cd Drosophila
```

> Keep the Git checkout intact. The project version is dynamic and is resolved from Git metadata by `setuptools_scm`.

## 2. Install system packages

- Fedora / RHEL
    ```bash
    sudo dnf install git go gtk4 libadwaita python3-gobject polkit
    ```

- Arch
    ```bash
    sudo pacman -S git go gtk4 libadwaita python-gobject polkit
    ```

- Debian
    ```bash
    sudo apt install git golang libgtk-4-dev gir1.2-gtk-4.0 gir1.2-adwaita-1 python3-gi policykit-1
    ```

## 3. Create a Python environment

```bash
python -m venv .venv
source .venv/bin/activate
pip install -U pip uv
uv sync
```

## 4. Build Yggdrasil and Yggstack

- Build `yggdrasil` and `yggdrasilctl` from the version used by release builds:
    ```bash
    git clone --branch v0.5.13 https://github.com/yggdrasil-network/yggdrasil-go.git ../yggdrasil-go
    cd ../yggdrasil-go
    ./build
    mkdir -p ~/.local/bin
    cp yggdrasil yggdrasilctl ~/.local/bin/
    cd ../Drosophila
    ```

- Build `yggstack` for the optional SOCKS proxy mode:
    ```bash
    git clone --branch 1.0.5 https://github.com/yggdrasil-network/yggstack.git ../yggstack
    cd ../yggstack
    ./build
    cp yggstack ~/.local/bin/
    cd ../Drosophila
    ```

> Ensure `~/.local/bin` is in your `PATH`.

## 5. Run the UI

```bash
source .venv/bin/activate
python -m yggui
```

> On Linux, Drosophila starts Yggdrasil through `pkexec` because the daemon needs elevated privileges. Yggstack SOCKS mode runs without root.

## Troubleshooting

| Symptom                 | Fix                                                           |
|-------------------------|---------------------------------------------------------------|
| Yggdrasil not found     | Make sure `yggdrasil` and `yggdrasilctl` are in your `PATH`.  |
| Polkit not found        | Install `polkit` and make sure `pkexec` is available.         |
| Version shows `0.0.0`   | Run from a Git checkout or install the package metadata.      |
| Need verbose GTK output | Run with `G_MESSAGES_DEBUG=all`.                              |
