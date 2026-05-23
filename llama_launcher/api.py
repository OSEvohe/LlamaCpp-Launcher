"""Service facade for LLama Launcher core operations."""
import threading
from pathlib import Path
from typing import Any, Dict, List

from llama_launcher import command as cmd_module
from llama_launcher import discovery
from llama_launcher import monitoring
from llama_launcher import process
from llama_launcher.config import (
    APP_DIR,
    DEFAULT_LLAMA_SERVER,
    _load_global_from_file,
    _load_profiles_from_file,
    _save_global_to_file,
    _save_profiles_to_file,
    load_global as _config_load_global,
    load_profiles as _config_load_profiles,
    save_global as _config_save_global,
    save_profiles as _config_save_profiles,
)
from llama_launcher.models import GlobalSettings, LlamaOption, Profile
from llama_launcher.options import load_options_from_exe, resolve_llama_server_path


def _coerce_bool(value, field_name: str) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        lowered = value.strip().lower()
        if lowered == "true":
            return True
        if lowered == "false":
            return False
    raise ValueError(f"{field_name} must be a boolean")


def _coerce_int(value, field_name: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"{field_name} must be an integer")
    if isinstance(value, int):
        return value
    if isinstance(value, str) and value.strip():
        try:
            return int(value)
        except ValueError:
            pass
    raise ValueError(f"{field_name} must be an integer")


def _coerce_float(value, field_name: str) -> float:
    if isinstance(value, bool):
        raise ValueError(f"{field_name} must be a number")
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str) and value.strip():
        try:
            return float(value)
        except ValueError:
            pass
    raise ValueError(f"{field_name} must be a number")


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
        self._lock = threading.RLock()

    # -- internal helpers --------------------------------------------------

    def _ensure_state(self) -> None:
        self._state_dir.mkdir(parents=True, exist_ok=True)

    def _load_global_path(self) -> GlobalSettings:
        return _load_global_from_file(self._global_file)

    def _save_global_path(self, settings: GlobalSettings) -> None:
        _save_global_to_file(self._global_file, settings)

    def _load_profiles_path(self) -> List[Profile]:
        return _load_profiles_from_file(self._profiles_file)

    def _save_profiles_path(self, profiles: List[Profile]) -> None:
        _save_profiles_to_file(self._profiles_file, profiles)

    # -- profiles ----------------------------------------------------------

    def load_profiles(self) -> List[Profile]:
        with self._lock:
            if self._app_dir is APP_DIR:
                return _config_load_profiles()
            return self._load_profiles_path()

    def save_profiles(self, profiles: List[Profile]) -> None:
        with self._lock:
            if self._app_dir is APP_DIR:
                _config_save_profiles(profiles)
            else:
                self._save_profiles_path(profiles)

    def add_profile(self, name: str) -> Profile:
        with self._lock:
            profiles = self.load_profiles()
            profile = Profile(name=name)
            profiles.append(profile)
            self.save_profiles(profiles)
            return profile

    def delete_profile(self, index: int) -> bool:
        with self._lock:
            profiles = self.load_profiles()
            if 0 <= index < len(profiles):
                profiles.pop(index)
                if not profiles:
                    profiles.append(Profile())
                self.save_profiles(profiles)
                return True
            return False

    def duplicate_profile(self, index: int) -> Profile:
        """Create a copy of the profile at *index* with a "(copy)" suffix."""
        with self._lock:
            profiles = self.load_profiles()
            if not (0 <= index < len(profiles)):
                raise IndexError(f"profile index {index} out of range")
            src = profiles[index]
            dup = Profile(
                name=f"{src.name} (copy)",
                model_path=src.model_path,
                host=src.host,
                port=src.port,
                ctx_size=src.ctx_size,
                threads=src.threads,
                n_gpu_layers=src.n_gpu_layers,
                temp=src.temp,
                top_p=src.top_p,
                top_k=src.top_k,
                min_p=src.min_p,
                presence_penalty=src.presence_penalty,
                np=src.np,
                batch_size=src.batch_size,
                enable_mtp=src.enable_mtp,
                spec_draft_n_max=src.spec_draft_n_max,
                embeddings=src.embeddings,
                flash_attn_mode=src.flash_attn_mode,
                kv_cache_type=src.kv_cache_type,
                extra_args=src.extra_args,
                advanced_values=dict(src.advanced_values),
                advanced_modes=dict(src.advanced_modes),
                advanced_favorites=list(src.advanced_favorites),
            )
            profiles.append(dup)
            self.save_profiles(profiles)
            return dup

    def update_profile(self, index: int, profile_data: Dict[str, Any]) -> Profile:
        """Atomically read-modify-write a single profile under one lock."""
        with self._lock:
            profiles = self.load_profiles()
            if not (0 <= index < len(profiles)):
                raise IndexError(f"profile index {index} out of range")
            existing = profiles[index]
            top_k = existing.top_k
            if "top_k" in profile_data:
                top_k = _coerce_int(profile_data.get("top_k"), "top_k")
            min_p = existing.min_p
            if "min_p" in profile_data:
                min_p = _coerce_float(profile_data.get("min_p"), "min_p")
            presence_penalty = existing.presence_penalty
            if "presence_penalty" in profile_data:
                presence_penalty = _coerce_float(profile_data.get("presence_penalty"), "presence_penalty")
            np_value = existing.np
            if "np" in profile_data:
                np_value = _coerce_int(profile_data.get("np"), "np")
            enable_mtp = existing.enable_mtp
            if "enable_mtp" in profile_data:
                enable_mtp = _coerce_bool(profile_data.get("enable_mtp"), "enable_mtp")
            spec_draft_n_max = existing.spec_draft_n_max
            if "spec_draft_n_max" in profile_data:
                spec_draft_n_max = _coerce_int(profile_data.get("spec_draft_n_max"), "spec_draft_n_max")
            updated = Profile(
                name=profile_data.get("name", existing.name),
                model_path=profile_data.get("model_path", existing.model_path),
                host=profile_data.get("host", existing.host),
                port=profile_data.get("port", existing.port),
                ctx_size=profile_data.get("ctx_size", existing.ctx_size),
                threads=profile_data.get("threads", existing.threads),
                n_gpu_layers=profile_data.get("n_gpu_layers", existing.n_gpu_layers),
                temp=profile_data.get("temp", existing.temp),
                top_p=profile_data.get("top_p", existing.top_p),
                top_k=top_k,
                min_p=min_p,
                presence_penalty=presence_penalty,
                np=np_value,
                batch_size=profile_data.get("batch_size", existing.batch_size),
                enable_mtp=enable_mtp,
                spec_draft_n_max=spec_draft_n_max,
                embeddings=profile_data.get("embeddings", existing.embeddings),
                flash_attn_mode=profile_data.get("flash_attn_mode", existing.flash_attn_mode),
                kv_cache_type=profile_data.get("kv_cache_type", existing.kv_cache_type),
                extra_args=profile_data.get("extra_args", existing.extra_args),
                advanced_values=profile_data.get("advanced_values", existing.advanced_values),
                advanced_modes=profile_data.get("advanced_modes", existing.advanced_modes),
                advanced_favorites=profile_data.get("advanced_favorites", existing.advanced_favorites),
            )
            profiles[index] = updated
            self.save_profiles(profiles)
            return updated

    # -- global settings ---------------------------------------------------

    def load_global(self) -> GlobalSettings:
        with self._lock:
            if self._app_dir is APP_DIR:
                return _config_load_global()
            return self._load_global_path()

    def save_global(self, settings: GlobalSettings) -> None:
        with self._lock:
            if self._app_dir is APP_DIR:
                _config_save_global(settings)
            else:
                self._save_global_path(settings)

    def update_global(self, settings_data: Dict[str, Any]) -> GlobalSettings:
        """Atomically read-modify-write global settings under one lock."""
        with self._lock:
            current = self.load_global()
            settings = GlobalSettings(
                llama_server_path=settings_data.get("llama_server_path", current.llama_server_path),
                model_dirs=settings_data.get("model_dirs", current.model_dirs),
                api_host=settings_data.get("api_host", current.api_host),
                api_port=settings_data.get("api_port", current.api_port),
            )
            self.save_global(settings)
            return settings

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
        if pid > 0 and process.is_process_running(pid):
            return (True, pid)
        # Fallback: search by process name when PID file is missing/stale
        fallback_pid = process.find_llama_server_pid()
        if fallback_pid > 0:
            return (True, fallback_pid)
        return (False, 0)

    def launch(self, cmd: list, exe_path: str = "") -> int:
        with self._lock:
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
        with self._lock:
            pid = process.read_pid(self._pid_file)
            if pid > 0 and process.is_process_running(pid):
                process.stop_server(pid)
                if self._pid_file.exists():
                    self._pid_file.unlink()
                return pid
            # Clean up stale PID file
            if self._pid_file.exists():
                self._pid_file.unlink()
            # Fallback: search by process name when PID file is missing/stale
            fallback_pid = process.find_llama_server_pid()
            if fallback_pid > 0:
                process.stop_server(fallback_pid)
                return fallback_pid
            return 0

    def restart(self, cmd: list, exe_path: str = "") -> int:
        """Atomically stop any running server and launch a new one.

        Both stop and launch execute under a single lock boundary so that
        concurrent launch/stop/restart calls cannot interleave.
        """
        with self._lock:
            self.stop()
            return self.launch(cmd, exe_path=exe_path)

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
