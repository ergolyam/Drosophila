import os, shlex, shutil, subprocess, sys
from pathlib import Path


def is_windows() -> bool:
    return os.name == "nt"


def is_frozen() -> bool:
    return getattr(sys, "frozen", False)


def app_dir() -> Path:
    if is_frozen():
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parents[2]


def xdg_config(app_name: str) -> Path:
    default_base = Path.home() / ".config"
    base = Path(os.environ.get("XDG_CONFIG_HOME", default_base)).expanduser()
    cfg_dir = base / app_name
    cfg_dir.mkdir(parents=True, exist_ok=True)
    return cfg_dir


def runtime_dir(app_name: str) -> Path:
    if is_windows():
        path = app_dir()
    else:
        path = Path(os.environ.get('XDG_RUNTIME_DIR', '/tmp')) / app_name
    path.mkdir(parents=True, exist_ok=True)
    return path


def data_dir(app_name: str) -> Path:
    if is_windows():
        path = app_dir()
    else:
        path = Path(os.environ.get('XDG_DATA_HOME', Path.home() / '.local/share')) / app_name
    path.mkdir(parents=True, exist_ok=True)
    return path


def config_path(app_name: str) -> Path:
    if is_windows():
        return app_dir() / "yggdrasil.conf"
    return xdg_config(app_name) / "config.json"


def admin_listen(socket_path: Path) -> str:
    if is_windows():
        return "tcp://127.0.0.1:9001"
    return f"unix://{socket_path}"


def admin_available(socket_path: Path) -> bool:
    if is_windows():
        return True
    return socket_path.exists()


def executable_name(name: str) -> str:
    if is_windows() and not name.lower().endswith(".exe"):
        return f"{name}.exe"
    return name


def binary_path(name: str) -> str | None:
    exe_name = executable_name(name)
    bundled = app_dir() / exe_name
    if bundled.exists():
        return str(bundled)
    return shutil.which(exe_name) or shutil.which(name)


def command_line(args) -> str:
    values = [str(arg) for arg in args]
    if is_windows():
        return subprocess.list2cmdline(values)
    return " ".join(shlex.quote(arg) for arg in values)


def popen_kwargs() -> dict:
    if not is_windows():
        return {}
    flags = 0
    if hasattr(subprocess, "CREATE_NO_WINDOW"):
        flags |= subprocess.CREATE_NO_WINDOW
    return {"creationflags": flags}


def background_popen_kwargs() -> dict:
    if not is_windows():
        return {}
    import ctypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    if not kernel32.GetConsoleWindow() and kernel32.AllocConsole():
        console_window = kernel32.GetConsoleWindow()
        if console_window:
            user32.ShowWindow(console_window, 0)
    flags = 0
    if hasattr(subprocess, "CREATE_NEW_PROCESS_GROUP"):
        flags |= subprocess.CREATE_NEW_PROCESS_GROUP
    startupinfo = subprocess.STARTUPINFO()
    startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startupinfo.wShowWindow = subprocess.SW_HIDE
    return {"creationflags": flags, "startupinfo": startupinfo}


def send_console_break(pid: int) -> bool:
    if not is_windows():
        return False
    import ctypes
    import time

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    if not kernel32.GetConsoleWindow():
        return False
    ignore_break = bool(kernel32.SetConsoleCtrlHandler(None, True))
    try:
        if not ignore_break:
            return False
        if not kernel32.GenerateConsoleCtrlEvent(1, pid):
            return False
        time.sleep(0.2)
        return True
    finally:
        if ignore_break:
            kernel32.SetConsoleCtrlHandler(None, False)
