from yggui.core.common import Binary, Runtime
from yggui.exec.shell import Shell
from yggui.exec.pkexec_shell import PkexecShell


def start_ygg(use_socks: bool, socks_args) -> int:
    Runtime.config.ensure_initialized()
    cmd = []
    if not use_socks:
        runner = PkexecShell
        cmd.append(str(Binary.ygg_path))
    else:
        listen: str = socks_args.get("listen", "127.0.0.1:1080")
        dns_ip: str = socks_args.get("dns_ip", "")
        dns_port: str = socks_args.get("dns_port", "53")
        runner = Shell
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
    return runner.run_background(' '.join(cmd))


def stop_ygg(use_socks: bool, pid: int) -> None:
    if not use_socks:
        runner = PkexecShell
    else:
        runner = Shell
    runner.run(f"/usr/bin/kill -s SIGINT {pid}")


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
