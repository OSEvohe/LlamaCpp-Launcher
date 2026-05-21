"""Backward-compat wrapper: runs the launcher TUI.

Delegates to the canonical startup path in llama_launcher.main.
"""

from llama_launcher.main import main  # noqa: F401

__all__ = ["main"]


if __name__ == "__main__":
    main()
