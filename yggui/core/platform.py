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


def popen_kwargs(debug: bool = False) -> dict:
    if not is_windows():
        return {}
    if debug:
        return {}
    flags = 0
    if hasattr(subprocess, "CREATE_NO_WINDOW"):
        flags |= subprocess.CREATE_NO_WINDOW
    return {"creationflags": flags}


def background_popen_kwargs(debug: bool = False) -> dict:
    if not is_windows():
        return {}
    if debug:
        flags = 0
        if hasattr(subprocess, "CREATE_NEW_PROCESS_GROUP"):
            flags |= subprocess.CREATE_NEW_PROCESS_GROUP
        return {"creationflags": flags}
    import ctypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    flags = 0
    if not kernel32.GetConsoleWindow() and hasattr(subprocess, "CREATE_NEW_CONSOLE"):
        flags |= subprocess.CREATE_NEW_CONSOLE
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
    kernel32.GetStdHandle.argtypes = [ctypes.c_ulong]
    kernel32.GetStdHandle.restype = ctypes.c_void_p
    kernel32.SetStdHandle.argtypes = [ctypes.c_ulong, ctypes.c_void_p]
    kernel32.SetStdHandle.restype = ctypes.c_bool
    std_handles = (
        (-10, kernel32.GetStdHandle(-10)),
        (-11, kernel32.GetStdHandle(-11)),
        (-12, kernel32.GetStdHandle(-12)),
    )
    attached = False
    if not kernel32.GetConsoleWindow():
        if not kernel32.AttachConsole(pid):
            return False
        attached = True
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
        if attached:
            kernel32.FreeConsole()
            for std_handle, value in std_handles:
                kernel32.SetStdHandle(std_handle, value)
