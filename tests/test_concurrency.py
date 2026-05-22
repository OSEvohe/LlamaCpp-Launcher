"""Threaded smoke tests for concurrent profile/settings update safety."""
import json
import threading
import tempfile
from pathlib import Path
from unittest.mock import patch

from llama_launcher.api import LlamaLauncherService
from llama_launcher.models import Profile


def test_concurrent_put_profile_no_data_loss() -> None:
    """Concurrent PUT profile operations must not lose or corrupt data."""
    tmpdir = tempfile.mkdtemp()
    svc = LlamaLauncherService(app_dir=Path(tmpdir))

    # Seed with 3 profiles
    svc.save_profiles([
        Profile(name="p0", ctx_size=4096, threads=8),
        Profile(name="p1", ctx_size=8192, threads=16),
        Profile(name="p2", ctx_size=16384, threads=32),
    ])

    errors: list[str] = []
    barrier = threading.Barrier(10)

    def worker(thread_id: int) -> None:
        try:
            barrier.wait(timeout=5)
            for i in range(50):
                idx = thread_id % 3
                data = {
                    "name": f"p{idx}-t{thread_id}-i{i}",
                    "ctx_size": 4096 + thread_id * 100 + i,
                    "threads": 8 + thread_id,
                }
                updated = svc.update_profile(idx, data)
                # Returned profile must match what we wrote
                assert updated.name == f"p{idx}-t{thread_id}-i{i}", \
                    f"Thread {thread_id}: expected p{idx}-t{thread_id}-i{i}, got {updated.name}"
                assert updated.ctx_size == 4096 + thread_id * 100 + i
        except Exception as e:
            errors.append(f"Thread {thread_id}: {e}")

    threads = [threading.Thread(target=worker, args=(t,)) for t in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=30)

    assert not errors, f"Errors: {errors}"

    # Verify on-disk JSON is well-formed (no partial writes)
    profiles_file = Path(tmpdir) / ".launcher" / "profiles.json"
    raw = profiles_file.read_text(encoding="utf-8")
    data = json.loads(raw)
    assert "profiles" in data
    assert len(data["profiles"]) == 3
    for p in data["profiles"]:
        assert isinstance(p["name"], str)
        assert isinstance(p["ctx_size"], int)


def test_concurrent_read_write_no_default_fallback() -> None:
    """Concurrent reads must not return default-fallback from partial JSON."""
    tmpdir = tempfile.mkdtemp()
    svc = LlamaLauncherService(app_dir=Path(tmpdir))

    # Seed with distinct profiles
    svc.save_profiles([
        Profile(name="alpha", ctx_size=2048),
        Profile(name="beta", ctx_size=4096),
    ])

    errors: list[str] = []
    barrier = threading.Barrier(6)

    def reader(thread_id: int) -> None:
        try:
            barrier.wait(timeout=5)
            for _ in range(100):
                profiles = svc.load_profiles()
                # Must always get >= 2 profiles (no fallback to [Profile()])
                assert len(profiles) >= 2, \
                    f"Thread {thread_id}: got {len(profiles)} profiles, expected >= 2"
                # Names must be non-empty (no default "default" fallback)
                for p in profiles:
                    assert isinstance(p.name, str) and len(p.name) > 0
        except Exception as e:
            errors.append(f"Reader {thread_id}: {e}")

    def writer() -> None:
        try:
            barrier.wait(timeout=5)
            for i in range(100):
                svc.update_profile(0, {"name": f"alpha-{i}", "ctx_size": 2048 + i})
                svc.update_profile(1, {"name": f"beta-{i}", "ctx_size": 4096 + i})
        except Exception as e:
            errors.append(f"Writer: {e}")

    threads = [threading.Thread(target=reader, args=(t,)) for t in range(5)]
    threads.append(threading.Thread(target=writer))
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=30)

    assert not errors, f"Errors: {errors}"


def test_concurrent_update_global_no_data_loss() -> None:
    """Concurrent global settings updates must not lose data."""
    tmpdir = tempfile.mkdtemp()
    svc = LlamaLauncherService(app_dir=Path(tmpdir))

    # Seed settings
    svc.save_global(type(svc.load_global())())

    errors: list[str] = []
    barrier = threading.Barrier(5)

    def writer(thread_id: int) -> None:
        try:
            barrier.wait(timeout=5)
            for i in range(50):
                result = svc.update_global({
                    "llama_server_path": f"/exe/path-{thread_id}-{i}",
                    "api_port": 8080 + thread_id,
                })
                assert result.llama_server_path == f"/exe/path-{thread_id}-{i}"
                assert result.api_port == 8080 + thread_id
        except Exception as e:
            errors.append(f"Writer {thread_id}: {e}")

    threads = [threading.Thread(target=writer, args=(t,)) for t in range(5)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=30)

    assert not errors, f"Errors: {errors}"

    # Verify on-disk JSON is well-formed
    global_file = Path(tmpdir) / ".launcher" / "global.json"
    raw = global_file.read_text(encoding="utf-8")
    data = json.loads(raw)
    assert "llama_server_path" in data
    assert "api_port" in data
    assert isinstance(data["api_port"], int)


def test_delete_profile_atomic_range_check() -> None:
    """delete_profile must atomically check range and delete."""
    tmpdir = tempfile.mkdtemp()
    svc = LlamaLauncherService(app_dir=Path(tmpdir))

    svc.save_profiles([Profile(name="only")])

    # Valid index
    assert svc.delete_profile(0) is True
    # After deletion, a default profile is created
    profiles = svc.load_profiles()
    assert len(profiles) == 1

    # Out of range
    assert svc.delete_profile(99) is False


def test_restart_atomic_no_interleave() -> None:
    """restart() must execute stop+launch under one lock boundary.

    Concurrent restart calls must not interleave: each stop_server call
    must be immediately followed by start_server from the same thread.
    """
    tmpdir = tempfile.mkdtemp()
    svc = LlamaLauncherService(app_dir=Path(tmpdir))

    # Seed profiles so launch can proceed past profile resolution
    svc.save_profiles([Profile(name="test", model_path="")])

    # Record (operation, thread_name) pairs in global order
    call_log: list[tuple[str, str]] = []
    log_lock = threading.Lock()

    def _log(op: str) -> None:
        with log_lock:
            call_log.append((op, threading.current_thread().name))

    fake_pid = 99999

    # Shared state to simulate a real process lifecycle
    state = {"running": False, "pid_file_exists": False}
    state_lock = threading.Lock()

    def mock_read_pid(pid_file):
        with state_lock:
            if state["pid_file_exists"]:
                return fake_pid
            return 0

    def mock_is_process_running(pid):
        with state_lock:
            return state["running"] and pid == fake_pid

    def mock_start_server(cmd, stdout_path, cwd):
        with state_lock:
            state["running"] = True
            state["pid_file_exists"] = True
        _log("start_server")
        return fake_pid

    def mock_stop_server(pid):
        with state_lock:
            state["running"] = False
        _log("stop_server")

    with patch("llama_launcher.process.read_pid", side_effect=mock_read_pid), \
         patch("llama_launcher.process.is_process_running", side_effect=mock_is_process_running), \
         patch("llama_launcher.process.start_server", side_effect=mock_start_server), \
         patch("llama_launcher.process.stop_server", side_effect=mock_stop_server):

        errors: list[str] = []
        go_event = threading.Event()

        def restart_worker(thread_id: int) -> None:
            try:
                go_event.wait(timeout=5)
                for i in range(30):
                    svc.restart(["dummy"], exe_path="")
            except Exception as e:
                errors.append(f"restart-{thread_id}: {type(e).__name__}: {e}")

        threads = [
            threading.Thread(target=restart_worker, args=(0,), name="R0"),
            threading.Thread(target=restart_worker, args=(1,), name="R1"),
            threading.Thread(target=restart_worker, args=(2,), name="R2"),
            threading.Thread(target=restart_worker, args=(3,), name="R3"),
            threading.Thread(target=restart_worker, args=(4,), name="R4"),
        ]
        for t in threads:
            t.start()
        go_event.set()
        for t in threads:
            t.join(timeout=30)

        assert not errors, f"Errors: {errors}"

        # Extract only the process-level calls (stop_server / start_server)
        proc_calls = [(op, name) for op, name in call_log if op in ("stop_server", "start_server")]

        # Each restart() should produce stop_server immediately followed by
        # start_server from the same thread, with no other thread's calls
        # in between.  Walk the log and pair them up.
        i = 0
        while i < len(proc_calls):
            op, name = proc_calls[i]
            if op == "stop_server":
                # Next call must be start_server from the same thread
                if i + 1 >= len(proc_calls):
                    assert False, f"stop_server at index {i} has no following call"
                next_op, next_name = proc_calls[i + 1]
                assert next_op == "start_server", \
                    f"Expected start_server after stop_server at {i}, got {next_op}"
                assert next_name == name, \
                    f"Thread mismatch: stop_server by {name} but start_server by {next_name}"
                i += 2
            else:
                # start_server without preceding stop_server means the
                # process was already stopped — fine, skip it.
                i += 1


if __name__ == "__main__":
    test_concurrent_put_profile_no_data_loss()
    print("PASS: test_concurrent_put_profile_no_data_loss")

    test_concurrent_read_write_no_default_fallback()
    print("PASS: test_concurrent_read_write_no_default_fallback")

    test_concurrent_update_global_no_data_loss()
    print("PASS: test_concurrent_update_global_no_data_loss")

    test_delete_profile_atomic_range_check()
    print("PASS: test_delete_profile_atomic_range_check")

    test_restart_atomic_no_interleave()
    print("PASS: test_restart_atomic_no_interleave")

    print("\nAll concurrency smoke tests passed.")
