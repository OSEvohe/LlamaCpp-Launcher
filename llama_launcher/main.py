"""Entry point for the LLama Launcher TUI."""
import argparse
import logging
import sys
import threading
from typing import Optional

logger = logging.getLogger(__name__)


def _resolve_api_settings(
    cli_host: Optional[str],
    cli_port: Optional[int],
    headless: bool,
) -> tuple[str, int]:
    """Resolve API host/port from CLI overrides and persisted global settings."""
    from llama_launcher.api import LlamaLauncherService

    service = LlamaLauncherService()
    settings = service.load_global()

    host = cli_host or settings.api_host or "127.0.0.1"
    raw_port = cli_port if cli_port is not None else settings.api_port
    try:
        port = int(raw_port)
    except (TypeError, ValueError):
        port = 0

    if port < 0 or port > 65535:
        port = 0

    # In headless mode, default to 7890 when no port is configured
    if headless and port <= 0:
        port = 7890

    return host, port


def _start_api_sidecar(host: str, port: int) -> None:
    """Start the API server in a daemon thread (sidecar mode)."""
    from llama_launcher.api import LlamaLauncherService
    from llama_launcher.server import create_api_server

    service = LlamaLauncherService()
    server = create_api_server(service, host, port)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    logger.info("API sidecar started on %s:%s", host, port)


def _run_api_headless(host: str, port: int) -> None:
    """Start the API server and block (headless mode)."""
    from llama_launcher.api import LlamaLauncherService
    from llama_launcher.server import run_api_server

    service = LlamaLauncherService()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    logger.info("Starting LLama Launcher API server on %s:%s", host, port)
    run_api_server(service, host, port)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="LLama Launcher - GUI and API server for llama.cpp",
    )
    parser.add_argument(
        "--headless",
        action="store_true",
        help="Run API server only (no TUI)",
    )
    parser.add_argument(
        "--api-host",
        type=str,
        default=None,
        help="API server bind address (default: from settings or 127.0.0.1)",
    )
    parser.add_argument(
        "--api-port",
        type=int,
        default=None,
        help="API server bind port (default: from settings, or 7890 in headless mode)",
    )
    args = parser.parse_args()

    host, port = _resolve_api_settings(args.api_host, args.api_port, args.headless)

    if args.headless:
        try:
            _run_api_headless(host, port)
        except OSError as exc:
            print(f"Error: failed to bind API server on {host}:{port} — {exc}")
            sys.exit(1)
    else:
        # Non-headless (TUI) mode
        if port > 0:
            try:
                _start_api_sidecar(host, port)
            except OSError as exc:
                logger.warning(
                    "API sidecar failed to start (%s); continuing without API", exc
                )
        from llama_launcher.ui.app import LlamaLauncherApp

        LlamaLauncherApp().run()


if __name__ == "__main__":
    main()
