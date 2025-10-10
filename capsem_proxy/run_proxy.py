#!/usr/bin/env python3
"""Production runner for CAPSEM Proxy

This script configures and launches the CAPSEM proxy with production-optimized settings.

Usage:
    python run_proxy.py                    # Run with defaults
    python run_proxy.py --host 0.0.0.0     # Bind to all interfaces
    python run_proxy.py --port 8080        # Custom port
    python run_proxy.py --workers 4        # Multiple workers
    python run_proxy.py --reload           # Development mode with auto-reload

Environment Variables:
    PROXY_HOST: Bind host (default: 127.0.0.1)
    PROXY_PORT: Bind port (default: 8000)
    PROXY_WORKERS: Number of workers (default: 1)
    PROXY_RELOAD: Enable auto-reload for development (default: false)
"""

import argparse
import os
import sys
import logging
import uvicorn

# Optional performance dependencies
try:
    import uvloop
    UVLOOP_AVAILABLE = True
except ImportError:
    UVLOOP_AVAILABLE = False

try:
    import httptools
    HTTPTOOLS_AVAILABLE = True
except ImportError:
    HTTPTOOLS_AVAILABLE = False


def parse_args() -> argparse.Namespace:
    """Parse command line arguments"""
    parser = argparse.ArgumentParser(
        description="CAPSEM Proxy - Multi-tenant LLM proxy with security policy enforcement",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter
    )

    parser.add_argument(
        "--host",
        type=str,
        default=os.getenv("PROXY_HOST", "127.0.0.1"),
        help="Bind host address"
    )

    parser.add_argument(
        "--port",
        type=int,
        default=int(os.getenv("PROXY_PORT", "8000")),
        help="Bind port"
    )

    parser.add_argument(
        "--workers",
        type=int,
        default=int(os.getenv("PROXY_WORKERS", "1")),
        help="Number of worker processes (use 1 for Cloud Run/serverless)"
    )

    parser.add_argument(
        "--reload",
        action="store_true",
        default=os.getenv("PROXY_RELOAD", "").lower() in ("true", "1", "yes"),
        help="Enable auto-reload for development (disables workers)"
    )

    parser.add_argument(
        "--log-level",
        type=str,
        default=os.getenv("PROXY_LOG_LEVEL", "info"),
        choices=["critical", "error", "warning", "info", "debug"],
        help="Logging level"
    )

    parser.add_argument(
        "--access-log",
        action="store_true",
        default=os.getenv("PROXY_ACCESS_LOG", "").lower() in ("true", "1", "yes"),
        help="Enable access logging"
    )

    parser.add_argument(
        "--limit-concurrency",
        type=int,
        default=int(os.getenv("PROXY_LIMIT_CONCURRENCY", "0")),
        help="Maximum number of concurrent connections (0 = unlimited)"
    )

    parser.add_argument(
        "--backlog",
        type=int,
        default=int(os.getenv("PROXY_BACKLOG", "2048")),
        help="Maximum number of connections to hold in backlog"
    )

    parser.add_argument(
        "--config-dir",
        type=str,
        default=os.getenv("PROXY_CONFIG_DIR", "config"),
        help="Path to directory containing policy configuration files (TOML)"
    )

    return parser.parse_args()


def setup_logging(log_level: str):
    """Configure logging"""
    logging.basicConfig(
        level=getattr(logging, log_level.upper()),
        format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S"
    )


def get_uvicorn_config(args: argparse.Namespace) -> dict:
    """Build uvicorn configuration from arguments"""
    config = {
        "app": "capsem_proxy.server:app",
        "host": args.host,
        "port": args.port,
        "log_level": args.log_level,
        "access_log": args.access_log,
        "backlog": args.backlog,
        "interface": "asgi3",
    }

    # Use performance libraries if available
    if UVLOOP_AVAILABLE and not args.reload:
        config["loop"] = "uvloop"
    else:
        config["loop"] = "asyncio"

    if HTTPTOOLS_AVAILABLE:
        config["http"] = "httptools"
    else:
        config["http"] = "h11"

    # Development mode with auto-reload
    if args.reload:
        config["reload"] = True
        logging.info("Running in DEVELOPMENT mode with auto-reload enabled")
        # Can't use workers with reload
        if args.workers > 1:
            logging.warning("Auto-reload enabled - ignoring --workers setting")
    else:
        # Production mode with workers
        config["workers"] = args.workers
        if args.limit_concurrency > 0:
            config["limit_concurrency"] = args.limit_concurrency

    return config


def validate_environment():
    """Check that necessary environment is set up"""
    # Check if we can import the app
    try:
        from capsem_proxy.server import app
        logging.info("Successfully imported capsem_proxy.server:app")
    except ImportError as e:
        logging.error(f"Failed to import capsem_proxy.server:app - {e}")
        logging.error("Make sure you're in the correct directory and dependencies are installed")
        sys.exit(1)

    # Check if CAPSEM is available
    try:
        import capsem
        logging.info(f"CAPSEM library available")
    except ImportError:
        logging.error("CAPSEM library not found - install with: uv sync")
        sys.exit(1)


def print_startup_banner(args: argparse.Namespace, config: dict):
    """Print startup information"""
    print("\n" + "="*60)
    print("  CAPSEM PROXY - Multi-tenant LLM Security Proxy")
    print("="*60)
    print(f"  Host:               {args.host}")
    print(f"  Port:               {args.port}")
    print(f"  Workers:            {config.get('workers', 1)}")
    print(f"  Event Loop:         {config['loop']}")
    print(f"  HTTP Parser:        {config['http']}")
    print(f"  Log Level:          {args.log_level.upper()}")
    print(f"  Access Log:         {'enabled' if args.access_log else 'disabled'}")
    if args.limit_concurrency > 0:
        print(f"  Concurrency Limit:  {args.limit_concurrency}")
    print(f"  Backlog:            {args.backlog}")

    # Show enabled policies
    print("\n  Security Policies:")
    if args.config_dir:
        print(f"    Config Dir: {args.config_dir}")
        try:
            from capsem_proxy.capsem_integration import security_manager
            if security_manager and security_manager.policies:
                print(f"    Enabled Policies: {len(security_manager.policies)}")
                for policy in security_manager.policies:
                    print(f"      ✓ {policy}")  # Uses __str__
            else:
                print("    - No policies loaded")
        except Exception:
            print("    - Loading...")
    else:
        print("    Enabled Policies: 1")
        print("      ✓ Debug (default)")

    print("\n  Providers:")
    print("    - OpenAI API Proxy")
    print("    - Gemini API Proxy")
    print("\n  Health Check:")
    print(f"    http://{args.host}:{args.port}/health")
    print("="*60 + "\n")


def main():
    """Main entry point"""
    args = parse_args()
    setup_logging(args.log_level)

    logging.info("Starting CAPSEM Proxy...")

    # Set config directory environment variable before importing server
    # (server imports capsem_integration which reads this env var)
    if args.config_dir:
        os.environ["PROXY_CONFIG_DIR"] = args.config_dir
        logging.info(f"Policy config directory: {args.config_dir}")

    # Validate environment
    validate_environment()

    # Build uvicorn config
    config = get_uvicorn_config(args)

    # Print startup banner
    print_startup_banner(args, config)

    # Performance recommendations
    if not UVLOOP_AVAILABLE and args.log_level == "debug":
        logging.info("For better performance, install uvloop: uv add uvloop")
    if not HTTPTOOLS_AVAILABLE and args.log_level == "debug":
        logging.info("For better performance, install httptools: uv add httptools")

    # Start server
    try:
        uvicorn.run(**config)
    except KeyboardInterrupt:
        logging.info("\nShutting down CAPSEM Proxy...")
    except Exception as e:
        logging.error(f"Failed to start server: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()
