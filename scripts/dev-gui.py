#!/usr/bin/env python3
"""Start a fresh, seeded Quill server in Docker and the desktop GUI with it.

For trying the GUI by hand. Each run:

  1. Throws away the previous dev server's database and starts a new one,
     with the server, in Docker (its own Compose project, so the database of
     a plain `docker compose up` is left alone).
  2. Fills it with demo data (`quill-seed`): an `admin` account and a few
     writers, all sharing one password, with personas and prompts.
  3. Optionally sets server rules, and when accounts must accept them.
  4. Starts the GUI with an empty settings folder, so it opens on first run.
     It's built while the server starts.
  5. When the GUI window closes, stops the server and deletes its database.

Needs Docker (with Compose) and Rust, as for building Quill. Uses only
Python's standard library, so it runs the same on Windows, macOS and Linux:

    python scripts/dev-gui.py                    # the usual run
    python scripts/dev-gui.py --rules on-join    # sign-up asks to accept rules
    python scripts/dev-gui.py --reuse-gui        # keep the GUI's settings and stores
    python scripts/dev-gui.py --no-gui           # just the server; --stop ends it
    python scripts/dev-gui.py --help             # everything else
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parent.parent
# A Compose project of its own: its own database volume and containers.
PROJECT = "quill-gui-dev"
DEFAULT_PORT = 8090
# quill-seed's defaults (crates/quill-devtools/src/main.rs).
ADMIN = "admin"
DEMO_PASSWORD = "quill demo password"
SAMPLE_RULES = """# Be kind

Write with care, and ask before you borrow someone's idea.

## Keep the board tidy

1. One prompt for one idea.
2. No advertising.

> These are demo rules from `scripts/dev-gui.py`.
"""


def say(text: str) -> None:
    print(f"\n==> {text}", flush=True)


def fail(text: str) -> NoReturn:
    print(f"\nerror: {text}", file=sys.stderr, flush=True)
    sys.exit(1)


def need(tool: str, hint: str) -> None:
    if shutil.which(tool) is None:
        fail(f"`{tool}` isn't on PATH. {hint}")


def compose_env(args: argparse.Namespace) -> dict[str, str]:
    env = dict(os.environ)
    env["QUILL_PORT"] = str(args.port)
    env["QUILL_DOMAIN"] = "localhost"
    env["QUILL_REGISTRATION_OPEN"] = "false" if args.closed else "true"
    return env


def compose(args: argparse.Namespace, *rest: str, check: bool = True) -> int:
    cmd = ["docker", "compose", "-p", PROJECT, "-f", str(ROOT / "compose.yaml"), *rest]
    result = subprocess.run(cmd, cwd=ROOT, env=compose_env(args))
    if check and result.returncode != 0:
        fail(f"`{' '.join(cmd[3:])}` failed (exit {result.returncode}).")
    return result.returncode


def api(port: int, method: str, path: str, body: dict | None = None,
        token: str | None = None) -> dict | None:
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/api/v1{path}", data=data, method=method
    )
    if data is not None:
        req.add_header("content-type", "application/json")
    if token:
        req.add_header("authorization", f"Bearer {token}")
    with urllib.request.urlopen(req, timeout=10) as resp:
        raw = resp.read()
    return json.loads(raw) if raw else None


def wait_for_server(args: argparse.Namespace, timeout: float = 180.0) -> dict:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            info = api(args.port, "GET", "/instance")
            if info:
                return info
        except (urllib.error.URLError, ConnectionError, OSError, ValueError):
            pass
        time.sleep(1)
    compose(args, "logs", "--tail", "40", "server", check=False)
    fail(f"the server didn't answer on port {args.port} within {int(timeout)} s.")


def set_rules(args: argparse.Namespace) -> None:
    text = SAMPLE_RULES
    if args.rules_file:
        text = Path(args.rules_file).read_text(encoding="utf-8")
    token = api(args.port, "POST", "/auth/login",
                {"username": ADMIN, "password": args.password})["token"]
    api(args.port, "PUT", "/admin/rules", {"body": text, "minor": False}, token)
    api(args.port, "PATCH", "/admin/settings", {"rulesAcceptance": args.rules}, token)


def target_dir() -> Path:
    return Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))


def gui_binary(release: bool) -> Path:
    name = "quill-gui.exe" if os.name == "nt" else "quill-gui"
    return target_dir() / ("release" if release else "debug") / name


def start_gui_build(args: argparse.Namespace, log: Path) -> subprocess.Popen:
    cmd = ["cargo", "build", "-p", "quill-gui"] + (["--release"] if args.release else [])
    log.parent.mkdir(parents=True, exist_ok=True)
    out = open(log, "w", encoding="utf-8")
    return subprocess.Popen(cmd, cwd=ROOT, stdout=out, stderr=subprocess.STDOUT)


def finish_gui_build(build: subprocess.Popen, log: Path) -> None:
    if build.wait() != 0:
        tail = log.read_text(encoding="utf-8", errors="replace").splitlines()[-30:]
        print("\n".join(tail), file=sys.stderr)
        fail(f"building the GUI failed; the whole log is in {log}")


def gui_env(args: argparse.Namespace, gui_dir: Path) -> dict[str, str]:
    env = dict(os.environ)
    env["QUILL_GUI_DIR"] = str(gui_dir)
    if args.safe_render:
        # WebKitGTK on some Linux GPUs, VMs and virtual displays draws a
        # blank window unless it renders without GPU buffers.
        env["WEBKIT_DISABLE_DMABUF_RENDERER"] = "1"
        env["WEBKIT_DISABLE_COMPOSITING_MODE"] = "1"
    return env


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="Start a fresh, seeded Quill server in Docker and the desktop GUI with it.",
    )
    p.add_argument("--port", type=int, default=DEFAULT_PORT,
                   help="port for the server on this machine (default: %(default)s)")
    p.add_argument("--stop", action="store_true",
                   help="stop the dev server, delete its database, and exit")
    p.add_argument("--no-gui", action="store_true",
                   help="start only the server and leave it running")
    p.add_argument("--keep-server", action="store_true",
                   help="leave the server running after the GUI closes")
    p.add_argument("--no-seed", action="store_true", help="start with an empty database")
    p.add_argument("--seed", type=int, default=1,
                   help="seed for the demo data; the same seed gives the same content (default: %(default)s)")
    p.add_argument("--accounts", type=int, default=6, help="demo accounts, admin included (default: %(default)s)")
    p.add_argument("--prompts", type=int, default=150, help="demo prompts (default: %(default)s)")
    p.add_argument("--password", default=DEMO_PASSWORD, help="every demo account's password (default: %(default)s)")
    p.add_argument("--closed", action="store_true",
                   help="close registration, to try signing in instead of signing up")
    p.add_argument("--rules", choices=["none", "on-join", "on-change", "both"],
                   help="set server rules, and when accounts must accept them")
    p.add_argument("--rules-file", help="Markdown for the rules (default: a short sample)")
    p.add_argument("--gui-dir", type=Path, default=target_dir() / "dev-gui" / "profile",
                   help="the GUI's settings and stores folder, QUILL_GUI_DIR (default: %(default)s)")
    p.add_argument("--reuse-gui", action="store_true",
                   help="keep the GUI folder from last time instead of starting at first run")
    p.add_argument("--release", action="store_true", help="build the GUI in release mode")
    p.add_argument("--safe-render", action="store_true",
                   help="Linux: render without GPU buffers, if the window comes up blank")
    args = p.parse_args()
    if args.rules and args.no_seed:
        p.error("--rules needs the seeded admin account; drop --no-seed")
    if args.rules_file and not args.rules:
        p.error("--rules-file goes with --rules")
    return args


def main() -> None:
    args = parse_args()
    need("docker", "Install Docker Desktop (Windows, macOS) or Docker Engine (Linux).")

    if args.stop:
        say(f"Stopping the dev server and deleting its database ({PROJECT})")
        compose(args, "--profile", "demo", "down", "-v", "--remove-orphans")
        return

    build = None
    log = target_dir() / "dev-gui" / "build.log"
    if not args.no_gui:
        need("cargo", "Install Rust from https://rustup.rs.")
        say(f"Building the GUI in the background (log: {log})")
        build = start_gui_build(args, log)

    try:
        say("Removing the last dev server and its database")
        compose(args, "--profile", "demo", "down", "-v", "--remove-orphans")
        say(f"Starting a new server on port {args.port} (the first build takes a while)")
        compose(args, "up", "--build", "-d", "db", "server")
        info = wait_for_server(args)
        print(f"    {info.get('software')} {info.get('softwareVersion')} is up.")

        if not args.no_seed:
            say("Filling it with demo data")
            compose(args, "--profile", "demo", "run", "--build", "--rm", "seed",
                    "--seed", str(args.seed), "--accounts", str(args.accounts),
                    "--prompts", str(args.prompts), "--password", args.password)
        if args.rules:
            say(f"Setting server rules (accept: {args.rules})")
            set_rules(args)

        address = f"localhost:{args.port}"
        say("Ready")
        print(f"    Server address for the GUI: {address}")
        print(f"    The board in a browser:     http://{address}/")
        if not args.no_seed:
            print(f"    Demo logins (above) share the password: {args.password}")

        if args.no_gui:
            print("\n    The server keeps running. Stop it with: "
                  "python scripts/dev-gui.py --stop")
            return

        assert build is not None
        say("Waiting for the GUI build")
        finish_gui_build(build, log)
        if not args.reuse_gui and args.gui_dir.exists():
            shutil.rmtree(args.gui_dir)
        args.gui_dir.mkdir(parents=True, exist_ok=True)
        say(f"Starting the GUI (its folder: {args.gui_dir}); close the window to finish")
        subprocess.run([str(gui_binary(args.release))], cwd=ROOT,
                       env=gui_env(args, args.gui_dir))
    except KeyboardInterrupt:
        print("\nInterrupted.")
    finally:
        if build is not None and build.poll() is None:
            build.terminate()
        if not args.no_gui and not args.keep_server:
            say("Stopping the dev server and deleting its database")
            compose(args, "--profile", "demo", "down", "-v", "--remove-orphans",
                    check=False)
        elif not args.no_gui:
            print(f"\nThe server is still on {args.port}; "
                  "stop it with: python scripts/dev-gui.py --stop")


if __name__ == "__main__":
    main()
