"""Droide project layout on disk.

project.json
forms/<Name>.form.json
forms/<Name>.droid
units/<Name>.droid
native/<Name>.nc
"""

import json
import re
from pathlib import Path

from .compiler.errors import CompileError
from .compiler.parser import parse_droid, parse_native
from .compiler.widgets import blank_form, new_component

IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*$")
JAVA_WORDS = {
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char",
    "class", "const", "continue", "default", "do", "double", "else", "enum",
    "extends", "final", "finally", "float", "for", "goto", "if", "implements",
    "import", "instanceof", "int", "interface", "long", "native", "new",
    "package", "private", "protected", "public", "return", "short", "static",
    "strictfp", "super", "switch", "synchronized", "this", "throw", "throws",
    "transient", "try", "void", "volatile", "while", "true", "false", "null",
}
RESERVED_CLASSES = {"droidert", "native"}


def valid_ident(name):
    return bool(name) and bool(IDENT.match(name)) and name.lower() not in JAVA_WORDS


def valid_package(package):
    parts = str(package).split(".")
    return bool(parts) and all(valid_ident(part) for part in parts)


def class_ok(name):
    return valid_ident(name) and name.lower() not in RESERVED_CLASSES


class Project:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.meta = json.loads(self.read("project.json") or "{}")

    @staticmethod
    def create(root, name, package, title=None):
        root = Path(root)
        if (root / "project.json").exists():
            raise FileExistsError(f"{root} already has a project.json")
        if not class_ok(name):
            raise ValueError(f"bad project name {name}")
        if not valid_package(package):
            raise ValueError(f"bad package {package}")
        title = title or name
        root.mkdir(parents=True, exist_ok=True)
        (root / "forms").mkdir(exist_ok=True)
        (root / "units").mkdir(exist_ok=True)
        (root / "native").mkdir(exist_ok=True)
        meta = {
            "name": name,
            "package": package,
            "title": title,
            "versionCode": 1,
            "versionName": "1.0",
            "mainForm": "Main",
            "minSdk": 24,
            "targetSdk": 28,
            "permissions": [],
        }
        (root / "project.json").write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
        form = blank_form("Main", title)
        label = new_component("Label", "TitleLabel", 16, 24)
        label["props"]["Text"] = title
        label["props"]["TextSize"] = 22
        label["width"] = 328
        button = new_component("Button", "OkButton", 16, 88)
        button["props"]["Text"] = "Click"
        button["events"]["OnClick"] = "OkClick"
        form["components"] = [label, button]
        (root / "forms" / "Main.form.json").write_text(json.dumps(form, indent=2) + "\n", encoding="utf-8")
        (root / "forms" / "Main.droid").write_text(
            "procedure OkClick(Sender: View);\nbegin\n  Toast('Hello from Droide')\nend;\n",
            encoding="utf-8",
        )
        (root / "units" / "Utils.droid").write_text(
            "unit Utils;\n\nfunction Clamp(Value, Lo, Hi: Integer): Integer;\nbegin\n"
            "  if Value < Lo then\n    Result := Lo\n  else if Value > Hi then\n"
            "    Result := Hi\n  else\n    Result := Value\nend;\n",
            encoding="utf-8",
        )
        (root / "native" / "app.nc").write_text(
            "fun echo(n: i32): i32 {\n  return n;\n}\n",
            encoding="utf-8",
        )
        return Project(root)

    def read(self, rel):
        path = self._path(rel)
        if not path.exists():
            return ""
        return path.read_text(encoding="utf-8")

    def write(self, rel, text):
        path = self._path(rel)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        if rel == "project.json":
            self.meta = json.loads(text or "{}")

    def _path(self, rel):
        rel = str(rel).replace("\\", "/").lstrip("/")
        parts = Path(rel).parts
        if not rel or any(part in ("", ".", "..") for part in parts):
            raise ValueError(f"bad project path {rel}")
        path = (self.root / rel).resolve()
        if self.root != path and self.root not in path.parents:
            raise ValueError(f"bad project path {rel}")
        return path

    def text(self, rel, buffers):
        if buffers and rel in buffers:
            return buffers[rel]
        return self.read(rel)

    def list_files(self, buffers=None):
        buffers = buffers or {}
        found = {}

        def add(info):
            found[info["source"]] = info

        forms_dir = self.root / "forms"
        if forms_dir.is_dir():
            for path in sorted(forms_dir.glob("*.form.json")):
                name = path.name[: -len(".form.json")]
                add({
                    "kind": "form",
                    "name": name,
                    "source": f"forms/{name}.droid",
                    "form": f"forms/{name}.form.json",
                })
        for rel in buffers:
            if rel.startswith("forms/") and rel.endswith(".form.json"):
                name = rel[len("forms/"):-len(".form.json")]
                add({
                    "kind": "form",
                    "name": name,
                    "source": f"forms/{name}.droid",
                    "form": rel,
                })
        units_dir = self.root / "units"
        if units_dir.is_dir():
            for path in sorted(units_dir.glob("*.droid")):
                add({"kind": "unit", "name": path.stem, "source": f"units/{path.name}"})
        for rel in buffers:
            if rel.startswith("units/") and rel.endswith(".droid"):
                add({"kind": "unit", "name": Path(rel).stem, "source": rel})
        native_dir = self.root / "native"
        if native_dir.is_dir():
            for path in sorted(native_dir.glob("*.nc")):
                add({"kind": "native", "name": path.stem, "source": f"native/{path.name}"})
        for rel in buffers:
            if rel.startswith("native/") and rel.endswith(".nc"):
                add({"kind": "native", "name": Path(rel).stem, "source": rel})
        files = list(found.values())
        files.sort(key=lambda item: ({"form": 0, "unit": 1, "native": 2}.get(item["kind"], 9), item["name"].lower()))
        files.append({"kind": "project", "name": "project.json", "source": "project.json"})
        return files

    def add_form(self, name, title=None):
        if not class_ok(name):
            raise ValueError(f"bad form name {name}")
        if (self.root / "forms" / f"{name}.form.json").exists():
            raise FileExistsError(f"form {name} already exists")
        form = blank_form(name, title or name)
        self.write(f"forms/{name}.form.json", json.dumps(form, indent=2) + "\n")
        self.write(f"forms/{name}.droid", "")
        return f"forms/{name}.droid"

    def add_unit(self, name):
        if not class_ok(name):
            raise ValueError(f"bad unit name {name}")
        rel = f"units/{name}.droid"
        if (self.root / rel).exists():
            raise FileExistsError(f"unit {name} already exists")
        self.write(rel, f"unit {name};\n")
        return rel

    def add_native(self, name):
        if not valid_ident(name):
            raise ValueError(f"bad native file name {name}")
        rel = f"native/{name}.nc"
        if (self.root / rel).exists():
            raise FileExistsError(f"native file {name} already exists")
        self.write(rel, f"// {name}\n")
        return rel

    def model(self, buffers=None):
        buffers = dict(buffers or {})
        errors = []
        meta_text = self.text("project.json", buffers)
        try:
            meta = json.loads(meta_text or "{}")
        except json.JSONDecodeError as exc:
            errors.append({"message": f"bad project.json: {exc}", "line": 1, "col": 1, "file": "project.json"})
            meta = dict(self.meta)
        forms = {}
        units = {}
        natives = {}
        for info in self.list_files(buffers):
            if info["kind"] == "form":
                raw = self.text(info["form"], buffers)
                try:
                    form = json.loads(raw or "{}")
                except json.JSONDecodeError as exc:
                    errors.append({
                        "message": f"bad form json: {exc}",
                        "line": 1, "col": 1, "file": info["form"],
                    })
                    continue
                if form.get("name") != info["name"]:
                    errors.append({
                        "message": f"form name {form.get('name')!r} does not match {info['name']}",
                        "line": 1, "col": 1, "file": info["form"],
                    })
                src = self.text(info["source"], buffers)
                try:
                    program = parse_droid(src, info["source"])
                except CompileError as exc:
                    errors.append(exc.to_dict())
                    program = parse_droid("", info["source"])
                forms[info["name"]] = {"program": program, "form": form, "file": info["source"]}
            elif info["kind"] == "unit":
                src = self.text(info["source"], buffers)
                try:
                    program = parse_droid(src, info["source"])
                except CompileError as exc:
                    errors.append(exc.to_dict())
                    continue
                units[info["name"]] = {"program": program, "file": info["source"]}
            elif info["kind"] == "native":
                src = self.text(info["source"], buffers)
                try:
                    program = parse_native(src, info["source"])
                except CompileError as exc:
                    errors.append(exc.to_dict())
                    continue
                natives[info["source"]] = {"program": program, "file": info["source"]}
        return meta, {"forms": forms, "units": units, "natives": natives}, errors
