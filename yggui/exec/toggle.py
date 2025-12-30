from yggui.core.common import Binary, Runtime
from yggui.exec.shell import Shell


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
    return Shell.run_background(' '.join(cmd), as_root=as_root)


def stop_ygg(use_socks: bool, pid: int) -> None:
    as_root = not use_socks
    Shell.run(f"/usr/bin/kill -s SIGINT {pid}", as_root=as_root)


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
