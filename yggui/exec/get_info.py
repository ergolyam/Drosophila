import json
from yggui.core.common import Binary, Runtime
from yggui.exec.shell import Shell
from yggui.exec.pkexec_shell import PkexecShell


def get_self_info(use_socks) -> tuple[str | None, str | None]:
    if not Runtime.admin_socket.exists():
        return None, None

    cmd = (
        f"{Binary.yggctl_path} -json "
        f"-endpoint=unix://{str(Runtime.admin_socket)} getSelf"
    )
    if not use_socks:
        runner = PkexecShell
    else:
        runner = Shell
    try:
        output = runner.run_capture(cmd)
        data = json.loads(output)
        return data.get("address"), data.get("subnet")
    except Exception:
        return None, None


def get_peers_status(use_socks) -> dict[str, bool]:
    if not Runtime.admin_socket.exists():
        return {}

    cmd = (
        f"{Binary.yggctl_path} -json "
        f"-endpoint=unix://{str(Runtime.admin_socket)} getPeers"
    )
    def _parse_output(output: str) -> dict[str, bool]:
        data = json.loads(output)
        status: dict[str, bool] = {}
        for entry in data.get("peers", []):
            remote = entry.get("remote", "")
            if remote:
                status[remote.split("?", 1)[0]] = bool(entry.get("up"))
        return status
    if not use_socks:
        runner = PkexecShell
    else:
        runner = Shell
    try:
        output = runner.run_capture(cmd)
        return _parse_output(output)
    except Exception:
        return {}


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
