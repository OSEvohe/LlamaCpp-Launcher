"""Regression tests for API server startup flow and entrypoint stability.

Verifies that the single-mode API server startup path works correctly.
Uses ``unittest.mock`` only — no real network bind required.
"""
import sys
from unittest.mock import patch

from llama_launcher.api import LlamaLauncherService
from llama_launcher.models import GlobalSettings

# ---------------------------------------------------------------------------
# 1. Root wrappers delegate to llama_launcher.main.main
# ---------------------------------------------------------------------------


def test_main_py_delegates_to_canonical_main() -> None:
    """root main.py must import main from llama_launcher.main."""
    import main as root_main

    assert hasattr(root_main, "main")
    # The imported symbol must be the canonical function
    from llama_launcher.main import main as canonical_main
    assert root_main.main is canonical_main, \
        "main.py.main must be the same object as llama_launcher.main.main"


def test_launcher_py_delegates_to_canonical_main() -> None:
    """root launcher.py must expose main from llama_launcher.main."""
    import launcher as root_launcher

    assert hasattr(root_launcher, "main")
    from llama_launcher.main import main as canonical_main
    assert root_launcher.main is canonical_main, \
        "launcher.py.main must be the same object as llama_launcher.main.main"


def test_launcher_py_all_exports_main() -> None:
    """launcher.py __all__ must include 'main'."""
    import launcher as root_launcher
    assert "main" in root_launcher.__all__


# ---------------------------------------------------------------------------
# 2. CLI parsing regression for common invocations
# ---------------------------------------------------------------------------


def test_cli_empty_args_defaults() -> None:
    """Empty argv → resolves to 7890 default port."""
    captured = {}

    def capture_resolve(cli_host, cli_port):
        captured["cli_host"] = cli_host
        captured["cli_port"] = cli_port
        return ("127.0.0.1", 7890)

    def fake_run_api_server(service, host, port):
        captured["host"] = host
        captured["port"] = port

    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()), \
         patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
         patch("llama_launcher.server.run_api_server", fake_run_api_server), \
         patch("sys.argv", ["launcher"]):
        from llama_launcher.main import main
        main()

    assert captured["cli_host"] is None
    assert captured["cli_port"] is None
    assert captured["host"] == "127.0.0.1"
    assert captured["port"] == 7890


def test_cli_api_port_parsed() -> None:
    """--api-port 3333 → cli_port=3333 passed to _resolve_api_settings."""
    captured = {}

    def capture_resolve(cli_host, cli_port):
        captured["cli_host"] = cli_host
        captured["cli_port"] = cli_port
        return ("127.0.0.1", cli_port if cli_port else 7890)

    def fake_run_api_server(service, host, port):
        captured["host"] = host
        captured["port"] = port

    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()), \
         patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
         patch("llama_launcher.server.run_api_server", fake_run_api_server), \
         patch("sys.argv", ["launcher", "--api-port", "3333"]):
        from llama_launcher.main import main
        main()

    assert captured["cli_port"] == 3333


def test_cli_api_host_parsed() -> None:
    """--api-host 0.0.0.0 → cli_host='0.0.0.0' passed to _resolve_api_settings."""
    captured = {}

    def capture_resolve(cli_host, cli_port):
        captured["cli_host"] = cli_host
        return ("0.0.0.0", 7890)

    def fake_run_api_server(service, host, port):
        captured["host"] = host
        captured["port"] = port

    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()), \
         patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
         patch("llama_launcher.server.run_api_server", fake_run_api_server), \
         patch("sys.argv", ["launcher", "--api-host", "0.0.0.0"]):
        from llama_launcher.main import main
        main()

    assert captured["cli_host"] == "0.0.0.0"


def test_api_server_starts_blocking() -> None:
    """main() calls run_api_server with resolved host/port."""
    server_called = False

    def fake_run_api_server(service, host, port):
        nonlocal server_called
        server_called = True

    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()), \
         patch("llama_launcher.main._resolve_api_settings", return_value=("127.0.0.1", 7890)), \
         patch("llama_launcher.server.run_api_server", fake_run_api_server), \
         patch("sys.argv", ["launcher"]):
        from llama_launcher.main import main
        main()

    assert server_called, "run_api_server should have been called"


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    tests = [
        # 1. Entrypoint delegation
        test_main_py_delegates_to_canonical_main,
        test_launcher_py_delegates_to_canonical_main,
        test_launcher_py_all_exports_main,
        # 2. CLI parsing
        test_cli_empty_args_defaults,
        test_cli_api_port_parsed,
        test_cli_api_host_parsed,
        test_api_server_starts_blocking,
    ]

    passed = 0
    failed = 0
    for test_fn in tests:
        try:
            test_fn()
            print(f"PASS: {test_fn.__name__}")
            passed += 1
        except Exception as e:
            print(f"FAIL: {test_fn.__name__}: {e}")
            failed += 1

    print(f"\n{passed} passed, {failed} failed out of {len(tests)} tests.")
    if failed:
        raise SystemExit(1)
