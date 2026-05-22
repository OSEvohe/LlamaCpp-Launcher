"""Process lifecycle helpers for llama-server."""
import subprocess
from pathlib import Path


def read_pid(pid_file: Path) -> int:
    """Return the PID stored in *pid_file*, or 0 on any failure."""
    if not pid_file.exists():
        return 0
    try:
        return int(pid_file.read_text(encoding="utf-8").strip())
    except Exception:
        return 0


def is_process_running(pid: int) -> bool:
    """Check whether a Windows process with *pid* is alive (tasklist)."""
    if pid <= 0:
        return False
    try:
        proc = subprocess.run(
            ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        out = (proc.stdout or "").strip()
        if not out:
            return False
        if "No tasks are running" in out:
            return False
        return f'"{pid}"' in out
    except Exception:
        return False


def start_server(cmd: list, stdout_path: Path, cwd: Path) -> int:
    """Launch *cmd* as a detached subprocess, writing stdout to *stdout_path*.

    Returns the child PID on success.
    """
    with stdout_path.open("w", encoding="utf-8") as out:
        p = subprocess.Popen(
            cmd,
            stdout=out,
            stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL,
            creationflags=0x08000000 | 0x00000200,
            cwd=str(cwd),
        )
    return p.pid


def stop_server(pid: int) -> None:
    """Force-kill the process tree rooted at *pid* (taskkill /F /T)."""
    subprocess.run(
        ["taskkill", "/PID", str(pid), "/F", "/T"],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
