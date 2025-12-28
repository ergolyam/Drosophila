import os, sys
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), '..')))
from yggui.funcs.config import ConfigManager
from yggui.core.common import Binary, Runtime
from yggui.core.window import MyApp


def _ensure_prerequisites():
    if Binary.ygg_path is None:
        raise FileNotFoundError(
            "The 'yggdrasil' executable was not found in your PATH. "
            "Please install Yggdrasil or adjust your PATH environment "
            "variable accordingly."
        )

    if Binary.yggctl_path is None:
        raise FileNotFoundError(
            "The 'yggdrasilctl' executable was not found in your PATH. "
            "Please install Yggdrasil or adjust your PATH environment "
            "variable accordingly."
        )


def main():
    _ensure_prerequisites()
    config = ConfigManager(
        Runtime.config_path,
        ygg_path=Binary.ygg_path,
        admin_socket=Runtime.admin_socket,
        auto_init=True,
    )
    Runtime.config = config
    app = MyApp(
        application_id=Runtime.app_id
    )
    app.run(sys.argv)


if __name__ == "__main__":
    sys.exit(main())
