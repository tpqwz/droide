"""Command line for the Droide IDE and compiler.

    python3 -m droide                 start the IDE
    python3 -m droide build PATH      write PATH/build/app.apk
    python3 -m droide run PATH        build, install, and start when a device is attached
    python3 -m droide analyze PATH    print errors and a metric summary
    python3 -m droide new PATH --name NAME --package PKG
"""

import argparse
import json
import os
import sys
from pathlib import Path

from . import __version__
from .build import analyze, build, run
from .ide.server import serve
from .project import Project


def _repo():
    return Path(__file__).resolve().parent.parent


def _default_project():
    hello = _repo() / "examples" / "Hello"
    if (hello / "project.json").is_file():
        return hello
    return None


def _print_result(result, verbose):
    for err in result.get("errors") or []:
        where = err.get("file") or ""
        line = err.get("line") or 1
        print(f"{where}:{line}: {err['message']}")
    if result.get("ok") and result.get("apk"):
        print(f"apk {result['apk']} ({result.get('size', 0)} bytes)")
    if verbose and result.get("verify"):
        print(result["verify"].rstrip())
    if result.get("log"):
        print(result["log"].rstrip())
    return 0 if result.get("ok") else 1


def _studio_binary():
    root = _repo()
    for rel in ("studio/target/release/droide-studio", "studio/target/debug/droide-studio"):
        path = root / rel
        if os.access(path, os.X_OK):
            return str(path)
    return None


def _ensure_qt():
    try:
        import PyQt6  # noqa: F401
        return True
    except ImportError:
        other = "/usr/bin/python3"
        if os.path.isfile(other) and os.path.realpath(other) != os.path.realpath(sys.executable):
            os.execv(other, [other, "-m", "droide", *sys.argv[1:]])
        return False


def cmd_ide(args):
    path = Path(args.path).resolve() if args.path else _default_project()
    if path is not None and not (path / "project.json").is_file():
        print(f"no project.json in {path}", file=sys.stderr)
        return 1
    if not args.web:
        binary = _studio_binary()
        if binary:
            cmd = [binary]
            if path is not None:
                cmd.append(str(path))
            os.execv(binary, cmd)
        if not _ensure_qt():
            print("PyQt6 is not installed. Use --web for the browser IDE.", file=sys.stderr)
            return 1
        from .ide.qtui import run_studio
        return run_studio(path)
    httpd = serve(args.host, args.port, path)
    host, port = httpd.server_address[:2]
    shown = "127.0.0.1" if host in ("0.0.0.0", "") else host
    print(f"Droide {__version__}  http://{shown}:{port}/")
    if path:
        print(f"project {path}")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\nstopped")
    finally:
        httpd.server_close()
    return 0


def cmd_build(args):
    result = build(args.path)
    return _print_result(result, args.verbose)


def cmd_run(args):
    result = run(args.path)
    return _print_result(result, args.verbose)


def cmd_analyze(args):
    result = analyze(args.path)
    if args.json:
        payload = {
            "ok": bool(result.get("ok")),
            "errors": result.get("errors") or [],
            "db": result.get("db") or {"entities": [], "refs": [], "cfgs": {}},
        }
        json.dump(payload, sys.stdout, ensure_ascii=False)
        sys.stdout.write("\n")
        return 0 if result.get("ok") else 1
    code = _print_result(result, False)
    rows = []
    for ent in result["db"]["entities"]:
        if ent["kind"] in ("procedure", "function", "native"):
            rows.append(ent)
    rows.sort(key=lambda ent: (-ent.get("complexity", 0), ent["qname"].lower()))
    for ent in rows:
        print(
            f"{ent['kind']:10} {ent['qname']:24} "
            f"loc {ent.get('loc', 0):<4} cc {ent.get('complexity', 0):<3} "
            f"in {ent.get('fanIn', 0):<3} out {ent.get('fanOut', 0)}"
        )
    return code


def cmd_new(args):
    project = Project.create(args.path, args.name, args.package, args.title)
    print(project.root)
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(prog="droide", description="Self-contained RAD IDE for native Android apps")
    parser.add_argument("--version", action="version", version=f"droide {__version__}")
    sub = parser.add_subparsers(dest="cmd")

    ide = sub.add_parser("ide", help="open the IDE")
    ide.add_argument("path", nargs="?", help="project directory")
    ide.add_argument("--web", action="store_true", help="open the browser IDE instead of the Qt window")
    ide.add_argument("--host", default="127.0.0.1")
    ide.add_argument("--port", type=int, default=8765)
    ide.set_defaults(func=cmd_ide)

    built = sub.add_parser("build", help="compile a signed APK")
    built.add_argument("path")
    built.add_argument("-v", "--verbose", action="store_true")
    built.set_defaults(func=cmd_build)

    ran = sub.add_parser("run", help="build and launch on a connected device")
    ran.add_argument("path")
    ran.add_argument("-v", "--verbose", action="store_true")
    ran.set_defaults(func=cmd_run)

    shown = sub.add_parser("analyze", help="typecheck and print the explorer index")
    shown.add_argument("path")
    shown.add_argument("--json", action="store_true", help="print the explorer database as JSON")
    shown.set_defaults(func=cmd_analyze)

    made = sub.add_parser("new", help="create a project")
    made.add_argument("path")
    made.add_argument("--name", required=True)
    made.add_argument("--package", required=True)
    made.add_argument("--title")
    made.set_defaults(func=cmd_new)

    if argv is None:
        argv = sys.argv[1:]
    if not argv:
        argv = ["ide"]
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except Exception as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
