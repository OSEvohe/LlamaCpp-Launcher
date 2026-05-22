"""Path constants and persistence helpers for LLama Launcher."""
import json
from dataclasses import asdict
from pathlib import Path
from typing import List

from llama_launcher.models import GlobalSettings, Profile

# Resolve to the repo/workspace root (parent of the llama_launcher package).
APP_DIR = Path(__file__).resolve().parent.parent
STATE_DIR = APP_DIR / ".launcher"
GLOBAL_FILE = STATE_DIR / "global.json"
PROFILES_FILE = STATE_DIR / "profiles.json"
PID_FILE = STATE_DIR / "llama-server.pid"
LOG_OUT = STATE_DIR / "llama-server.log"
LOG_ERR = STATE_DIR / "llama-server.err.log"
DEFAULT_LLAMA_SERVER = Path(r"C:\llama-cpp\llama-server.exe")


def ensure_state() -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)


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


def load_global() -> GlobalSettings:
    ensure_state()
    if not GLOBAL_FILE.exists():
        return GlobalSettings()
    try:
        data = json.loads(GLOBAL_FILE.read_text(encoding="utf-8"))
        return GlobalSettings(
            llama_server_path=data.get("llama_server_path", ""),
            model_dirs=data.get("model_dirs", []),
            api_host=_safe_str(data.get("api_host"), "127.0.0.1"),
            api_port=_safe_int(data.get("api_port"), 0),
        )
    except Exception:
        return GlobalSettings()


def save_global(settings: GlobalSettings) -> None:
    ensure_state()
    GLOBAL_FILE.write_text(json.dumps(asdict(settings), indent=2), encoding="utf-8")


def load_profiles() -> List[Profile]:
    ensure_state()
    if not PROFILES_FILE.exists():
        return [Profile()]
    try:
        raw = json.loads(PROFILES_FILE.read_text(encoding="utf-8"))
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


def save_profiles(profiles: List[Profile]) -> None:
    ensure_state()
    payload = {"profiles": [asdict(p) for p in profiles]}
    PROFILES_FILE.write_text(json.dumps(payload, indent=2), encoding="utf-8")
