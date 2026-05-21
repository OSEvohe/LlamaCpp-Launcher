"""Command-line assembly helpers for llama-server."""
import shlex
from pathlib import Path
from typing import Dict, List

from llama_launcher.models import LlamaOption, Profile


def canonical_adv_key(raw_key: str, options: Dict[str, LlamaOption]) -> str:
    """Resolve *raw_key* to its canonical long-option key.

    If *raw_key* is already a key in *options* it is returned as-is.
    Otherwise the first option whose aliases contain *raw_key* is used.
    """
    if raw_key in options:
        return raw_key
    for key, opt in options.items():
        if raw_key in opt.aliases:
            return key
    return raw_key


def favorite_string_value(
    raw_key: str,
    key: str,
    opt: LlamaOption | None,
    profile: Profile,
) -> str | None:
    """Return the argument string for a favourite advanced option.

    Checks ``profile.advanced_values`` first (by raw key, then canonical key),
    then falls back to legacy ``advanced_modes`` with optional negative flag.

    Returns ``None`` when the option should be omitted entirely
    (legacy ``off`` with no negative alias).
    """
    if raw_key in profile.advanced_values:
        return profile.advanced_values.get(raw_key, "")
    if key in profile.advanced_values:
        return profile.advanced_values.get(key, "")
    legacy_modes = getattr(profile, "advanced_modes", {}) or {}
    mode = legacy_modes.get(raw_key, legacy_modes.get(key, "default"))
    if mode == "on":
        return ""
    if mode == "off" and opt and opt.negative_flag:
        return opt.negative_flag
    if mode == "off":
        return None
    return ""


def build_command(
    exe: Path,
    profile: Profile,
    options: Dict[str, LlamaOption],
) -> List[str]:
    """Assemble the full command-line list for llama-server.

    This is the pure business-logic part: it does NOT interact with the UI.
    Validation and UI bookkeeping remain in the caller.
    """
    cmd: List[str] = [
        str(exe),
        "--model", profile.model_path,
        "--host", profile.host,
        "--port", str(profile.port),
        "--ctx-size", str(profile.ctx_size),
        "--threads", str(profile.threads),
        "--n-gpu-layers", str(profile.n_gpu_layers),
        "--temp", str(profile.temp),
        "--top-p", str(profile.top_p),
        "--batch-size", str(profile.batch_size),
        "--flash-attn", profile.flash_attn_mode,
        "--cache-type-k", profile.kv_cache_type,
        "--cache-type-v", profile.kv_cache_type,
    ]
    if profile.embeddings:
        cmd.append("--embeddings")

    for raw_key in profile.advanced_favorites:
        ckey = canonical_adv_key(raw_key, options)
        opt = options.get(ckey)
        val = favorite_string_value(raw_key, ckey, opt, profile)
        if val is None:
            continue
        val = val.strip()
        if val.startswith("--"):
            cmd.extend(shlex.split(val, posix=False))
        else:
            cmd.append(ckey)
            if val:
                cmd.extend(shlex.split(val, posix=False))

    if profile.extra_args:
        cmd.extend(shlex.split(profile.extra_args, posix=False))
    return cmd
