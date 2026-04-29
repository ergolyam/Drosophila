from yggui.core.common import Binary, Runtime
from yggui.core.logs import get_logger
from yggui.core import platform as ygg_platform
from yggui.exec.shell import Shell


log = get_logger(__name__)


def start_ygg(use_socks: bool, socks_args) -> int:
    Runtime.config.ensure_initialized()
    cmd = []
    as_root = not use_socks
    if not use_socks:
        cmd.append(str(Binary.ygg_path))
    else:
        listen: str = socks_args.get("listen", "127.0.0.1:1080")
        dns_ip: str = socks_args.get("dns_ip", "")
        dns_port: str = socks_args.get("dns_port", "53")
        cmd.append(str(Binary.yggstack_path))
        if listen:
            cmd.extend(["-socks", listen])
        if dns_ip:
            if ":" in dns_ip and not dns_ip.startswith("["):
                nameserver = f"[{dns_ip}]:{dns_port}"
            else:
                nameserver = f"{dns_ip}:{dns_port}"
            cmd.extend(["-nameserver", nameserver])
    cmd.extend(["-useconffile", str(Runtime.config_path.resolve())])
    ygg_app = "Yggstack" if use_socks else "Yggdrasil"
    log.info("Starting %s: %s", ygg_app, ygg_platform.command_line(cmd))
    pid = Shell.run_background_args(cmd, as_root=as_root)
    log.info("%s started with PID %s", ygg_app, pid)
    return pid


def stop_ygg(use_socks: bool, pid: int) -> None:
    as_root = not use_socks
    ygg_app = "Yggstack" if use_socks else "Yggdrasil"
    log.info("Stopping %s with PID %s", ygg_app, pid)
    Shell.stop_pid(pid, as_root=as_root)
    log.info("%s stop requested", ygg_app)


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
