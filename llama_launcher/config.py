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


def load_global() -> GlobalSettings:
    ensure_state()
    if not GLOBAL_FILE.exists():
        return GlobalSettings()
    try:
        data = json.loads(GLOBAL_FILE.read_text(encoding="utf-8"))
        return GlobalSettings(
            llama_server_path=data.get("llama_server_path", ""),
            model_dirs=data.get("model_dirs", []),
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
            profiles.append(Profile(**item))
        return profiles or [Profile()]
    except Exception:
        return [Profile()]


def save_profiles(profiles: List[Profile]) -> None:
    ensure_state()
    payload = {"profiles": [asdict(p) for p in profiles]}
    PROFILES_FILE.write_text(json.dumps(payload, indent=2), encoding="utf-8")
