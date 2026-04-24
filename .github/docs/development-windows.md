# Windows development

These steps build the portable Windows version of **Drosophila**. The release build is a PyInstaller folder wrapped into a 7-Zip self-extracting executable.

## 1. Requirements

- Windows 10 or newer
- Git for Windows
- Go
- MSYS2
- 7-Zip

- Open an **MSYS2 MINGW64** shell and install the build dependencies:
    ```bash
    pacman -Syu
    pacman -S --needed \
      git \
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
      mingw-w64-x86_64-python-setuptools-scm
    ```

## 2. Clone the repository

```powershell
git clone https://github.com/ergolyam/Drosophila.git
Set-Location Drosophila
```

> Keep the `.git` directory in place. The Windows spec resolves the version through `setuptools_scm` and writes it to the generated `METADATA` file.

## 3. Build Yggdrasil and Yggstack

- From the repository root, use PowerShell:
    ```powershell
    git clone --branch v0.5.13 https://github.com/yggdrasil-network/yggdrasil-go.git ..\yggdrasil-go
    Push-Location ..\yggdrasil-go
    $env:CGO_ENABLED = "0"
    $env:GOARCH = "amd64"
    $env:GOOS = "windows"
    bash ./build
    Copy-Item yggdrasil.exe ..\Drosophila\
    Copy-Item yggdrasilctl.exe ..\Drosophila\
    Pop-Location
    ```
    ```powershell
    git clone --branch 1.0.5 https://github.com/yggdrasil-network/yggstack.git ..\yggstack
    Push-Location ..\yggstack
    $env:CGO_ENABLED = "0"
    $env:GOARCH = "amd64"
    $env:GOOS = "windows"
    bash ./build
    Copy-Item yggstack.exe ..\Drosophila\
    Pop-Location
    ```

## 4. Add Wintun

- Download Wintun and copy the amd64 DLL into the repository root:
    ```powershell
    $wintunVersion = "0.14.1"
    $expectedHash = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51"
    $url = "https://www.wintun.net/builds/wintun-$wintunVersion.zip"
    Invoke-WebRequest -Uri $url -OutFile wintun.zip
    $hash = (Get-FileHash wintun.zip -Algorithm SHA256).Hash.ToLower()
    if ($hash -ne $expectedHash) {
      throw "wintun package did not match expected checksum"
    }
    Expand-Archive wintun.zip -DestinationPath wintun
    Copy-Item wintun\wintun\bin\amd64\wintun.dll .
    ```

- Before packaging, the repository root must contain:
    - `yggdrasil.exe`
    - `yggdrasilctl.exe`
    - `yggstack.exe`
    - `wintun.dll`

## 5. Build the portable folder

- Run PyInstaller from the **MSYS2 MINGW64** shell in the repository root:
    ```bash
    MINGW_PREFIX=/mingw64 /mingw64/bin/python -m PyInstaller \
      --clean \
      --noconfirm \
      --distpath dist \
      --workpath build \
      drosophila-windows.spec
    ```

> The result is `dist/Drosophila`. This folder is portable. `Drosophila.exe`, bundled libraries, Yggdrasil binaries, `wintun.dll` must stay together.

- Run the local build:
    ```powershell
    .\dist\Drosophila\Drosophila.exe
    ```

> The Windows configuration file is created next to the executable as `yggdrasil.conf`.

## Troubleshooting

| Symptom                    | Fix                                                                 |
|----------------------------|----------------------------------------------------------------------|
| Version build failure      | Make sure the checkout has Git metadata; CI uses `fetch-depth: 0`.   |
| Missing `wintun.dll`       | Copy `wintun\bin\amd64\wintun.dll` to the repository root.          |
| Missing GTK DLLs           | Build from the MSYS2 MINGW64 shell with the listed packages.         |
| SFX package does not start | Extract the package manually with 7-Zip and run `Drosophila.exe`.    |
