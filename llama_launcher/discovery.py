"""Model-discovery helpers."""
from pathlib import Path
from typing import List


def scan_gguf_models(model_dirs: List[str]) -> List[str]:
    """Recursively find ``.gguf`` files in *model_dirs*.

    Returns a sorted, deduplicated list of absolute-like path strings.
    """
    models: List[str] = []
    for folder in model_dirs:
        d = Path(folder)
        if d.exists() and d.is_dir():
            try:
                models.extend(str(x) for x in d.rglob("*.gguf"))
            except Exception:
                pass
    return sorted(set(models), key=str.lower)
