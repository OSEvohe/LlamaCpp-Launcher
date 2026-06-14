import argparse
import json
import subprocess
import threading
import uuid
from datetime import datetime
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse


ROOT = Path(__file__).resolve().parent
REPO_ROOT = ROOT.parent
RESULTS_DIR = ROOT / "results"
TASKS_FILE = ROOT / "tasks.json"
RUNNER_FILE = ROOT / "run_coding_bench.py"
HTML_FILE = ROOT / "webui.html"
RESTART_EXIT_CODE = 42

JOBS_LOCK = threading.Lock()
JOBS: dict[str, dict] = {}
SERVER = None
REQUEST_RESTART = False


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Serve the coding bench web UI.")
    parser.add_argument("--host", default="127.0.0.1", help="Bind host")
    parser.add_argument("--port", type=int, default=8765, help="Bind port")
    return parser.parse_args()


def load_json(path: Path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, data):
    path.write_text(json.dumps(data, indent=2), encoding="utf-8")


def available_model_files() -> list[str]:
    files = []
    for path in sorted(ROOT.glob("models*.json")):
        if path.name == "models.example.json":
            continue
        files.append(path.name)
    return files


def available_tasks() -> list[dict]:
    return load_json(TASKS_FILE)


def list_result_runs() -> list[dict]:
    if not RESULTS_DIR.exists():
        return []
    runs = []
    for path in sorted(RESULTS_DIR.iterdir(), reverse=True):
        if not path.is_dir():
            continue
        summary_file = path / "summary.json"
        if not summary_file.exists():
            continue
        summary = load_json(summary_file)
        runs.append(
            {
                "id": path.name,
                "generated_at": summary.get("generated_at", ""),
                "results_count": len(summary.get("results", [])),
                "summary": summary,
            }
        )
    return runs


def safe_run_path(run_id: str) -> Path | None:
    path = (RESULTS_DIR / run_id).resolve()
    try:
        path.relative_to(RESULTS_DIR.resolve())
    except ValueError:
        return None
    if not path.exists() or not path.is_dir():
        return None
    return path


def read_text_if_exists(path: Path) -> str:
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8", errors="ignore")


def job_snapshot() -> list[dict]:
    with JOBS_LOCK:
        return [dict(job) for job in sorted(JOBS.values(), key=lambda item: item["created_at"], reverse=True)]


def update_job(job_id: str, **fields):
    with JOBS_LOCK:
        JOBS[job_id].update(fields)


def run_job(job_id: str, command: list[str], cwd: Path, results_dir: Path):
    proc = subprocess.run(command, cwd=str(cwd), text=True, capture_output=True)
    combined = (proc.stdout or "") + (proc.stderr or "")
    log_path = results_dir / "webui-launch.log"
    log_path.write_text(combined, encoding="utf-8")
    update_job(
        job_id,
        status="completed" if proc.returncode == 0 else "failed",
        returncode=proc.returncode,
        finished_at=datetime.now().isoformat(),
        log_file=str(log_path.relative_to(REPO_ROOT)),
    )


def create_job(models_file: str, selected_models: list[str], selected_tasks: list[str]) -> dict:
    run_stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    results_dir = RESULTS_DIR / run_stamp
    job_id = str(uuid.uuid4())
    command = [
        "python",
        str(RUNNER_FILE),
        "--models-file",
        str(ROOT / models_file),
        "--output-dir",
        str(results_dir),
    ]
    for item in selected_models:
        command.extend(["--model", item])
    for item in selected_tasks:
        command.extend(["--task", item])

    job = {
        "id": job_id,
        "created_at": datetime.now().isoformat(),
        "status": "running",
        "models_file": models_file,
        "selected_models": selected_models,
        "selected_tasks": selected_tasks,
        "results_dir": str(results_dir.relative_to(REPO_ROOT)),
        "returncode": None,
        "finished_at": None,
        "log_file": None,
    }
    with JOBS_LOCK:
        JOBS[job_id] = job

    thread = threading.Thread(target=run_job, args=(job_id, command, REPO_ROOT, results_dir), daemon=True)
    thread.start()
    return job


class Handler(BaseHTTPRequestHandler):
    def _send_json(self, payload, status=HTTPStatus.OK):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_text(self, text: str, content_type: str = "text/plain; charset=utf-8", status=HTTPStatus.OK):
        body = text.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_json(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length > 0 else b"{}"
        return json.loads(raw.decode("utf-8"))

    def do_GET(self):
        parsed = urlparse(self.path)
        if parsed.path == "/":
            self._send_text(HTML_FILE.read_text(encoding="utf-8"), content_type="text/html; charset=utf-8")
            return
        if parsed.path == "/api/config":
            self._send_json({
                "model_files": available_model_files(),
                "tasks": available_tasks(),
                "jobs": job_snapshot(),
                "runs": list_result_runs(),
            })
            return
        if parsed.path == "/api/jobs":
            self._send_json({"jobs": job_snapshot()})
            return
        if parsed.path == "/api/runs":
            self._send_json({"runs": list_result_runs()})
            return
        if parsed.path.startswith("/api/run/"):
            run_id = parsed.path.split("/", 3)[3]
            run_path = safe_run_path(run_id)
            if run_path is None:
                self._send_json({"error": "run not found"}, status=HTTPStatus.NOT_FOUND)
                return
            summary_file = run_path / "summary.json"
            if not summary_file.exists():
                self._send_json({"error": "summary not found"}, status=HTTPStatus.NOT_FOUND)
                return
            self._send_json(load_json(summary_file))
            return
        if parsed.path.startswith("/api/artifact"):
            query = parse_qs(parsed.query)
            run_id = query.get("run", [""])[0]
            rel = query.get("path", [""])[0]
            run_path = safe_run_path(run_id)
            if run_path is None or not rel:
                self._send_json({"error": "artifact not found"}, status=HTTPStatus.NOT_FOUND)
                return
            artifact_path = (run_path / rel).resolve()
            try:
                artifact_path.relative_to(run_path)
            except ValueError:
                self._send_json({"error": "artifact not found"}, status=HTTPStatus.NOT_FOUND)
                return
            if not artifact_path.exists() or not artifact_path.is_file():
                self._send_json({"error": "artifact not found"}, status=HTTPStatus.NOT_FOUND)
                return
            self._send_text(read_text_if_exists(artifact_path))
            return
        self._send_json({"error": "not found"}, status=HTTPStatus.NOT_FOUND)

    def do_POST(self):
        parsed = urlparse(self.path)
        if parsed.path == "/api/launch":
            data = self._read_json()
            models_file = str(data.get("models_file", "")).strip()
            selected_models = [str(x) for x in data.get("selected_models", []) if str(x).strip()]
            selected_tasks = [str(x) for x in data.get("selected_tasks", []) if str(x).strip()]
            if not models_file:
                self._send_json({"error": "models_file is required"}, status=HTTPStatus.BAD_REQUEST)
                return
            if models_file not in available_model_files():
                self._send_json({"error": "unknown models_file"}, status=HTTPStatus.BAD_REQUEST)
                return
            job = create_job(models_file, selected_models, selected_tasks)
            self._send_json(job, status=HTTPStatus.CREATED)
            return
        if parsed.path == "/api/restart":
            self._send_json({"restarting": True}, status=HTTPStatus.OK)
            threading.Thread(target=trigger_restart, daemon=True).start()
            return
        self._send_json({"error": "not found"}, status=HTTPStatus.NOT_FOUND)


def trigger_restart():
    global REQUEST_RESTART
    REQUEST_RESTART = True
    if SERVER is not None:
        SERVER.shutdown()


def main() -> int:
    global SERVER
    args = parse_args()
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    SERVER = server
    print(f"Bench UI: http://{args.host}:{args.port}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return RESTART_EXIT_CODE if REQUEST_RESTART else 0


if __name__ == "__main__":
    raise SystemExit(main())
