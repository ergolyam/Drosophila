import shutil, os, subprocess, re, logging
from importlib.metadata import version, PackageNotFoundError
from pathlib import Path
from importlib.resources import files
import xml.etree.ElementTree as ET

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


def xdg_config(app_name: str) -> Path:
    default_base = Path.home() / ".config"
    base = Path(os.environ.get("XDG_CONFIG_HOME", default_base)).expanduser()
    cfg_dir = base / app_name
    cfg_dir.mkdir(parents=True, exist_ok=True)
    return cfg_dir


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
    is_flatpak = Path('/.flatpak-info').is_file()
    is_appimage = os.getenv("APPIMAGE") is not None
    runtime_dir = Path(os.environ.get('XDG_RUNTIME_DIR', '/tmp')) / 'yggui'
    runtime_dir.mkdir(parents=True, exist_ok=True)
    admin_socket = runtime_dir / 'yggdrasil.sock'
    config_path = xdg_config('yggui') / 'config.json'
    try:
        version = version("Drosophila")
    except PackageNotFoundError:
        version = "0.0.0"


class Binary:
    ygg_path = shutil.which('yggdrasil')
    yggctl_path = shutil.which('yggdrasilctl')
    yggctl_path_stack = yggctl_path
    yggstack_path = shutil.which('yggstack')
    pkexec_path = shutil.which('pkexec')
    if Runtime.is_flatpak or Runtime.is_appimage:
        if Runtime.is_flatpak:
            pkexec_path = which_in_flatpak('pkexec')
        if ygg_path:
            dst = Runtime.runtime_dir / 'yggdrasil'
            shutil.copy2(ygg_path, dst)
            ygg_path = str(dst)
        if yggctl_path:
            dst = Runtime.runtime_dir / 'yggdrasilctl'
            shutil.copy2(yggctl_path, dst)
            yggctl_path = str(dst)


class Gui:
    ui_file = files('yggui.ui').joinpath('ui.ui')
    ui_main_file = files('yggui.ui').joinpath('main.ui')
    ui_settings_file = files('yggui.ui').joinpath('settings.ui')
    peer_ui_file = files('yggui.ui').joinpath('peer_dialog.ui')
    about_ui_file = files("yggui.ui").joinpath("about_dialog.ui")
    css_file = files('yggui.ui').joinpath('ui.css')


def _find_metainfo_file() -> Path | None:
    app_id = Runtime.app_id
    names = [
        f"{app_id}.metainfo.xml",
        f"{app_id}.appdata.xml",
    ]
    prefixes = [
        Path("/app/share/metainfo"),
        Path("/usr/share/metainfo"),
        Path("/usr/local/share/metainfo"),
        Path("xdg/")
    ]
    if Runtime.is_appimage and (appdir := os.getenv("APPDIR")):
        prefixes = [
            Path(appdir) / "usr/share/metainfo",
            Path(appdir) / "share/metainfo"
        ]
    for prefix in prefixes:
        for name in names:
            path = prefix / name
            if path.is_file():
                return path
    return None


def get_app_info() -> dict:
    path = _find_metainfo_file()
    if not path:
        return {}

    try:
        root = ET.parse(path).getroot()

        name = root.findtext("name", default="")
        project_license = root.findtext("project_license", default="")
        summary = root.findtext("summary", default="")

        desc_elem = root.find("description")
        if desc_elem is not None:
            description = "".join(desc_elem.itertext()).strip()
        else:
            description = ""

        release = root.find(".//release")
        version = release.attrib.get("version") if release is not None else "dev"
        developer_name = root.findtext("developer/name", default="")

        homepage_url = ""
        issue_url = ""

        for url_elem in root.findall("url"):
            url_type = url_elem.attrib.get("type", "")
            if url_type == "homepage":
                homepage_url = url_elem.text or ""
            elif url_type == "bugtracker":
                issue_url = url_elem.text or ""

        return {
            "name": name,
            "project_license": project_license,
            "summary": summary,
            "description": description,
            "version": version,
            "developer_name": developer_name,
            "website": homepage_url,
            "issue_url": issue_url
        }

    except ET.ParseError:
        return {}


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
