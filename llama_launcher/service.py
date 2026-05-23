"""Windows auto-start helpers for LLama Launcher.

Two mechanisms are provided:

1. **Scheduled task** (``install_task`` / ``uninstall_task``) — works with
   user-level Python installations.  Runs ``pythonw.exe main.py`` at logon
   with no console window.  No Administrator rights required.

2. **Native Windows service** (``install_service`` / ``uninstall_service``) —
   uses ``pywin32`` to register a proper SCM service.  Requires a system-wide
   Python installation (e.g. ``C:\\Python312\\``) because the service runs as
   ``LocalSystem`` which cannot access ``%LOCALAPPDATA%``.

The scheduled task is the recommended default for most users.
"""
import logging
import logging.handlers
import os
import subprocess
import sys
import threading
from pathlib import Path

# Ensure the package root is on sys.path when the SCM launches this script
# from C:\Windows\System32 (the default service working directory).
_script_root = Path(__file__).resolve().parent.parent
if str(_script_root) not in sys.path:
    sys.path.insert(0, str(_script_root))

try:
    import win32event
    import win32service
    import win32serviceutil
except ImportError:
    _has_pywin32 = False
    win32event = None  # type: ignore[misc,assignment]
    win32service = None  # type: ignore[misc,assignment]
    win32serviceutil = None  # type: ignore[misc,assignment]
else:
    _has_pywin32 = True

from llama_launcher.config import APP_DIR, STATE_DIR, ensure_state

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

_TASK_NAME = "LLama Launcher"

# ---------------------------------------------------------------------------
# Service class (only available when pywin32 is installed)
# ---------------------------------------------------------------------------


if _has_pywin32:

    class _LlamaLauncherService(win32serviceutil.ServiceFramework):
        """Windows service that runs the LLama Launcher API server."""

        _svc_name_ = "LlamaLauncher"
        _svc_display_name_ = "LLama Launcher"
        _svc_description_ = (
            "LLama Launcher — API server and web dashboard for llama.cpp"
        )

        def __init__(self, args):
            win32serviceutil.ServiceFramework.__init__(self, args)
            self._stop_event = win32event.CreateEvent(None, 0, 0, None)
            self._server = None

        # -- lifecycle -----------------------------------------------------------

        def SvcDoRun(self) -> None:
            """Entry point called by the SCM after SvcStop() returns."""
            ensure_state()
            _setup_service_logging(self)

            # Resolve host/port from persisted settings (same logic as CLI).
            host, port = _resolve_service_api_settings()

            self.logger.info(
                "LLama Launcher service starting on %s:%s  (PID %d)",
                host, port, os.getpid(),
            )

            from llama_launcher.api import LlamaLauncherService
            from llama_launcher.server import create_api_server

            svc = LlamaLauncherService()
            self._server = create_api_server(svc, host, port)

            # Run server in a daemon thread so SvcDoRun can wait on the stop event.
            server_thread = threading.Thread(
                target=self._server.serve_forever, daemon=True,
            )
            server_thread.start()

            self.logger.info("API server listening on %s:%s", host, port)

            # Block until the SCM signals a stop.
            rc = win32event.WaitForSingleObject(self._stop_event, win32event.INFINITE)
            if rc != win32event.WAIT_OBJECT_0 or self._server is None:
                return

            self.logger.info("Stopping API server …")
            self._server.shutdown()
            self._server.server_close()
            self._server = None
            self.logger.info("LLama Launcher service stopped.")

        def SvcStop(self) -> None:
            """Called by the SCM when a stop is requested."""
            self.ReportServiceStatus(win32service.SERVICE_STOP_PENDING)
            win32event.SetEvent(self._stop_event)
            self.ReportServiceStatus(win32service.SERVICE_STOPPED)

        # -- CLI helper (run by win32serviceutil) --------------------------------

        def SvcShutdown(self) -> None:
            """Called on system shutdown."""
            self.SvcStop()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


if _has_pywin32:

    def _setup_service_logging(service: _LlamaLauncherService) -> None:  # noqa: F821
        """Configure Python logging to write to ``.launcher/service.log``."""
        log_path = STATE_DIR / "service.log"
        root_logger = logging.getLogger()
        root_logger.setLevel(logging.INFO)

        # Remove any existing handlers (win32serviceutil adds an EventLog handler).
        root_logger.handlers.clear()

        handler = logging.handlers.RotatingFileHandler(
            log_path, maxBytes=2 * 1024 * 1024, backupCount=3, encoding="utf-8",
        )
        handler.setFormatter(logging.Formatter("%(asctime)s  %(message)s"))
        root_logger.addHandler(handler)

        service.logger = logging.getLogger(__name__)


def _resolve_service_api_settings() -> tuple[str, int]:
    """Resolve API host/port from persisted global settings."""
    from llama_launcher.api import LlamaLauncherService

    svc = LlamaLauncherService()
    settings = svc.load_global()

    host = settings.api_host or "127.0.0.1"
    raw_port = settings.api_port
    try:
        port = int(raw_port)
    except (TypeError, ValueError):
        port = 0

    if port < 0 or port > 65535:
        port = 0

    if port <= 0:
        port = 7890

    return host, port


# ---------------------------------------------------------------------------
# Scheduled task (recommended — works with user-level Python)
# ---------------------------------------------------------------------------


def install_task(python_exe: str | None = None) -> None:
    """Install a logon scheduled task that starts the API server.

    Uses ``pythonw.exe`` (no console window) and runs with least privileges.
    No Administrator rights required.

    Parameters
    ----------
    python_exe:
        Path to the Python interpreter.  Defaults to auto-detected
        ``pythonw.exe`` (console-less variant).
    """
    if python_exe is None:
        python_exe = _find_pythonw()

    main_script = str(APP_DIR / "main.py")
    trigger = f'"{python_exe}" "{main_script}"'

    subprocess.run(
        [
            "schtasks",
            "/create",
            "/tn", _TASK_NAME,
            "/tr", trigger,
            "/sc", "onlogon",
            "/f",               # overwrite if exists
        ],
        check=True,
    )


def uninstall_task() -> None:
    """Remove the LLama Launcher scheduled task."""
    subprocess.run(
        ["schtasks", "/delete", "/tn", _TASK_NAME, "/f"],
        check=True,
    )


def task_exists() -> bool:
    """Return ``True`` if the scheduled task is registered."""
    result = subprocess.run(
        ["schtasks", "/query", "/tn", _TASK_NAME, "/fo", "LIST"],
        capture_output=True,
    )
    return result.returncode == 0


def _find_pythonw() -> str:
    """Locate ``pythonw.exe`` next to the current interpreter."""
    candidate = APP_DIR / "pythonw.exe"  # unlikely but check
    if candidate.is_file():
        return str(candidate)
    candidate = Path(sys.executable).parent / "pythonw.exe"
    if candidate.is_file():
        return str(candidate)
    # Fallback: assume same directory as sys.executable
    return str(Path(sys.executable).parent / "pythonw.exe")


# ---------------------------------------------------------------------------
# Native Windows service (requires pywin32 + system-wide Python)
# ---------------------------------------------------------------------------


def install_service(
    python_exe: str | None = None,
    script_path: str | None = None,
) -> None:
    """Install the LLama Launcher as a native Windows SCM service.

    Requires ``pywin32`` and a **system-wide** Python installation
    (e.g. ``C:\\Python312\\``).  The service runs as ``LocalSystem`` which
    cannot access user-profile directories like ``%LOCALAPPDATA%``.

    Parameters
    ----------
    python_exe:
        Path to the Python interpreter.  Defaults to ``sys.executable``.
    script_path:
        Path to this ``service.py`` module.  Defaults to auto-detected path.
    """
    if not _has_pywin32:
        raise ImportError(
            "pywin32 is required for Windows service support. "
            "Install it with: pip install pywin32"
        )

    if python_exe is None:
        python_exe = sys.executable

    if script_path is None:
        script_path = str(Path(__file__).resolve())

    win32serviceutil.InstallService(
        pythonClassString=f"{__name__}._LlamaLauncherService",
        serviceName=_LlamaLauncherService._svc_name_,
        displayName=_LlamaLauncherService._svc_display_name_,
        description=_LlamaLauncherService._svc_description_,
        startType=win32service.SERVICE_AUTO_START,
        exeName=python_exe,
        exeArgs=f'"{script_path}"',
    )


def uninstall_service() -> None:
    """Remove the LLama Launcher Windows service."""
    if not _has_pywin32:
        raise ImportError("pywin32 is required.")
    win32serviceutil.RemoveService(_LlamaLauncherService._svc_name_)


# ---------------------------------------------------------------------------
# Entry point (called by win32serviceutil when the SCM launches the service)
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    if not _has_pywin32:
        print("Error: pywin32 is required for Windows service support.")
        print("Install it with: pip install pywin32")
        sys.exit(1)

    # When launched by the SCM, sys.argv contains only the script path.
    # HandleCommandLine detects this and calls StartService() internally.
    # Wrap in try/except to catch startup errors that would otherwise
    # surface as opaque error 1053 (timeout).
    try:
        win32serviceutil.HandleCommandLine(_LlamaLauncherService)  # noqa: F821
    except Exception:
        # Write to a crash log so the user can diagnose error 1053.
        ensure_state()
        crash_log = STATE_DIR / "service-crash.log"
        with open(crash_log, "a", encoding="utf-8") as f:
            import traceback
            f.write(f"--- {__name__} crash at {__import__('datetime').datetime.now()} ---\n")
            traceback.print_exc(file=f)
        raise
