# -*- mode: python ; coding: utf-8 -*-

import os, sys
from pathlib import Path


project_root = Path(SPECPATH)
python_prefix = Path(sys.executable).resolve().parents[1]
mingw_prefix = Path(
    os.environ.get("MINGW_PREFIX")
    or python_prefix
)

datas = [
    (str(project_root / "yggui" / "ui"), "yggui/ui"),
    (str(project_root / "license"), "."),
]
binaries = []


def add_data_tree(src: Path, dst: str) -> None:
    if src.exists():
        datas.append((str(src), dst))


for rel in (
    "lib/girepository-1.0",
    "lib/gdk-pixbuf-2.0",
    "lib/gtk-4.0",
    "share/glib-2.0",
    "share/icons",
    "share/themes",
    "share/locale",
    "share/libadwaita",
    "etc/gtk-4.0",
):
    add_data_tree(mingw_prefix / rel, rel)

typelib_dir = mingw_prefix / "lib" / "girepository-1.0"
if typelib_dir.exists():
    for name in (
        "Gdk-4.0.typelib",
        "GdkWin32-4.0.typelib",
        "Graphene-1.0.typelib",
        "Gsk-4.0.typelib",
        "Gtk-4.0.typelib",
        "PangoCairo-1.0.typelib",
    ):
        typelib = typelib_dir / name
        if typelib.exists():
            binaries.append((str(typelib), "gi_typelibs"))

bin_dir = mingw_prefix / "bin"
if bin_dir.exists():
    for dll in sorted(bin_dir.glob("*.dll")):
        binaries.append((str(dll), "."))
    for helper in (
        "gspawn-win64-helper.exe",
        "gspawn-win64-helper-console.exe",
        "gdbus.exe",
    ):
        helper_path = bin_dir / helper
        if helper_path.exists():
            binaries.append((str(helper_path), "."))

for name in (
    "yggdrasil.exe",
    "yggdrasilctl.exe",
    "yggstack.exe",
    "wintun.dll",
):
    path = project_root / name
    if path.exists():
        binaries.append((str(path), "."))

icon_path = project_root / "xdg" / "io.github.ergolyam.Drosophila.ico"
if not icon_path.exists():
    icon_path = project_root / "xdg" / "io.github.ergolyam.Drosophila.svg"

hiddenimports = [
    "gi",
    "gi.repository.Adw",
    "gi.repository.Gdk",
    "gi.repository.GdkPixbuf",
    "gi.repository.Gio",
    "gi.repository.GLib",
    "gi.repository.GObject",
    "gi.repository.Gtk",
    "gi.repository.Pango",
]

a = Analysis(
    [str(project_root / "yggui" / "__main__.py")],
    pathex=[str(project_root)],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="Drosophila",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=False,
    icon=str(icon_path),
    uac_admin=True,
    contents_directory=".",
)

coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    upx_exclude=[],
    name="Drosophila",
)
