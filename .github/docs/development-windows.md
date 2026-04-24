# Windows development

These steps build the portable Windows version of **Drosophila**.

## 1. Requirements

- Windows 10 or newer
- MSYS2

- Open an **MSYS2 MINGW64** shell and install the build dependencies:
    ```bash
    pacman -Syu
    pacman -S --needed \
      git \
      unzip \
      mingw-w64-x86_64-adwaita-icon-theme \
      mingw-w64-x86_64-gdk-pixbuf2 \
      mingw-w64-x86_64-gobject-introspection \
      mingw-w64-x86_64-gtk4 \
      mingw-w64-x86_64-hicolor-icon-theme \
      mingw-w64-x86_64-libadwaita \
      mingw-w64-x86_64-librsvg \
      mingw-w64-x86_64-pyinstaller \
      mingw-w64-x86_64-pyinstaller-hooks-contrib \
      mingw-w64-x86_64-python \
      mingw-w64-x86_64-python-gobject \
      mingw-w64-x86_64-python-pip \
      mingw-w64-x86_64-python-setuptools-scm \
      mingw-w64-x86_64-go
    ```

## 2. Clone the repository

```bash
git clone https://github.com/ergolyam/Drosophila.git $HOME/Drosophila
```

> Keep the `.git` directory in place. The Windows spec resolves the version through `setuptools_scm` and writes it to the generated `METADATA` file.

## 3. Build Yggdrasil and Yggstack

- From the repository root, use PowerShell:
    ```bash
    export GOROOT=/mingw64/lib/go
    export PATH=/mingw64/lib/go/bin:/mingw64/bin:$PATH
    ```
    ```bash
    git clone --branch v0.5.13 https://github.com/yggdrasil-network/yggdrasil-go.git $HOME/yggdrasil-go
    cd $HOME/yggdrasil-go
    bash ./build
    cp yggdrasil.exe $HOME/Drosophila
    cp yggdrasilctl.exe $HOME/Drosophila
    ```
    ```bash
    git clone --branch 1.0.5 https://github.com/yggdrasil-network/yggstack.git $HOME/yggstack
    cd $HOME/yggstack
    bash ./build
    cp yggstack.exe $HOME/Drosophila
    ```

## 4. Add Wintun

- Download Wintun and copy the amd64 DLL into the repository root:
    ```bash
    curl -L "https://www.wintun.net/builds/wintun-0.14.1.zip" -o $HOME/wintun.zip
    unzip -q $HOME/wintun.zip -d $HOME/wintun
    cp $HOME/wintun/wintun/bin/amd64/wintun.dll $HOME/Drosophila
    ```

- Before packaging, the repository root must contain:
    - `yggdrasil.exe`
    - `yggdrasilctl.exe`
    - `yggstack.exe`
    - `wintun.dll`

## 5. Build the portable folder

- Run PyInstaller from the **MSYS2 MINGW64** shell in the repository root:
    ```bash
    cd $HOME/Drosophila
    MINGW_PREFIX=/mingw64 /mingw64/bin/python -m PyInstaller \
      --clean \
      --noconfirm \
      --distpath dist \
      --workpath build \
      drosophila-windows.spec
    ```

> The result is `dist/Drosophila`. This folder is portable. `Drosophila.exe`, bundled libraries, Yggdrasil binaries, `wintun.dll` must stay together.

- Run the local build:
    ```batch
    C:\msys64\home\%USERNAME%\Drosophila\dist\Drosophila\Drosophila.exe
    ```

> The Windows configuration file is created next to the executable as `yggdrasil.conf`.

## Troubleshooting

| Symptom                    | Fix                                                                 |
|----------------------------|----------------------------------------------------------------------|
| Version build failure      | Make sure the checkout has Git metadata; CI uses `fetch-depth: 0`.   |
| Missing `wintun.dll`       | Copy `wintun\bin\amd64\wintun.dll` to the repository root.          |
| Missing GTK DLLs           | Build from the MSYS2 MINGW64 shell with the listed packages.         |
