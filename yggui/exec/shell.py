import os
import subprocess
import time
from threading import Lock, Thread
from yggui.core.common import Runtime, Binary
from yggui.core import platform as ygg_platform
from yggui.core.logs import (
    debug_enabled,
    get_logger,
    subprocess_output_kwargs,
)

log = get_logger(__name__)


class Shell:
    _procs: dict[bool, subprocess.Popen[str] | None] = {False: None, True: None}
    _locks: dict[bool, Lock] = {False: Lock(), True: Lock()}
    _direct_procs: dict[int, subprocess.Popen] = {}

    @classmethod
    def _log_output_pipe(cls, path) -> None:
        try:
            with path.open("r", encoding="utf-8", errors="replace") as stream:
                for line in stream:
                    log.info(line.rstrip("\n"))
        except OSError as exc:
            log.info("Failed to read process output: %s", exc)
        finally:
            try:
                path.unlink(missing_ok=True)
            except OSError:
                pass

    @classmethod
    def _background_command(cls, command: str) -> str:
        if not debug_enabled():
            return f"( exec > /dev/null 2>&1; exec {command} )"

        try:
            pipe_path = Runtime.bin_dir / f"yggui-output-{time.time_ns()}.pipe"
            os.mkfifo(pipe_path, 0o600)
        except OSError as exc:
            log.info("Failed to create debug output pipe: %s", exc)
            return f"( exec > /dev/null 2>&1; exec {command} )"

        Thread(target=cls._log_output_pipe, args=(pipe_path,), daemon=True).start()
        output_path = ygg_platform.command_line([pipe_path])
        return f"( exec > {output_path} 2>&1; exec {command} )"

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
            start_new_session=as_root,
        )

    @classmethod
    def _ensure_shell(cls, as_root: bool) -> subprocess.Popen[str]:
        proc = cls._procs[as_root]
        if proc is None or proc.poll() is not None:
            proc = cls._spawn_shell(as_root)
            cls._procs[as_root] = proc
        return proc

    @classmethod
    def is_alive(cls, pid: int, as_root: bool = False) -> bool:
        if Runtime.is_windows:
            proc = cls._direct_procs.get(pid)
            if proc is not None:
                alive = proc.poll() is None
                if not alive:
                    cls._direct_procs.pop(pid, None)
                return alive
            result = subprocess.run(
                ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
                capture_output=True,
                text=True,
                check=False,
                **ygg_platform.popen_kwargs(debug_enabled()),
            )
            return str(pid) in result.stdout

        cmd = f"kill -0 {pid} 2>/dev/null && echo __ALIVE__"
        try:
            output = cls.run_capture(
                cmd,
                timeout=2.0,
                as_root=as_root,
                log_output=False,
            )
            return "__ALIVE__" in output
        except Exception:
            return False

    @classmethod
    def run_capture(
        cls,
        command: str,
        timeout: float = 15.0,
        as_root: bool = False,
        log_output: bool = True,
    ) -> str:
        if Runtime.is_windows:
            result = subprocess.run(
                command,
                shell=True,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
                **ygg_platform.popen_kwargs(debug_enabled()),
            )
            output = (result.stdout or "") + (result.stderr or "")
            if log_output:
                for line in output.splitlines():
                    log.info(line)
            return output

        lock = cls._locks[as_root]
        with lock:
            proc = cls._ensure_shell(as_root)

            stdin = proc.stdin
            stdout = proc.stdout
            assert stdin is not None
            assert stdout is not None

            marker = f"__YGGUI_DONE_{time.time_ns()}__"

            try:
                stdin.write(f"{command}; echo {marker}\n")
                stdin.flush()
            except (BrokenPipeError, OSError):
                if proc.poll() is None:
                    proc.kill()
                cls._procs[as_root] = None
                proc = cls._ensure_shell(as_root)
                stdin = proc.stdin
                stdout = proc.stdout
                assert stdin is not None
                assert stdout is not None
                if stdin:
                    stdin.write(f"{command}; echo {marker}\n")
                    stdin.flush()

            output_lines: list[str] = []
            start = time.time()
            
            while True:
                line = stdout.readline()
                if not line:
                    break
                if line.strip() == marker:
                    break
                if log_output:
                    log.info(line.rstrip("\n"))
                output_lines.append(line)
                if time.time() - start > timeout:
                    break

            return "".join(output_lines)

    @classmethod
    def run_background(cls, command: str, as_root: bool = False) -> int:
        if Runtime.is_windows:
            return cls.run_background_args([command], as_root=as_root, shell=True)

        lock = cls._locks[as_root]
        with lock:
            proc = cls._ensure_shell(as_root)

            stdin = proc.stdin
            stdout = proc.stdout
            assert stdin is not None
            assert stdout is not None

            sentinel = "__YGGUI_PID__"

            try:
                background_command = cls._background_command(command)
                stdin.write(f"{background_command} & echo $! {sentinel}\n")
                stdin.flush()
            except (BrokenPipeError, OSError):
                if proc.poll() is None:
                    proc.kill()
                cls._procs[as_root] = None
                proc = cls._ensure_shell(as_root)
                stdin = proc.stdin
                stdout = proc.stdout
                assert stdin is not None
                assert stdout is not None
                if stdin:
                    background_command = cls._background_command(command)
                    stdin.write(f"{background_command} & echo $! {sentinel}\n")
                    stdin.flush()

            while True:
                line = stdout.readline()
                if not line:
                    break
                if sentinel in line:
                    return int(line.split()[0])
                log.info(line.rstrip("\n"))

            raise RuntimeError("Failed to capture background PID")

    @classmethod
    def run_background_args(cls, command, as_root: bool = False, shell: bool = False) -> int:
        if Runtime.is_windows:
            output = subprocess_output_kwargs()
            proc = subprocess.Popen(
                [str(arg) for arg in command] if not shell else str(command[0]),
                stdin=subprocess.DEVNULL,
                text=True,
                shell=shell,
                **output,
                **ygg_platform.background_popen_kwargs(debug_enabled()),
            )
            cls._direct_procs[proc.pid] = proc
            return proc.pid
        return cls.run_background(ygg_platform.command_line(command), as_root=as_root)

    @classmethod
    def run(cls, command: str, as_root: bool = False) -> None:
        if Runtime.is_windows:
            output = subprocess_output_kwargs()
            subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                text=True,
                shell=True,
                **output,
                **ygg_platform.popen_kwargs(debug_enabled()),
            )
            return

        lock = cls._locks[as_root]
        with lock:
            proc = cls._ensure_shell(as_root)
            stdin = proc.stdin
            if stdin:
                try:
                    stdin.write(f"{command}\n")
                    stdin.flush()
                except BrokenPipeError:
                    pass

    @classmethod
    def stop_pid(cls, pid: int, as_root: bool = False) -> None:
        if Runtime.is_windows:
            proc = cls._direct_procs.pop(pid, None)
            if proc is not None and proc.poll() is not None:
                return
            if ygg_platform.send_console_break(pid):
                if proc is not None:
                    try:
                        proc.wait(timeout=15)
                        return
                    except subprocess.TimeoutExpired:
                        log.info("Timed out waiting for process %s to stop", pid)
                else:
                    deadline = time.time() + 15
                    while time.time() < deadline:
                        if not cls.is_alive(pid):
                            return
                        time.sleep(0.2)
                    log.info("Timed out waiting for process %s to stop", pid)
            else:
                log.info("Failed to send CTRL+BREAK to process %s", pid)
            if proc is not None and proc.poll() is None:
                proc.kill()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
            return
        cls.run(f"/usr/bin/kill -s SIGINT {pid}", as_root=as_root)

    @classmethod
    def stop(cls, as_root: bool = False) -> None:
        lock = cls._locks[as_root]
        with lock:
            proc = cls._procs[as_root]
            if proc and proc.poll() is None:
                try:
                    stdin = proc.stdin
                    if stdin:
                        stdin.write("exit\n")
                        stdin.flush()
                    proc.wait(timeout=3)
                except Exception:
                    proc.kill()
            cls._procs[as_root] = None


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
