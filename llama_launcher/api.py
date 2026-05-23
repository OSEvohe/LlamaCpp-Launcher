"""Service facade for LLama Launcher core operations."""
import json
import threading
from dataclasses import asdict
from pathlib import Path
from typing import Any, Dict, List

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


def _safe_int(value, default: int = 0) -> int:
    """Return *value* as int only if it is a genuine int (not bool)."""
    if isinstance(value, bool) or not isinstance(value, int):
        return default
    return value


def _safe_str(value, default: str = "") -> str:
    """Return *value* as str only if it is a genuine str."""
    if not isinstance(value, str):
        return default
    return value


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


def _safe_bool(value, default: bool = False) -> bool:
    """Return *value* as bool; coerce ``"true"``/``"false"`` strings gracefully."""
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.strip().lower() == "true"
    return default


def _normalize_mtp(item: dict) -> None:
    """Normalize MTP fields on a raw profile dict before ``Profile(**item)``.

    1. Migrate legacy ``--spec-type draft-mtp`` from advanced settings.
    2. Migrate legacy ``--spec-draft-n-max`` from advanced settings.
    3. Coerce ``enable_mtp`` to bool and ``spec_draft_n_max`` to int.
    4. Replace malformed advanced_favorites/advanced_values with safe defaults.
    """
    adv_favs_raw = item.get("advanced_favorites")
    adv_vals_raw = item.get("advanced_values")

    # Replace malformed types with safe defaults before any migration
    if not isinstance(adv_favs_raw, list):
        item["advanced_favorites"] = []
        adv_favs_raw = item["advanced_favorites"]
    if not isinstance(adv_vals_raw, dict):
        item["advanced_values"] = {}
        adv_vals_raw = item["advanced_values"]

    # -- legacy migration --------------------------------------------------
    has_legacy_spec_type = False
    legacy_draft_n_max = None

    if "--spec-type" in adv_favs_raw or "--spec-type" in adv_vals_raw:
        val = adv_vals_raw.get("--spec-type", "")
        if val and val.strip() == "draft-mtp":
            has_legacy_spec_type = True

    if "--spec-draft-n-max" in adv_favs_raw or "--spec-draft-n-max" in adv_vals_raw:
        raw = adv_vals_raw.get("--spec-draft-n-max", "")
        if raw:
            try:
                legacy_draft_n_max = int(raw.strip())
            except (ValueError, TypeError):
                pass

    if has_legacy_spec_type:
        item["enable_mtp"] = True
        if "--spec-type" in adv_favs_raw:
            adv_favs_raw.remove("--spec-type")
        adv_vals_raw.pop("--spec-type", None)

    if legacy_draft_n_max is not None:
        item["spec_draft_n_max"] = legacy_draft_n_max
        if "--spec-draft-n-max" in adv_favs_raw:
            adv_favs_raw.remove("--spec-draft-n-max")
        adv_vals_raw.pop("--spec-draft-n-max", None)

    # -- type coercion -----------------------------------------------------
    if "enable_mtp" in item:
        item["enable_mtp"] = _safe_bool(item["enable_mtp"])

    if "spec_draft_n_max" in item:
        val = item["spec_draft_n_max"]
        if isinstance(val, bool) or not isinstance(val, int):
            if isinstance(val, str):
                try:
                    item["spec_draft_n_max"] = int(val.strip())
                except (ValueError, TypeError):
                    item["spec_draft_n_max"] = 2
            else:
                item["spec_draft_n_max"] = 2


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
        self._ensure_state()
        if not self._global_file.exists():
            return GlobalSettings()
        try:
            data = json.loads(self._global_file.read_text(encoding="utf-8"))
            return GlobalSettings(
                llama_server_path=data.get("llama_server_path", ""),
                model_dirs=data.get("model_dirs", []),
                api_host=_safe_str(data.get("api_host"), "127.0.0.1"),
                api_port=_safe_int(data.get("api_port"), 0),
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
                _normalize_mtp(item)
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
