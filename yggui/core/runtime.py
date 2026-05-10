import os
from pathlib import Path

from yggui.core import platform as ygg_platform


def _prepend_env(name: str, path: Path) -> None:
    if not path.exists():
        return
    current = os.environ.get(name)
    value = str(path)
    if current:
        value = value + os.pathsep + current
    os.environ[name] = value


def configure_runtime() -> None:
    if not ygg_platform.is_windows() or not ygg_platform.is_frozen():
        return

    base = ygg_platform.app_dir()
    share_dir = base / "share"

    _prepend_env("PATH", base)
    _prepend_env("GI_TYPELIB_PATH", base / "gi_typelibs")
    _prepend_env("GI_TYPELIB_PATH", base / "lib" / "girepository-1.0")
    _prepend_env("XDG_DATA_DIRS", share_dir)

    schemas = share_dir / "glib-2.0" / "schemas"
    if schemas.exists():
        os.environ["GSETTINGS_SCHEMA_DIR"] = str(schemas)
