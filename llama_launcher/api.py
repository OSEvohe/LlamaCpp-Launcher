"""Service facade for LLama Launcher core operations."""
import json
from dataclasses import asdict
from pathlib import Path
from typing import Dict, List

from llama_launcher import command as cmd_module
from llama_launcher import discovery
from llama_launcher import monitoring
from llama_launcher import process
from llama_launcher.config import (
    APP_DIR,
    DEFAULT_LLAMA_SERVER,
    load_global as _config_load_global,
    load_profiles as _config_load_profiles,
    save_global as _config_save_global,
    save_profiles as _config_save_profiles,
)
from llama_launcher.models import GlobalSettings, LlamaOption, Profile
from llama_launcher.options import load_options_from_exe, resolve_llama_server_path


class LlamaLauncherService:
    """Facade encapsulating all core LLama Launcher operations."""

    def __init__(self, app_dir: Path | None = None) -> None:
        self._app_dir = app_dir or APP_DIR
        self._state_dir = self._app_dir / ".launcher"
        self._global_file = self._state_dir / "global.json"
        self._profiles_file = self._state_dir / "profiles.json"
        self._pid_file = self._state_dir / "llama-server.pid"
        self._log_out = self._state_dir / "llama-server.log"
        self._log_err = self._state_dir / "llama-server.err.log"
        self._last_log_size = 0
        self._last_log_marker = ""

    # -- internal helpers --------------------------------------------------

    def _ensure_state(self) -> None:
        self._state_dir.mkdir(parents=True, exist_ok=True)

    def _load_global_path(self) -> GlobalSettings:
        self._ensure_state()
        if not self._global_file.exists():
            return GlobalSettings()
        try:
            data = json.loads(self._global_file.read_text(encoding="utf-8"))
            return GlobalSettings(
                llama_server_path=data.get("llama_server_path", ""),
                model_dirs=data.get("model_dirs", []),
            )
        except Exception:
            return GlobalSettings()

    def _save_global_path(self, settings: GlobalSettings) -> None:
        self._ensure_state()
        self._global_file.write_text(
            json.dumps(asdict(settings), indent=2), encoding="utf-8"
        )

    def _load_profiles_path(self) -> List[Profile]:
        self._ensure_state()
        if not self._profiles_file.exists():
            return [Profile()]
        try:
            raw = json.loads(self._profiles_file.read_text(encoding="utf-8"))
            entries = raw.get("profiles", [])
            profiles: List[Profile] = []
            for item in entries:
                if not isinstance(item, dict):
                    continue
                if "flash_attn" in item and "flash_attn_mode" not in item:
                    item["flash_attn_mode"] = "on" if item.get("flash_attn") else "off"
                item.pop("flash_attn", None)
                item.setdefault("advanced_values", {})
                item.setdefault("advanced_favorites", [])
                profiles.append(Profile(**item))
            return profiles or [Profile()]
        except Exception:
            return [Profile()]

    def _save_profiles_path(self, profiles: List[Profile]) -> None:
        self._ensure_state()
        payload = {"profiles": [asdict(p) for p in profiles]}
        self._profiles_file.write_text(json.dumps(payload, indent=2), encoding="utf-8")

    # -- profiles ----------------------------------------------------------

    def load_profiles(self) -> List[Profile]:
        if self._app_dir is APP_DIR:
            return _config_load_profiles()
        return self._load_profiles_path()

    def save_profiles(self, profiles: List[Profile]) -> None:
        if self._app_dir is APP_DIR:
            _config_save_profiles(profiles)
        else:
            self._save_profiles_path(profiles)

    def add_profile(self, name: str) -> Profile:
        profiles = self.load_profiles()
        profile = Profile(name=name)
        profiles.append(profile)
        self.save_profiles(profiles)
        return profile

    def delete_profile(self, index: int) -> None:
        profiles = self.load_profiles()
        if 0 <= index < len(profiles):
            profiles.pop(index)
            if not profiles:
                profiles.append(Profile())
            self.save_profiles(profiles)

    # -- global settings ---------------------------------------------------

    def load_global(self) -> GlobalSettings:
        if self._app_dir is APP_DIR:
            return _config_load_global()
        return self._load_global_path()

    def save_global(self, settings: GlobalSettings) -> None:
        if self._app_dir is APP_DIR:
            _config_save_global(settings)
        else:
            self._save_global_path(settings)

    # -- options -----------------------------------------------------------

    def load_options(self, exe_path: str) -> Dict[str, LlamaOption]:
        exe = resolve_llama_server_path(exe_path)
        if not exe.exists():
            raise RuntimeError("Chemin llama-server invalide.")
        return load_options_from_exe(exe)

    # -- model discovery ---------------------------------------------------

    def discover_models(self, model_dirs: List[str]) -> List[str]:
        return discovery.scan_gguf_models(model_dirs)

    # -- command assembly --------------------------------------------------

    def build_command(
        self, profile: Profile, exe_path: str, options: Dict[str, LlamaOption]
    ) -> List[str]:
        if not exe_path or not exe_path.strip():
            raise RuntimeError("Chemin llama-server non defini")
        exe = resolve_llama_server_path(exe_path)
        if exe.suffix.lower() != ".exe":
            raise RuntimeError("Le chemin doit pointer vers llama-server.exe")
        if not exe.exists():
            raise RuntimeError("llama-server.exe introuvable")
        if not profile.model_path or not Path(profile.model_path).exists():
            raise RuntimeError("Modele GGUF introuvable")
        return cmd_module.build_command(exe, profile, options)

    # -- process lifecycle -------------------------------------------------

    def is_server_running(self) -> tuple[bool, int]:
        pid = process.read_pid(self._pid_file)
        return (process.is_process_running(pid), pid)

    def launch(self, cmd: list, exe_path: str = "") -> int:
        self._ensure_state()
        existing_pid = process.read_pid(self._pid_file)
        if existing_pid > 0 and process.is_process_running(existing_pid):
            raise RuntimeError(f"llama-server deja en cours (PID {existing_pid}). Stop avant relance.")
        if existing_pid > 0 and self._pid_file.exists():
            self._pid_file.unlink()
        self._last_log_size = 0
        self._last_log_marker = ""
        if self._log_out.exists():
            self._log_out.unlink()
        if self._log_err.exists():
            self._log_err.unlink()
        child_pid = process.start_server(cmd, self._log_out, self._app_dir)
        self._pid_file.write_text(str(child_pid), encoding="utf-8")
        if exe_path:
            settings = self.load_global()
            settings.llama_server_path = exe_path.strip()
            self.save_global(settings)
        return child_pid

    def stop(self) -> int:
        pid = process.read_pid(self._pid_file)
        if pid <= 0:
            return 0
        if not process.is_process_running(pid):
            if self._pid_file.exists():
                self._pid_file.unlink()
            return 0
        process.stop_server(pid)
        if self._pid_file.exists():
            self._pid_file.unlink()
        return pid

    # -- monitoring --------------------------------------------------------

    def get_ram_usage(self) -> tuple[int, int]:
        return monitoring.ram_usage_bytes()

    def get_process_ram(self, pid: int) -> int:
        return monitoring.process_ram_bytes(pid)

    def get_gpu_vram(self) -> tuple[int, int]:
        return monitoring.gpu_vram_info()

    def format_bytes(self, value: int) -> str:
        return monitoring.bytes_to_gb(value)

    # -- log tailing -------------------------------------------------------

    def tail_log(self, last_size: int, last_marker: str = "") -> tuple[str, int, bool, str]:
        if not self._log_out.exists():
            return ("", last_size, False, last_marker)
        return monitoring.tail_log_chunk(self._log_out, last_size, last_marker)

    # -- monitoring text ---------------------------------------------------

    def build_monitoring_text(self) -> str:
        return monitoring.build_monitoring_text()

    # -- read-only path properties -----------------------------------------

    @property
    def log_out_path(self) -> Path:
        return self._log_out

    @property
    def default_server_path(self) -> Path:
        return DEFAULT_LLAMA_SERVER

    # -- command helpers (used by UI for advanced-option bookkeeping) -------

    def canonical_adv_key(self, raw_key: str, options: Dict[str, LlamaOption]) -> str:
        return cmd_module.canonical_adv_key(raw_key, options)

    def favorite_string_value(
        self, raw_key: str, key: str, opt: LlamaOption | None, profile: Profile
    ) -> str | None:
        return cmd_module.favorite_string_value(raw_key, key, opt, profile)
