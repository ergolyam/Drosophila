import json
import os
import subprocess
import tempfile
import time
from pathlib import Path
from threading import RLock
from typing import Any


class ConfigManager:
    _REQUIRED_KEYS: set[str] = {"Listen", "IfName"}

    def __init__(
        self,
        path: Path,
        *,
        ygg_path: str | None,
        admin_socket: Path,
        auto_init: bool = True,
    ) -> None:
        self.path = Path(path)
        self._ygg_path = ygg_path
        self._admin_socket = Path(admin_socket)
        self._auto_init = auto_init
        self._lock = RLock()

    def ensure_initialized(self) -> None:
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)

            current: dict[str, Any] = {}
            has_file = self.path.exists()
            if has_file:
                try:
                    current = self._load_unlocked(strict=True)
                except Exception:
                    ts = int(time.time())
                    bak = self.path.with_suffix(self.path.suffix + f".bak.{ts}")
                    try:
                        os.replace(self.path, bak)
                    except Exception:
                        pass
                    current = {}
                    has_file = False

            if self._ygg_path:
                needs_gen = (not has_file) or (not self._looks_like_ygg_config(current))
                if needs_gen:
                    base = self._genconf_dict()
                    base.update(current)
                    current = base
            else:
                if not has_file:
                    current = {}

            desired_admin = self._desired_admin_listen()
            if current.get("AdminListen") != desired_admin:
                current["AdminListen"] = desired_admin

            if (not self.path.exists()) or current:
                self._save_unlocked(current)

    def load(self) -> dict[str, Any]:
        with self._lock:
            self._maybe_init_unlocked()
            return self._load_unlocked(strict=False)

    def save(self, cfg: dict[str, Any]) -> None:
        with self._lock:
            self._maybe_init_unlocked()
            self._save_unlocked(cfg)

    def get(self, key: str, default: Any = None) -> Any:
        return self.load().get(key, default)

    def set(self, key: str, value: Any) -> None:
        with self._lock:
            self._maybe_init_unlocked()
            cfg = self._load_unlocked(strict=False)
            cfg[key] = value
            self._save_unlocked(cfg)

    def update(self, mapping: dict[str, Any] | None = None, **kwargs: Any) -> None:
        with self._lock:
            self._maybe_init_unlocked()
            cfg = self._load_unlocked(strict=False)
            if mapping:
                cfg.update(mapping)
            if kwargs:
                cfg.update(kwargs)
            self._save_unlocked(cfg)

    def __getitem__(self, key: str) -> Any:
        return self.get(key)

    def __setitem__(self, key: str, value: Any) -> None:
        self.set(key, value)

    def _maybe_init_unlocked(self) -> None:
        if self._auto_init:
            self.ensure_initialized()

    def _desired_admin_listen(self) -> str:
        return f"unix://{self._admin_socket}"

    def _looks_like_ygg_config(self, cfg: dict[str, Any]) -> bool:
        return all(k in cfg for k in self._REQUIRED_KEYS)

    def _genconf_dict(self) -> dict[str, Any]:
        if not self._ygg_path:
            raise FileNotFoundError("yggdrasil executable not found (ygg_path is None)")
        res = subprocess.run(
            [self._ygg_path, "-genconf", "-json"],
            capture_output=True,
            text=True,
            check=True,
        )
        return json.loads(res.stdout) or {}

    def _load_unlocked(self, *, strict: bool) -> dict[str, Any]:
        if not self.path.exists():
            return {}
        try:
            with self.path.open("r", encoding="utf-8") as f:
                data = json.load(f)
                return data if isinstance(data, dict) else {}
        except Exception:
            if strict:
                raise
            return {}

    def _save_unlocked(self, cfg: dict[str, Any]) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd, tmp_name = tempfile.mkstemp(
            prefix=self.path.name + ".",
            suffix=".tmp",
            dir=str(self.path.parent),
            text=True,
        )
        tmp = Path(tmp_name)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump(cfg, f, indent=2)
                f.write("\n")
            os.replace(tmp, self.path)
        finally:
            try:
                if tmp.exists():
                    tmp.unlink(missing_ok=True)
            except Exception:
                pass

if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
