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
from droide.project import Project, class_ok

ROOT = Path(__file__).resolve().parents[1]
HELLO = ROOT / "examples" / "Hello"
CALC = ROOT / "examples" / "NativeCalc"
SEARCH = ROOT / "examples" / "Search"
LIBRARY = ROOT / "examples" / "Library"


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

    def test_search_sample_gates_appsearch(self):
        self.assertFalse(class_ok("DroideSearch"))
        self.assertTrue(class_ok("Main"))
        compiled = compile_project(SEARCH)
        self.assertEqual(compiled["errors"], [], compiled["errors"])
        dex = compiled["dex"]
        verify_image(dex.image)
        classes = dex.image["classes"]
        self.assertIn("Lcom/droide/search/DroideSearch;", classes)
        self.assertIn("Lcom/droide/search/DroideSearch$Cb;", classes)
        self.assertIn("Landroid/app/appsearch/BatchResultCallback;", dex.image["types"])
        self.assertIn("app_search", dex.image["strings"])
        form = json.loads((SEARCH / "forms" / "Main.form.json").read_text(encoding="utf-8"))
        hits = next(comp for comp in form["components"] if comp["name"] == "Hits")
        lines = hits["props"]["Items"].split("\n")
        self.assertEqual(len(lines), 5)
        for line in lines:
            self.assertIn(line, dex.image["strings"])
        search = classes["Lcom/droide/search/DroideRt;"]["methods"][
            ("search", "Ljava/lang/String;", ("Landroid/content/Context;", "Ljava/lang/String;"))
        ]
        body = "\n".join(search["listing"])
        self.assertIn("SDK_INT", body)
        self.assertIn('"app_search"', body)
        self.assertIn("DroideSearch;->query", body)
        self.assertNotIn("Landroid/app/appsearch", body)
        entities = compiled["db"]["entities"]
        click = next(ent for ent in entities if ent["qname"] == "Main.FindClick")
        builtin = next(ent for ent in entities if ent["qname"] == "builtin.Search")
        self.assertEqual(click["fanOut"], 1)
        self.assertTrue(any(
            ref["src"] == click["id"] and ref["dst"] == builtin["id"] and ref["kind"] == "call"
            for ref in compiled["db"]["refs"]
        ))
        packed = build(SEARCH)
        self.assertTrue(packed["ok"], packed["errors"])
        self.assertIn("Verified using v2 scheme", packed["verify"])

    def test_library_sample_uses_appsearch_or_fts4(self):
        self.assertFalse(class_ok("DroideIndex"))
        compiled = compile_project(LIBRARY)
        self.assertEqual(compiled["errors"], [], compiled["errors"])
        dex = compiled["dex"]
        verify_image(dex.image)
        classes = dex.image["classes"]
        self.assertIn("Lcom/droide/library/DroideIndex;", classes)
        self.assertIn("Lcom/droide/library/DroideIndex$Cb;", classes)
        self.assertIn("Landroid/app/appsearch/BatchResultCallback;", dex.image["types"])
        self.assertIn("app_search", dex.image["strings"])
        self.assertTrue(any("fts4" in text for text in dex.image["strings"]))
        self.assertIn(
            "Ink is a deep blue mixed on the swatch and kept in a glass jar.",
            dex.image["strings"],
        )
        lookup = classes["Lcom/droide/library/DroideRt;"]["methods"][
            ("lookup", "Ljava/lang/String;", ("Landroid/content/Context;", "Ljava/lang/String;"))
        ]
        body = "\n".join(lookup["listing"])
        self.assertIn("SDK_INT", body)
        self.assertIn('"app_search"', body)
        self.assertIn("DroideIndex;->query", body)
        self.assertNotIn("Landroid/app/appsearch", body)
        rows = classes["Lcom/droide/library/DroideRt;"]["methods"][
            ("rows", "Ljava/lang/String;", ("Landroid/database/sqlite/SQLiteDatabase;", "Ljava/lang/String;"))
        ]
        listed = "\n".join(rows["listing"])
        self.assertIn("offsets(docs)", listed)
        self.assertIn("DroideRt;->snip", listed)
        self.assertNotIn("substr(body", listed)
        snip = classes["Lcom/droide/library/DroideRt;"]["methods"][
            ("snip", "Ljava/lang/String;", ("Ljava/lang/String;", "Ljava/lang/String;", "I"))
        ]
        clip = "\n".join(snip["listing"])
        self.assertIn("utfAt", clip)
        self.assertIn("DroideRt;->span", clip)
        self.assertIn("DroideRt;->pour", listed)
        span = classes["Lcom/droide/library/DroideRt;"]["methods"][
            ("span", "Ljava/lang/String;", ("Ljava/lang/String;", "I", "I"))
        ]
        window = "\n".join(span["listing"])
        self.assertIn('"«"', window)
        line = classes["Lcom/droide/library/DroideIndex;"]["methods"][
            ("line", "Ljava/lang/String;", ("Landroid/app/appsearch/SearchResult;",))
        ]
        drawn = "\n".join(line["listing"])
        self.assertIn("getMatchInfos", drawn)
        self.assertIn("getExactMatchRange", drawn)
        self.assertIn("DroideRt;->span", drawn)
        spec = classes["Lcom/droide/library/DroideIndex;"]["methods"][
            ("spec", "Landroid/app/appsearch/SearchSpec;", ())
        ]
        shaped = "\n".join(spec["listing"])
        self.assertIn("setSnippetCount", shaped)
        self.assertIn("setResultCountPerPage", shaped)
        accept = classes["Lcom/droide/library/DroideIndex$Cb;"]["methods"][
            ("accept", "V", ("Ljava/lang/Object;",))
        ]
        page = "\n".join(accept["listing"])
        self.assertIn("getNextPage", page)
        self.assertTrue(any("offsets(docs)" in text for text in dex.image["strings"]))
        self.assertFalse(any("substr(body" in text for text in dex.image["strings"]))
        entities = compiled["db"]["entities"]
        click = next(ent for ent in entities if ent["qname"] == "Main.FindClick")
        save = next(ent for ent in entities if ent["qname"] == "Add.SaveClick")
        found = next(ent for ent in entities if ent["qname"] == "builtin.Lookup")
        ingest = next(ent for ent in entities if ent["qname"] == "builtin.Ingest")
        self.assertEqual(click["fanOut"], 1)
        self.assertEqual(save["fanOut"], 1)
        refs = compiled["db"]["refs"]
        self.assertTrue(any(
            ref["src"] == click["id"] and ref["dst"] == found["id"] and ref["kind"] == "call"
            for ref in refs
        ))
        self.assertTrue(any(
            ref["src"] == save["id"] and ref["dst"] == ingest["id"] and ref["kind"] == "call"
            for ref in refs
        ))
        packed = build(LIBRARY)
        self.assertTrue(packed["ok"], packed["errors"])
        self.assertIn("Verified using v2 scheme", packed["verify"])

    def test_change_and_long_press_compile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Events"
            Project.create(root, "Events", "com.example.events")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            specs = (
                ("Button", "Press", {"OnClick": "PressClick", "OnLongClick": "PressLong"}),
                ("Edit", "Edit1", {"OnChange": "EditChange"}),
                ("Check", "Check1", {"OnChange": "CheckChange"}),
                ("Seek", "Seek1", {"OnChange": "SeekChange"}),
                ("Rating", "Rating1", {"OnChange": "RatingChange"}),
                ("Picker", "Picker1", {"OnChange": "PickerChange"}),
                ("Time", "Time1", {"OnChange": "TimeChange"}),
                ("Date", "Date1", {"OnChange": "DateChange"}),
                ("List", "Rows", {"OnItemClick": "RowClick", "OnItemLongClick": "RowLong"}),
            )
            form["components"] = []
            top = 16
            for kind, name, events in specs:
                comp = new_component(kind, name, 16, top)
                comp["events"] = events
                form["components"].append(comp)
                top += 8
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure PressClick(Sender: View);\nbegin\nend;\n\n"
                "procedure PressLong(Sender: View);\nbegin\nend;\n\n"
                "procedure EditChange(Text: String);\nbegin\nend;\n\n"
                "procedure CheckChange(On: Boolean);\nbegin\nend;\n\n"
                "procedure SeekChange(Value: Integer);\nbegin\nend;\n\n"
                "procedure RatingChange(Value: Integer);\nbegin\nend;\n\n"
                "procedure PickerChange(Value: Integer);\nbegin\nend;\n\n"
                "procedure TimeChange();\nbegin\nend;\n\n"
                "procedure DateChange();\nbegin\nend;\n\n"
                "procedure RowClick(Row: String);\nbegin\nend;\n\n"
                "procedure RowLong(Row: String);\nbegin\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            for needle in (
                "class Lcom/example/events/Main$Long$Press;",
                "class Lcom/example/events/Main$Change$Edit1;",
                "class Lcom/example/events/Main$Change$Check1;",
                "class Lcom/example/events/Main$Change$Seek1;",
                "class Lcom/example/events/Main$Change$Rating1;",
                "class Lcom/example/events/Main$Change$Picker1;",
                "class Lcom/example/events/Main$Change$Time1;",
                "class Lcom/example/events/Main$Change$Date1;",
                "class Lcom/example/events/Main$ItemLong$Rows;",
                "onLongClick",
                "onItemLongClick",
                "beforeTextChanged",
                "onTextChanged",
                "afterTextChanged",
                "onCheckedChanged",
                "onProgressChanged",
                "onStartTrackingTouch",
                "onStopTrackingTouch",
                "onRatingChanged",
                "float-to-int",
                "onValueChange",
                "setOnValueChangedListener",
                "onTimeChanged",
                "onDateChanged",
                "addTextChangedListener",
                "setOnLongClickListener",
                "setOnItemLongClickListener",
                "DatePicker;->init",
            ):
                self.assertIn(needle, listing, needle)
            self.assertNotIn("updateDate", listing)
            method = compiled["dex"].image["classes"]["Lcom/example/events/Main;"]["methods"][
                ("onCreate", "V", ("Landroid/os/Bundle;",))
            ]
            self.assertGreaterEqual(method["outs"], 5)

    def test_change_handler_rejects_a_bad_parameter(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Bad"
            Project.create(root, "Bad", "com.example.bad")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            edit = new_component("Edit", "Edit1", 16, 16)
            edit["events"] = {"OnChange": "EditChange"}
            time = new_component("Time", "Time1", 16, 80)
            time["events"] = {"OnChange": "TimeChange"}
            form["components"] = [edit, time]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure EditChange(Value: Integer);\nbegin\nend;\n\n"
                "procedure TimeChange(Value: Integer);\nbegin\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            self.assertTrue(any("EditChange" in msg and "String" in msg for msg in messages), messages)
            self.assertTrue(
                any("TimeChange" in msg and "takes no parameters" in msg for msg in messages),
                messages,
            )

    def test_spinner_compiles(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Spin"
            Project.create(root, "Spin", "com.example.spin")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            rows = new_component("List", "Rows", 16, 16)
            rows["props"]["Items"] = "One\nTwo"
            kind = new_component("Spinner", "Kind1", 16, 80)
            kind["props"]["Items"] = "Red\nGreen\nBlue"
            kind["props"]["Selection"] = 2
            kind["events"] = {"OnItemSelected": "Pick"}
            form["components"] = [rows, kind]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Pick(Row: String);\nvar\n  N: Integer;\nbegin\n"
                "  N := Kind1.Selection;\n  Kind1.Items := Row;\n  Log(IntToStr(N))\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            for needle in (
                "class Lcom/example/spin/Main$Select$Kind1;",
                "onItemSelected",
                "onNothingSelected",
                "getItemAtPosition",
                "getSelectedItemPosition",
                "Landroid/widget/AbsSpinner;->setAdapter",
                "Landroid/widget/AbsListView;->setAdapter",
                "Landroid/widget/ArrayAdapter;",
                "const v2, #17367043",
                "Landroid/widget/Spinner;",
            ):
                self.assertIn(needle, listing, needle)
            main = _class_body(listing, "Lcom/example/spin/Main;")
            spin_at = main.index("->setSpinner")
            lines_at = main.index("->setLines")
            sel_at = main.index("AdapterView;->setSelection")
            listen_at = main.index("setOnItemSelectedListener")
            self.assertLess(spin_at, sel_at)
            self.assertLess(sel_at, listen_at)
            self.assertIn("const/4 v4, #2", main)
            self.assertNotEqual(lines_at, spin_at)
            methods = compiled["dex"].image["methods"]
            self.assertIn(
                ("Landroid/widget/AbsSpinner;", "setAdapter", "V", ("Landroid/widget/SpinnerAdapter;",)),
                methods,
            )
            self.assertIn(
                ("Landroid/widget/AbsListView;", "setAdapter", "V", ("Landroid/widget/ListAdapter;",)),
                methods,
            )

    def test_spinner_handler_rejects_a_bad_parameter(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadSpin"
            Project.create(root, "BadSpin", "com.example.badspin")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            kind = new_component("Spinner", "Kind1", 16, 16)
            kind["events"] = {"OnItemSelected": "Pick"}
            form["components"] = [kind]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Pick(On: Boolean);\nbegin\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            self.assertTrue(
                any(
                    "Pick" in msg
                    and "takes no parameters, a String, an Integer, or the row View" in msg
                    for msg in messages
                ),
                messages,
            )

    def test_list_details_compile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Rows"
            Project.create(root, "Rows", "com.example.rows")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            rows = {
                "type": "List",
                "name": "Rows",
                "left": 16,
                "top": 16,
                "width": 328,
                "height": 200,
                "props": {"Items": "Ada\nGrace", "Details": "Lovelace\nHopper", "Color": "#FBF9F4"},
                "events": {"OnItemClick": "Pick"},
            }
            form["components"] = [rows]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Pick(Row: String);\nvar\n  Title: String;\n  Sub: String;\nbegin\n"
                "  Title := Rows.Items\n  Sub := Rows.Details\n"
                "  Rows.Items := Title\n  Rows.Details := Sub\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            main = _class_body(listing, "Lcom/example/rows/Main;")
            self.assertLess(main.index("->setLines"), main.index("->setDetails"))
            pick = _method_body(listing, "Pick(")
            self.assertIn("->itemsOf", pick)
            self.assertIn("->detailsOf", pick)
            self.assertIn("->setLines", pick)
            self.assertIn("->setDetails", pick)
            click = _class_body(listing, "Lcom/example/rows/Main$Item$Rows;")
            self.assertIn("getItemAtPosition", click)
            for needle in (
                "class Lcom/example/rows/DroideRows;",
                "getView",
                "array-length",
                "aget-object",
                "findViewById",
                "#17367043",
                "#17367044",
                "#16908308",
                "#16908309",
                "#2130771969",
                "#2130771970",
                "setTag",
                "getTag",
                "invoke-direct {v1, v2, v3, v0, v4}, Landroid/widget/ArrayAdapter;-><init>",
                "check-cast v2, Landroid/widget/TextView;",
                "setCornerRadius",
                "setDivider",
            ):
                self.assertIn(needle, listing, needle)
            self.assertFalse(class_ok("DroideRows"))
            methods = compiled["dex"].image["methods"]
            self.assertIn(
                (
                    "Lcom/example/rows/DroideRows;",
                    "<init>",
                    "V",
                    ("Landroid/content/Context;", "I", "[Ljava/lang/Object;", "[Ljava/lang/String;"),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Lcom/example/rows/DroideRt;",
                    "setDetails",
                    "V",
                    ("Landroid/widget/ListView;", "Landroid/content/Context;", "Ljava/lang/String;"),
                ),
                methods,
            )
            self.assertIn(
                ("Lcom/example/rows/DroideRows;", "details", "[Ljava/lang/String;"),
                compiled["dex"].image["fields"],
            )

    def test_list_details_reject(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadRows"
            Project.create(root, "BadRows", "com.example.badrows")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = [
                {
                    "type": "List", "name": "Rows",
                    "left": 16, "top": 16, "width": 328, "height": 200,
                    "props": {"Items": "One", "Details": "", "Color": "#FBF9F4"},
                    "events": {},
                },
                {
                    "type": "Spinner", "name": "Kind1",
                    "left": 16, "top": 80, "width": 240, "height": 48,
                    "props": {"Items": "Red", "Selection": 0, "Color": "#FBF9F4"},
                    "events": {},
                },
            ]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show();\nbegin\n  Kind1.Details := 'x'\n  Rows.Details := 1\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            for needle in ("Spinner has no property Details", "Details expects String"):
                self.assertTrue(any(needle in msg for msg in messages), (needle, messages))

    def test_timer_compiles(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Tick"
            Project.create(root, "Tick", "com.example.tick")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = [{
                "type": "Timer",
                "name": "Clock",
                "left": 16,
                "top": 16,
                "width": 160,
                "height": 36,
                "props": {"Interval": 250, "Enabled": True},
                "events": {"OnTimer": "Tick"},
            }]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Tick();\nvar\n  N: Integer;\n  On: Boolean;\nbegin\n"
                "  N := Clock.Interval\n  On := Clock.Enabled\n"
                "  Clock.Interval := 500\n  Clock.Enabled := false\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            main = _class_body(listing, "Lcom/example/tick/Main;")
            self.assertLess(main.index("setInterval"), main.index("setEnabled"))
            self.assertIn("#250", main)
            self.assertEqual(main.count("->addView"), 1, main)
            tick = _method_body(listing, "Tick(")
            self.assertIn("->interval", tick)
            self.assertIn("->enabled", tick)
            self.assertIn("->setInterval", tick)
            self.assertIn("->setEnabled", tick)
            run = _class_body(listing, "Lcom/example/tick/Main$Timer$Clock;")
            self.assertIn("->Tick", run)
            self.assertIn("->arm", run)
            for needle in (
                "class Lcom/example/tick/DroideTick;",
                "getMainLooper",
                "postDelayed",
                "removeCallbacks",
                "int-to-long",
                "iget-boolean",
                "iput-boolean",
            ):
                self.assertIn(needle, listing, needle)
            self.assertFalse(class_ok("DroideTick"))
            methods = compiled["dex"].image["methods"]
            self.assertIn(
                (
                    "Landroid/os/Handler;",
                    "postDelayed",
                    "Z",
                    ("Ljava/lang/Runnable;", "J"),
                ),
                methods,
            )
            self.assertIn(
                "Lcom/example/tick/Main$Timer$Clock;",
                compiled["dex"].image["classes"],
            )

    def test_timer_rejects_a_bad_handler(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadTick"
            Project.create(root, "BadTick", "com.example.badtick")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = [
                {
                    "type": "Timer", "name": "Clock", "parent": "Box",
                    "left": 16, "top": 16, "width": 160, "height": 36,
                    "props": {"Interval": 1000, "Enabled": False},
                    "events": {"OnTimer": "Tick"},
                },
                {
                    "type": "Panel", "name": "Box",
                    "left": 16, "top": 80, "width": 200, "height": 80,
                    "props": {"Direction": "Vertical", "Color": "#E4E8EE"},
                    "events": {},
                },
            ]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Tick(Sender: View);\nbegin\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            for needle in ("Tick takes no parameters", "Clock cannot have a parent"):
                self.assertTrue(any(needle in msg for msg in messages), (needle, messages))

    def test_tlnotes_sample_compiles(self):
        compiled = compile_project(ROOT / "examples" / "TLNotes")
        self.assertEqual(compiled["errors"], [], compiled["errors"])
        verify_image(compiled["dex"].image)
        listing = "\n".join(compiled["dex"].listing)
        for needle in (
            "class Lcom/droide/tlnotes/Main$Timer$Pulse;",
            "->ingest",
            "->lookup",
            "->takePhoto",
            "->openDocument",
            "->setDetails",
            "->setStackFromBottom",
            "->setFillViewport",
            "->today",
            "->time",
            "bottomMargin",
            "Card$Select$KindSpin",
        ):
            self.assertIn(needle, listing, needle)

    def test_desk_sample_compiles(self):
        compiled = compile_project(ROOT / "examples" / "Desk")
        self.assertEqual(compiled["errors"], [], compiled["errors"])
        verify_image(compiled["dex"].image)
        listing = "\n".join(compiled["dex"].listing)
        for needle in (
            "class Lcom/droide/desk/Main$Timer$Pulse;",
            "class Lcom/droide/desk/DroideTick;",
            "postDelayed",
            "->takePhoto",
            "->openDocument",
            "->setDetails",
            "->arg",
            "#17367044",
        ):
            self.assertIn(needle, listing, needle)

    def test_message_and_question_compile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Ask"
            Project.create(root, "Ask", "com.example.ask")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show(Sender: View);\nbegin\n"
                "  Message('Saved')\n"
                "  if IsInt('1') then\n"
                "    Question('Delete this?', 'yesclick')\n"
                "  else begin\n"
                "    Question('Keep it?', 'YesClick')\n"
                "  end\n"
                "end;\n\n"
                "procedure YesClick();\nbegin\n  Log('yes')\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            for needle in (
                "Landroid/app/AlertDialog$Builder;",
                "setMessage",
                "setPositiveButton",
                "setNegativeButton",
                "AlertDialog$Builder;->show",
                "\"OK\"",
                "\"Yes\"",
                "\"No\"",
                "class Lcom/example/ask/Main$Ask$YesClick;",
                "onClick",
                "Main;->YesClick",
            ):
                self.assertIn(needle, listing, needle)
            self.assertEqual(listing.count("class Lcom/example/ask/Main$Ask$YesClick;"), 1)
            methods = compiled["dex"].image["methods"]
            listener = (
                "Ljava/lang/CharSequence;",
                "Landroid/content/DialogInterface$OnClickListener;",
            )
            for name in ("setPositiveButton", "setNegativeButton"):
                self.assertIn(
                    ("Landroid/app/AlertDialog$Builder;", name, "Landroid/app/AlertDialog$Builder;", listener),
                    methods,
                )

    def test_question_rejects_a_bad_procedure(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadAsk"
            Project.create(root, "BadAsk", "com.example.badask")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show();\nvar\n  Name: String;\nbegin\n"
                "  Question('Hi?', Name)\n"
                "  Question('Hi?', 'Missing')\n"
                "  Question('Hi?', 'YesClick')\n"
                "  Message(1)\n"
                "end;\n\n"
                "procedure YesClick(Sender: View);\nbegin\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            for needle in (
                "needs a string literal procedure name",
                "unknown procedure Missing",
                "YesClick takes no parameters",
                "Message expects String",
            ):
                self.assertTrue(any(needle in msg for msg in messages), (needle, messages))

    def test_share_and_clipboard_compile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Clip"
            Project.create(root, "Clip", "com.example.clip")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show(Sender: View);\nvar\n  Text: String;\nbegin\n"
                "  CopyText('Hello')\n  Text := PasteText()\n  Share(Text)\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            for needle in (
                "android.intent.action.SEND",
                "android.intent.extra.TEXT",
                "text/plain",
                "createChooser",
                "queryIntentActivities",
                '"clipboard"',
                "Landroid/content/ClipboardManager;",
                "newPlainText",
                "setPrimaryClip",
                "getPrimaryClip",
                "hasPrimaryClip",
                "getText",
                "->share",
                "->copyText",
                "->pasteText",
            ):
                self.assertIn(needle, listing, needle)
            after = listing[listing.index("android.intent.action.SEND"):]
            query_at = after.index("queryIntentActivities")
            chooser_at = after.index("createChooser")
            start_at = after.index("startActivity")
            self.assertLess(query_at, chooser_at)
            self.assertLess(chooser_at, start_at)
            plain_at = listing.index("newPlainText")
            self.assertIn("setPrimaryClip", listing[plain_at:])
            clip_at = listing.index("getPrimaryClip")
            self.assertIn("getText", listing[clip_at:])
            methods = compiled["dex"].image["methods"]
            self.assertIn(
                (
                    "Landroid/content/Intent;",
                    "createChooser",
                    "Landroid/content/Intent;",
                    ("Landroid/content/Intent;", "Ljava/lang/CharSequence;"),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Landroid/content/Intent;",
                    "putExtra",
                    "Landroid/content/Intent;",
                    ("Ljava/lang/String;", "Ljava/lang/String;"),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Landroid/content/ClipboardManager;",
                    "setPrimaryClip",
                    "V",
                    ("Landroid/content/ClipData;",),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Landroid/content/ClipData$Item;",
                    "getText",
                    "Ljava/lang/CharSequence;",
                    (),
                ),
                methods,
            )

    def test_share_rejects_a_bad_argument(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadClip"
            Project.create(root, "BadClip", "com.example.badclip")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show();\nbegin\n  Share(1)\n  CopyText(1)\n  PasteText('x')\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            for needle in (
                "Share expects String",
                "CopyText expects String",
                "PasteText expects 0 arguments",
            ):
                self.assertTrue(any(needle in msg for msg in messages), (needle, messages))

    def test_activity_results_compile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Shot"
            Project.create(root, "Shot", "com.example.shot")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show();\nbegin\n"
                "  TakePhoto('Got')\n  TakePhoto('Got')\n  OpenDocument('Got')\n"
                "  OpenDocument('Other')\n  TakePhoto('Next')\n  OpenCamera()\nend;\n"
                "procedure Got(Path: String);\nbegin\nend;\n"
                "procedure Other(Path: String);\nbegin\nend;\n"
                "procedure Next(Path: String);\nbegin\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            self.assertNotIn("$Ask$", listing)
            show = _method_body(listing, "Show(")
            self.assertEqual(show.count("->takePhoto"), 3, show)
            self.assertEqual(show.count("->openDocument"), 2, show)
            self.assertEqual(show.count("->openCamera"), 1, show)
            self.assertEqual(show.count("->Got"), 3, show)
            answered = _method_body(listing, "onActivityResult(")
            self.assertEqual(answered.count("->Got"), 2, answered)
            self.assertEqual(answered.count("->Other"), 1, answered)
            self.assertEqual(answered.count("->Next"), 1, answered)
            self.assertIn("->photoPath", answered)
            self.assertIn("->documentPath", answered)
            photo_code = answered.index("const/4 v0, #1")
            got_doc = answered.index("const/16 v0, #256")
            other_doc = answered.index("const/16 v0, #257")
            next_code = answered.index("const/4 v0, #2")
            self.assertLess(photo_code, got_doc)
            self.assertLess(got_doc, other_doc)
            self.assertLess(other_doc, next_code)
            got = answered.index("->Got")
            got_again = answered.index("->Got", got + 4)
            other = answered.index("->Other")
            nxt = answered.index("->Next")
            self.assertLess(got, got_again)
            self.assertLess(got_again, other)
            self.assertLess(other, nxt)
            camera = _method_body(listing, "openCamera(")
            self.assertIn("android.media.action.IMAGE_CAPTURE", camera)
            self.assertIn("->startActivity", camera)
            self.assertNotIn("startActivityForResult", camera)
            take = _method_body(listing, "takePhoto(")
            self.assertIn("android.media.action.IMAGE_CAPTURE", take)
            self.assertIn("startActivityForResult", take)
            self.assertLess(take.index("queryIntentActivities"), take.index("startActivityForResult"))
            document = _method_body(listing, "openDocument(")
            self.assertIn("android.intent.action.OPEN_DOCUMENT", document)
            self.assertIn("android.intent.category.OPENABLE", document)
            self.assertIn("*/*", document)
            self.assertIn("addCategory", document)
            self.assertIn("setType", document)
            self.assertIn("startActivityForResult", document)
            photo = _method_body(listing, "photoPath(")
            self.assertIn("droide-photo.jpg", photo)
            self.assertIn("sget-object", photo)
            self.assertIn("JPEG", photo)
            self.assertIn("getParcelableExtra", photo)
            self.assertIn('"data"', photo)
            compress = photo.index("compress")
            self.assertLess(photo.index("move-result", compress), photo.index("close", compress))
            self.assertIn("#90", photo)
            copied = _method_body(listing, "documentPath(")
            self.assertIn("droide-document", copied)
            self.assertIn("openInputStream", copied)
            self.assertIn("#4096", copied)
            self.assertIn("if-le", copied)
            self.assertLess(copied.index("->read"), copied.index("->write"))
            methods = compiled["dex"].image["methods"]
            self.assertIn(
                (
                    "Landroid/app/Activity;",
                    "startActivityForResult",
                    "V",
                    ("Landroid/content/Intent;", "I"),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Lcom/example/shot/Main;",
                    "onActivityResult",
                    "V",
                    ("I", "I", "Landroid/content/Intent;"),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Landroid/graphics/Bitmap;",
                    "compress",
                    "Z",
                    ("Landroid/graphics/Bitmap$CompressFormat;", "I", "Ljava/io/OutputStream;"),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Landroid/content/ContentResolver;",
                    "openInputStream",
                    "Ljava/io/InputStream;",
                    ("Landroid/net/Uri;",),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Ljava/io/OutputStream;",
                    "write",
                    "V",
                    ("[B", "I", "I"),
                ),
                methods,
            )
            self.assertIn(
                (
                    "Landroid/graphics/Bitmap$CompressFormat;",
                    "JPEG",
                    "Landroid/graphics/Bitmap$CompressFormat;",
                ),
                compiled["dex"].image["fields"],
            )

    def test_activity_results_reject_a_bad_handler(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadShot"
            Project.create(root, "BadShot", "com.example.badshot")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show();\nvar\n  Name: String;\nbegin\n"
                "  TakePhoto(Name)\n  OpenDocument(Name)\n  TakePhoto('Missing')\n"
                "  TakePhoto('None')\n  TakePhoto('Num')\n  TakePhoto('Count')\n"
                "  TakePhoto('Wide')\n  OpenCamera('x')\nend;\n"
                "procedure None();\nbegin\nend;\n"
                "function Num(Path: String): Integer;\nbegin\n  Result := 0\nend;\n"
                "procedure Count(Path: String; Extra: Integer);\nbegin\nend;\n"
                "procedure Wide(Path: Integer);\nbegin\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            for needle in (
                "TakePhoto needs a string literal procedure name",
                "OpenDocument needs a string literal procedure name",
                "unknown procedure Missing",
                "None takes one String",
                "Num must be a procedure",
                "Count takes one String",
                "Wide takes one String",
                "OpenCamera expects 0 arguments",
            ):
                self.assertTrue(any(needle in msg for msg in messages), (needle, messages))
            self.assertFalse(any("Num takes one String" in msg for msg in messages), messages)

    def test_open_form_arguments_compile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Args"
            Project.create(root, "Args", "com.example.pass")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Show();\nvar\n  Name: String;\n  Count: Integer;\n"
                "  Flag: Boolean;\n  Extra: String;\nbegin\n"
                "  OpenForm('Detail')\n  if Flag then\n    OpenForm('Detail', Name)\n"
                "  OpenForm('Detail', 'hello', 7, true)\n"
                "  OpenForm('detail', Name, Count, Flag, Extra)\nend;\n"
                "procedure Plain();\nbegin\n  OpenForm('Detail')\nend;\n",
                encoding="utf-8",
            )
            (root / "forms" / "Detail.form.json").write_text(
                '{"name":"Detail","title":"Detail","background":"#F7F8FA",'
                '"width":360,"height":640,"events":{"OnCreate":"Start"},"components":[]}',
                encoding="utf-8",
            )
            (root / "forms" / "Detail.droid").write_text(
                "procedure Start();\nbegin\n  Toast(Arg(0))\nend;\n"
                "procedure Read();\nvar\n  Title: String;\n  Count: Integer;\n  On: Boolean;\n"
                "begin\n  Title := Arg(0)\n  Count := ArgInt(1)\n  On := ArgBool(2)\nend;\n"
                "procedure Indexed(N: Integer);\nvar\n  Title: String;\nbegin\n"
                "  Title := Arg(N)\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            plain = _method_body(listing, "Plain(")
            self.assertIn("setClass", plain)
            self.assertIn("startActivity", plain)
            self.assertNotIn("putExtra", plain)
            self.assertNotIn('"Detail"', plain)
            show = _method_body(listing, "Show(")
            self.assertLess(show.index("startActivity"), show.index("putExtra"))
            self.assertEqual(show.count("->putExtra"), 16, show)
            self.assertLess(show.index('"a0"'), show.index('"t0"'))
            self.assertLess(show.index('"a3"'), show.index('"t3"'))
            for needle in ('"S"', '"I"', '"B"', '"hello"'):
                self.assertIn(needle, show, needle)
            self.assertIn("pass/Detail;", listing)
            self.assertNotIn("pass/detail;", listing)
            start = _method_body(listing, "Start(")
            self.assertIn("->arg", start)
            read = _method_body(listing, "Read(")
            self.assertIn("->arg", read)
            self.assertIn("->argInt", read)
            self.assertIn("->argBool", read)
            indexed = _method_body(listing, "Indexed(")
            self.assertIn("->arg", indexed)
            self.assertNotIn('"a0"', indexed)
            created = _method_body(listing, "onCreate(")
            self.assertIn("->Start", created)
            arg = _method_body(listing, "arg(")
            self.assertIn("getIntent", arg)
            self.assertIn("getStringExtra", arg)
            self.assertIn("equals", arg)
            self.assertIn('"t0"', arg)
            self.assertIn('"a3"', arg)
            self.assertIn('"S"', arg)
            self.assertNotIn("getIntExtra", arg)
            self.assertNotIn("getBooleanExtra", arg)
            self.assertEqual(arg.count("return-object"), 2, arg)
            arg_int = _method_body(listing, "argInt(")
            self.assertIn("getIntExtra", arg_int)
            self.assertIn('"I"', arg_int)
            self.assertNotIn("getBooleanExtra", arg_int)
            self.assertEqual(arg_int.count("return v"), 2, arg_int)
            arg_bool = _method_body(listing, "argBool(")
            self.assertIn("getBooleanExtra", arg_bool)
            self.assertIn('"B"', arg_bool)
            self.assertEqual(arg_bool.count("return v"), 2, arg_bool)
            methods = compiled["dex"].image["methods"]
            for name, ret in (("arg", "Ljava/lang/String;"), ("argInt", "I"), ("argBool", "Z")):
                self.assertIn(
                    ("Lcom/example/pass/DroideRt;", name, ret, ("Landroid/app/Activity;", "I")),
                    methods,
                    name,
                )
            for params in (
                ("Ljava/lang/String;", "Ljava/lang/String;"),
                ("Ljava/lang/String;", "I"),
                ("Ljava/lang/String;", "Z"),
            ):
                self.assertIn(
                    ("Landroid/content/Intent;", "putExtra", "Landroid/content/Intent;", params),
                    methods,
                    params,
                )
            self.assertIn(
                ("Landroid/app/Activity;", "getIntent", "Landroid/content/Intent;", ()),
                methods,
            )

    def test_open_form_arguments_reject(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadArgs"
            Project.create(root, "BadArgs", "com.example.badargs")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = []
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Detail.form.json").write_text(
                '{"name":"Detail","title":"Detail","background":"#F7F8FA",'
                '"width":360,"height":640,"events":{},"components":[]}',
                encoding="utf-8",
            )
            (root / "forms" / "Detail.droid").write_text(
                "procedure Unused();\nbegin\nend;\n", encoding="utf-8",
            )
            (root / "forms" / "Main.droid").write_text(
                "procedure Show(Sender: View);\nvar\n  Name: String;\nbegin\n"
                "  OpenForm()\n  OpenForm(Name)\n  OpenForm('Missing')\n"
                "  OpenForm('Detail', Sender)\n  OpenForm('Detail', 1, 2, 3, 4, 5)\n"
                "  Arg('no')\n  ArgInt(true)\n  ArgBool()\nend;\n",
                encoding="utf-8",
            )
            (root / "units" / "Utils.droid").write_text(
                "unit Utils;\n\nfunction Clamp(Value: Integer): Integer;\nbegin\n"
                "  Result := Arg(Value)\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            for needle in (
                "OpenForm needs a string literal form name",
                "unknown form Missing",
                "OpenForm cannot pass a View",
                "OpenForm passes at most 4 arguments",
                "Arg expects Integer, got String",
                "ArgInt expects Integer, got Boolean",
                "ArgBool expects 1 arguments",
                "Arg is only available on a form",
            ):
                self.assertTrue(any(needle in msg for msg in messages), (needle, messages))
            self.assertFalse(any("OpenForm expects" in msg for msg in messages), messages)

    def test_panel_holds_children(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Panel"
            Project.create(root, "Panel", "com.example.panel")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = [
                {
                    "type": "Button", "name": "Early", "parent": "Box",
                    "left": 4, "top": 2, "width": 80, "height": 40,
                    "props": {"Text": "Early"}, "events": {},
                },
                {
                    "type": "Panel", "name": "Box",
                    "left": 16, "top": 24, "width": 200, "height": 160,
                    "props": {"Color": "#E4E8EE"}, "events": {},
                },
                {
                    "type": "Panel", "name": "Pad",
                    "left": 16, "top": 200, "width": 80, "height": 40,
                    "props": {"Color": "#E4E8EE"}, "events": {},
                },
                {
                    "type": "Panel", "name": "Row",
                    "left": 16, "top": 260, "width": 240, "height": 64,
                    "props": {"Direction": "Horizontal", "Color": "#E4E8EE"}, "events": {},
                },
                {
                    "type": "Button", "name": "Side", "parent": "Row",
                    "left": 900, "top": 0, "width": 40, "height": 36,
                    "props": {"Text": "Side"}, "events": {},
                },
                {
                    "type": "Panel", "name": "Inner", "parent": "Box",
                    "left": 8, "top": 4, "width": 120, "height": 80,
                    "props": {"Color": "#D8DEE8"}, "events": {},
                },
                {
                    "type": "Button", "name": "Go", "parent": "Inner",
                    "left": 2, "top": 2, "width": 72, "height": 36,
                    "props": {"Text": "Go"}, "events": {},
                },
            ]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text(
                "procedure Turn();\nbegin\n  Box.Direction := 'Horizontal'\nend;\n",
                encoding="utf-8",
            )
            compiled = compile_project(root)
            self.assertEqual(compiled["errors"], [], compiled["errors"])
            verify_image(compiled["dex"].image)
            listing = "\n".join(compiled["dex"].listing)
            create = _method_body(listing, "onCreate(")
            frame = create.split("FrameLayout$LayoutParams", 1)[0]
            self.assertIn("#360", frame)
            self.assertIn("#640", frame)
            self.assertNotIn("#900", frame)
            self.assertIn("#900", create)
            self.assertIn("Landroid/widget/LinearLayout;", create)
            self.assertIn("Landroid/widget/LinearLayout$LayoutParams;", create)
            self.assertEqual(create.count("setOrientation"), 4)
            early_put = create.index("iput-object v2, v6, Early")
            box_put = create.index("iput-object v2, v6, Box")
            self.assertLess(early_put, box_put)
            early_get = create.index("iget-object v2, v6, Early")
            self.assertLess(box_put, early_get)
            self.assertIn("iget-object v3, v6, Box", create[box_put:early_get])
            lines = create.splitlines()
            saw_vertical = saw_horizontal = False
            for i, line in enumerate(lines):
                if "setOrientation" not in line:
                    continue
                prev = next(item for item in reversed(lines[:i]) if "const/" in item)
                saw_horizontal = saw_horizontal or "#0" in prev
                saw_vertical = saw_vertical or "#1" in prev
            self.assertTrue(saw_vertical and saw_horizontal, create)
            turn = _method_body(listing, "Turn(")
            self.assertIn("equalsIgnoreCase", turn)
            self.assertIn("setOrientation", turn)
            methods = compiled["dex"].image["methods"]
            self.assertIn(
                ("Landroid/widget/LinearLayout;", "setOrientation", "V", ("I",)),
                methods,
            )
            self.assertIn(
                ("Landroid/widget/LinearLayout$LayoutParams;", "<init>", "V", ("I", "I")),
                methods,
            )

    def test_panel_rejects_a_bad_parent(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "BadPanel"
            Project.create(root, "BadPanel", "com.example.badpanel")
            form = json.loads((root / "forms" / "Main.form.json").read_text(encoding="utf-8"))
            form["components"] = [
                {
                    "type": "Button", "name": "Early", "parent": "Missing",
                    "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {},
                },
                {
                    "type": "Label", "name": "Title",
                    "left": 0, "top": 0, "width": 40, "height": 40,
                    "props": {"Text": "T"}, "events": {},
                },
                {
                    "type": "Button", "name": "Late", "parent": "Title",
                    "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {},
                },
                {
                    "type": "Panel", "name": "Box", "parent": "Box",
                    "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {},
                },
                {
                    "type": "Panel", "name": "A", "parent": "B",
                    "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {},
                },
                {
                    "type": "Panel", "name": "B", "parent": "A",
                    "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {},
                },
                {
                    "type": "Panel", "name": "Wide",
                    "left": 0, "top": 0, "width": 40, "height": 40,
                    "props": {"Direction": "Sideways"}, "events": {},
                },
            ]
            (root / "forms" / "Main.form.json").write_text(json.dumps(form), encoding="utf-8")
            (root / "forms" / "Main.droid").write_text("procedure Show();\nbegin\nend;\n", encoding="utf-8")
            compiled = compile_project(root, codegen=False)
            messages = [err["message"] for err in compiled["errors"]]
            for needle in (
                "Early parent Missing is unknown",
                "Late parent Title is not a Panel",
                "Box parents itself",
                "parent cycle",
                "Wide Direction must be Vertical or Horizontal",
            ):
                self.assertTrue(any(needle in msg for msg in messages), (needle, messages))


def _method_body(listing, name):
    header = "\n  " + name
    start = listing.find(header)
    if start < 0:
        raise AssertionError("missing " + name)
    rest = listing[start + 1:]
    out = []
    for line in rest.splitlines()[1:]:
        if line.startswith("class ") or (line.startswith("  ") and not line.startswith("   ")):
            break
        out.append(line)
    return "\n".join(out)


def _class_body(listing, desc):
    header = "class " + desc
    start = listing.find(header)
    rest = listing[start + len(header):]
    end = rest.find("\nclass ")
    if end < 0:
        end = len(rest)
    return rest[:end]


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
            self.assertEqual(catalog["widgets"]["Button"]["events"][0], "OnClick")
            self.assertIn("OnLongClick", catalog["widgets"]["Button"]["events"])
            self.assertIn("OnChange", catalog["widgets"]["Edit"]["events"])
            self.assertEqual(catalog["widgets"]["List"]["events"][0], "OnItemClick")
            self.assertIn("OnItemLongClick", catalog["widgets"]["List"]["events"])
            self.assertEqual(catalog["widgets"]["Spinner"]["events"], ["OnItemSelected"])
            self.assertEqual(catalog["widgets"]["Spinner"]["props"][:2], ["Items", "Selection"])
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
