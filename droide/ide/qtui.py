"""Native Droide studio.

The window is Qt 6. The form on the canvas is the same Qt widgets, so the kit
the IDE is built with is also the kit you place, edit, and preview.
"""

import json

from PyQt6.QtCore import QDate, QEvent, QRectF, Qt, QThread, QTime, pyqtSignal
from PyQt6.QtGui import QColor, QFont, QIntValidator, QPainter, QPen, QSyntaxHighlighter, QTextCharFormat
from PyQt6.QtWidgets import (
    QApplication,
    QButtonGroup,
    QCheckBox,
    QDateEdit,
    QDialog,
    QFrame,
    QGraphicsRectItem,
    QGraphicsScene,
    QGraphicsSimpleTextItem,
    QGraphicsView,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMainWindow,
    QMenu,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QRadioButton,
    QScrollArea,
    QSlider,
    QSpinBox,
    QStackedWidget,
    QStatusBar,
    QTextEdit,
    QTimeEdit,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..build import analyze, build, run
from ..compiler.widgets import PROP_TYPES, WIDGETS, new_component
from ..project import Project

PAPER = "#e7e4de"
INK = "#1e1d1b"
LINE = "#ddd9d2"
ACCENT = "#1d4e89"


def _snap(value):
    return int(round(value / 4.0) * 4)


def _int_prop(props, key, default):
    try:
        return int(props.get(key, default))
    except (TypeError, ValueError):
        return default


def _qss():
    return f"""
    QMainWindow, QDialog, QWidget {{ background: {PAPER}; color: {INK}; }}
    QFrame#bar {{ background: #fbfbfa; border-bottom: 1px solid {LINE}; }}
    QFrame#phone {{ background: #F7F8FA; border: 1px solid {LINE}; }}
    QFrame#side, QFrame#index, QScrollArea#index {{ background: #fbfbfa; border: 0; }}
    QFrame#side {{ border-left: 1px solid {LINE}; }}
    QFrame#index, QScrollArea#index {{ border-right: 1px solid {LINE}; }}
    QPushButton {{ background: transparent; border: 1px solid transparent; border-radius: 6px; padding: 4px 10px; }}
    QPushButton:hover {{ background: white; }}
    QPushButton:checked {{ background: {INK}; color: white; }}
    QPushButton#run {{ background: {INK}; color: white; padding: 5px 14px; }}
    QPushButton#run:hover {{ background: black; }}
    QListWidget, QPlainTextEdit, QTextEdit {{ background: white; border: 0; }}
    QLineEdit {{ background: white; border: 1px solid {LINE}; border-radius: 6px; padding: 4px 8px; }}
    QStatusBar {{ background: #fbfbfa; border-top: 1px solid {LINE}; }}
    """


class Highlighter(QSyntaxHighlighter):
    def __init__(self, document, kind):
        super().__init__(document)
        self.kind = kind
        self.kw = QTextCharFormat()
        self.kw.setForeground(QColor(ACCENT))
        self.string = QTextCharFormat()
        self.string.setForeground(QColor("#1d6b42"))
        self.comment = QTextCharFormat()
        self.comment.setForeground(QColor("#9a948c"))
        if kind == "native":
            self.words = {"fun", "return", "if", "else", "while", "var", "i32"}
        else:
            self.words = {
                "unit", "uses", "procedure", "function", "var", "begin", "end", "if", "then",
                "else", "while", "do", "exit", "and", "or", "not", "div", "mod", "true",
                "false", "integer", "string", "boolean", "view",
            }

    def highlightBlock(self, text):
        if text.strip().startswith("//"):
            self.setFormat(0, len(text), self.comment)
            return
        i = 0
        while i < len(text):
            ch = text[i]
            if ch in ("'", '"'):
                j = i + 1
                while j < len(text) and text[j] != ch:
                    j += 1
                j = min(len(text), j + 1)
                self.setFormat(i, j - i, self.string)
                i = j
                continue
            if ch.isalpha() or ch == "_":
                j = i + 1
                while j < len(text) and (text[j].isalnum() or text[j] == "_"):
                    j += 1
                if text[i:j].lower() in self.words:
                    self.setFormat(i, j - i, self.kw)
                i = j
                continue
            i += 1


class BuildJob(QThread):
    finished_ok = pyqtSignal(object)

    def __init__(self, root, launch, buffers):
        super().__init__()
        self.root = str(root)
        self.launch = launch
        self.buffers = buffers

    def run(self):
        try:
            result = run(self.root, self.buffers) if self.launch else build(self.root, self.buffers)
        except Exception as exc:
            result = {"ok": False, "errors": [{"message": str(exc), "file": "", "line": 1}], "log": str(exc)}
        self.finished_ok.emit(result)


class Studio(QMainWindow):
    def __init__(self, root=None):
        super().__init__()
        self.setWindowTitle("Droide")
        self.resize(1280, 820)
        self.project = Project(root) if root else None
        self.files = []
        self.current = None
        self.form = None
        self.texts = {}
        self.dirty = set()
        self.form_dirty = False
        self.selected = None
        self.pin = False
        self.armed = None
        self.live = {}
        self.db = None
        self.errors = []
        self.focus_id = None
        self.lens = "calls"
        self._drag = None
        self._job = None
        self._log = ""
        self._graph_count = 0
        self._build_ui()
        if self.project:
            self.reload_files()
            main = self.project.meta.get("mainForm") or "Main"
            info = next((item for item in self.files if item["name"] == main and item["kind"] == "form"), None)
            if info:
                self.open_file(info)
            elif self.files:
                self.open_file(self.files[0])
        self.set_mode("design")

    def _build_ui(self):
        host = QWidget()
        self.setCentralWidget(host)
        outer = QVBoxLayout(host)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)
        outer.addWidget(self._bar())
        self.stack = QStackedWidget()
        self.stack.addWidget(self._design())
        self.stack.addWidget(self._code())
        self.stack.addWidget(self._explore())
        outer.addWidget(self.stack, 1)
        self.setStatusBar(QStatusBar())
        self.health = QToolButton()
        self.health.setAutoRaise(True)
        self.health.clicked.connect(self.show_messages)
        self.statusBar().addPermanentWidget(self.health)
        self._shortcuts()
        self._say("Droide")

    def _bar(self):
        bar = QFrame()
        bar.setObjectName("bar")
        bar.setFixedHeight(44)
        row = QHBoxLayout(bar)
        row.setContentsMargins(12, 6, 12, 6)
        brand = QLabel("Droide")
        brand.setStyleSheet("font-weight: 650;")
        row.addWidget(brand)
        self.project_btn = QToolButton()
        self.project_btn.setText("Open")
        self.project_btn.setPopupMode(QToolButton.ToolButtonPopupMode.InstantPopup)
        self.project_btn.setMenu(QMenu(self.project_btn))
        self.project_btn.menu().aboutToShow.connect(self._fill_project_menu)
        row.addWidget(self.project_btn)
        row.addStretch(1)
        self.modes = QButtonGroup(self)
        self.modes.setExclusive(True)
        self.mode_btns = {}
        for key, label in (("design", "Design"), ("code", "Code"), ("explore", "Explore")):
            button = QPushButton(label)
            button.setCheckable(True)
            button.clicked.connect(lambda _=False, name=key: self.set_mode(name))
            self.modes.addButton(button)
            self.mode_btns[key] = button
            row.addWidget(button)
        row.addSpacing(8)
        compile_btn = QPushButton("Compile")
        compile_btn.clicked.connect(self.compile)
        row.addWidget(compile_btn)
        preview = QPushButton("Preview")
        preview.setToolTip("Open this form with the Qt widgets this IDE is built from")
        preview.clicked.connect(self.open_preview)
        row.addWidget(preview)
        run_btn = QPushButton("Run")
        run_btn.setObjectName("run")
        run_btn.clicked.connect(self.launch)
        row.addWidget(run_btn)
        return bar

    def _design(self):
        page = QWidget()
        row = QHBoxLayout(page)
        row.setContentsMargins(0, 0, 0, 0)
        row.setSpacing(0)
        scroll = QScrollArea()
        scroll.setObjectName("index")
        scroll.setFixedWidth(148)
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        palette = QWidget()
        col = QVBoxLayout(palette)
        col.setContentsMargins(8, 10, 8, 10)
        groups = []
        for kind, spec in WIDGETS.items():
            if spec["group"] not in groups:
                groups.append(spec["group"])
                title = QLabel(spec["group"].upper())
                title.setStyleSheet("color: #6e6a64; font-size: 11px;")
                col.addWidget(title)
            button = QPushButton(kind)
            button.setStyleSheet("text-align: left; padding: 4px 8px;")
            button.clicked.connect(lambda _=False, name=kind: self.arm(name))
            col.addWidget(button)
        col.addStretch(1)
        scroll.setWidget(palette)
        row.addWidget(scroll)
        center = QWidget()
        center_col = QVBoxLayout(center)
        self.phone = QFrame()
        self.phone.setObjectName("phone")
        self.phone.setFixedSize(360, 640)
        self.phone.mousePressEvent = self._phone_press
        holder = QHBoxLayout()
        holder.addStretch(1)
        holder.addWidget(self.phone)
        holder.addStretch(1)
        center_col.addStretch(1)
        center_col.addLayout(holder)
        self.caption = QLabel("360 × 640 dp")
        self.caption.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.caption.setStyleSheet("color: #6e6a64;")
        center_col.addWidget(self.caption)
        center_col.addStretch(1)
        row.addWidget(center, 1)
        self.side = QFrame()
        self.side.setObjectName("side")
        self.side.setFixedWidth(260)
        side_col = QVBoxLayout(self.side)
        side_col.setContentsMargins(12, 12, 12, 12)
        head = QLabel("Inspector")
        head.setStyleSheet("font-weight: 650;")
        side_col.addWidget(head)
        self.inspector = QWidget()
        self.inspector_col = QVBoxLayout(self.inspector)
        self.inspector_col.setContentsMargins(0, 0, 0, 0)
        side_col.addWidget(self.inspector)
        side_col.addStretch(1)
        self.side.setVisible(False)
        row.addWidget(self.side)
        return page

    def _code(self):
        page = QWidget()
        col = QVBoxLayout(page)
        col.setContentsMargins(0, 0, 0, 0)
        self.code_caption = QLabel("")
        self.code_caption.setStyleSheet("padding: 8px 16px 0; color: #6e6a64;")
        col.addWidget(self.code_caption)
        self.editor = QPlainTextEdit()
        self.editor.setFont(QFont("DejaVu Sans Mono", 11))
        self.editor.setLineWrapMode(QPlainTextEdit.LineWrapMode.NoWrap)
        self.editor.textChanged.connect(self._on_edit)
        self.editor.cursorPositionChanged.connect(self._on_caret)
        col.addWidget(self.editor, 1)
        self.highlighter = None
        return page

    def _explore(self):
        page = QWidget()
        row = QHBoxLayout(page)
        row.setContentsMargins(0, 0, 0, 0)
        index = QFrame()
        index.setObjectName("index")
        index.setFixedWidth(260)
        col = QVBoxLayout(index)
        self.find_ent = QLineEdit()
        self.find_ent.setPlaceholderText("Find a routine, widget, or file")
        self.find_ent.textChanged.connect(self._fill_index)
        col.addWidget(self.find_ent)
        self.index = QListWidget()
        self.index.currentItemChanged.connect(self._index_chosen)
        col.addWidget(self.index, 1)
        row.addWidget(index)
        right = QWidget()
        detail = QVBoxLayout(right)
        self.ex_title = QLabel("")
        self.ex_title.setStyleSheet("font-weight: 650; font-size: 16px;")
        self.ex_meta = QLabel("")
        self.ex_meta.setWordWrap(True)
        detail.addWidget(self.ex_title)
        detail.addWidget(self.ex_meta)
        lenses = QHBoxLayout()
        self.lens_btns = {}
        group = QButtonGroup(self)
        group.setExclusive(True)
        for key, label in (("calls", "Call Graph"), ("flow", "Control Flow"), ("refs", "References"), ("arch", "Architecture")):
            button = QPushButton(label)
            button.setCheckable(True)
            button.clicked.connect(lambda _=False, name=key: self._set_lens(name))
            group.addButton(button)
            self.lens_btns[key] = button
            lenses.addWidget(button)
        lenses.addStretch(1)
        detail.addLayout(lenses)
        self.graph = QGraphicsView()
        self.scene = QGraphicsScene(self.graph)
        self.graph.setScene(self.scene)
        self.graph.setRenderHint(QPainter.RenderHint.Antialiasing, False)
        self.graph.setStyleSheet("background: white; border: 0;")
        detail.addWidget(self.graph, 1)
        self.ex_text = QTextEdit()
        self.ex_text.setReadOnly(True)
        self.ex_text.setFont(QFont("DejaVu Sans Mono", 10))
        self.ex_text.hide()
        detail.addWidget(self.ex_text, 1)
        row.addWidget(right, 1)
        return page

    def _shortcuts(self):
        self._key("F9", self.launch)
        self._key("Ctrl+F9", self.compile)
        self._key("F12", self._toggle_code)
        self._key("F4", lambda: self.set_mode("explore"))
        self._key("F11", self._pin)
        self._key("Ctrl+S", self.save)
        self._key("F8", self.open_preview)

    def _key(self, seq, fn):
        from PyQt6.QtGui import QShortcut, QKeySequence
        QShortcut(QKeySequence(seq), self, fn)

    def _say(self, text):
        self.statusBar().showMessage(text)
        count = len(self.errors)
        self.health.setText("clean" if not count else f"{count} error" + ("" if count == 1 else "s"))

    def reload_files(self):
        self.files = self.project.list_files() if self.project else []
        title = (self.project.meta.get("title") or self.project.meta.get("name")) if self.project else "Open"
        self.project_btn.setText(title)
        self.setWindowTitle("Droide — " + title)

    def _fill_project_menu(self):
        menu = self.project_btn.menu()
        menu.clear()
        for info in self.files:
            label = f"{info['kind']}  {info['name']}"
            menu.addAction(label, lambda info=info: self.open_file(info))
        menu.addSeparator()
        menu.addAction("Save", self.save)

    def open_file(self, info):
        self._store_editor()
        self.current = info
        rel = info["source"]
        if rel not in self.texts:
            self.texts[rel] = self.project.read(rel) if self.project else ""
        self.form = None
        self.selected = None
        if info.get("form"):
            raw = self.project.read(info["form"]) if self.project else ""
            try:
                self.form = json.loads(raw or "{}")
            except json.JSONDecodeError:
                self.form = None
        self.editor.blockSignals(True)
        self.editor.setPlainText(self.texts[rel])
        self.editor.blockSignals(False)
        kind = "native" if info["kind"] == "native" else "unit"
        self.highlighter = Highlighter(self.editor.document(), kind)
        self.code_caption.setText(rel)
        self._paint_phone()
        self._sync_side()
        if self.stack.currentIndex() == 0 and not self.form:
            self.set_mode("code")
        self._say(f"{info['name']} · {self._mode_name()}")

    def _store_editor(self):
        if not self.current:
            return
        rel = self.current["source"]
        text = self.editor.toPlainText()
        if self.texts.get(rel) != text:
            self.texts[rel] = text
            self.dirty.add(rel)

    def _on_edit(self):
        if not self.current:
            return
        rel = self.current["source"]
        self.texts[rel] = self.editor.toPlainText()
        self.dirty.add(rel)

    def _on_caret(self):
        if not self.current:
            return
        line = self.editor.textCursor().blockNumber() + 1
        self._say(f"{self.current['source']}:{line}")

    def buffers(self):
        self._store_editor()
        buf = {rel: self.texts[rel] for rel in self.dirty}
        if self.form_dirty and self.current and self.current.get("form") and self.form:
            buf[self.current["form"]] = json.dumps(self.form, indent=2) + "\n"
        return buf

    def set_mode(self, mode):
        if mode == "design" and not self.form:
            mode = "code"
        self.stack.setCurrentIndex({"design": 0, "code": 1, "explore": 2}[mode])
        for key, button in self.mode_btns.items():
            button.setChecked(key == mode)
        self._sync_side()
        if mode == "explore":
            self.refresh_explore()
        self._say(f"{self.current['name'] if self.current else 'Droide'} · {mode}")

    def _mode_name(self):
        return ("design", "code", "explore")[self.stack.currentIndex()]

    def _toggle_code(self):
        self.set_mode("design" if self._mode_name() == "code" else "code")

    def _pin(self):
        self.pin = not self.pin
        self._sync_side()

    def arm(self, kind):
        self.armed = kind
        self.caption.setText("Click the form to place " + kind)
        self.set_mode("design")

    def _phone_press(self, ev):
        if self.armed and self.form is not None:
            self._place(ev.position().toPoint())
            return
        self.select(None)

    def _place(self, pos):
        used = {comp["name"].lower() for comp in self.form.get("components", [])}
        n = 1
        while f"{self.armed}{n}".lower() in used:
            n += 1
        comp = new_component(self.armed, self.armed + str(n), _snap(pos.x()), _snap(pos.y()))
        self.form.setdefault("components", []).append(comp)
        self.form_dirty = True
        self.armed = None
        self.caption.setText("360 × 640 dp")
        self._paint_phone()
        self.select(comp["name"])

    def _paint_phone(self):
        for child in list(self.phone.children()):
            if isinstance(child, QWidget):
                child.setParent(None)
                child.deleteLater()
        self.live = {}
        if not self.form:
            return
        bg = self.form.get("background") or "#F7F8FA"
        self.phone.setStyleSheet(f"QFrame#phone {{ background: {bg}; border: 1px solid {LINE}; }}")
        for comp in self.form.get("components") or []:
            widget = self._make_widget(comp, design=True)
            widget.setParent(self.phone)
            widget.setGeometry(int(comp["left"]), int(comp["top"]), int(comp["width"]), int(comp["height"]))
            widget.show()
            widget.installEventFilter(self)
            self.live[comp["name"]] = widget
        self._sync_side()

    def _make_widget(self, comp, design):
        kind = comp["type"]
        props = comp.get("props") or {}
        if kind == "Button":
            widget = QPushButton(str(props.get("Text", "")))
        elif kind == "Memo":
            widget = QPlainTextEdit(str(props.get("Text", "")))
            widget.setPlaceholderText(str(props.get("Hint", "")))
        elif kind == "Password":
            widget = QLineEdit(str(props.get("Text", "")))
            widget.setPlaceholderText(str(props.get("Hint", "")))
            widget.setEchoMode(QLineEdit.EchoMode.Password)
        elif kind == "Toggle":
            on = bool(props.get("Checked", False))
            label = props.get("TextOn", "On") if on else props.get("TextOff", "Off")
            widget = QPushButton(str(label))
            widget.setCheckable(True)
            widget.setChecked(on)
        elif kind in ("Edit", "Number", "Email", "Phone"):
            widget = QLineEdit(str(props.get("Text", "")))
            widget.setPlaceholderText(str(props.get("Hint", "")))
            if kind == "Number":
                widget.setValidator(QIntValidator(-2147483647, 2147483647, widget))
        elif kind == "Radio":
            widget = QRadioButton(str(props.get("Text", "")))
            widget.setChecked(bool(props.get("Checked", False)))
        elif kind in ("Check", "Switch"):
            widget = QCheckBox(str(props.get("Text", "")))
            widget.setChecked(bool(props.get("Checked", False)))
        elif kind == "Seek":
            widget = QSlider(Qt.Orientation.Horizontal)
            widget.setMaximum(max(1, _int_prop(props, "Max", 100)))
            widget.setValue(_int_prop(props, "Progress", 0))
        elif kind == "Progress":
            widget = QProgressBar()
            widget.setTextVisible(False)
            widget.setMaximum(max(1, _int_prop(props, "Max", 100)))
            widget.setValue(_int_prop(props, "Progress", 0))
        elif kind == "Rating":
            stars = max(1, _int_prop(props, "Stars", 5))
            rating = min(stars, max(0, _int_prop(props, "Rating", 0)))
            widget = QLabel("★" * rating + "☆" * (stars - rating))
        elif kind == "Clock":
            widget = QLabel(str(props.get("Format", "HH:mm")))
        elif kind == "Picker":
            widget = QSpinBox()
            lo = _int_prop(props, "MinValue", 0)
            hi = max(lo, _int_prop(props, "MaxValue", 10))
            widget.setRange(lo, hi)
            widget.setValue(_int_prop(props, "Value", lo))
            widget.setWrapping(bool(props.get("Wrap", True)))
        elif kind == "Time":
            widget = QTimeEdit()
            hour = _int_prop(props, "Hour", 12) % 24
            minute = _int_prop(props, "Minute", 0) % 60
            widget.setTime(QTime(hour, minute))
            widget.setDisplayFormat("HH:mm" if props.get("Is24Hour", True) else "hh:mm AP")
        elif kind == "Date":
            widget = QDateEdit()
            widget.setDisplayFormat("yyyy-MM-dd")
            widget.setCalendarPopup(True)
            widget.setDate(QDate(
                _int_prop(props, "Year", 2026),
                max(1, min(12, _int_prop(props, "Month", 1))),
                max(1, min(31, _int_prop(props, "Day", 1))),
            ))
        elif kind == "Busy":
            widget = QProgressBar()
            widget.setTextVisible(False)
            widget.setRange(0, 0)
        elif kind == "Panel":
            widget = QFrame()
        else:
            widget = QLabel(str(props.get("Text", "")))
        widget.setObjectName(comp["name"])
        widget.setProperty("droideName", comp["name"])
        font = widget.font()
        if props.get("TextSize"):
            font.setPointSize(int(props["TextSize"]))
            widget.setFont(font)
        if "Enabled" in props:
            widget.setEnabled(bool(props["Enabled"]))
        if props.get("Visible") is False:
            widget.setVisible(False)
        widget.setStyleSheet(self._look(comp, comp["name"] == self.selected and design))
        if design:
            widget.setFocusPolicy(Qt.FocusPolicy.NoFocus)
        return widget

    def _look(self, comp, selected):
        props = comp.get("props") or {}
        color = props.get("TextColor") or INK
        border = f"1px solid {ACCENT}" if selected else "1px solid transparent"
        if comp["type"] == "Panel":
            bg = props.get("Color") or "#E4E8EE"
            border = f"1px solid {ACCENT}" if selected else "1px solid #e4e8ee"
            return f"background: {bg}; border: {border};"
        if comp["type"] in ("Label", "Clock"):
            return f"color: {color}; background: transparent; border: {border};"
        if comp["type"] == "Rating":
            return f"color: #c48a1a; background: transparent; border: {border};"
        if comp["type"] in ("Seek", "Progress", "Busy"):
            return f"background: transparent; border: {border};"
        bg = props.get("Color") or "#ffffff"
        edge = ACCENT if selected else "#ddd9d2"
        return f"color: {color}; background: {bg}; border: 1px solid {edge}; border-radius: 4px; padding: 2px 6px;"

    def eventFilter(self, obj, ev):
        name = obj.property("droideName")
        if not name or name not in self.live:
            return False
        kind = ev.type()
        if kind == QEvent.Type.MouseButtonDblClick:
            self._ensure_handler(name)
            return True
        if kind == QEvent.Type.MouseButtonPress and ev.button() == Qt.MouseButton.LeftButton:
            self.select(name)
            pos = ev.position().toPoint()
            resize = pos.x() >= obj.width() - 12 and pos.y() >= obj.height() - 12
            self._drag = {"name": name, "grab": pos, "resize": resize, "moved": False}
            obj.grabMouse()
            return True
        if kind == QEvent.Type.MouseMove and self._drag and self._drag["name"] == name:
            delta = ev.position().toPoint() - self._drag["grab"]
            comp = self._comp(name)
            if self._drag["resize"]:
                comp["width"] = max(24, _snap(obj.width() + delta.x()))
                comp["height"] = max(16, _snap(obj.height() + delta.y()))
            else:
                comp["left"] = max(0, _snap(obj.x() + delta.x()))
                comp["top"] = max(0, _snap(obj.y() + delta.y()))
            obj.setGeometry(int(comp["left"]), int(comp["top"]), int(comp["width"]), int(comp["height"]))
            self._drag["grab"] = ev.position().toPoint()
            self._drag["moved"] = True
            self.form_dirty = True
            return True
        if kind == QEvent.Type.MouseButtonRelease and self._drag and self._drag["name"] == name:
            obj.releaseMouse()
            self._drag = None
            self._fill_inspector()
            return True
        return False

    def _comp(self, name):
        for comp in self.form.get("components") or []:
            if comp["name"] == name:
                return comp
        return None

    def select(self, name):
        self.selected = name
        for comp_name, widget in self.live.items():
            comp = self._comp(comp_name)
            if comp:
                widget.setStyleSheet(self._look(comp, comp_name == name))
        self._sync_side()
        if name:
            self._say(f"{self.form.get('name', '')}.{name}")

    def _sync_side(self):
        show = self._mode_name() == "design" and self.form is not None and (self.pin or bool(self.selected))
        self.side.setVisible(show)
        if show:
            self._fill_inspector()

    def _clear_layout(self, layout):
        while layout.count():
            item = layout.takeAt(0)
            if item.widget():
                item.widget().deleteLater()

    def _fill_inspector(self):
        self._clear_layout(self.inspector_col)
        comp = self._comp(self.selected) if self.selected else None
        if not comp:
            note = QLabel("Select a widget.")
            note.setStyleSheet("color: #6e6a64;")
            self.inspector_col.addWidget(note)
            return
        spec = WIDGETS.get(comp["type"], {})
        title = QLabel(comp["type"])
        title.setStyleSheet("color: #6e6a64;")
        self.inspector_col.addWidget(title)
        self._field("Name", comp["name"], lambda text, comp=comp: self._rename(comp, text))
        for key in ("left", "top", "width", "height"):
            self._field(key, str(comp[key]), lambda text, comp=comp, key=key: self._geom(comp, key, text))
        for prop in spec.get("props") or []:
            value = (comp.get("props") or {}).get(prop, "")
            self._field(prop, value, lambda text, comp=comp, prop=prop: self._prop(comp, prop, text))

    def _field(self, label, value, apply):
        row = QHBoxLayout()
        name = QLabel(label)
        name.setFixedWidth(72)
        editor = QLineEdit(str(value))
        editor.editingFinished.connect(lambda editor=editor, apply=apply: apply(editor.text()))
        row.addWidget(name)
        row.addWidget(editor)
        self.inspector_col.addLayout(row)

    def _rename(self, comp, text):
        text = text.strip()
        if not text or text == comp["name"]:
            return
        if any(other["name"] == text for other in self.form["components"]):
            return
        comp["name"] = text
        self.form_dirty = True
        self.selected = text
        self._paint_phone()

    def _geom(self, comp, key, text):
        try:
            comp[key] = max(0, int(text))
        except ValueError:
            return
        self.form_dirty = True
        widget = self.live.get(comp["name"])
        if widget:
            widget.setGeometry(int(comp["left"]), int(comp["top"]), int(comp["width"]), int(comp["height"]))

    def _prop(self, comp, prop, text):
        props = comp.setdefault("props", {})
        if prop in ("Enabled", "Visible", "Checked"):
            props[prop] = text.strip().lower() in ("1", "true", "yes", "on")
        elif PROP_TYPES.get(prop) == "Integer" and prop not in ("TextColor", "Color"):
            try:
                props[prop] = int(text)
            except ValueError:
                return
        else:
            props[prop] = text
        self.form_dirty = True
        self._paint_phone()

    def _ensure_handler(self, name):
        comp = self._comp(name)
        if not comp or not self.current:
            return
        handler = comp["name"] + "Click"
        comp.setdefault("events", {})["OnClick"] = handler
        self.form_dirty = True
        rel = self.current["source"]
        text = self.texts.get(rel, "")
        if handler.lower() not in text.lower():
            if text and not text.endswith("\n"):
                text += "\n"
            text += f"\nprocedure {handler}(Sender: View);\nbegin\n\nend;\n"
            self.texts[rel] = text
            self.dirty.add(rel)
            self.editor.setPlainText(text)
        self.set_mode("code")
        cursor = self.editor.textCursor()
        found = text.lower().rfind("procedure " + handler.lower())
        if found >= 0:
            cursor.setPosition(found)
            self.editor.setTextCursor(cursor)

    def open_preview(self):
        if not self.form:
            self._say("This file has no form")
            return None
        dialog = QDialog(self)
        dialog.setWindowTitle((self.form.get("title") or self.form.get("name") or "Form") + " — Qt")
        dialog.resize(400, 720)
        col = QVBoxLayout(dialog)
        note = QLabel("These are the Qt widgets this IDE is built with. Run compiles the same form for Android.")
        note.setWordWrap(True)
        note.setStyleSheet("color: #6e6a64;")
        col.addWidget(note)
        phone = QFrame()
        phone.setObjectName("phone")
        phone.setFixedSize(int(self.form.get("width") or 360), int(self.form.get("height") or 640))
        bg = self.form.get("background") or "#F7F8FA"
        phone.setStyleSheet(f"QFrame#phone {{ background: {bg}; border: 1px solid {LINE}; }}")
        for comp in self.form.get("components") or []:
            widget = self._make_widget(comp, design=False)
            widget.setParent(phone)
            widget.setGeometry(int(comp["left"]), int(comp["top"]), int(comp["width"]), int(comp["height"]))
            widget.show()
        holder = QHBoxLayout()
        holder.addStretch(1)
        holder.addWidget(phone)
        holder.addStretch(1)
        col.addLayout(holder)
        dialog.show()
        self._preview = dialog
        return dialog

    def save(self):
        if not self.project or not self.current:
            return
        self._store_editor()
        for rel in list(self.dirty):
            self.project.write(rel, self.texts.get(rel, ""))
        self.dirty.clear()
        if self.form_dirty and self.current.get("form") and self.form:
            self.project.write(self.current["form"], json.dumps(self.form, indent=2) + "\n")
            self.form_dirty = False
        self._say("Saved")

    def compile(self):
        self._start_job(False)

    def launch(self):
        self._start_job(True)

    def _start_job(self, launch):
        if not self.project or self._job:
            return
        self._say("Launching…" if launch else "Compiling…")
        self._job = BuildJob(self.project.root, launch, self.buffers())
        self._job.finished_ok.connect(self._job_done)
        self._job.start()

    def _job_done(self, result):
        self._job = None
        self.errors = result.get("errors") or []
        self._log = result.get("log") or ""
        if result.get("db"):
            self.db = result["db"]
        if not result.get("ok"):
            self._say("Build failed")
            self.show_messages()
            return
        if result.get("launched"):
            self._say("Started on " + (result.get("device") or "device"))
            return
        if self._log and result.get("device"):
            self._say("Not launched")
            self.show_messages()
            return
        self._say("APK ready")

    def show_messages(self):
        if not self.errors and not getattr(self, "_log", ""):
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("Messages")
        dialog.resize(640, 360)
        col = QVBoxLayout(dialog)
        text = QPlainTextEdit()
        text.setReadOnly(True)
        lines = [f"{err.get('file') or ''}:{err.get('line') or 1}: {err.get('message')}" for err in self.errors]
        if getattr(self, "_log", ""):
            lines.append(self._log)
        text.setPlainText("\n".join(lines))
        col.addWidget(text)
        dialog.show()

    def refresh_explore(self):
        if not self.project:
            return
        try:
            report = analyze(self.project.root, self.buffers())
        except Exception as exc:
            self._say(str(exc))
            return
        self.errors = report.get("errors") or []
        self.db = report.get("db") or {"entities": [], "refs": [], "cfgs": {}}
        if self.focus_id is None or not self._entity(self.focus_id):
            self.focus_id = self._pick_focus()
        self._fill_index()
        self._show_entity()
        self._say(f"{self.current['name'] if self.current else 'Droide'} · explore")

    def _entity(self, ident):
        for ent in (self.db or {}).get("entities") or []:
            if ent["id"] == ident:
                return ent
        return None

    def _pick_focus(self):
        ents = [ent for ent in (self.db or {}).get("entities") or [] if ent["kind"] in ("procedure", "function", "native")]
        if not ents:
            return None
        ents.sort(key=lambda ent: (-(ent.get("fanIn") or 0), -(ent.get("fanOut") or 0), ent["id"]))
        return ents[0]["id"]

    def _fill_index(self):
        self.index.blockSignals(True)
        self.index.clear()
        query = self.find_ent.text().strip().lower()
        current = None
        for ent in (self.db or {}).get("entities") or []:
            if ent["kind"] in ("param", "local"):
                continue
            label = ent.get("qname") or ent["name"]
            if query and query not in label.lower() and query not in (ent.get("file") or "").lower():
                continue
            item = QListWidgetItem(f"{ent['kind']}   {label}")
            item.setData(Qt.ItemDataRole.UserRole, ent["id"])
            self.index.addItem(item)
            if ent["id"] == self.focus_id:
                current = item
        if current:
            self.index.setCurrentItem(current)
        self.index.blockSignals(False)

    def _index_chosen(self, current, _previous):
        if not current:
            return
        self.focus_id = current.data(Qt.ItemDataRole.UserRole)
        self._show_entity()

    def _set_lens(self, name):
        self.lens = name
        self._show_entity()

    def _show_entity(self):
        for key, button in self.lens_btns.items():
            button.setChecked(key == self.lens)
        ent = self._entity(self.focus_id)
        if not ent:
            self.ex_title.setText("")
            self.ex_meta.setText("")
            self.scene.clear()
            return
        self.ex_title.setText(ent.get("qname") or ent["name"])
        self.ex_meta.setText(
            f"{ent['kind']}  ·  {ent.get('loc', 0)} lines  ·  complexity {ent.get('complexity', 0)}"
            f"  ·  {ent.get('fanIn', 0)} in  ·  {ent.get('fanOut', 0)} out"
        )
        calls = self.lens == "calls"
        self.graph.setVisible(calls)
        self.ex_text.setVisible(not calls)
        if calls:
            self._draw_calls(ent)
        elif self.lens == "flow":
            self.ex_text.setPlainText(self._flow_text(ent))
        elif self.lens == "refs":
            self.ex_text.setPlainText(self._refs_text(ent))
        else:
            self.ex_text.setPlainText(self._arch_text())

    def _refs_of(self, ident):
        refs = (self.db or {}).get("refs") or []
        incoming = [ref for ref in refs if ref.get("dst") == ident and ref.get("kind") == "call"]
        outgoing = [ref for ref in refs if ref.get("src") == ident and ref.get("kind") == "call"]
        return incoming, outgoing

    def _unique(self, refs, key, skip):
        found = []
        seen = set(skip)
        for ref in refs:
            other = self._entity(ref[key])
            if not other or other["id"] in seen:
                continue
            seen.add(other["id"])
            found.append(other)
        return found

    def _draw_calls(self, ent):
        self.scene.clear()
        incoming, outgoing = self._refs_of(ent["id"])
        callers = self._unique(incoming, "src", {ent["id"]})
        callees = self._unique(outgoing, "dst", {ent["id"]})
        seen = {ent["id"], *[node["id"] for node in callers], *[node["id"] for node in callees]}
        more = []
        for callee in callees:
            _inn, outs = self._refs_of(callee["id"])
            for node in self._unique(outs, "dst", seen):
                more.append(node)
                seen.add(node["id"])
        columns = [callers, [ent], callees, more[:8]]
        boxes = {}
        pen = QPen(QColor("#c4bfb6"))
        x = 24
        for column in columns:
            if not column:
                continue
            y = 24
            for node in column:
                rect = QGraphicsRectItem(QRectF(0, 0, 180, 48))
                rect.setPos(x, y)
                rect.setBrush(QColor("#f4f1eb" if node["id"] == ent["id"] else "white"))
                rect.setPen(pen)
                self.scene.addItem(rect)
                label = QGraphicsSimpleTextItem(node.get("qname") or node["name"], rect)
                label.setPos(8, 8)
                kind = QGraphicsSimpleTextItem(node["kind"], rect)
                kind.setPos(8, 26)
                kind.setBrush(QColor("#6e6a64"))
                boxes[node["id"]] = (x, y, 180, 48)
                y += 64
            x += 230
        for ref in (self.db or {}).get("refs") or []:
            if ref.get("kind") != "call" or ref["src"] not in boxes or ref["dst"] not in boxes:
                continue
            if ref["src"] == ref["dst"]:
                continue
            x1, y1, w1, h1 = boxes[ref["src"]]
            x2, y2, _w2, h2 = boxes[ref["dst"]]
            self.scene.addLine(x1 + w1, y1 + h1 / 2, x2, y2 + h2 / 2, pen)
        if any(ref.get("src") == ent["id"] and ref.get("dst") == ent["id"] for ref in outgoing):
            x1, y1, w1, h1 = boxes[ent["id"]]
            self.scene.addLine(x1 + w1, y1 + 12, x1 + w1 + 22, y1 + 12, pen)
            self.scene.addLine(x1 + w1 + 22, y1 + 12, x1 + w1 + 22, y1 + h1 - 8, pen)
            self.scene.addLine(x1 + w1 + 22, y1 + h1 - 8, x1 + w1, y1 + h1 - 8, pen)
        self._graph_count = len(boxes)
        self.scene.setSceneRect(self.scene.itemsBoundingRect().adjusted(-16, -16, 32, 16))

    def graph_nodes(self):
        return self._graph_count

    def _flow_text(self, ent):
        cfg = ((self.db or {}).get("cfgs") or {}).get(str(ent["id"])) or {}
        blocks = cfg.get("blocks") or []
        if not blocks:
            return "No control-flow graph for this entity."
        parts = []
        for block in blocks:
            body = "\n".join(block.get("text") or [])
            parts.append(f"{block.get('id')}  {block.get('label') or ''}\n{body}".rstrip())
        return "\n\n".join(parts)

    def _refs_text(self, ent):
        lines = []
        for ref in (self.db or {}).get("refs") or []:
            if ref.get("src") != ent["id"] and ref.get("dst") != ent["id"]:
                continue
            other = self._entity(ref["dst"] if ref.get("src") == ent["id"] else ref["src"])
            name = (other or {}).get("qname") or "?"
            lines.append(f"{ref.get('kind', ''):8} {name:24} {ref.get('file') or ''}:{ref.get('line') or ''}")
        return "\n".join(lines) or "No references."

    def _arch_text(self):
        rows = [
            ent for ent in (self.db or {}).get("entities") or []
            if ent["kind"] in ("procedure", "function", "native", "form", "unit")
        ]
        rows.sort(key=lambda ent: (-(ent.get("complexity") or 0), ent.get("qname") or ""))
        lines = [f"{'name':24} {'loc':>4} {'cc':>4} {'in':>4} {'out':>4}"]
        for ent in rows:
            lines.append(
                f"{(ent.get('qname') or ent['name'])[:24]:24} {ent.get('loc', 0):4} "
                f"{ent.get('complexity', 0):4} {ent.get('fanIn', 0):4} {ent.get('fanOut', 0):4}"
            )
        return "\n".join(lines)


def run_studio(root=None):
    app = QApplication.instance() or QApplication([])
    app.setStyleSheet(_qss())
    window = Studio(root)
    window.show()
    return app.exec()
