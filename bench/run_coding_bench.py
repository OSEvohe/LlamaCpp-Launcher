import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
import urllib.error
import urllib.request
from datetime import datetime
from pathlib import Path


SYSTEM_PROMPT = (
    "You are a careful coding agent. "
    "Make the smallest correct change. "
    "Follow local code style and existing patterns. "
    "Do not refactor unrelated code. "
    "Return ONLY an OpenCode-style apply_patch patch. "
    "Use *** Begin Patch / *** End Patch and Add/Update/Delete File sections. "
    "If you cannot do that, a unified git diff patch is accepted as fallback. "
    "If you cannot produce a valid patch, return exactly NO_CHANGES."
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the local coding benchmark.")
    parser.add_argument("--models-file", required=True, help="JSON file describing models to test")
    parser.add_argument("--tasks-file", default=str(Path("bench") / "tasks.json"), help="JSON file describing tasks")
    parser.add_argument("--repo", default=None, help="Repository root (defaults to script parent parent)")
    parser.add_argument("--ref", default="HEAD", help="Git ref used for temporary worktrees")
    parser.add_argument("--output-dir", default=None, help="Results directory")
    parser.add_argument("--timeout", type=int, default=1800, help="HTTP timeout in seconds")
    parser.add_argument("--task", action="append", default=[], help="Only run the given task id (repeatable)")
    parser.add_argument("--model", action="append", default=[], help="Only run the given model name (repeatable)")
    parser.add_argument("--keep-worktrees", action="store_true", help="Keep temporary worktrees for inspection")
    parser.add_argument("--dry-run", action="store_true", help="Show selected tasks and models without running")
    return parser.parse_args()


def repo_root_from_args(args: argparse.Namespace) -> Path:
    if args.repo:
        return Path(args.repo).resolve()
    return Path(__file__).resolve().parent.parent


def load_json(path: Path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def select_items(items, key, selected):
    if not selected:
        return items
    wanted = set(selected)
    return [item for item in items if item.get(key) in wanted]


def run_command(command, cwd, stdin_text=None):
    return subprocess.run(
        command,
        cwd=str(cwd),
        input=stdin_text,
        text=True,
        capture_output=True,
        shell=False,
    )


def ensure_git_repo(repo_root: Path):
    result = run_command(["git", "rev-parse", "--show-toplevel"], repo_root)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "not a git repository")


def build_results_dir(repo_root: Path, output_dir_arg: str | None) -> Path:
    if output_dir_arg:
        out = Path(output_dir_arg).resolve()
    else:
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        out = repo_root / "bench" / "results" / stamp
    out.mkdir(parents=True, exist_ok=True)
    return out


def build_chat_url(base_url: str) -> str:
    base = base_url.rstrip("/")
    if base.endswith("/chat/completions"):
        return base
    if base.endswith("/v1"):
        return base + "/chat/completions"
    return base + "/v1/chat/completions"


def create_worktree(repo_root: Path, ref: str, worktree_dir: Path):
    worktree_dir.parent.mkdir(parents=True, exist_ok=True)
    result = run_command(["git", "worktree", "add", "--detach", str(worktree_dir), ref], repo_root)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or "failed to create worktree")


def remove_worktree(repo_root: Path, worktree_dir: Path):
    run_command(["git", "worktree", "remove", "--force", str(worktree_dir)], repo_root)
    if worktree_dir.exists():
        shutil.rmtree(worktree_dir, ignore_errors=True)


def read_context_files(worktree_dir: Path, paths: list[str]) -> str:
    parts = []
    for rel in paths:
        file_path = worktree_dir / rel
        text = file_path.read_text(encoding="utf-8", errors="ignore")
        parts.append(f"=== FILE: {rel} ===\n{text}")
    return "\n\n".join(parts)


def build_user_prompt(task: dict, context_text: str) -> str:
    rels = "\n".join(f"- {path}" for path in task["context_files"])
    return textwrap.dedent(
        f"""
        Repository task: {task['title']}

        Requirements:
        {task['prompt']}

        Verification command:
        {task['verify_command']}

        Files provided:
        {rels}

        Output format:
        - Preferred: return ONLY an OpenCode-style apply_patch patch.
        - Use this format:
          *** Begin Patch
          *** Update File: path
          @@
          -old
          +new
          *** End Patch
        - Or use Add File / Delete File sections when needed.
        - Fallback: a unified `diff --git` patch is accepted.
        - Paths must be repo-relative.
        - If no valid patch can be produced, return exactly NO_CHANGES.

        Context files:

        {context_text}
        """
    ).strip()


def call_model(model_cfg: dict, user_prompt: str, timeout_seconds: int) -> str:
    url = build_chat_url(model_cfg["base_url"])
    payload = {
        "model": model_cfg["model"],
        "temperature": model_cfg.get("temperature", 0.0),
        "max_tokens": model_cfg.get("max_tokens", 8192),
        "messages": [
            {"role": "system", "content": model_cfg.get("system_prompt", SYSTEM_PROMPT)},
            {"role": "user", "content": user_prompt},
        ],
    }
    data = json.dumps(payload).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    api_key = model_cfg.get("api_key", "")
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"
    for key, value in model_cfg.get("headers", {}).items():
        headers[str(key)] = str(value)
    request = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            raw = response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as err:
        body = err.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {err.code}: {body}") from err
    except urllib.error.URLError as err:
        raise RuntimeError(str(err)) from err

    parsed = json.loads(raw)
    choices = parsed.get("choices") or []
    if not choices:
        raise RuntimeError(f"missing choices in response: {raw}")
    message = choices[0].get("message") or {}
    content = message.get("content", "")
    if isinstance(content, list):
        content = "".join(part.get("text", "") for part in content if isinstance(part, dict))
    return str(content)


def extract_patch(response_text: str) -> str:
    text = response_text.strip()
    if text == "NO_CHANGES":
        return ""
    if text.startswith("{"):
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            parsed = None
        if isinstance(parsed, dict):
            diff = parsed.get("diff") or parsed.get("patch") or ""
            text = str(diff).strip()
    fence_match = re.search(r"```(?:diff|patch)?\s*(.*?)```", text, re.DOTALL)
    if fence_match:
        text = fence_match.group(1).strip()
    begin_patch = text.find("*** Begin Patch")
    if begin_patch >= 0:
        end_patch = text.find("*** End Patch", begin_patch)
        if end_patch >= 0:
            end_patch += len("*** End Patch")
            return text[begin_patch:end_patch].strip() + "\n"
    diff_index = text.find("diff --git ")
    if diff_index >= 0:
        return text[diff_index:].strip() + "\n"
    return ""


def split_lines_keepends(text: str) -> list[str]:
    return text.splitlines(keepends=True)


def ensure_trailing_newline(lines: list[str]) -> list[str]:
    if lines and not lines[-1].endswith("\n"):
        lines[-1] += "\n"
    return lines


def parse_apply_patch_sections(patch_text: str) -> list[dict]:
    lines = patch_text.splitlines()
    if not lines or lines[0].strip() != "*** Begin Patch" or lines[-1].strip() != "*** End Patch":
        raise ValueError("invalid apply_patch envelope")

    ops = []
    i = 1
    while i < len(lines) - 1:
        line = lines[i]
        if line.startswith("*** Add File: "):
            path = line[len("*** Add File: "):].strip()
            i += 1
            content = []
            while i < len(lines) - 1 and not lines[i].startswith("*** "):
                if not lines[i].startswith("+"):
                    raise ValueError(f"invalid add line: {lines[i]}")
                content.append(lines[i][1:] + "\n")
                i += 1
            ops.append({"action": "add", "path": path, "content": content})
            continue
        if line.startswith("*** Delete File: "):
            path = line[len("*** Delete File: "):].strip()
            ops.append({"action": "delete", "path": path})
            i += 1
            continue
        if line.startswith("*** Update File: "):
            path = line[len("*** Update File: "):].strip()
            i += 1
            move_to = None
            if i < len(lines) - 1 and lines[i].startswith("*** Move to: "):
                move_to = lines[i][len("*** Move to: "):].strip()
                i += 1
            hunks = []
            current_hunk = None
            while i < len(lines) - 1 and not lines[i].startswith("*** "):
                if lines[i].startswith("@@"):
                    if current_hunk is not None:
                        hunks.append(current_hunk)
                    current_hunk = []
                    i += 1
                    continue
                if current_hunk is None:
                    raise ValueError("update section missing @@ hunk header")
                current_hunk.append(lines[i])
                i += 1
            if current_hunk is not None:
                hunks.append(current_hunk)
            ops.append({"action": "update", "path": path, "move_to": move_to, "hunks": hunks})
            continue
        if not line.strip():
            i += 1
            continue
        raise ValueError(f"unknown apply_patch section: {line}")
    return ops


def find_subsequence(lines: list[str], needle: list[str], start: int) -> int:
    if not needle:
        return start
    limit = len(lines) - len(needle) + 1
    for idx in range(start, max(start, limit)):
        if lines[idx:idx + len(needle)] == needle:
            return idx
    return -1


def leading_context(lines_with_prefix: list[str]) -> list[str]:
    out = []
    for raw in lines_with_prefix:
        if raw.startswith(" "):
            out.append(raw[1:] + "\n")
        else:
            break
    return out


def trailing_context(lines_with_prefix: list[str]) -> list[str]:
    out = []
    for raw in reversed(lines_with_prefix):
        if raw.startswith(" "):
            out.append(raw[1:] + "\n")
        else:
            break
    out.reverse()
    return out


def middle_old_new(lines_with_prefix: list[str], prefix_len: int, suffix_len: int) -> tuple[list[str], list[str]]:
    body = lines_with_prefix[prefix_len:len(lines_with_prefix) - suffix_len if suffix_len else len(lines_with_prefix)]
    old_middle = []
    new_middle = []
    for raw in body:
        if raw == "":
            old_middle.append("\n")
            new_middle.append("\n")
            continue
        marker = raw[0]
        text = raw[1:] + "\n"
        if marker == " ":
            old_middle.append(text)
            new_middle.append(text)
        elif marker == "-":
            old_middle.append(text)
        elif marker == "+":
            new_middle.append(text)
    return old_middle, new_middle


def apply_hunk_by_anchors(lines: list[str], hunk: list[str], cursor: int) -> tuple[list[str], int] | None:
    prefix = leading_context(hunk)
    suffix = trailing_context(hunk)
    old_middle, new_middle = middle_old_new(hunk, len(prefix), len(suffix))

    prefix_pos = find_subsequence(lines, prefix, cursor) if prefix else cursor
    if prefix_pos < 0 and prefix:
        prefix_pos = find_subsequence(lines, prefix, 0)
    if prefix_pos < 0:
        return None

    insert_start = prefix_pos + len(prefix)
    suffix_pos = find_subsequence(lines, suffix, insert_start) if suffix else insert_start
    if suffix_pos < 0 and suffix:
        suffix_pos = find_subsequence(lines, suffix, 0)
    if suffix_pos < 0:
        return None

    candidate_middle = lines[insert_start:suffix_pos]

    if old_middle:
        if candidate_middle == old_middle:
            replacement = prefix + new_middle + suffix
            span_start = prefix_pos
            span_end = suffix_pos + len(suffix)
            new_lines = lines[:span_start] + replacement + lines[span_end:]
            return new_lines, span_start + len(replacement)
        return None

    replacement = prefix + new_middle + suffix
    span_start = prefix_pos
    span_end = suffix_pos + len(suffix)
    new_lines = lines[:span_start] + replacement + lines[span_end:]
    return new_lines, span_start + len(replacement)


def apply_update_hunks_to_text(original_text: str, hunks: list[list[str]]) -> str:
    lines = split_lines_keepends(original_text)
    cursor = 0
    for hunk in hunks:
        old_lines = []
        new_lines = []
        for raw in hunk:
            if raw == "":
                old_lines.append("\n")
                new_lines.append("\n")
                continue
            prefix = raw[0]
            body = raw[1:]
            if prefix == " ":
                old_lines.append(body + "\n")
                new_lines.append(body + "\n")
            elif prefix == "-":
                old_lines.append(body + "\n")
            elif prefix == "+":
                new_lines.append(body + "\n")
            else:
                raise ValueError(f"invalid hunk line prefix: {raw}")
        old_lines = ensure_trailing_newline(old_lines)
        new_lines = ensure_trailing_newline(new_lines)
        pos = find_subsequence(lines, old_lines, cursor)
        if pos < 0:
            pos = find_subsequence(lines, old_lines, 0)
        if pos >= 0:
            lines = lines[:pos] + new_lines + lines[pos + len(old_lines):]
            cursor = pos + len(new_lines)
            continue

        anchored = apply_hunk_by_anchors(lines, hunk, cursor)
        if anchored is None:
            raise ValueError("could not match update hunk in file")
        lines, cursor = anchored
    return "".join(lines)


def apply_opencode_patch(worktree_dir: Path, patch_text: str):
    ops = parse_apply_patch_sections(patch_text)
    for op in ops:
        path = worktree_dir / op["path"]
        if op["action"] == "add":
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("".join(op["content"]), encoding="utf-8")
            continue
        if op["action"] == "delete":
            if path.exists():
                path.unlink()
            continue
        if op["action"] == "update":
            if not path.exists():
                raise ValueError(f"update target missing: {op['path']}")
            original = path.read_text(encoding="utf-8", errors="ignore")
            updated = apply_update_hunks_to_text(original, op["hunks"])
            path.write_text(updated, encoding="utf-8")
            move_to = op.get("move_to")
            if move_to:
                target = worktree_dir / move_to
                target.parent.mkdir(parents=True, exist_ok=True)
                path.replace(target)
            continue
    return True, "apply_patch applied"


def apply_patch(worktree_dir: Path, patch_text: str):
    if not patch_text.strip():
        return False, "empty patch"
    if patch_text.lstrip().startswith("*** Begin Patch"):
        try:
            return apply_opencode_patch(worktree_dir, patch_text)
        except Exception as exc:  # noqa: BLE001
            return False, str(exc)
    result = run_command(["git", "apply", "--whitespace=nowarn", "-"], worktree_dir, stdin_text=patch_text)
    log = (result.stdout or "") + (result.stderr or "")
    return result.returncode == 0, log.strip()


def run_verify(worktree_dir: Path, command: str):
    result = subprocess.run(
        ["powershell", "-NoProfile", "-Command", command],
        cwd=str(worktree_dir),
        text=True,
        capture_output=True,
    )
    log = (result.stdout or "") + (result.stderr or "")
    return result.returncode == 0, log.strip()


def diff_stat(worktree_dir: Path) -> str:
    result = run_command(["git", "diff", "--stat", "--", "."], worktree_dir)
    return (result.stdout or result.stderr).strip()


def score_run(patch_applied: bool, verify_ok: bool) -> int:
    if patch_applied and verify_ok:
        return 2
    if patch_applied:
        return 1
    return 0


def write_text(path: Path, text: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def sanitize_name(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]+", "-", value).strip("-") or "item"


def run_one(repo_root: Path, results_dir: Path, model_cfg: dict, task: dict, args: argparse.Namespace) -> dict:
    run_name = f"{sanitize_name(model_cfg['name'])}__{sanitize_name(task['id'])}"
    run_dir = results_dir / run_name
    run_dir.mkdir(parents=True, exist_ok=True)

    worktree_parent = results_dir / "_worktrees"
    worktree_dir = worktree_parent / run_name
    create_worktree(repo_root, args.ref, worktree_dir)

    started = time.time()
    api_ok = False
    patch_applied = False
    verify_ok = False
    error = ""

    try:
        context_text = read_context_files(worktree_dir, task["context_files"])
        user_prompt = build_user_prompt(task, context_text)
        write_text(run_dir / "prompt.txt", user_prompt)

        response_text = call_model(model_cfg, user_prompt, args.timeout)
        api_ok = True
        write_text(run_dir / "response.txt", response_text)

        patch_text = extract_patch(response_text)
        write_text(run_dir / "patch.diff", patch_text)

        patch_applied, apply_log = apply_patch(worktree_dir, patch_text)
        write_text(run_dir / "apply.log", apply_log)

        if patch_applied:
            verify_ok, verify_log = run_verify(worktree_dir, task["verify_command"])
        else:
            verify_log = "verification skipped because patch was not applied"
        write_text(run_dir / "verify.log", verify_log)
        write_text(run_dir / "diff.stat.txt", diff_stat(worktree_dir))
    except Exception as exc:  # noqa: BLE001
        error = str(exc)
        write_text(run_dir / "error.txt", error)
    finally:
        if not args.keep_worktrees:
            remove_worktree(repo_root, worktree_dir)

    ended = time.time()
    score = score_run(patch_applied, verify_ok)
    return {
        "model": model_cfg["name"],
        "task": task["id"],
        "title": task["title"],
        "score": score,
        "api_ok": api_ok,
        "patch_applied": patch_applied,
        "verify_ok": verify_ok,
        "seconds": round(ended - started, 2),
        "error": error,
        "artifacts_dir": str(run_dir.relative_to(results_dir)),
    }


def render_summary_md(results: list[dict]) -> str:
    lines = [
        "# Coding Bench Results",
        "",
        "| Model | Task | Score | Verify | Seconds | Error |",
        "|---|---|---:|---|---:|---|",
    ]
    for item in results:
        lines.append(
            f"| {item['model']} | {item['task']} | {item['score']} | "
            f"{'ok' if item['verify_ok'] else 'fail'} | {item['seconds']} | {item['error'].replace('|', '/')} |"
        )

    totals = {}
    for item in results:
        model = item["model"]
        totals.setdefault(model, {"score": 0, "count": 0, "verify_ok": 0})
        totals[model]["score"] += item["score"]
        totals[model]["count"] += 1
        totals[model]["verify_ok"] += 1 if item["verify_ok"] else 0

    lines.extend([
        "",
        "## Totals",
        "",
        "| Model | Total score | Tasks | Verify ok |",
        "|---|---:|---:|---:|",
    ])
    for model, data in sorted(totals.items()):
        lines.append(f"| {model} | {data['score']} | {data['count']} | {data['verify_ok']} |")
    return "\n".join(lines) + "\n"


def main() -> int:
    args = parse_args()
    repo_root = repo_root_from_args(args)
    ensure_git_repo(repo_root)

    models = load_json(Path(args.models_file))
    tasks = load_json(Path(args.tasks_file))
    models = select_items(models, "name", args.model)
    tasks = select_items(tasks, "id", args.task)

    if not models:
        print("No models selected.", file=sys.stderr)
        return 1
    if not tasks:
        print("No tasks selected.", file=sys.stderr)
        return 1

    if args.dry_run:
        print("Models:")
        for model in models:
            print(f"- {model['name']}")
        print("Tasks:")
        for task in tasks:
            print(f"- {task['id']}: {task['title']}")
        return 0

    results_dir = build_results_dir(repo_root, args.output_dir)
    results = []
    for model in models:
        for task in tasks:
            print(f"[run] model={model['name']} task={task['id']}")
            results.append(run_one(repo_root, results_dir, model, task, args))

    summary = {
        "repo": str(repo_root),
        "ref": args.ref,
        "generated_at": datetime.now().isoformat(),
        "results": results,
    }
    write_text(results_dir / "summary.json", json.dumps(summary, indent=2))
    write_text(results_dir / "summary.md", render_summary_md(results))
    print(f"Results written to: {results_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
