"""Compiler, packer, native, and IDE checks for Droide."""

import contextlib
import ctypes
import io
import json
import subprocess
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

from droide.__main__ import main
from droide.build import _emulator_args, _online, _tool, analyze, build, compile_project
from droide.compiler.native import emulate_aarch64
from droide.compiler.parser import parse_droid, parse_native
from droide.compiler.vm import _len_at, run_static, verify_image
from droide.compiler.widgets import WIDGETS, new_component
from droide.ide.server import serve
from droide.project import Project

ROOT = Path(__file__).resolve().parents[1]
HELLO = ROOT / "examples" / "Hello"
CALC = ROOT / "examples" / "NativeCalc"


def _get(url):
    with urllib.request.urlopen(url) as res:
        return res.status, res.read()


def _json(url, payload=None, method=None):
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, method=method or ("POST" if payload is not None else "GET"))
    if data is not None:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req) as res:
        return json.loads(res.read().decode("utf-8"))


class LanguageTests(unittest.TestCase):
    def test_optional_semicolon_before_else(self):
        program = parse_droid(
            "function F(): Integer;\nbegin\n  if 1 < 2 then\n    Result := 1\n  else\n    Result := 2\nend;\n",
            "F.droid",
        )
        self.assertEqual(program.routines[0].name, "F")
        self.assertIsNotNone(program.routines[0].body[0].else_)

    def test_native_div_and_call(self):
        program = parse_native("fun div(a: i32, b: i32): i32 { return a / b; }\n", "t.nc")
        self.assertEqual(program.funs[0].name, "div")


class ProjectTests(unittest.TestCase):
    def test_hello_executes_and_packs(self):
        compiled = compile_project(HELLO)
        self.assertEqual(compiled["errors"], [])
        dex = compiled["dex"]
        verify_image(dex.image)
        init = dex.image["classes"]["Lcom/droide/hello/Main;"]["methods"][("<init>", "V", ())]
        invoke = init["words"][0]
        self.assertEqual(invoke & 0xFF, 0x70)
        self.assertEqual((invoke >> 12) & 0xF, 1)
        utils = "Lcom/droide/hello/Utils;"
        self.assertEqual(run_static(dex.image, utils, "Clamp", [5, 1, 3]), 3)
        self.assertEqual(run_static(dex.image, utils, "Clamp", [0, 1, 3]), 1)
        self.assertEqual(run_static(dex.image, utils, "SumTo", [10]), 55)
        self.assertEqual(run_static(dex.image, utils, "InRange", [2, 1, 3]), 1)
        self.assertEqual(run_static(dex.image, utils, "InRange", [9, 1, 3]), 0)
        image = zipfile.ZipFile(io.BytesIO(compiled["apk_bytes"]))
        self.assertEqual(image.read("lib/x86_64/libdroide.so"), compiled["lib"]["x86"])
        self.assertEqual(image.read("lib/arm64-v8a/libdroide.so"), compiled["lib"]["arm"])
        with tempfile.TemporaryDirectory() as tmp:
            so = Path(tmp) / "libdroide.so"
            so.write_bytes(compiled["lib"]["x86"])
            lib = ctypes.CDLL(str(so))
            lib.fib.restype = ctypes.c_int32
            lib.fib.argtypes = [ctypes.c_int32]
            self.assertEqual(lib.fib(10), 55)
        got, _steps = emulate_aarch64(compiled["lib"]["arm"], compiled["lib"]["arm_symbols"], "fib", [10])
        self.assertEqual(got, 55)
        packed = build(HELLO)
        self.assertTrue(packed["ok"], packed["errors"])
        self.assertIn("Verified using v2 scheme", packed["verify"])
        self.assertIn("true", packed["verify"])
        self.assertIn("android.intent.action.MAIN", packed["manifest"])
        self.assertIn("android.intent.category.LAUNCHER", packed["manifest"])
        dexdump = _tool("dexdump")
        self.assertTrue(dexdump)
        import subprocess
        dump = subprocess.run([dexdump, "-d", packed["apk"]], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.assertEqual(dump.returncode, 0, dump.stdout[-500:])
        self.assertIn("Clamp", dump.stdout)
        self.assertIn("fib", dump.stdout)
        report = analyze(HELLO)
        names = {ent["qname"] for ent in report["db"]["entities"]}
        self.assertIn("Utils.Clamp", names)
        self.assertIn("native.fib", names)
        clamp = next(ent for ent in report["db"]["entities"] if ent["qname"] == "Utils.Clamp")
        self.assertGreaterEqual(clamp["complexity"], 3)
        self.assertIn(str(clamp["id"]), report["db"]["cfgs"])
        fib = next(ent for ent in report["db"]["entities"] if ent["qname"] == "native.fib")
        callers = [ref for ref in report["db"]["refs"] if ref["dst"] == fib["id"] and ref["kind"] == "call"]
        self.assertTrue(callers)
        fib_cfg = report["db"]["cfgs"][str(fib["id"])]
        blob = " ".join(" ".join(block["text"]) for block in fib_cfg["blocks"])
        self.assertIn("n < 2", blob)

    def test_nativecalc(self):
        compiled = compile_project(CALC)
        self.assertEqual(compiled["errors"], [], compiled["errors"])
        verify_image(compiled["dex"].image)
        with tempfile.TemporaryDirectory() as tmp:
            so = Path(tmp) / "libdroide.so"
            so.write_bytes(compiled["lib"]["x86"])
            lib = ctypes.CDLL(str(so))
            def call(name, args):
                fn = getattr(lib, name)
                fn.restype = ctypes.c_int32
                fn.argtypes = [ctypes.c_int32] * len(args)
                return fn(*args)
            self.assertEqual(call("add", [20, 22]), 42)
            self.assertEqual(call("quo", [20, 3]), 6)
            self.assertEqual(call("rem", [-7, 3]), -1)
            self.assertEqual(call("fib", [10]), 55)
            self.assertEqual(call("sumto", [10]), 55)
            self.assertEqual(call("dist", [3, 10]), 7)
            arm = compiled["lib"]
            for name, args, expect in (("add", [20, 22], 42), ("dist", [3, 10], 7), ("quo", [-7, 3], -2)):
                got, _steps = emulate_aarch64(arm["arm"], arm["arm_symbols"], name, args)
                self.assertEqual(got, expect, name)

    def test_new_project_builds(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Demo"
            Project.create(root, "Demo", "com.example.demo", "Demo")
            result = build(root)
            self.assertTrue(result["ok"], result["errors"])
            self.assertTrue(result["apk"])
            self.assertIn("Verified using v2 scheme", result["verify"])

    def test_reserved_class_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Bad"
            Project.create(root, "Bad", "com.example.bad")
            form = json.loads((root / "forms" / "Main.form.json").read_text())
            form["name"] = "DroideRt"
            (root / "forms" / "DroideRt.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "DroideRt.droid").write_text("", encoding="utf-8")
            meta = json.loads((root / "project.json").read_text())
            meta["mainForm"] = "DroideRt"
            (root / "project.json").write_text(json.dumps(meta), encoding="utf-8")
            (root / "forms" / "Main.form.json").unlink()
            (root / "forms" / "Main.droid").unlink()
            result = analyze(root)
            self.assertFalse(result["ok"])
            self.assertTrue(any("DroideRt" in err["message"] for err in result["errors"]))


class IdeTests(unittest.TestCase):
    def test_api_roundtrip(self):
        httpd = serve("127.0.0.1", 0, HELLO)
        thread = threading.Thread(target=httpd.serve_forever, daemon=True)
        thread.start()
        host, port = httpd.server_address[:2]
        base = f"http://{host}:{port}"
        try:
            status, page = _get(base + "/")
            self.assertEqual(status, 200)
            text = page.decode("utf-8")
            self.assertIn('id="palette"', text)
            self.assertIn('id="inspector"', text)
            self.assertIn('id="explore"', text)
            self.assertNotIn('id="dock"', text)
            self.assertIn("/static/app.js", text)
            _status, script = _get(base + "/static/app.js")
            self.assertIn(b"Call Graph", script)
            self.assertIn(b"ensureHandler", script)
            self.assertIn(b"References", script)
            _status, css = _get(base + "/static/app.css")
            self.assertIn(b"#form", css)
            catalog = _json(base + "/api/catalog")
            self.assertIn("Button", catalog["widgets"])
            self.assertIn("OnClick", catalog["widgets"]["Button"]["events"])
            self.assertIn("Seek", catalog["widgets"])
            self.assertIn("Clock", catalog["widgets"])
            self.assertEqual(catalog["widgets"]["Rating"]["props"][0], "Stars")
            self.assertEqual(catalog["widgets"]["Progress"]["props"][:2], ["Max", "Progress"])
            self.assertIn("Toggle", catalog["widgets"])
            self.assertEqual(catalog["widgets"]["Picker"]["props"][:4], ["MinValue", "MaxValue", "Value", "Wrap"])
            self.assertEqual(catalog["widgets"]["Time"]["props"][:3], ["Is24Hour", "Hour", "Minute"])
            self.assertEqual(catalog["widgets"]["Date"]["props"][:3], ["Year", "Month", "Day"])
            self.assertEqual(catalog["widgets"]["Busy"]["props"], ["Enabled", "Visible"])
            project = _json(base + "/api/project")
            self.assertTrue(project["open"])
            self.assertEqual(project["project"]["mainForm"], "Main")
            names = {item["name"] for item in project["files"]}
            self.assertIn("Main", names)
            self.assertIn("Utils", names)
            try:
                _json(base + "/api/file?path=../secrets")
                self.fail("path traversal should be rejected")
            except urllib.error.HTTPError as exc:
                self.assertEqual(exc.code, 400)
            source = _json(base + "/api/file?path=" + "forms/Main.droid")
            self.assertIn("FibClick", source["text"])
            checked = _json(base + "/api/analyze", {})
            self.assertTrue(checked["ok"], checked["errors"])
            self.assertTrue(any(ent["qname"] == "Utils.Clamp" for ent in checked["db"]["entities"]))
            broken = _json(base + "/api/analyze", {"buffers": {"units/Utils.droid": "unit Utils;\nprocedure Nope\n"}})
            self.assertFalse(broken["ok"])
            disk = (HELLO / "units" / "Utils.droid").read_text(encoding="utf-8")
            self.assertIn("function Clamp", disk)
            built = _json(base + "/api/build", {})
            self.assertTrue(built["ok"], built["errors"])
            self.assertIn("app.apk", built["apk"])
            self.assertIn("invoke-", built["listing"])
        finally:
            httpd.shutdown()
            httpd.server_close()
            thread.join(timeout=3)


class QtStudioTests(unittest.TestCase):
    def test_canvas_uses_qt_widgets(self):
        py = Path("/usr/bin/python3")
        if not py.is_file():
            self.skipTest("system python has no PyQt6")
        script = r"""
import os
os.environ["QT_QPA_PLATFORM"] = "offscreen"
from pathlib import Path
from PyQt6.QtCore import QDate, QTime
from PyQt6.QtWidgets import (
    QApplication, QDateEdit, QLabel, QLineEdit, QPlainTextEdit, QProgressBar, QPushButton,
    QRadioButton, QSlider, QSpinBox, QTimeEdit,
)
from droide.ide.qtui import Studio
app = QApplication([])
win = Studio(Path("examples/Hello"))
names = set(win.live)
for required in ("AboutButton", "FibButton", "NameEdit", "OkButton", "ResultLabel", "TitleLabel"):
    assert required in names, sorted(names)
assert isinstance(win.live["TitleLabel"], QLabel)
assert win.live["TitleLabel"].text() == "Hello"
assert isinstance(win.live["OkButton"], QPushButton)
assert win.live["OkButton"].text() == "Greet"
assert isinstance(win.live["NameEdit"], QLineEdit)
memo = win._make_widget({"type": "Memo", "name": "Memo1", "props": {"Text": "note", "Hint": "Notes", "TextSize": 16}}, True)
assert isinstance(memo, QPlainTextEdit) and memo.toPlainText() == "note"
pw = win._make_widget({"type": "Password", "name": "Pw", "props": {"Text": "secret", "TextSize": 16}}, True)
assert isinstance(pw, QLineEdit) and pw.echoMode() == QLineEdit.EchoMode.Password
radio = win._make_widget({"type": "Radio", "name": "R", "props": {"Text": "A", "Checked": True, "TextSize": 15}}, True)
assert isinstance(radio, QRadioButton) and radio.isChecked()
seek = win._make_widget({"type": "Seek", "name": "S", "props": {"Max": 10, "Progress": 4}}, True)
assert isinstance(seek, QSlider) and seek.maximum() == 10 and seek.value() == 4
bar = win._make_widget({"type": "Progress", "name": "Bar", "props": {"Max": 100, "Progress": 25}}, True)
assert isinstance(bar, QProgressBar) and bar.value() == 25
clock = win._make_widget({"type": "Clock", "name": "Clk", "props": {"Format": "HH:mm", "TextSize": 18}}, True)
assert isinstance(clock, QLabel) and clock.text() == "HH:mm"
toggle = win._make_widget({"type": "Toggle", "name": "Tg", "props": {"TextOn": "On", "TextOff": "Off", "Checked": True, "TextSize": 15}}, True)
assert isinstance(toggle, QPushButton) and toggle.isCheckable() and toggle.isChecked() and toggle.text() == "On"
email = win._make_widget({"type": "Email", "name": "Em", "props": {"Hint": "name@example.com", "Text": "", "TextSize": 16}}, True)
assert isinstance(email, QLineEdit) and email.placeholderText() == "name@example.com"
phone = win._make_widget({"type": "Phone", "name": "Ph", "props": {"Hint": "Phone", "Text": "555", "TextSize": 16}}, True)
assert isinstance(phone, QLineEdit) and phone.text() == "555"
picker = win._make_widget({"type": "Picker", "name": "Pk", "props": {"MinValue": 0, "MaxValue": 10, "Value": 3, "Wrap": True}}, True)
assert isinstance(picker, QSpinBox) and picker.minimum() == 0 and picker.maximum() == 10 and picker.value() == 3 and picker.wrapping()
clock_time = win._make_widget({"type": "Time", "name": "Tm", "props": {"Is24Hour": True, "Hour": 12, "Minute": 0}}, True)
assert isinstance(clock_time, QTimeEdit) and clock_time.time() == QTime(12, 0) and clock_time.displayFormat() == "HH:mm"
when = win._make_widget({"type": "Date", "name": "Dt", "props": {"Year": 2026, "Month": 10, "Day": 24}}, True)
assert isinstance(when, QDateEdit) and when.date() == QDate(2026, 10, 24)
busy = win._make_widget({"type": "Busy", "name": "Bz", "props": {}}, True)
assert isinstance(busy, QProgressBar) and busy.minimum() == 0 and busy.maximum() == 0
preview = win.open_preview()
assert preview.findChild(QLabel, "TitleLabel").text() == "Hello"
win.set_mode("explore")
assert win.graph_nodes() >= 1
print("OK")
"""
        proc = subprocess.run([str(py), "-c", script], cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        self.assertEqual(proc.returncode, 0, proc.stdout)
        self.assertIn("OK", proc.stdout)


class WidgetCatalogTests(unittest.TestCase):
    def test_range_and_input_widgets_compile(self):
        self.assertEqual(WIDGETS["Progress"]["props"][:2], ["Max", "Progress"])
        self.assertEqual(WIDGETS["Rating"]["props"][:2], ["Stars", "Rating"])
        self.assertEqual(WIDGETS["Picker"]["props"][:4], ["MinValue", "MaxValue", "Value", "Wrap"])
        self.assertEqual(WIDGETS["Date"]["defaults"]["props"]["Month"], 10)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Kit"
            Project.create(root, "Kit", "com.example.kit")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            top = 160
            for kind in (
                "Memo", "Number", "Password", "Radio", "Seek", "Progress", "Rating", "Clock",
                "Toggle", "Email", "Phone", "Picker", "Time", "Date", "Busy",
            ):
                form["components"].append(new_component(kind, kind + "1", 16, top))
                top += 8
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            source = (root / "forms" / "Main.droid").read_text(encoding="utf-8")
            source += (
                "\nprocedure ReadKit(Sender: View);\n"
                "var\n  N, M, R, S: Integer;\n  T, F: String;\n"
                "begin\n"
                "  T := Number1.Text;\n"
                "  T := Memo1.Text;\n"
                "  T := Password1.Text;\n"
                "  if Radio1.Checked then\n    N := 1;\n"
                "  N := Seek1.Progress;\n"
                "  N := Progress1.Progress;\n"
                "  M := Progress1.Max;\n"
                "  R := Rating1.Rating;\n"
                "  S := Rating1.Stars;\n"
                "  F := Clock1.Format;\n"
                "  Seek1.Progress := N;\n"
                "  Progress1.Max := M;\n"
                "  Rating1.Rating := R;\n"
                "  Rating1.Stars := S;\n"
                "  Clock1.Format := F;\n"
                "  T := Toggle1.TextOn;\n"
                "  T := Toggle1.TextOff;\n"
                "  if Toggle1.Checked then\n    N := 1;\n"
                "  T := Email1.Text;\n"
                "  T := Phone1.Text;\n"
                "  N := Picker1.MinValue;\n"
                "  N := Picker1.MaxValue;\n"
                "  N := Picker1.Value;\n"
                "  if Picker1.Wrap then\n    N := 1;\n"
                "  N := Time1.Hour;\n"
                "  N := Time1.Minute;\n"
                "  if Time1.Is24Hour then\n    N := 1;\n"
                "  N := Date1.Year;\n"
                "  N := Date1.Month;\n"
                "  N := Date1.Day;\n"
                "  Toggle1.TextOn := T;\n"
                "  Toggle1.Checked := Toggle1.Checked;\n"
                "  Picker1.MaxValue := N;\n"
                "  Picker1.MinValue := N;\n"
                "  Picker1.Value := N;\n"
                "  Picker1.Wrap := Picker1.Wrap;\n"
                "  Time1.Is24Hour := Time1.Is24Hour;\n"
                "  Time1.Hour := N;\n"
                "  Time1.Minute := N;\n"
                "  Date1.Year := N;\n"
                "  Date1.Month := N;\n"
                "  Date1.Day := N;\n"
                "  Log(T);\n"
                "  Log(IntToStr(N + M + R + S));\n"
                "  Log(F)\n"
                "end;\n"
            )
            (root / "forms" / "Main.droid").write_text(source, encoding="utf-8")
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            for needle in (
                "Landroid/widget/SeekBar;",
                "Landroid/widget/ProgressBar;",
                "Landroid/widget/RatingBar;",
                "Landroid/widget/TextClock;",
                "Landroid/widget/RadioButton;",
                "setInputType",
                "setFormat12Hour",
                "setFormat24Hour",
                "setGravity",
                "setStepSize",
                "setIndeterminate",
                "float-to-int",
                "16842872",  # android.R.attr.progressBarStyleHorizontal, 0x01010078
                "8388659",
                "4098",
                "129",
                "Landroid/widget/ToggleButton;",
                "Landroid/widget/NumberPicker;",
                "Landroid/widget/TimePicker;",
                "Landroid/widget/DatePicker;",
                "setTextOn",
                "setTextOff",
                "setWrapSelectorWheel",
                "setIs24HourView",
                "Ljava/lang/Boolean;->valueOf",
                "updateDate",
                "const/16 v4, #33",
                "const/4 v4, #3",
                "const/16 v3, #2026",
                "const/16 v4, #9",
                "const/16 v5, #24",
            ):
                self.assertIn(needle, listing, needle)
            max_at = listing.find("NumberPicker;->setMaxValue")
            min_at = listing.find("NumberPicker;->setMinValue")
            self.assertGreaterEqual(max_at, 0)
            self.assertLess(max_at, min_at)
            self.assertRegex(
                listing,
                r"DatePicker;->getMonth\n\s*[0-9a-f]+  move-result v\d+\n\s*[0-9a-f]+  const/4 v\d+, #1\n\s*[0-9a-f]+  add-int",
            )
            image = compiled["dex"].image
            method = image["classes"]["Lcom/example/kit/Main;"]["methods"][
                ("onCreate", "V", ("Landroid/os/Bundle;",))
            ]
            self.assertGreaterEqual(method["outs"], 4)
            found = False
            found_busy = False
            found_date = False
            pc = 0
            words = method["words"]
            while pc < len(words):
                op = words[pc] & 0xFF
                if op in (0x6E, 0x70):
                    owner, name, _ret, params = image["methods"][words[pc + 1]]
                    count = (words[pc] >> 12) & 0xF
                    if op == 0x70 and owner == "Landroid/widget/ProgressBar;" and name == "<init>":
                        if params == ("Landroid/content/Context;", "Landroid/util/AttributeSet;", "I"):
                            self.assertEqual(count, 4)
                            found = True
                        elif params == ("Landroid/content/Context;",):
                            self.assertEqual(count, 2)
                            found_busy = True
                    if op == 0x6E and owner == "Landroid/widget/DatePicker;" and name == "updateDate":
                        self.assertEqual(count, 4)
                        self.assertEqual(params, ("I", "I", "I"))
                        found_date = True
                pc += _len_at(words, pc)
            self.assertTrue(found)
            self.assertTrue(found_busy)
            self.assertTrue(found_date)


class AnalyzeJsonTests(unittest.TestCase):
    def test_json_includes_fib(self):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = main(["analyze", str(HELLO), "--json"])
        self.assertEqual(code, 0)
        payload = json.loads(buf.getvalue())
        self.assertTrue(payload["ok"])
        self.assertIn("entities", payload["db"])
        self.assertTrue(any(ent["qname"] == "native.fib" for ent in payload["db"]["entities"]))


class LaunchTests(unittest.TestCase):
    def test_online_ignores_offline(self):
        offline = "List of devices attached\nemulator-5554\toffline\n"
        self.assertEqual(_online(offline), [])
        ready = "List of devices attached\nemulator-5554\tdevice product:sdk_gphone64_x86_64\n"
        self.assertEqual(_online(ready), ["emulator-5554"])

    def test_emulator_args(self):
        headless = _emulator_args("/sdk/emulator/emulator", "Pixel", False)
        self.assertEqual(headless[headless.index("-avd") + 1], "Pixel")
        self.assertIn("-no-window", headless)
        shown = _emulator_args("/sdk/emulator/emulator", "Pixel", True)
        self.assertNotIn("-no-window", shown)


if __name__ == "__main__":
    unittest.main()
