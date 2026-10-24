"""Compile a Droide project to a signed APK and launch it on a device or emulator."""

import os
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path

from .compiler.apk import pack_apk
from .compiler.axml import build_manifest
from .compiler.check import check_model
from .compiler.codegen import compile_dex
from .compiler.dex import check_dex
from .compiler.errors import CompileError
from .compiler.native import compile_library
from .project import Project, class_ok, valid_ident, valid_package


def _tool(name):
    roots = []
    env = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if env:
        roots.append(Path(env))
    roots.append(Path.home() / ".android-sdk")
    for root in roots:
        tools = root / "build-tools"
        if not tools.is_dir():
            continue
        for version in sorted(tools.iterdir(), reverse=True):
            candidate = version / name
            if candidate.is_file():
                return str(candidate)
    return shutil.which(name)


def _adb():
    found = shutil.which("adb")
    if found:
        return found
    candidate = Path.home() / ".android-sdk" / "platform-tools" / "adb"
    if candidate.is_file():
        return str(candidate)
    return None


def _run(cmd):
    proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return proc.returncode, proc.stdout


def _online(text):
    serials = []
    for line in text.splitlines():
        if "\tdevice" not in line:
            continue
        serial = line.split("\t", 1)[0].strip()
        if serial:
            serials.append(serial)
    return serials


def _emulator():
    roots = []
    env = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    if env:
        roots.append(Path(env))
    roots.append(Path.home() / ".android-sdk")
    for root in roots:
        candidate = root / "emulator" / "emulator"
        if candidate.is_file():
            return str(candidate)
    return shutil.which("emulator")


def _avds(emulator):
    code, text = _run([emulator, "-list-avds"])
    if code != 0:
        return []
    names = []
    for line in text.splitlines():
        name = line.strip()
        if not name or " " in name or name.startswith(("INFO", "ERROR", "WARNING")):
            continue
        names.append(name)
    return names


def _emulator_args(emulator, avd, window):
    cmd = [emulator, "-avd", avd, "-no-audio", "-no-boot-anim"]
    if not window:
        cmd += ["-no-window", "-gpu", "swiftshader_indirect"]
    return cmd


def _booted(adb, serial):
    code, out = _run([adb, "-s", serial, "shell", "getprop", "sys.boot_completed"])
    return code == 0 and out.strip().replace("\r", "") == "1"


def _start_and_wait(adb, emulator, avd, window, timeout=150):
    cmd = _emulator_args(emulator, avd, window)
    log_path = Path(tempfile.gettempdir()) / "droide-emulator.log"
    note = f"starting emulator {avd}\n"
    with open(log_path, "ab") as logf:
        proc = subprocess.Popen(cmd, stdout=logf, stderr=subprocess.STDOUT, start_new_session=True)
    deadline = time.time() + timeout
    while time.time() < deadline:
        if proc.poll() is not None:
            tail = ""
            try:
                tail = log_path.read_text(errors="replace")[-1200:]
            except OSError:
                pass
            return None, note + f"emulator exited ({proc.returncode})\n" + tail
        code, text = _run([adb, "devices"])
        for serial in _online(text) if code == 0 else []:
            if _booted(adb, serial):
                return serial, note
        time.sleep(2)
    if proc.poll() is None:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except OSError:
            pass
    return None, note + "emulator did not finish booting\n"


def _ensure_serial(adb):
    code, text = _run([adb, "devices"])
    online = _online(text) if code == 0 else []
    if online:
        serial = online[0]
        if _booted(adb, serial) or _wait_boot(adb, serial, 60):
            return serial, ""
        return None, f"{serial} did not finish booting\n"
    emulator = _emulator()
    if not emulator:
        return None, f"no Android device is attached, and the emulator was not found.\n{text}"
    avds = _avds(emulator)
    if not avds:
        return None, f"no Android device is attached, and no emulator image is installed.\n{text}"
    window = bool(os.environ.get("DISPLAY"))
    serial, note = _start_and_wait(adb, emulator, avds[0], window)
    if serial is None and window:
        serial, again = _start_and_wait(adb, emulator, avds[0], False)
        note += "the window did not open, so Droide started it headless.\n" + again
    return serial, note


def _wait_boot(adb, serial, timeout):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if _booted(adb, serial):
            return True
        time.sleep(2)
    return False


def _launch_failure(adb, serial, package):
    time.sleep(1)
    _code, fatal = _run([adb, "-s", serial, "logcat", "-d", "-t", "80", "AndroidRuntime:E", "*:S"])
    if "FATAL EXCEPTION" in fatal and package in fatal:
        return fatal.strip()
    return ""


def _err(message, file="", line=1, col=1):
    return {"message": message, "line": line, "col": col, "file": file}


def _structural(meta, checked):
    errors = []
    package = str(meta.get("package") or "")
    if not valid_package(package):
        errors.append(_err(f"bad package {package}", "project.json"))
    main = str(meta.get("mainForm") or "")
    if main not in checked["forms"]:
        errors.append(_err(f"mainForm {main} is not a form", "project.json"))
    seen = {}
    for form in checked["forms"].values():
        if not class_ok(form["name"]):
            errors.append(_err(f"form name {form['name']} cannot be an Android class", form["file"]))
        key = form["name"].lower()
        if key in seen:
            errors.append(_err(f"{form['name']} clashes with {seen[key]}", form["file"]))
        seen[key] = form["name"]
        for widget in form["widgets"].values():
            if not valid_ident(widget["name"]):
                errors.append(_err(f"component name {widget['name']} is not an identifier", form["file"]))
    for unit in checked["units"].values():
        if not class_ok(unit["name"]):
            errors.append(_err(f"unit name {unit['name']} cannot be an Android class", unit["file"]))
        key = unit["name"].lower()
        if key in seen:
            errors.append(_err(f"{unit['name']} clashes with {seen[key]}", unit["file"]))
        seen[key] = unit["name"]
    for native in checked["natives"].values():
        if not valid_ident(native["name"]):
            errors.append(_err(
                f"native function {native['name']} is not an identifier",
                native["file"], native["fun"].line, native["fun"].col,
            ))
    return errors


def compile_project(root, buffers=None, codegen=True):
    """Parse and check. When codegen is set, also emit the DEX, native libraries, and signed APK."""
    project = Project(root)
    meta, model, errors = project.model(buffers)
    empty_db = {"entities": [], "refs": [], "cfgs": {}}
    checked = None
    dex = None
    lib = None
    apk = None
    try:
        checked = check_model(model)
    except CompileError as exc:
        errors.append(exc.to_dict())
    else:
        errors.extend(checked["errors"])
        errors.extend(_structural(meta, checked))
    if codegen and checked is not None and not errors:
        try:
            dex = compile_dex(meta["package"], checked)
            data = dex.build()
            check_dex(data)
            funs = [native["fun"] for native in checked["natives"].values()]
            lib = compile_library(meta["package"], funs) if funs else None
            names = [form["name"] for form in checked["forms"].values()]
            main = meta.get("mainForm")
            if main in names:
                names.remove(main)
                names.insert(0, main)
            manifest = build_manifest(meta, names)
            entries = [("AndroidManifest.xml", manifest), ("classes.dex", data)]
            if lib:
                entries.append(("lib/arm64-v8a/libdroide.so", lib["arm"]))
                entries.append(("lib/x86_64/libdroide.so", lib["x86"]))
            apk = pack_apk(entries)
        except CompileError as exc:
            errors.append(exc.to_dict())
            dex = None
            apk = None
        except Exception as exc:
            errors.append(_err(str(exc)))
            dex = None
            apk = None
    return {
        "project": project,
        "meta": meta,
        "checked": checked,
        "errors": errors,
        "db": checked["db"].to_dict() if checked is not None else empty_db,
        "dex": dex,
        "lib": lib,
        "apk_bytes": apk,
        "files": project.list_files(buffers),
    }


def _public(result):
    listing = ""
    if result["dex"] is not None and hasattr(result["dex"], "listing"):
        listing = "\n".join(result["dex"].listing)
    return {
        "ok": not result["errors"],
        "errors": result["errors"],
        "db": result["db"],
        "project": result["meta"],
        "files": result["files"],
        "root": str(result["project"].root),
        "listing": listing,
    }


def analyze(root, buffers=None):
    result = compile_project(root, buffers, codegen=False)
    # analyze should still return the explorer DB when the checker found errors
    # but codegen did not run. compile_project already does that.
    payload = _public(result)
    payload.pop("listing", None)
    return payload


def build(root, buffers=None):
    result = compile_project(root, buffers)
    payload = _public(result)
    payload["apk"] = None
    payload["size"] = 0
    payload["verify"] = ""
    payload["manifest"] = ""
    if result["apk_bytes"] is None or result["errors"]:
        return payload
    out = result["project"].root / "build"
    out.mkdir(exist_ok=True)
    apk_path = out / "app.apk"
    apk_path.write_bytes(result["apk_bytes"])
    if payload["listing"]:
        (out / "classes.lst").write_text(payload["listing"], encoding="utf-8")
    payload["apk"] = str(apk_path)
    payload["size"] = apk_path.stat().st_size
    signer = _tool("apksigner")
    if signer:
        _code, payload["verify"] = _run([signer, "verify", "--min-sdk-version", "24", "-v", str(apk_path)])
    aapt = _tool("aapt")
    if aapt:
        _code, payload["manifest"] = _run([aapt, "dump", "xmltree", str(apk_path), "AndroidManifest.xml"])
    return payload


def run(root, buffers=None):
    result = build(root, buffers)
    result["launched"] = False
    result["device"] = ""
    result["log"] = ""
    if not result["ok"]:
        result["log"] = "build failed"
        return result
    adb = _adb()
    if not adb:
        result["log"] = f"adb was not found. APK is at {result['apk']}"
        return result
    serial, note = _ensure_serial(adb)
    result["log"] = note
    if not serial:
        result["log"] += f"APK is at {result['apk']}\n"
        return result
    result["device"] = serial
    code, install = _run([adb, "-s", serial, "install", "-r", result["apk"]])
    result["log"] += install
    if code != 0 or "Success" not in install:
        return result
    package = result["project"]["package"]
    main = result["project"]["mainForm"]
    _run([adb, "-s", serial, "logcat", "-c"])
    code, started = _run([adb, "-s", serial, "shell", "am", "start", "-W", "-n", f"{package}/.{main}"])
    result["log"] += started
    crash = _launch_failure(adb, serial, package)
    if crash:
        result["log"] += "\n" + crash + "\n"
        return result
    result["launched"] = code == 0 and "Error" not in started
    return result
