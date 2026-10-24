"""Local IDE server. The browser is the UI; the compiler stays in this process."""

import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from ..build import analyze, build, run
from ..compiler.widgets import FORM_EVENTS, PROP_TYPES, WIDGETS
from ..project import Project

STATIC = Path(__file__).resolve().parent / "static"
REPO = Path(__file__).resolve().parents[2]
TYPES = {
    ".html": "text/html; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".json": "application/json; charset=utf-8",
    ".svg": "image/svg+xml",
}


class Studio:
    def __init__(self, root=None):
        self.root = str(Path(root).resolve()) if root else None


def _examples():
    root = REPO / "examples"
    found = []
    if root.is_dir():
        for path in sorted(root.iterdir()):
            if (path / "project.json").is_file():
                found.append({"name": path.name, "path": str(path)})
    return found


def _resolve_project(path):
    candidate = Path(path).expanduser()
    if not candidate.is_absolute():
        candidate = Path.cwd() / candidate
    candidate = candidate.resolve()
    if not (candidate / "project.json").is_file():
        raise FileNotFoundError(f"no project.json in {candidate}")
    return candidate


class Handler(BaseHTTPRequestHandler):
    studio = None

    def log_message(self, fmt, *args):
        return

    def do_GET(self):
        try:
            self._get()
        except Exception as exc:
            self._send(400, {"error": str(exc)})

    def do_POST(self):
        try:
            self._post()
        except Exception as exc:
            self._send(400, {"error": str(exc)})

    def do_PUT(self):
        try:
            self._put()
        except Exception as exc:
            self._send(400, {"error": str(exc)})

    def _get(self):
        parsed = urlparse(self.path)
        if parsed.path in ("/", "/index.html"):
            self._file(STATIC / "index.html")
            return
        if parsed.path.startswith("/static/"):
            rel = parsed.path[len("/static/"):]
            path = (STATIC / rel).resolve()
            if STATIC not in path.parents or not path.is_file():
                self._send(404, {"error": "not found"})
                return
            self._file(path)
            return
        if parsed.path == "/api/catalog":
            self._send(200, {"widgets": WIDGETS, "propTypes": PROP_TYPES, "formEvents": FORM_EVENTS})
            return
        if parsed.path == "/api/examples":
            self._send(200, {"examples": _examples()})
            return
        if parsed.path == "/api/project":
            self._send(200, self._project_view())
            return
        if parsed.path == "/api/file":
            query = parse_qs(parsed.query)
            rel = (query.get("path") or [""])[0]
            project = self._project()
            self._send(200, {"path": rel, "text": project.read(rel)})
            return
        self._send(404, {"error": "not found"})

    def _post(self):
        parsed = urlparse(self.path)
        body = self._body()
        if parsed.path == "/api/open":
            path = _resolve_project(body["path"])
            self.studio.root = str(path)
            self._send(200, self._project_view())
            return
        if parsed.path == "/api/new":
            project = Project.create(body["path"], body["name"], body["package"], body.get("title"))
            self.studio.root = str(project.root)
            self._send(200, self._project_view())
            return
        if parsed.path == "/api/add":
            project = self._project()
            kind = body["kind"]
            name = body["name"]
            if kind == "form":
                rel = project.add_form(name, body.get("title"))
            elif kind == "unit":
                rel = project.add_unit(name)
            elif kind == "native":
                rel = project.add_native(name)
            else:
                raise ValueError(f"unknown kind {kind}")
            view = self._project_view()
            view["created"] = rel
            self._send(200, view)
            return
        buffers = body.get("buffers") or {}
        if parsed.path == "/api/analyze":
            self._send(200, analyze(self._root(), buffers))
            return
        if parsed.path == "/api/build":
            self._send(200, build(self._root(), buffers))
            return
        if parsed.path == "/api/run":
            self._send(200, run(self._root(), buffers))
            return
        self._send(404, {"error": "not found"})

    def _put(self):
        body = self._body()
        project = self._project()
        project.write(body["path"], body.get("text", ""))
        if body["path"] == "project.json":
            project.meta = json.loads(body.get("text") or "{}")
        self._send(200, {"ok": True, "path": body["path"]})

    def _project(self):
        if not self.studio.root:
            raise FileNotFoundError("no project is open")
        return Project(self.studio.root)

    def _root(self):
        if not self.studio.root:
            raise FileNotFoundError("no project is open")
        return self.studio.root

    def _project_view(self):
        if not self.studio.root:
            return {"open": False, "examples": _examples()}
        project = Project(self.studio.root)
        return {
            "open": True,
            "root": str(project.root),
            "project": project.meta,
            "files": project.list_files(),
            "examples": _examples(),
        }

    def _body(self):
        length = int(self.headers.get("Content-Length") or 0)
        if length > 5_000_000:
            raise ValueError("request is too large")
        raw = self.rfile.read(length) if length else b"{}"
        data = json.loads(raw.decode("utf-8"))
        if not isinstance(data, dict):
            raise ValueError("expected a JSON object")
        return data

    def _boot_payload(self):
        view = self._project_view()
        payload = {
            "catalog": {"widgets": WIDGETS, "propTypes": PROP_TYPES, "formEvents": FORM_EVENTS},
            "view": view,
        }
        if view.get("open"):
            main_name = (view.get("project") or {}).get("mainForm")
            current = next((item for item in view["files"] if item["kind"] == "form" and item["name"] == main_name), None)
            if current is None:
                current = next((item for item in view["files"] if item["kind"] == "form"), None)
            if current:
                project = Project(self.studio.root)
                payload["current"] = current
                payload["source"] = project.read(current["source"])
                try:
                    payload["form"] = json.loads(project.read(current["form"]) or "{}")
                except json.JSONDecodeError:
                    payload["form"] = None
            try:
                checked = analyze(self.studio.root)
                payload["db"] = checked.get("db")
                payload["errors"] = checked.get("errors") or []
            except Exception:
                payload["db"] = None
                payload["errors"] = []
        return payload

    def _file(self, path):
        data = path.read_bytes()
        if path.name == "index.html":
            blob = json.dumps(self._boot_payload()).replace("<", "\\u003c")
            text = data.decode("utf-8").replace(
                "</head>",
                f"<script>window.DROIDE_BOOT={blob};</script></head>",
                1,
            )
            data = text.encode("utf-8")
        kind = TYPES.get(path.suffix, "application/octet-stream")
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def _send(self, status, payload):
        data = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)


class Server(ThreadingHTTPServer):
    allow_reuse_address = True


def serve(host="127.0.0.1", port=8765, root=None):
    studio = Studio(root)
    handler = type("BoundHandler", (Handler,), {"studio": studio})
    return Server((host, port), handler)
