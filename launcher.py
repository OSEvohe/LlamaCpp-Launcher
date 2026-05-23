"""Backward-compat wrapper: runs the launcher API server and web dashboard.

Delegates to the canonical startup path in llama_launcher.main.
"""

from llama_launcher.main import main  # noqa: F401

__all__ = ["main"]


if __name__ == "__main__":
    main()
