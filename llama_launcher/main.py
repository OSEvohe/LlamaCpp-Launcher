"""Entry point for the LLama Launcher API server and web dashboard."""
import argparse
import logging
import sys

logger = logging.getLogger(__name__)


def _resolve_api_settings(
    cli_host: str | None,
    cli_port: int | None,
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

    # Default to 7890 only when no explicit port was configured
    # Explicit --api-port 0 (ephemeral) is preserved; clamped/invalid ports still default.
    if port <= 0 and not (cli_port is not None and cli_port == 0):
        port = 7890

    return host, port


def main() -> None:
    parser = argparse.ArgumentParser(
        description="LLama Launcher — API server and web dashboard for llama.cpp",
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
        help="API server bind port (default: from settings or 7890)",
    )
    args = parser.parse_args()

    host, port = _resolve_api_settings(args.api_host, args.api_port)

    logging.basicConfig(level=logging.INFO, format="%(message)s")
    logger.info("Starting LLama Launcher API server on %s:%s", host, port)

    from llama_launcher.api import LlamaLauncherService
    from llama_launcher.server import run_api_server

    service = LlamaLauncherService()
    try:
        run_api_server(service, host, port)
    except OSError as exc:
        print(f"Error: failed to bind API server on {host}:{port} — {exc}")
        sys.exit(1)


if __name__ == "__main__":
    main()
