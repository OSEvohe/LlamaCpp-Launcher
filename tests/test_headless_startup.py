"""Headless startup/shutdown and bind-robustness tests for ``llama_launcher.main``.

Uses ``unittest.mock`` only — no real TUI launch, no network bind required,
no ``llama_launcher.ui.app`` (textual) import.
"""
import io
import sys
import types
from unittest.mock import MagicMock, patch

from llama_launcher.api import LlamaLauncherService
from llama_launcher.main import _resolve_api_settings, main
from llama_launcher.models import GlobalSettings

# ---------------------------------------------------------------------------
# 1. _resolve_api_settings sanitisation boundaries
# ---------------------------------------------------------------------------


def test_resolve_api_settings_defaults() -> None:
    """No CLI overrides, no stored settings → 127.0.0.1:0 (non-headless)."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, None, headless=False)
    assert host == "127.0.0.1"
    assert port == 0


def test_resolve_api_settings_cli_port_overrides_stored() -> None:
    """CLI port takes precedence over stored settings."""
    stored = GlobalSettings(api_host="0.0.0.0", api_port=9999)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, 8080, headless=False)
    assert host == "0.0.0.0"
    assert port == 8080


def test_resolve_api_settings_cli_host_overrides_stored() -> None:
    """CLI host takes precedence over stored settings."""
    stored = GlobalSettings(api_host="0.0.0.0", api_port=9999)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings("10.0.0.1", None, headless=False)
    assert host == "10.0.0.1"
    assert port == 9999


def test_resolve_api_settings_port_zero_clamped_non_headless() -> None:
    """Port 0 stays 0 in non-headless mode (API will be skipped)."""
    stored = GlobalSettings(api_port=0)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=False)
    assert port == 0


def test_resolve_api_settings_port_one_valid() -> None:
    """Port 1 is within valid range and kept as-is."""
    stored = GlobalSettings(api_port=1)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=False)
    assert port == 1


def test_resolve_api_settings_port_65535_valid() -> None:
    """Port 65535 is the upper boundary and kept as-is."""
    stored = GlobalSettings(api_port=65535)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=False)
    assert port == 65535


def test_resolve_api_settings_port_65536_clamped() -> None:
    """Port 65536 exceeds max → clamped to 0."""
    stored = GlobalSettings(api_port=65536)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=False)
    assert port == 0


def test_resolve_api_settings_negative_port_clamped() -> None:
    """Negative port → clamped to 0."""
    stored = GlobalSettings(api_port=-1)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=False)
    assert port == 0


def test_resolve_api_settings_cli_port_negative_clamped() -> None:
    """CLI negative port → clamped to 0 (non-headless)."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, -5, headless=False)
    assert port == 0


def test_resolve_api_settings_invalid_stored_port_clamped() -> None:
    """Stored port that is not an int (e.g. None) → clamped to 0."""
    stored = GlobalSettings()
    stored.api_port = None  # type: ignore[assignment]
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=False)
    assert port == 0


# ---------------------------------------------------------------------------
# 2. Headless default port fallback (7890)
# ---------------------------------------------------------------------------


def test_headless_fallback_7890_no_port() -> None:
    """Headless with no stored port → defaults to 7890."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, None, headless=True)
    assert port == 7890


def test_headless_fallback_7890_zero_port() -> None:
    """Headless with stored port 0 → defaults to 7890."""
    stored = GlobalSettings(api_port=0)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=True)
    assert port == 7890


def test_headless_fallback_7890_negative_port() -> None:
    """Headless with negative stored port → defaults to 7890."""
    stored = GlobalSettings(api_port=-100)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=True)
    assert port == 7890


def test_headless_fallback_7890_invalid_stored_port() -> None:
    """Headless with invalid (non-int) stored port → defaults to 7890."""
    stored = GlobalSettings()
    stored.api_port = None  # type: ignore[assignment]
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=True)
    assert port == 7890


def test_headless_cli_port_positive_no_fallback() -> None:
    """Headless with valid CLI port → no fallback, use the CLI value."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, 3000, headless=True)
    assert port == 3000


def test_headless_stored_port_positive_no_fallback() -> None:
    """Headless with valid stored port → no fallback, use the stored value."""
    stored = GlobalSettings(api_port=5000)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None, headless=True)
    assert port == 5000


# ---------------------------------------------------------------------------
# 3. Headless bind failure → concise error + sys.exit(1)
# ---------------------------------------------------------------------------


def test_headless_bind_failure_prints_error_and_exits() -> None:
    """When _run_api_headless raises OSError, main prints a concise error and exits 1."""
    def fake_run_api_headless(host, port):
        raise OSError("Address already in use")

    def _fake_exit(code=None):
        raise SystemExit(code)

    with patch.object(sys, "exit", side_effect=_fake_exit), \
         patch("llama_launcher.main._resolve_api_settings", return_value=("127.0.0.1", 7890)), \
         patch("llama_launcher.main._run_api_headless", fake_run_api_headless), \
         patch("sys.argv", ["launcher", "--headless"]):
        out = io.StringIO()
        with patch("sys.stdout", out):
            try:
                main()
            except SystemExit as exc:
                assert exc.code == 1, f"expected exit code 1, got {exc.code}"

    stdout_text = out.getvalue()
    assert "Error:" in stdout_text
    assert "failed to bind" in stdout_text
    assert "127.0.0.1:7890" in stdout_text
    # The error message should NOT contain a traceback
    assert "Traceback" not in stdout_text


# ---------------------------------------------------------------------------
# 4. Sidecar bind failure in non-headless → does NOT block TUI path
# ---------------------------------------------------------------------------


def test_sidecar_bind_failure_non_headless_continues() -> None:
    """When _start_api_sidecar raises OSError, main continues to TUI import."""
    tui_run_called = False

    def fake_start_api_sidecar(host, port):
        raise OSError("Address already in use")

    def fake_resolve_api_settings(cli_host, cli_port, headless):
        return ("127.0.0.1", 8080)

    def fake_app_run():
        nonlocal tui_run_called
        tui_run_called = True

    # Inject a fake UI module to avoid importing the real one (textual dep).
    fake_ui = types.ModuleType("llama_launcher.ui.app")
    fake_ui.LlamaLauncherApp = MagicMock()
    fake_ui.LlamaLauncherApp.return_value.run = fake_app_run

    with patch("llama_launcher.main._resolve_api_settings", fake_resolve_api_settings), \
         patch("llama_launcher.main._start_api_sidecar", fake_start_api_sidecar), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher"]):
        main()

    assert tui_run_called, "TUI .run() should have been called despite sidecar failure"


def test_sidecar_zero_port_non_headless_skips_sidecar() -> None:
    """When resolved port is 0 in non-headless, sidecar is skipped entirely."""
    sidecar_called = False

    def fake_start_api_sidecar(host, port):
        nonlocal sidecar_called
        sidecar_called = True

    def fake_resolve_api_settings(cli_host, cli_port, headless):
        return ("127.0.0.1", 0)

    # Inject a fake UI module to avoid importing the real one (textual dep).
    fake_ui = types.ModuleType("llama_launcher.ui.app")
    fake_ui.LlamaLauncherApp = MagicMock()

    with patch("llama_launcher.main._resolve_api_settings", fake_resolve_api_settings), \
         patch("llama_launcher.main._start_api_sidecar", fake_start_api_sidecar), \
         patch.dict("sys.modules", {"llama_launcher.ui.app": fake_ui}), \
         patch("sys.argv", ["launcher"]):
        main()

    assert not sidecar_called, "sidecar should NOT be called when port is 0"


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    tests = [
        test_resolve_api_settings_defaults,
        test_resolve_api_settings_cli_port_overrides_stored,
        test_resolve_api_settings_cli_host_overrides_stored,
        test_resolve_api_settings_port_zero_clamped_non_headless,
        test_resolve_api_settings_port_one_valid,
        test_resolve_api_settings_port_65535_valid,
        test_resolve_api_settings_port_65536_clamped,
        test_resolve_api_settings_negative_port_clamped,
        test_resolve_api_settings_cli_port_negative_clamped,
        test_resolve_api_settings_invalid_stored_port_clamped,
        test_headless_fallback_7890_no_port,
        test_headless_fallback_7890_zero_port,
        test_headless_fallback_7890_negative_port,
        test_headless_fallback_7890_invalid_stored_port,
        test_headless_cli_port_positive_no_fallback,
        test_headless_stored_port_positive_no_fallback,
        test_headless_bind_failure_prints_error_and_exits,
        test_sidecar_bind_failure_non_headless_continues,
        test_sidecar_zero_port_non_headless_skips_sidecar,
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
