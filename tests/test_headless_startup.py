"""API server startup and bind-robustness tests for ``llama_launcher.main``.

Uses ``unittest.mock`` only — no real network bind required
"""
import io
import sys
from unittest.mock import patch

from llama_launcher.api import LlamaLauncherService
from llama_launcher.main import _resolve_api_settings, main
from llama_launcher.models import GlobalSettings

# ---------------------------------------------------------------------------
# 1. _resolve_api_settings sanitisation boundaries
# ---------------------------------------------------------------------------


def test_default_port_is_7890() -> None:
    """No stored port → defaults to 7890."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, None)
    assert host == "127.0.0.1"
    assert port == 7890


def test_resolve_api_settings_defaults() -> None:
    """No CLI overrides, no stored settings → 127.0.0.1:7890."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, None)
    assert host == "127.0.0.1"
    assert port == 7890


def test_resolve_api_settings_cli_port_overrides_stored() -> None:
    """CLI port takes precedence over stored settings."""
    stored = GlobalSettings(api_host="0.0.0.0", api_port=9999)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, 8080)
    assert host == "0.0.0.0"
    assert port == 8080


def test_resolve_api_settings_cli_host_overrides_stored() -> None:
    """CLI host takes precedence over stored settings."""
    stored = GlobalSettings(api_host="0.0.0.0", api_port=9999)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings("10.0.0.1", None)
    assert host == "10.0.0.1"
    assert port == 9999


def test_resolve_api_settings_port_zero_defaults_to_7890() -> None:
    """Port 0 defaults to 7890."""
    stored = GlobalSettings(api_port=0)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None)
    assert port == 7890


def test_resolve_api_settings_port_one_valid() -> None:
    """Port 1 is within valid range and kept as-is."""
    stored = GlobalSettings(api_port=1)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None)
    assert port == 1


def test_resolve_api_settings_port_65535_valid() -> None:
    """Port 65535 is the upper boundary and kept as-is."""
    stored = GlobalSettings(api_port=65535)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None)
    assert port == 65535


def test_resolve_api_settings_port_65536_clamped() -> None:
    """Port 65536 exceeds max → clamped to 0 then defaulted to 7890."""
    stored = GlobalSettings(api_port=65536)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None)
    assert port == 7890


def test_resolve_api_settings_negative_port_clamped() -> None:
    """Negative port → clamped to 0 then defaulted to 7890."""
    stored = GlobalSettings(api_port=-1)
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None)
    assert port == 7890


def test_resolve_api_settings_cli_port_negative_clamped() -> None:
    """CLI negative port → clamped to 0 then defaulted to 7890."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, -5)
    assert port == 7890


def test_cli_port_zero_keeps_ephemeral() -> None:
    """Explicit --api-port 0 preserves ephemeral port (port 0), not defaulted to 7890."""
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=GlobalSettings()):
        host, port = _resolve_api_settings(None, 0)
    assert port == 0


def test_resolve_api_settings_invalid_stored_port_clamped() -> None:
    """Stored port that is not an int (e.g. None) → clamped to 0 then defaulted to 7890."""
    stored = GlobalSettings()
    stored.api_port = None  # type: ignore[assignment]
    with patch.object(LlamaLauncherService, "__init__", lambda self, app_dir=None: None), \
         patch.object(LlamaLauncherService, "load_global", return_value=stored):
        host, port = _resolve_api_settings(None, None)
    assert port == 7890


# ---------------------------------------------------------------------------
# 2. Bind failure → concise error + sys.exit(1)
# ---------------------------------------------------------------------------


def test_bind_failure_prints_error_and_exits() -> None:
    """When run_api_server raises OSError, main prints a concise error and exits 1."""
    def fake_run_api_server(service, host, port):
        raise OSError("Address already in use")

    def _fake_exit(code=None):
        raise SystemExit(code)

    with patch.object(sys, "exit", side_effect=_fake_exit), \
         patch("llama_launcher.main._resolve_api_settings", return_value=("127.0.0.1", 7890)), \
         patch("llama_launcher.server.run_api_server", fake_run_api_server), \
         patch("sys.argv", ["launcher"]):
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
# Main
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    tests = [
        test_default_port_is_7890,
        test_resolve_api_settings_defaults,
        test_resolve_api_settings_cli_port_overrides_stored,
        test_resolve_api_settings_cli_host_overrides_stored,
        test_resolve_api_settings_port_zero_defaults_to_7890,
        test_resolve_api_settings_port_one_valid,
        test_resolve_api_settings_port_65535_valid,
        test_resolve_api_settings_port_65536_clamped,
        test_resolve_api_settings_negative_port_clamped,
        test_resolve_api_settings_cli_port_negative_clamped,
        test_cli_port_zero_keeps_ephemeral,
        test_resolve_api_settings_invalid_stored_port_clamped,
        test_bind_failure_prints_error_and_exits,
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
