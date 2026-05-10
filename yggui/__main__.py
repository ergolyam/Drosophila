import os, sys
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..')))
from yggui.core.runtime import configure_runtime
configure_runtime()
from yggui.funcs.config import ConfigManager
from yggui.core.common import Binary, Runtime
from yggui.core.logs import configure_logging, get_logger


log = get_logger(__name__)


def _extract_debug_arg(argv: list[str]) -> tuple[bool, list[str]]:
    debug = False
    filtered = []
    for arg in argv:
        if arg == "--debug":
            debug = True
            continue
        filtered.append(arg)
    return debug, filtered


def _ensure_prerequisites():
    if Binary.ygg_path is None:
        if Runtime.is_windows:
            raise FileNotFoundError(
                "The 'yggdrasil.exe' executable was not found next to "
                "Drosophila.exe or in your PATH."
            )
        else:
            raise FileNotFoundError(
                "The 'yggdrasil' executable was not found in your PATH. "
                "Please install Yggdrasil or adjust your PATH environment "
                "variable accordingly."
            )

    if Binary.yggctl_path is None:
        if Runtime.is_windows:
            raise FileNotFoundError(
                "The 'yggdrasilctl.exe' executable was not found next to "
                "Drosophila.exe or in your PATH."
            )
        else:
            raise FileNotFoundError(
                "The 'yggdrasilctl' executable was not found in your PATH. "
                "Please install Yggdrasil or adjust your PATH environment "
                "variable accordingly."
            )


def main(argv: list[str] | None = None):
    debug, app_argv = _extract_debug_arg(list(sys.argv if argv is None else argv))
    sys.argv = app_argv
    configure_logging(debug)
    if debug:
        log.info("Debug logging enabled")

    from yggui.core.window import MyApp

    _ensure_prerequisites()
    config = ConfigManager(
        Runtime.config_path,
        ygg_path=Binary.ygg_path,
        admin_listen=Runtime.admin_listen,
        auto_init=True,
    )
    Runtime.config = config
    app = MyApp(
        application_id=Runtime.app_id
    )
    return app.run(app_argv)


if __name__ == "__main__":
    sys.exit(main())
