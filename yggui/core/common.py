import shutil, os, subprocess, re, logging, hashlib
from yggui.funcs.config import ConfigManager
from importlib.metadata import PackageNotFoundError, version as package_version
from pathlib import Path
from importlib.resources import files
from yggui.core import platform as ygg_platform

def which_in_flatpak(cmd: str) -> str | None:
    result = subprocess.run(
        ["flatpak-spawn", "--host", "sh", "-c", f"command -v {cmd}"],
        capture_output=True,
        text=True,
        check=False
    )
    if result.returncode == 0:
        return result.stdout.strip()
    return None


def sha256sum(path, chunk_size=1024 * 1024):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(chunk_size), b''):
            h.update(chunk)
    return h.hexdigest()


def copy_different(src, dst):
    dst = Path(dst)
    if not dst.exists():
        shutil.copy2(src, dst)
        return
    if Path(src).stat().st_size != dst.stat().st_size:
        shutil.copy2(src, dst)
        return
    if sha256sum(src) != sha256sum(dst):
        shutil.copy2(src, dst)


def _metadata_file_version(path: Path) -> str | None:
    try:
        with path.open(encoding="utf-8") as metadata:
            for line in metadata:
                key, separator, value = line.partition(":")
                if separator and key.lower() == "version":
                    version = value.strip()
                    if version:
                        return version
    except OSError:
        return None
    return None


def _app_version() -> str:
    try:
        return package_version("Drosophila")
    except PackageNotFoundError:
        if ygg_platform.is_windows() and ygg_platform.is_frozen():
            version = _metadata_file_version(ygg_platform.app_dir() / "METADATA")
            if version:
                return version
        return "0.0.0"


class Common:
    urls = [ "https://publicpeers.neilalexander.dev/publicnodes.json", "https://peers.yggdrasil.link/publicnodes.json"]
    protocols = ["tcp", "tls", "ws", "wss", "quic"]

class Regexp:
    domain_re = re.compile(
        r"""^(
            (?:[a-zA-Z0-9-]+\.)+[a-zA-Z]{2,}(?::\d{1,5})?
            |
            (?:\d{1,3}\.){3}\d{1,3}:\d{1,5}
        )$""",
        re.X,
    )
    sni_re = re.compile(r"^(?:[a-zA-Z0-9-]+\.)+[a-zA-Z]{2,}$")


class Runtime:
    app_id = "io.github.ergolyam.Drosophila"
    is_windows = ygg_platform.is_windows()
    is_flatpak = Path('/.flatpak-info').is_file()
    is_appimage = os.getenv("APPIMAGE") is not None
    runtime_dir = ygg_platform.runtime_dir('yggui')
    bin_dir = ygg_platform.data_dir('yggui')
    admin_socket = runtime_dir / 'yggdrasil.sock'
    admin_listen = ygg_platform.admin_listen(admin_socket)
    config_path = ygg_platform.config_path('yggui')
    config: ConfigManager
    version = _app_version()


class Binary:
    if Runtime.is_windows:
        ygg_path = ygg_platform.binary_path('yggdrasil')
        yggctl_path = ygg_platform.binary_path('yggdrasilctl')
        yggstack_path = ygg_platform.binary_path('yggstack')
        pkexec_path = None
    else:
        ygg_path = shutil.which('yggdrasil')
        yggctl_path = shutil.which('yggdrasilctl')
        yggstack_path = shutil.which('yggstack')
        pkexec_path = shutil.which('pkexec')
    if not Runtime.is_windows and (Runtime.is_flatpak or Runtime.is_appimage):
        if Runtime.is_flatpak:
            pkexec_path = which_in_flatpak('pkexec')
        if ygg_path:
            dst = Runtime.bin_dir / 'yggdrasil'
            copy_different(ygg_path, dst)
            ygg_path = str(dst)
        if yggctl_path:
            dst = Runtime.bin_dir / 'yggdrasilctl'
            copy_different(yggctl_path, dst)
            yggctl_path = str(dst)

class Gui:
    ui_file = files('yggui.ui').joinpath('ui.ui')
    ui_main_file = files('yggui.ui').joinpath('main.ui')
    ui_settings_file = files('yggui.ui').joinpath('settings.ui')
    peer_ui_file = files('yggui.ui').joinpath('peer_dialog.ui')
    peer_discovery_ui_file = files('yggui.ui').joinpath('peer_discovery.ui')
    about_ui_file = files("yggui.ui").joinpath("about_dialog.ui")
    css_file = files('yggui.ui').joinpath('ui.css')


def get_logger(name: str) -> logging.Logger:
    logger = logging.getLogger(name)

    if not logger.handlers:
        logger.setLevel(logging.INFO)

        fname = name.rsplit('.', 1)[-1]
        file_path = Runtime.runtime_dir / f'{fname}.log'
        handler = logging.FileHandler(file_path, encoding='utf-8', delay=True)
        handler.setFormatter(logging.Formatter('%(asctime)s  %(message)s'))

        logger.addHandler(handler)
        logger.propagate = False

    return logger
