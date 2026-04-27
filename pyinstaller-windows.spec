# -*- mode: python ; coding: utf-8 -*-

import os, runpy, sys
from pathlib import Path

from PyInstaller.config import CONF


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


def add_data_tree(
    src: Path,
    dst: str,
    exclude_relative: tuple[str, ...] = (),
) -> None:
    if src.exists():
        excluded = {Path(item) for item in exclude_relative}
        for item in src.rglob("*"):
            if not item.is_file():
                continue

            relative = item.relative_to(src)
            if relative in excluded:
                continue

            target = (Path(dst) / relative.parent).as_posix()
            datas.append((str(item), target))


def get_project_version() -> str:
    errors = []

    try:
        from setuptools_scm import get_version

        return get_version(
            root=str(project_root),
            version_scheme="guess-next-dev",
            local_scheme="no-local-version",
        )
    except Exception as exc:
        errors.append(f"setuptools_scm: {exc}")

    generated_version = project_root / "yggui" / "_version.py"
    if generated_version.exists():
        try:
            namespace = runpy.run_path(str(generated_version))
            return str(namespace.get("__version__") or namespace["version"])
        except Exception as exc:
            errors.append(f"generated _version.py: {exc}")

    try:
        from importlib.metadata import PackageNotFoundError, version

        return version("Drosophila")
    except PackageNotFoundError as exc:
        errors.append(f"installed metadata: {exc}")

    raise RuntimeError(
        "Unable to determine Drosophila version from dynamic project metadata. "
        + " | ".join(errors)
    )


def build_metadata() -> Path:
    project_version = get_project_version()
    metadata_root = Path(CONF.get("workpath") or project_root / "build")
    metadata_file = metadata_root / "_drosophila_metadata" / "METADATA"
    metadata_file.parent.mkdir(parents=True, exist_ok=True)
    metadata_file.write_text(
        "\n".join(
            (
                "Metadata-Version: 2.1",
                "Name: Drosophila",
                f"Version: {project_version}",
                "",
            )
        ),
        encoding="utf-8",
    )
    return metadata_file


metadata_file = build_metadata()
datas.append((str(metadata_file), "."))

app_icon = project_root / "xdg" / "io.github.ergolyam.Drosophila.svg"
if app_icon.exists():
    datas.append((str(app_icon), "share/icons/hicolor/scalable/apps"))


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
    add_data_tree(
        mingw_prefix / rel,
        rel,
        ("hicolor/icon-theme.cache",) if rel == "share/icons" else (),
    )

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
