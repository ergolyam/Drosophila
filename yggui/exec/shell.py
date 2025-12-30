import subprocess
import time
from threading import Lock
from yggui.core.common import Runtime, Binary, get_logger

log = get_logger(__name__)


class Shell:
    _procs: dict[bool, subprocess.Popen[str] | None] = {False: None, True: None}
    _locks: dict[bool, Lock] = {False: Lock(), True: Lock()}

    @classmethod
    def _spawn_shell(cls, as_root: bool) -> subprocess.Popen[str]:
        cmd = []
        if as_root:
            if Runtime.is_flatpak:
                cmd.extend(["flatpak-spawn", "--host"])
            cmd.extend([str(Binary.pkexec_path), "--disable-internal-agent", "/bin/sh"])
        else:
            cmd.append("/bin/sh")

        return subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )

    @classmethod
    def _ensure_shell(cls, as_root: bool) -> subprocess.Popen[str]:
        lock = cls._locks[as_root]
        with lock:
            proc = cls._procs[as_root]
            if proc is None or proc.poll() is not None:
                proc = cls._spawn_shell(as_root)
                cls._procs[as_root] = proc
            return proc

    @classmethod
    def run_capture(cls, command: str, timeout: float = 15.0, as_root: bool = False) -> str:
        proc = cls._ensure_shell(as_root)

        stdin = proc.stdin
        stdout = proc.stdout
        assert stdin is not None
        assert stdout is not None

        marker = f"__YGGUI_DONE_{time.time_ns()}__"

        try:
            stdin.write(f"{command}; echo {marker}\n")
            stdin.flush()
        except BrokenPipeError:
            cls.stop(as_root)
            return cls.run_capture(command, timeout, as_root)

        output_lines: list[str] = []
        start = time.time()
        
        while True:
            line = stdout.readline()
            if not line:
                break
            if line.strip() == marker:
                break
            log.info(line.rstrip("\n"))
            output_lines.append(line)
            if time.time() - start > timeout:
                break

        return "".join(output_lines)

    @classmethod
    def run_background(cls, command: str, as_root: bool = False) -> int:
        proc = cls._ensure_shell(as_root)

        stdin = proc.stdin
        stdout = proc.stdout
        assert stdin is not None
        assert stdout is not None

        sentinel = "__YGGUI_PID__"

        try:
            stdin.write(f"{command} & echo $! {sentinel}\n")
            stdin.flush()
        except BrokenPipeError:
            cls.stop(as_root)
            return cls.run_background(command, as_root)

        while True:
            line = stdout.readline()
            if not line:
                break
            if sentinel in line:
                return int(line.split()[0])
            log.info(line.rstrip("\n"))

        raise RuntimeError("Failed to capture background PID")

    @classmethod
    def run(cls, command: str, as_root: bool = False) -> None:
        proc = cls._ensure_shell(as_root)

        stdin = proc.stdin
        assert stdin is not None

        try:
            stdin.write(f"{command}\n")
            stdin.flush()
        except BrokenPipeError:
            cls.stop(as_root)
            proc = cls._ensure_shell(as_root)
            if proc.stdin:
                proc.stdin.write(f"{command}\n")
                proc.stdin.flush()

    @classmethod
    def is_alive(cls, pid: int, as_root: bool = False) -> bool:
        cmd = f"kill -0 {pid} 2>/dev/null && echo __ALIVE__"
        try:
            output = cls.run_capture(cmd, timeout=2.0, as_root=as_root)
            return "__ALIVE__" in output
        except Exception:
            return False

    @classmethod
    def stop(cls, as_root: bool = False) -> None:
        lock = cls._locks[as_root]
        with lock:
            proc = cls._procs[as_root]
            if proc and proc.poll() is None:
                try:
                    stdin = proc.stdin
                    assert stdin is not None
                    stdin.write("exit\n")
                    stdin.flush()
                    proc.wait(timeout=3)
                except Exception:
                    proc.kill()
            cls._procs[as_root] = None


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
