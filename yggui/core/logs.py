import logging
import subprocess
import sys

from yggui.core import platform as ygg_platform


_DEBUG = False
_ROOT_LOGGER_NAME = "yggui"


def _ensure_windows_console() -> None:
    if not ygg_platform.is_windows():
        return

    try:
        import ctypes
        import msvcrt
    except Exception:
        return

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.AttachConsole.argtypes = [ctypes.c_ulong]
    kernel32.AttachConsole.restype = ctypes.c_bool
    kernel32.AllocConsole.restype = ctypes.c_bool
    kernel32.GetConsoleWindow.restype = ctypes.c_void_p
    kernel32.SetStdHandle.argtypes = [ctypes.c_ulong, ctypes.c_void_p]
    kernel32.SetStdHandle.restype = ctypes.c_bool
    if kernel32.GetConsoleWindow():
        return

    attach_parent_process = ctypes.c_ulong(0xFFFFFFFF).value
    if not kernel32.AttachConsole(attach_parent_process):
        kernel32.AllocConsole()

    try:
        stdout = open("CONOUT$", "w", encoding="utf-8", buffering=1)
        stderr = open("CONOUT$", "w", encoding="utf-8", buffering=1)
        sys.stdout = stdout
        sys.stderr = stderr
        kernel32.SetStdHandle(
            ctypes.c_ulong(-11).value,
            ctypes.c_void_p(msvcrt.get_osfhandle(stdout.fileno())),
        )
        kernel32.SetStdHandle(
            ctypes.c_ulong(-12).value,
            ctypes.c_void_p(msvcrt.get_osfhandle(stderr.fileno())),
        )
    except OSError:
        pass


def configure_logging(debug: bool) -> None:
    global _DEBUG

    _DEBUG = debug
    root = logging.getLogger(_ROOT_LOGGER_NAME)
    root.handlers.clear()
    root.setLevel(logging.INFO)
    root.propagate = False

    if not debug:
        root.addHandler(logging.NullHandler())
        return

    _ensure_windows_console()

    handler = logging.StreamHandler()
    handler.setFormatter(
        logging.Formatter("[%(asctime)s] %(name)s: %(message)s", "%H:%M:%S")
    )
    root.addHandler(handler)


def debug_enabled() -> bool:
    return _DEBUG


def get_logger(name: str) -> logging.Logger:
    if name != _ROOT_LOGGER_NAME and not name.startswith(f"{_ROOT_LOGGER_NAME}."):
        name = f"{_ROOT_LOGGER_NAME}.{name}"
    logger = logging.getLogger(name)
    logger.setLevel(logging.INFO)
    return logger


def subprocess_output_kwargs() -> dict:
    if _DEBUG:
        return {"stdout": None, "stderr": None}
    return {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}


configure_logging(False)
