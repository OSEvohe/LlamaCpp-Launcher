"""Llama-server option discovery helpers."""
import re
import subprocess
from pathlib import Path
from typing import Dict, List

from llama_launcher.config import DEFAULT_LLAMA_SERVER
from llama_launcher.models import LlamaOption


def resolve_llama_server_path(raw: str) -> Path:
    p = Path(raw.strip().strip('"'))
    if p.is_dir():
        p = p / "llama-server.exe"
    if not str(raw).strip():
        return DEFAULT_LLAMA_SERVER
    return p


def parse_help_options(help_text: str) -> Dict[str, LlamaOption]:
    lines = help_text.splitlines()
    sections: List[List[str]] = []
    current: List[str] = []
    for line in lines:
        if re.match(r"^\s*-[^-\s]|^\s*--", line):
            if current:
                sections.append(current)
            current = [line]
        elif current and line.strip():
            current.append(line)
        else:
            if current:
                sections.append(current)
                current = []
    if current:
        sections.append(current)

    options: Dict[str, LlamaOption] = {}
    for block in sections:
        first = block[0].strip()
        parts = re.split(r"\s{2,}", first, maxsplit=1)
        names_raw = parts[0].strip()
        desc = parts[1].strip() if len(parts) > 1 else ""
        if len(block) > 1:
            desc += " " + " ".join(x.strip() for x in block[1:])
        desc = re.sub(r"\s+", " ", desc).strip()

        alias_specs = [x.strip() for x in names_raw.split(",") if x.strip()]
        aliases: List[str] = []
        arity = 0
        for spec in alias_specs:
            chunks = spec.split()
            if not chunks:
                continue
            flag = chunks[0]
            aliases.append(flag)
            if len(chunks) > 1:
                arity = max(arity, len(chunks) - 1)

        if not aliases:
            continue

        long_aliases = [a for a in aliases if a.startswith("--")]
        non_no = [a for a in long_aliases if not a.startswith("--no-")]
        key = (non_no[0] if non_no else (long_aliases[0] if long_aliases else aliases[0]))
        positive = non_no[0] if non_no else ""
        negative = ""
        if positive:
            neg_candidate = "--no-" + positive[2:]
            if neg_candidate in aliases:
                negative = neg_candidate

        default_match = re.search(r"\(default:\s*([^\)]+)\)", desc)
        default_value = default_match.group(1).strip() if default_match else ""

        options[key] = LlamaOption(
            key=key,
            aliases=aliases,
            arity=arity,
            default=default_value,
            description=desc,
            positive_flag=positive,
            negative_flag=negative,
        )
    return options


def load_options_from_exe(exe_path: Path) -> Dict[str, LlamaOption]:
    try:
        proc = subprocess.run(
            [str(exe_path), "--help"],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        txt = (proc.stdout or "") + "\n" + (proc.stderr or "")
        return parse_help_options(txt)
    except Exception:
        return {}
