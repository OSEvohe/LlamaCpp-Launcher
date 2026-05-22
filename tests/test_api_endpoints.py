"""HTTP API endpoint tests for LLama Launcher.

Uses stdlib ``http.client`` and ``http.server`` only — no external
dependencies.  Boots a real ``ThreadingHTTPServer`` in a background
thread backed by a ``LlamaLauncherService`` with an isolated temp dir.
"""
import http.client
import json
import shutil
import tempfile
import threading
from pathlib import Path

from llama_launcher.api import LlamaLauncherService
from llama_launcher.models import Profile
from llama_launcher.server import create_api_server

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _boot_server(app_dir: Path) -> tuple:
    """Start a test API server in a background thread.

    Returns ``(host, port, stop_event, server, thread)``.  The stop
    event, when set, causes the server to shut down gracefully.
    """
    svc = LlamaLauncherService(app_dir=app_dir)
    stop_event = threading.Event()
    ready_event = threading.Event()

    server = create_api_server(svc, host="127.0.0.1", port=0)
    actual_port = server.server_address[1]

    def _run() -> None:
        ready_event.set()
        while not stop_event.is_set():
            server.handle_request()

    t = threading.Thread(target=_run, daemon=True)
    t.start()
    # Wait for server thread to be ready (up to 5 s)
    ready_event.wait(timeout=5)
    return ("127.0.0.1", actual_port, stop_event, server, t)


def _client(host: str, port: int) -> http.client.HTTPConnection:
    return http.client.HTTPConnection(host, port, timeout=5)


def _request(
    method: str, path: str, host: str, port: int, body: dict | None = None
) -> tuple[int, dict | None]:
    """Send *method* *path* and return ``(status, parsed_json)``."""
    conn = _client(host, port)
    headers = {}
    if body is not None:
        raw = json.dumps(body).encode("utf-8")
        headers["Content-Length"] = str(len(raw))
        headers["Content-Type"] = "application/json"
        conn.request(method, path, body=raw, headers=headers)
    else:
        conn.request(method, path, headers=headers)
    resp = conn.getresponse()
    status = resp.status
    raw_body = resp.read().decode("utf-8")
    conn.close()
    try:
        data = json.loads(raw_body) if raw_body else None
    except json.JSONDecodeError:
        data = None
    return status, data


def _request_raw(
    method: str, path: str, host: str, port: int, raw_body: bytes
) -> tuple[int, dict | None]:
    """Send *method* *path* with raw bytes body and return ``(status, parsed_json)``."""
    conn = _client(host, port)
    headers = {
        "Content-Length": str(len(raw_body)),
        "Content-Type": "application/json",
    }
    conn.request(method, path, body=raw_body, headers=headers)
    resp = conn.getresponse()
    status = resp.status
    raw_resp = resp.read().decode("utf-8")
    conn.close()
    try:
        data = json.loads(raw_resp) if raw_resp else None
    except json.JSONDecodeError:
        data = None
    return status, data


# ---------------------------------------------------------------------------
# Fixtures (inline, no pytest dependency)
# ---------------------------------------------------------------------------


def _make_server_with_profiles(
    profiles: list[Profile] | None = None,
) -> tuple:
    """Boot a server pre-seeded with *profiles* and return test handles."""
    tmpdir = tempfile.mkdtemp()
    app_dir = Path(tmpdir)
    svc = LlamaLauncherService(app_dir=app_dir)
    if profiles:
        svc.save_profiles(profiles)
    else:
        svc.save_profiles([Profile(name="default")])
    host, port, stop_event, server, thread = _boot_server(app_dir)
    return host, port, stop_event, server, thread, tmpdir


# ---------------------------------------------------------------------------
# Tests: GET /api/status
# ---------------------------------------------------------------------------


def test_get_status_200_schema() -> None:
    """GET /api/status returns 200 with 'running' and 'pid' keys."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request("GET", "/api/status", host, port)
        assert status == 200
        assert "running" in data
        assert "pid" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: GET /api/profiles
# ---------------------------------------------------------------------------


def test_get_profiles_200() -> None:
    """GET /api/profiles returns 200 with a list of profiles."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="alpha"), Profile(name="beta")]
    )
    try:
        status, data = _request("GET", "/api/profiles", host, port)
        assert status == 200
        assert isinstance(data, list)
        assert len(data) == 2
        assert data[0]["name"] == "alpha"
        assert data[1]["name"] == "beta"
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: GET /api/profiles/:index
# ---------------------------------------------------------------------------


def test_get_profile_by_index_200() -> None:
    """GET /api/profiles/0 returns 200 with the profile dict."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="first"), Profile(name="second")]
    )
    try:
        status, data = _request("GET", "/api/profiles/0", host, port)
        assert status == 200
        assert data["name"] == "first"
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_get_profile_by_index_404() -> None:
    """GET /api/profiles/99 returns 404 when index is out of range."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="only")]
    )
    try:
        status, data = _request("GET", "/api/profiles/99", host, port)
        assert status == 404
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: POST /api/profiles
# ---------------------------------------------------------------------------


def test_post_profile_201() -> None:
    """POST /api/profiles with valid name returns 201."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request(
            "POST", "/api/profiles", host, port, body={"name": "new-profile"}
        )
        assert status == 201
        assert data["name"] == "new-profile"
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_post_profile_invalid_name_400() -> None:
    """POST /api/profiles with non-string name returns 400."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request(
            "POST", "/api/profiles", host, port, body={"name": 12345}
        )
        assert status == 400
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_post_profile_oversized_413() -> None:
    """POST /api/profiles with oversized Content-Length returns 413."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        # Declare a body much larger than 1 MB; server rejects on Content-Length alone
        raw = b'{"name": "x"}'
        # Override Content-Length to exceed _MAX_BODY (1 MB)
        conn = _client(host, port)
        headers = {
            "Content-Length": "2000000",
            "Content-Type": "application/json",
        }
        conn.request("POST", "/api/profiles", body=raw, headers=headers)
        resp = conn.getresponse()
        status = resp.status
        raw_resp = resp.read().decode("utf-8")
        conn.close()
        data = json.loads(raw_resp) if raw_resp else None
        assert status == 413
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: PUT /api/profiles/:index
# ---------------------------------------------------------------------------


def test_put_profile_200() -> None:
    """PUT /api/profiles/0 with valid body returns 200."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="original")]
    )
    try:
        status, data = _request(
            "PUT", "/api/profiles/0", host, port, body={"name": "updated"}
        )
        assert status == 200
        assert data["name"] == "updated"
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_put_profile_out_of_range_404() -> None:
    """PUT /api/profiles/99 returns 404 when index is out of range."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="only")]
    )
    try:
        status, data = _request(
            "PUT", "/api/profiles/99", host, port, body={"name": "x"}
        )
        assert status == 404
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_put_profile_missing_body_400() -> None:
    """PUT /api/profiles/0 with no body returns 400."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="only")]
    )
    try:
        status, data = _request("PUT", "/api/profiles/0", host, port, body=None)
        assert status == 400
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_put_profile_advanced_fields_200() -> None:
    """PUT /api/profiles/0 with advanced_favorites + advanced_values returns 200."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="original")]
    )
    try:
        status, data = _request(
            "PUT",
            "/api/profiles/0",
            host,
            port,
            body={
                "advanced_favorites": ["--verbose", "--log-disable"],
                "advanced_values": {"--verbose": "1", "--log-disable": ""},
            },
        )
        assert status == 200
        assert data["advanced_favorites"] == ["--verbose", "--log-disable"]
        assert data["advanced_values"] == {"--verbose": "1", "--log-disable": ""}
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_get_profile_returns_advanced_fields() -> None:
    """GET /api/profiles/0 returns previously saved advanced fields."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="original")]
    )
    try:
        _request(
            "PUT",
            "/api/profiles/0",
            host,
            port,
            body={
                "advanced_favorites": ["--flash-attn"],
                "advanced_values": {"--flash-attn": "q2"},
            },
        )
        status, data = _request("GET", "/api/profiles/0", host, port)
        assert status == 200
        assert data["advanced_favorites"] == ["--flash-attn"]
        assert data["advanced_values"] == {"--flash-attn": "q2"}
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_put_profile_partial_preserves_unrelated_fields() -> None:
    """Partial PUT with only advanced fields preserves unrelated profile fields."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="keep-me", model_path="/models/m.gguf", ctx_size=8192)]
    )
    try:
        status, data = _request(
            "PUT",
            "/api/profiles/0",
            host,
            port,
            body={
                "advanced_favorites": ["--temp"],
                "advanced_values": {"--temp": "0.5"},
            },
        )
        assert status == 200
        assert data["name"] == "keep-me"
        assert data["model_path"] == "/models/m.gguf"
        assert data["ctx_size"] == 8192
        assert data["advanced_favorites"] == ["--temp"]
        assert data["advanced_values"] == {"--temp": "0.5"}
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: DELETE /api/profiles/:index
# ---------------------------------------------------------------------------


def test_delete_profile_200() -> None:
    """DELETE /api/profiles/0 returns 200 with 'deleted' key."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="a"), Profile(name="b")]
    )
    try:
        status, data = _request("DELETE", "/api/profiles/0", host, port)
        assert status == 200
        assert data["deleted"] == 0
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_delete_profile_out_of_range_404() -> None:
    """DELETE /api/profiles/99 returns 404 when index is out of range."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles(
        [Profile(name="only")]
    )
    try:
        status, data = _request("DELETE", "/api/profiles/99", host, port)
        assert status == 404
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: GET /api/settings
# ---------------------------------------------------------------------------


def test_get_settings_200() -> None:
    """GET /api/settings returns 200 with expected keys."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request("GET", "/api/settings", host, port)
        assert status == 200
        assert "llama_server_path" in data
        assert "model_dirs" in data
        assert "api_host" in data
        assert "api_port" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: PUT /api/settings
# ---------------------------------------------------------------------------


def test_put_settings_200() -> None:
    """PUT /api/settings with valid body returns 200."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request(
            "PUT",
            "/api/settings",
            host,
            port,
            body={"api_host": "0.0.0.0", "api_port": 9090},
        )
        assert status == 200
        assert data["api_host"] == "0.0.0.0"
        assert data["api_port"] == 9090
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_put_settings_invalid_api_port_400() -> None:
    """PUT /api/settings with non-integer api_port returns 400."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request(
            "PUT", "/api/settings", host, port, body={"api_port": "not-a-number"}
        )
        assert status == 400
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_put_settings_api_port_out_of_range_400() -> None:
    """PUT /api/settings with api_port > 65535 returns 400."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request(
            "PUT", "/api/settings", host, port, body={"api_port": 70000}
        )
        assert status == 400
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


def test_put_settings_invalid_api_host_400() -> None:
    """PUT /api/settings with non-string api_host returns 400."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request(
            "PUT", "/api/settings", host, port, body={"api_host": 12345}
        )
        assert status == 400
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: GET /api/models
# ---------------------------------------------------------------------------


def test_get_models_200_shape() -> None:
    """GET /api/models returns 200 with 'models' key (list)."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request("GET", "/api/models", host, port)
        assert status == 200
        assert "models" in data
        assert isinstance(data["models"], list)
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: GET /api/logs
# ---------------------------------------------------------------------------


def test_get_logs_200_shape() -> None:
    """GET /api/logs returns 200 with expected keys."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request("GET", "/api/logs", host, port)
        assert status == 200
        assert "chunk" in data
        assert "last_size" in data
        assert "reset" in data
        assert "last_marker" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Tests: Unknown route 404
# ---------------------------------------------------------------------------


def test_unknown_route_404() -> None:
    """GET /api/nonexistent returns 404."""
    host, port, stop, server, thread, tmpdir = _make_server_with_profiles()
    try:
        status, data = _request("GET", "/api/nonexistent", host, port)
        assert status == 404
        assert "error" in data
    finally:
        stop.set()
        server.server_close()
        thread.join(timeout=2)
        shutil.rmtree(tmpdir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


if __name__ == "__main__":
    tests = [
        test_get_status_200_schema,
        test_get_profiles_200,
        test_get_profile_by_index_200,
        test_get_profile_by_index_404,
        test_post_profile_201,
        test_post_profile_invalid_name_400,
        test_post_profile_oversized_413,
        test_put_profile_200,
        test_put_profile_out_of_range_404,
        test_put_profile_missing_body_400,
        test_put_profile_advanced_fields_200,
        test_get_profile_returns_advanced_fields,
        test_put_profile_partial_preserves_unrelated_fields,
        test_delete_profile_200,
        test_delete_profile_out_of_range_404,
        test_get_settings_200,
        test_put_settings_200,
        test_put_settings_invalid_api_port_400,
        test_put_settings_api_port_out_of_range_400,
        test_put_settings_invalid_api_host_400,
        test_get_models_200_shape,
        test_get_logs_200_shape,
        test_unknown_route_404,
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
