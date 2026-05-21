"""System monitoring and log-tailing helpers."""
import ctypes
import subprocess
from pathlib import Path


def bytes_to_gb(value: int) -> str:
    """Format *value* (bytes) as a human-readable GB string like ``12.3GB``."""
    gb = value / (1024 ** 3)
    return f"{gb:.1f}GB"


def ram_usage_bytes() -> tuple[int, int]:
    """Return ``(used_bytes, total_bytes)`` of physical RAM via Windows API."""
    class MemoryStatusEx(ctypes.Structure):
        _fields_ = [
            ("dwLength", ctypes.c_ulong),
            ("dwMemoryLoad", ctypes.c_ulong),
            ("ullTotalPhys", ctypes.c_ulonglong),
            ("ullAvailPhys", ctypes.c_ulonglong),
            ("ullTotalPageFile", ctypes.c_ulonglong),
            ("ullAvailPageFile", ctypes.c_ulonglong),
            ("ullTotalVirtual", ctypes.c_ulonglong),
            ("ullAvailVirtual", ctypes.c_ulonglong),
            ("ullAvailExtendedVirtual", ctypes.c_ulonglong),
        ]

    status = MemoryStatusEx()
    status.dwLength = ctypes.sizeof(MemoryStatusEx)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        return (0, 0)
    total = int(status.ullTotalPhys)
    avail = int(status.ullAvailPhys)
    used = max(0, total - avail)
    return (used, total)


def process_ram_bytes(pid: int) -> int:
    """Return approximate RAM usage (bytes) of the process with *pid*."""
    if pid <= 0:
        return 0
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
        if not out or "INFO:" in out.upper():
            return 0
        row = [x.strip('"') for x in out.split('","')]
        if len(row) < 5:
            return 0
        mem_field = row[4].replace(",", "").replace(" ", "").replace("K", "").strip()
        kb = int(mem_field) if mem_field.isdigit() else 0
        return kb * 1024
    except Exception:
        return 0


def gpu_vram_info() -> tuple[int, int]:
    """Return ``(used_bytes, total_bytes)`` of GPU VRAM via nvidia-smi.

    Aggregates across all GPUs when multiple are present.
    """
    try:
        gpus = subprocess.run(
            [
                "nvidia-smi",
                "--query-gpu=memory.used,memory.total",
                "--format=csv,noheader,nounits",
            ],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        lines = [ln.strip() for ln in (gpus.stdout or "").splitlines() if ln.strip()]
        if not lines:
            return (0, 0)

        used_sum = 0
        total_sum = 0
        for line in lines:
            parts = [x.strip() for x in line.split(",")]
            if len(parts) < 2:
                continue
            if parts[0].isdigit():
                used_sum += int(parts[0]) * 1024 * 1024
            if parts[1].isdigit():
                total_sum += int(parts[1]) * 1024 * 1024

        return (used_sum, total_sum)
    except Exception:
        return (0, 0)


def build_monitoring_text() -> str:
    """Return a two-line ``RAM … / VRAM …`` string suitable for display."""
    used_ram, total_ram = ram_usage_bytes()
    if total_ram > 0:
        ram_line = f"RAM: {bytes_to_gb(used_ram)}/{bytes_to_gb(total_ram)}"
    else:
        ram_line = "RAM: N/A"

    used_vram, total_vram = gpu_vram_info()
    if total_vram > 0:
        vram_line = f"VRAM: {bytes_to_gb(used_vram)}/{bytes_to_gb(total_vram)}"
    else:
        vram_line = "VRAM: N/A"

    return f"{ram_line}\n{vram_line}"


_MARKER_LEN = 64


def tail_log_chunk(
    path: Path, last_size: int, prev_marker: str = ""
) -> tuple[str, int, bool, str]:
    """Read new content appended to *path* since *last_size*.

    Returns ``(chunk_text, new_size, reset_required, new_marker)``.

    - ``reset_required`` is ``True`` only when the file was genuinely
      truncated (current size < last_size), when the file is empty
      and the caller had previously seen content (last_size > 0),
      or when a rewrite/replacement is detected (marker mismatch).
    - When ``reset_required`` is ``True``, ``chunk_text`` contains the
      full current file content so the caller can clear and repopulate
      in a single step without a second read.
    - A steady empty file (last_size == 0, current == 0) returns
      ``("", 0, False, "")`` — a no-op, not a truncation.
    - *prev_marker* is the tail of the previously-seen prefix (last
      ``_MARKER_LEN`` chars up to *last_size*).  When the file grows
      or stays the same size, the marker is re-checked at the
      expected boundary; a mismatch indicates a rewrite and triggers
      a reset.  The returned *new_marker* should be stored by the
      caller for the next poll.
    """
    data = path.read_text(encoding="utf-8", errors="replace")
    current = len(data)
    if last_size > current:
        # Genuine truncation: return full content for single-step recovery
        marker_start = max(0, current - _MARKER_LEN)
        return (data, current, True, data[marker_start:current])
    if last_size > 0 and current == 0:
        # File was emptied (truncated to zero): treat as truncation
        return ("", 0, True, "")
    if current <= last_size:
        # current == last_size > 0: check for equal-size rewrite
        if prev_marker:
            marker_start = max(0, last_size - _MARKER_LEN)
            if data[marker_start:last_size] != prev_marker:
                marker_start = max(0, current - _MARKER_LEN)
                return (data, current, True, data[marker_start:current])
        # Steady state (covers last_size == 0, current == 0)
        marker_start = max(0, current - _MARKER_LEN)
        new_marker = data[marker_start:current]
        return ("", current, False, new_marker)
    chunk = data[last_size:]
    # current > last_size: verify marker at boundary (rewrite detection)
    if prev_marker and last_size > 0:
        marker_start = max(0, last_size - _MARKER_LEN)
        if data[marker_start:last_size] != prev_marker:
            marker_start = max(0, current - _MARKER_LEN)
            return (data, current, True, data[marker_start:current])
    marker_start = max(0, current - _MARKER_LEN)
    new_marker = data[marker_start:current]
    return (chunk, current, False, new_marker)
