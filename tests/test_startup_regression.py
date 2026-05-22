"""Regression tests for TUI / API sidecar startup flow and entrypoint stability.

Verifies that legacy startup behaviour remains intact while API features
are enabled.  Uses ``unittest.mock`` only — no real TUI launch, no network
bind required.
"""
import sys
import types
from unittest.mock import MagicMock, patch

from llama_launcher.api import LlamaLauncherService
from llama_launcher.models import GlobalSettings

# ---------------------------------------------------------------------------
# Helpers shared across tests
# ---------------------------------------------------------------------------


def _patch_service_defaults():
    """Context manager: mock LlamaLauncherService with empty defaults."""
    return (
        patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None),
        patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()),
    )


def _make_fake_ui(run_side_effect=None):
    """Return a fake ``llama_launcher.ui.app`` module with a controllable run()."""
    fake = types.ModuleType("llama_launcher.ui.app")
    fake.LlamaLauncherApp = MagicMock()
    if run_side_effect is not None:
        fake.LlamaLauncherApp.return_value.run.side_effect = run_side_effect
    return fake


# ---------------------------------------------------------------------------
# 1. Default non-headless path still launches TUI flow
# ---------------------------------------------------------------------------


def test_default_non_headless_launches_tui() -> None:
    """Running with no flags (port 0) still reaches LlamaLauncherApp.run()."""
    tui_run_called = False

    def fake_app_run():
        nonlocal tui_run_called
        tui_run_called = True

    fake_ui = _make_fake_ui(run_side_effect=fake_app_run)

    with patch("llama_launcher.main._resolve_api_settings", return_value=("127.0.0.1", 0)), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher"]):
        from llama_launcher.main import main
        main()

    assert tui_run_called, "TUI .run() should have been called in default non-headless mode"


def test_non_headless_with_api_port_still_launches_tui() -> None:
    """Non-headless with a positive port starts sidecar AND still launches TUI."""
    tui_run_called = False
    sidecar_called = False

    def fake_start_api_sidecar(host, port):
        nonlocal sidecar_called
        sidecar_called = True

    def fake_app_run():
        nonlocal tui_run_called
        tui_run_called = True

    fake_ui = _make_fake_ui(run_side_effect=fake_app_run)

    with patch("llama_launcher.main._resolve_api_settings", return_value=("127.0.0.1", 8080)), \
         patch("llama_launcher.main._start_api_sidecar", fake_start_api_sidecar), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher"]):
        from llama_launcher.main import main
        main()

    assert sidecar_called, "sidecar should start when port > 0 in non-headless mode"
    assert tui_run_called, "TUI .run() should still be called after sidecar starts"


# ---------------------------------------------------------------------------
# 2. Sidecar starts only when configured; skipped otherwise
# ---------------------------------------------------------------------------


def test_sidecar_starts_when_port_positive_non_headless() -> None:
    """Sidecar _is_ invoked when resolved port > 0 in non-headless mode."""
    sidecar_host = None
    sidecar_port = None

    def fake_start_api_sidecar(host, port):
        nonlocal sidecar_host, sidecar_port
        sidecar_host = host
        sidecar_port = port

    fake_ui = _make_fake_ui()

    with patch("llama_launcher.main._resolve_api_settings", return_value=("0.0.0.0", 9090)), \
         patch("llama_launcher.main._start_api_sidecar", fake_start_api_sidecar), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher"]):
        from llama_launcher.main import main
        main()

    assert sidecar_host == "0.0.0.0"
    assert sidecar_port == 9090


def test_sidecar_skipped_when_port_zero_non_headless() -> None:
    """Sidecar is NOT invoked when resolved port is 0 in non-headless mode."""
    sidecar_called = False

    def fake_start_api_sidecar(host, port):
        nonlocal sidecar_called
        sidecar_called = True

    fake_ui = _make_fake_ui()

    with patch("llama_launcher.main._resolve_api_settings", return_value=("127.0.0.1", 0)), \
         patch("llama_launcher.main._start_api_sidecar", fake_start_api_sidecar), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher"]):
        from llama_launcher.main import main
        main()

    assert not sidecar_called, "sidecar should NOT be called when port is 0"


def test_headless_does_not_launch_tui() -> None:
    """--headless mode must NOT import or call LlamaLauncherApp."""
    ui_imported = False

    original_import = __builtins__["__import__"] if isinstance(__builtins__, dict) else __builtins__.__import__

    def tracking_import(name, *args, **kwargs):
        nonlocal ui_imported
        if name == "llama_launcher.ui.app" or name.startswith("llama_launcher.ui.app"):
            ui_imported = True
        return original_import(name, *args, **kwargs)

    def fake_run_api_headless(host, port):
        pass  # do nothing, just return

    with patch("llama_launcher.main._resolve_api_settings", return_value=("127.0.0.1", 7890)), \
         patch("llama_launcher.main._run_api_headless", fake_run_api_headless), \
         patch("sys.argv", ["launcher", "--headless"]):
        from llama_launcher.main import main
        main()

    assert not ui_imported, "TUI module must NOT be imported in headless mode"


# ---------------------------------------------------------------------------
# 3. Root wrappers delegate to llama_launcher.main.main
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
# 4. CLI parsing regression for common invocations
# ---------------------------------------------------------------------------


def test_cli_empty_args_defaults() -> None:
    """Empty argv → headless=False, port resolved to 0 (non-headless)."""
    with _patch_service_defaults()[0], \
         _patch_service_defaults()[1], \
         patch("sys.argv", ["launcher"]):
        from llama_launcher.main import _resolve_api_settings
        # We need to call main which parses args, but we can inspect the
        # argparse result indirectly via _resolve_api_settings being called.
        # Instead, mock _resolve_api_settings to capture its call.
        captured = {}

        def capture_resolve(cli_host, cli_port, headless):
            captured["cli_host"] = cli_host
            captured["cli_port"] = cli_port
            captured["headless"] = headless
            return ("127.0.0.1", 0)

        fake_ui = _make_fake_ui()

        with patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
             patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
             patch("sys.argv", ["launcher"]):
            from llama_launcher.main import main
            main()

    assert captured["headless"] is False
    assert captured["cli_host"] is None
    assert captured["cli_port"] is None


def test_cli_headless_flag_parsed() -> None:
    """--headless → headless=True passed to _resolve_api_settings."""
    captured = {}

    def capture_resolve(cli_host, cli_port, headless):
        captured["headless"] = headless
        return ("127.0.0.1", 7890)

    def fake_run_api_headless(host, port):
        pass

    with patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
         patch("llama_launcher.main._run_api_headless", fake_run_api_headless), \
         patch("sys.argv", ["launcher", "--headless"]):
        from llama_launcher.main import main
        main()

    assert captured["headless"] is True


def test_cli_api_port_parsed() -> None:
    """--api-port 3333 → cli_port=3333 passed to _resolve_api_settings."""
    captured = {}

    def capture_resolve(cli_host, cli_port, headless):
        captured["cli_host"] = cli_host
        captured["cli_port"] = cli_port
        captured["headless"] = headless
        return ("127.0.0.1", cli_port if cli_port else 0)

    fake_ui = _make_fake_ui()

    with patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher", "--api-port", "3333"]):
        from llama_launcher.main import main
        main()

    assert captured["cli_port"] == 3333
    assert captured["headless"] is False


def test_cli_api_host_parsed() -> None:
    """--api-host 0.0.0.0 → cli_host='0.0.0.0' passed to _resolve_api_settings."""
    captured = {}

    def capture_resolve(cli_host, cli_port, headless):
        captured["cli_host"] = cli_host
        return ("0.0.0.0", 0)

    fake_ui = _make_fake_ui()

    with patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher", "--api-host", "0.0.0.0"]):
        from llama_launcher.main import main
        main()

    assert captured["cli_host"] == "0.0.0.0"


def test_cli_headless_with_api_port_combined() -> None:
    """--headless --api-port 5555 → both flags parsed correctly."""
    captured = {}

    def capture_resolve(cli_host, cli_port, headless):
        captured["cli_host"] = cli_host
        captured["cli_port"] = cli_port
        captured["headless"] = headless
        return ("127.0.0.1", cli_port if cli_port else 7890)

    def fake_run_api_headless(host, port):
        pass

    with patch("llama_launcher.main._resolve_api_settings", capture_resolve), \
         patch("llama_launcher.main._run_api_headless", fake_run_api_headless), \
         patch("sys.argv", ["launcher", "--headless", "--api-port", "5555"]):
        from llama_launcher.main import main
        main()

    assert captured["headless"] is True
    assert captured["cli_port"] == 5555


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    tests = [
        # 1. TUI flow
        test_default_non_headless_launches_tui,
        test_non_headless_with_api_port_still_launches_tui,
        # 2. Sidecar gating
        test_sidecar_starts_when_port_positive_non_headless,
        test_sidecar_skipped_when_port_zero_non_headless,
        test_headless_does_not_launch_tui,
        # 3. Entrypoint delegation
        test_main_py_delegates_to_canonical_main,
        test_launcher_py_delegates_to_canonical_main,
        test_launcher_py_all_exports_main,
        # 4. CLI parsing
        test_cli_empty_args_defaults,
        test_cli_headless_flag_parsed,
        test_cli_api_port_parsed,
        test_cli_api_host_parsed,
        test_cli_headless_with_api_port_combined,
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
