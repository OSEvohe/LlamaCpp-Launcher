"""Stdlib-only HTTP API server for LLama Launcher.

Exposes JSON REST endpoints backed by ``LlamaLauncherService``.
No external dependencies; uses ``http.server``, ``json``, and ``urllib.parse``.
"""
import json
import os
import re
from dataclasses import asdict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, List, Optional
from urllib.parse import parse_qs, urlparse

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

_MAX_BODY = 1 * 1024 * 1024  # 1 MB

_INDEX_RE = re.compile(r"^/api/profiles/(\d+)$")

# Path to the bundled dashboard HTML (sibling directory to this module).
_DASHBOARD_PATH = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "static", "dashboard.html"
)

# ---------------------------------------------------------------------------
# Request handler
# ---------------------------------------------------------------------------


class _APIHandler(BaseHTTPRequestHandler):
    """HTTP request handler wired to a ``LlamaLauncherService`` instance."""

    # suppress default stderr logging per request
    def log_message(self, format, *args):  # noqa: A002
        pass

    # -- helpers -------------------------------------------------------------

    def _json_response(self, code: int, body: Any) -> None:
        payload = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _error(self, code: int, message: str) -> None:
        self._json_response(code, {"error": message})

    def _read_json_body(self) -> Optional[Dict[str, Any]]:
        """Read and parse the request body.

        Returns ``None`` on missing body, raises ``ValueError`` on malformed
        JSON or oversized payload (the caller catches and returns 400/413).
        """
        length = int(self.headers.get("Content-Length", 0))
        if length == 0:
            return None
        if length > _MAX_BODY:
            raise OverflowError("body exceeds 1 MB limit")
        raw = self.rfile.read(length)
        return json.loads(raw)

    def _service(self) -> Any:
        return self.server.service  # type: ignore[attr-defined]

    # -- routing -------------------------------------------------------------

    def do_GET(self) -> None:
        parsed = urlparse(self.path)
        path = parsed.path.rstrip("/") or "/"
        qs = parse_qs(parsed.query)

        if path == "/":
            self._handle_root()
        elif path == "/api/status":
            self._handle_status()
        elif path == "/api/profiles":
            self._handle_get_profiles()
        elif path == "/api/settings":
            self._handle_get_settings()
        elif path == "/api/options":
            self._handle_get_options()
        elif path == "/api/models":
            self._handle_get_models()
        elif path == "/api/logs":
            self._handle_get_logs(qs)
        elif path == "/api/monitoring":
            self._handle_get_monitoring()
        else:
            m = _INDEX_RE.match(f"/{path}" if not path.startswith("/") else path)
            if m:
                self._handle_get_profile(int(m.group(1)))
            else:
                self._error(404, "not found")

    def do_POST(self) -> None:
        path = urlparse(self.path).path.rstrip("/") or "/"
        if path == "/api/profiles":
            self._handle_post_profile()
        elif path == "/api/launch":
            self._handle_launch()
        elif path == "/api/stop":
            self._handle_stop()
        elif path == "/api/restart":
            self._handle_restart()
        else:
            self._error(404, "not found")

    def do_PUT(self) -> None:
        path = urlparse(self.path).path.rstrip("/") or "/"
        if path == "/api/settings":
            self._handle_put_settings()
        else:
            m = _INDEX_RE.match(f"/{path}" if not path.startswith("/") else path)
            if m:
                self._handle_put_profile(int(m.group(1)))
            else:
                self._error(404, "not found")

    def do_DELETE(self) -> None:
        path = urlparse(self.path).path.rstrip("/") or "/"
        m = _INDEX_RE.match(f"/{path}" if not path.startswith("/") else path)
        if m:
            self._handle_delete_profile(int(m.group(1)))
        else:
            self._error(404, "not found")

    # -- route handlers ------------------------------------------------------

    # Root
    def _handle_root(self) -> None:
        """Serve the dashboard HTML file, or a concise fallback if missing."""
        try:
            html = open(_DASHBOARD_PATH, encoding="utf-8").read()
        except OSError:
            html = (
                "<!DOCTYPE html><html><head><meta charset='utf-8'>"
                "<title>LLama Launcher</title></head>"
                "<body><h1>LLama Launcher</h1>"
                "<p>API running. Dashboard file not found.</p>"
                "</body></html>"
            )
        payload = html.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    # Status
    def _handle_status(self) -> None:
        svc = self._service()
        running, pid = svc.is_server_running()
        self._json_response(200, {
            "running": running,
            "pid": pid if running else None,
        })

    # Profiles
    def _handle_get_profiles(self) -> None:
        svc = self._service()
        profiles = svc.load_profiles()
        self._json_response(200, [asdict(p) for p in profiles])

    def _handle_get_profile(self, index: int) -> None:
        svc = self._service()
        profiles = svc.load_profiles()
        if not (0 <= index < len(profiles)):
            self._error(404, f"profile index {index} out of range")
            return
        self._json_response(200, asdict(profiles[index]))

    def _handle_post_profile(self) -> None:
        svc = self._service()
        try:
            body = self._read_json_body()
        except (ValueError, json.JSONDecodeError):
            self._error(400, "invalid JSON")
            return
        except OverflowError:
            self._error(413, "request body too large")
            return
        name = (body or {}).get("name", "default")
        if not isinstance(name, str):
            self._error(400, "name must be a string")
            return
        profile = svc.add_profile(name)
        self._json_response(201, asdict(profile))

    def _handle_put_profile(self, index: int) -> None:
        svc = self._service()
        try:
            body = self._read_json_body()
        except (ValueError, json.JSONDecodeError):
            self._error(400, "invalid JSON")
            return
        except OverflowError:
            self._error(413, "request body too large")
            return
        if body is None:
            self._error(400, "missing request body")
            return
        try:
            updated = svc.update_profile(index, body)
        except IndexError:
            self._error(404, f"profile index {index} out of range")
            return
        except ValueError as exc:
            self._error(400, str(exc))
            return
        self._json_response(200, asdict(updated))

    def _handle_delete_profile(self, index: int) -> None:
        svc = self._service()
        if not svc.delete_profile(index):
            self._error(404, f"profile index {index} out of range")
            return
        self._json_response(200, {"deleted": index})

    # Settings
    def _handle_get_settings(self) -> None:
        svc = self._service()
        settings = svc.load_global()
        self._json_response(200, asdict(settings))

    def _handle_put_settings(self) -> None:
        svc = self._service()
        try:
            body = self._read_json_body()
        except (ValueError, json.JSONDecodeError):
            self._error(400, "invalid JSON")
            return
        except OverflowError:
            self._error(413, "request body too large")
            return
        if body is None:
            self._error(400, "missing request body")
            return

        # --- validate api_host (only if present) ---
        if "api_host" in body:
            if not isinstance(body["api_host"], str):
                self._error(400, "api_host must be a string")
                return

        # --- validate/coerce api_port (only if present) ---
        if "api_port" in body:
            raw_port = body["api_port"]
            if isinstance(raw_port, bool):
                self._error(400, "api_port must be an integer")
                return
            try:
                api_port = int(raw_port)
            except (TypeError, ValueError):
                self._error(400, "api_port must be an integer")
                return
            if not (0 <= api_port <= 65535):
                self._error(400, "api_port must be between 0 and 65535")
                return
            body["api_port"] = api_port

        settings = svc.update_global(body)
        self._json_response(200, asdict(settings))

    # Options
    def _handle_get_options(self) -> None:
        svc = self._service()
        settings = svc.load_global()
        exe_path = settings.llama_server_path or str(svc.default_server_path)
        try:
            opts = svc.load_options(exe_path)
        except RuntimeError as exc:
            self._error(500, str(exc))
            return
        self._json_response(200, {
            k: asdict(v) for k, v in opts.items()
        })

    # Models
    def _handle_get_models(self) -> None:
        svc = self._service()
        settings = svc.load_global()
        models = svc.discover_models(settings.model_dirs)
        self._json_response(200, {"models": models})

    # Launch / Stop / Restart
    def _handle_launch(self) -> None:
        svc = self._service()
        try:
            body = self._read_json_body()
        except (ValueError, json.JSONDecodeError):
            self._error(400, "invalid JSON")
            return
        except OverflowError:
            self._error(413, "request body too large")
            return
        if body is None:
            self._error(400, "missing request body")
            return
        profile_index = body.get("profile_index", 0)
        if not isinstance(profile_index, int) or isinstance(profile_index, bool):
            self._error(400, "profile_index must be an integer")
            return
        exe_path = body.get("exe_path", "")

        profiles = svc.load_profiles()
        if not (0 <= profile_index < len(profiles)):
            self._error(400, f"profile index {profile_index} out of range")
            return
        profile = profiles[profile_index]

        settings = svc.load_global()
        resolved_exe = exe_path or settings.llama_server_path
        if not resolved_exe:
            self._error(400, "no exe_path provided and none saved in settings")
            return

        try:
            opts = svc.load_options(resolved_exe)
        except RuntimeError as exc:
            self._error(500, str(exc))
            return

        try:
            cmd = svc.build_command(profile, resolved_exe, opts)
        except RuntimeError as exc:
            self._error(400, str(exc))
            return

        try:
            pid = svc.launch(cmd, exe_path=resolved_exe)
        except RuntimeError as exc:
            self._error(500, str(exc))
            return

        self._json_response(200, {"pid": pid, "command": cmd})

    def _handle_stop(self) -> None:
        svc = self._service()
        pid = svc.stop()
        self._json_response(200, {"stopped": pid > 0, "pid": pid})

    def _handle_restart(self) -> None:
        svc = self._service()
        try:
            body = self._read_json_body()
        except (ValueError, json.JSONDecodeError):
            self._error(400, "invalid JSON")
            return
        except OverflowError:
            self._error(413, "request body too large")
            return

        if body is None:
            self._error(400, "missing request body")
            return
        profile_index = body.get("profile_index", 0)
        if not isinstance(profile_index, int) or isinstance(profile_index, bool):
            self._error(400, "profile_index must be an integer")
            return
        exe_path = body.get("exe_path", "")

        profiles = svc.load_profiles()
        if not (0 <= profile_index < len(profiles)):
            self._error(400, f"profile index {profile_index} out of range")
            return
        profile = profiles[profile_index]

        settings = svc.load_global()
        resolved_exe = exe_path or settings.llama_server_path
        if not resolved_exe:
            self._error(400, "no exe_path provided and none saved in settings")
            return

        try:
            opts = svc.load_options(resolved_exe)
        except RuntimeError as exc:
            self._error(500, str(exc))
            return

        try:
            cmd = svc.build_command(profile, resolved_exe, opts)
        except RuntimeError as exc:
            self._error(400, str(exc))
            return

        try:
            pid = svc.restart(cmd, exe_path=resolved_exe)
        except RuntimeError as exc:
            self._error(500, str(exc))
            return

        self._json_response(200, {"pid": pid, "command": cmd})

    # Logs
    def _handle_get_logs(self, qs: Dict[str, List[str]]) -> None:
        svc = self._service()
        try:
            last_size = int(qs.get("last_size", ["0"])[0])
        except (ValueError, IndexError):
            last_size = 0
        last_marker = qs.get("last_marker", [""])[0]

        chunk, new_size, reset, new_marker = svc.tail_log(last_size, last_marker)
        self._json_response(200, {
            "chunk": chunk,
            "last_size": new_size,
            "reset": reset,
            "last_marker": new_marker,
        })

    # Monitoring
    def _handle_get_monitoring(self) -> None:
        svc = self._service()
        running, pid = svc.is_server_running()
        used_ram, total_ram = svc.get_ram_usage()
        used_vram, total_vram = svc.get_gpu_vram()
        process_ram = svc.get_process_ram(pid) if running else 0
        self._json_response(200, {
            "running": running,
            "pid": pid if running else None,
            "ram": {
                "used": used_ram,
                "total": total_ram,
                "used_human": svc.format_bytes(used_ram),
                "total_human": svc.format_bytes(total_ram),
            },
            "vram": {
                "used": used_vram,
                "total": total_vram,
                "used_human": svc.format_bytes(used_vram),
                "total_human": svc.format_bytes(total_vram),
            },
            "process_ram": process_ram,
            "process_ram_human": svc.format_bytes(process_ram),
        })


# ---------------------------------------------------------------------------
# Public factory / runner
# ---------------------------------------------------------------------------


def create_api_server(
    service: Any,
    host: str = "127.0.0.1",
    port: int = 8080,
) -> ThreadingHTTPServer:
    """Create a ``ThreadingHTTPServer`` bound to *host:port* using *service*."""
    server = ThreadingHTTPServer((host, port), _APIHandler)
    server.service = service  # type: ignore[attr-defined]
    return server


def run_api_server(
    service: Any,
    host: str = "127.0.0.1",
    port: int = 8080,
) -> None:
    """Start the API server and block via ``serve_forever()``."""
    server = create_api_server(service, host, port)
    try:
        server.serve_forever()
    finally:
        server.server_close()
