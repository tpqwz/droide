//! Native Droide studio. The window is egui. Compile, run, and analyze shell out
//! to the Rust compiler in this repository.

mod highlight;
mod nav;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use eframe::egui::{
    self, Button, CentralPanel, Checkbox, Color32, DragValue, Id, Label, ProgressBar, RadioButton,
    Rect, RichText, ScrollArea, Sense, Slider, Stroke, TextEdit, Vec2,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PAPER: Color32 = Color32::from_rgb(0xE7, 0xE4, 0xDE);
const INK: Color32 = Color32::from_rgb(0x1E, 0x1D, 0x1B);
const SELECT: Color32 = Color32::from_rgb(0x2F, 0x6F, 0xAD);

struct Kind {
    name: &'static str,
    group: &'static str,
    w: i32,
    h: i32,
}

const KINDS: &[Kind] = &[
    Kind { name: "Label", group: "Text", w: 200, h: 36 },
    Kind { name: "Clock", group: "Text", w: 200, h: 36 },
    Kind { name: "Button", group: "Buttons", w: 160, h: 44 },
    Kind { name: "Toggle", group: "Buttons", w: 120, h: 44 },
    Kind { name: "Edit", group: "Input", w: 240, h: 44 },
    Kind { name: "Memo", group: "Input", w: 240, h: 96 },
    Kind { name: "Number", group: "Input", w: 160, h: 44 },
    Kind { name: "Password", group: "Input", w: 240, h: 44 },
    Kind { name: "Email", group: "Input", w: 240, h: 44 },
    Kind { name: "Phone", group: "Input", w: 200, h: 44 },
    Kind { name: "Check", group: "Input", w: 180, h: 36 },
    Kind { name: "Radio", group: "Input", w: 180, h: 36 },
    Kind { name: "Switch", group: "Input", w: 180, h: 36 },
    Kind { name: "Seek", group: "Range", w: 240, h: 36 },
    Kind { name: "Progress", group: "Range", w: 240, h: 24 },
    Kind { name: "Busy", group: "Range", w: 48, h: 48 },
    Kind { name: "Rating", group: "Range", w: 200, h: 40 },
    Kind { name: "Picker", group: "Pickers", w: 120, h: 160 },
    Kind { name: "Time", group: "Pickers", w: 240, h: 180 },
    Kind { name: "Date", group: "Pickers", w: 320, h: 280 },
    Kind { name: "Panel", group: "Layout", w: 200, h: 80 },
    Kind { name: "List", group: "Lists", w: 328, h: 200 },
    Kind { name: "Spinner", group: "Lists", w: 240, h: 48 },
    Kind { name: "Web", group: "Web", w: 328, h: 200 },
];

fn kind_events(kind: &str) -> &'static [&'static str] {
    match kind {
        "List" => &["OnItemClick", "OnItemLongClick"],
        "Spinner" => &["OnItemSelected"],
        "Edit" | "Memo" | "Number" | "Password" | "Email" | "Phone" | "Check" | "Radio"
        | "Switch" | "Toggle" | "Seek" | "Rating" | "Picker" | "Time" | "Date" => {
            &["OnClick", "OnLongClick", "OnChange"]
        }
        _ => &["OnClick", "OnLongClick"],
    }
}

fn kind_props(kind: &str) -> &'static [&'static str] {
    match kind {
        "Label" => &["Text", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "Clock" => &["Format", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "Button" => &["Text", "TextSize", "TextColor", "Color", "Enabled", "Visible"],
        "Toggle" => &["TextOn", "TextOff", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        "Edit" | "Memo" | "Number" | "Password" | "Email" | "Phone" => {
            &["Text", "Hint", "TextSize", "TextColor", "Color", "Enabled", "Visible"]
        }
        "Check" | "Switch" | "Radio" => &["Text", "Checked", "TextSize", "TextColor", "Enabled", "Visible"],
        "Seek" | "Progress" => &["Max", "Progress", "Enabled", "Visible"],
        "Busy" => &["Enabled", "Visible"],
        "Rating" => &["Stars", "Rating", "Enabled", "Visible"],
        "Picker" => &["MinValue", "MaxValue", "Value", "Wrap", "Enabled", "Visible"],
        "Time" => &["Is24Hour", "Hour", "Minute", "Enabled", "Visible"],
        "Date" => &["Year", "Month", "Day", "Enabled", "Visible"],
        "Panel" => &["Direction", "Color", "Enabled", "Visible"],
        "List" => &["Items", "Color", "Enabled", "Visible"],
        "Spinner" => &["Items", "Selection", "Color", "Enabled", "Visible"],
        "Web" => &["Html", "Url", "Enabled", "Visible"],
        _ => &[],
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Form {
    name: String,
    #[serde(default)]
    title: String,
    #[serde(default = "default_bg")]
    background: String,
    #[serde(default = "default_w")]
    width: i32,
    #[serde(default = "default_h")]
    height: i32,
    #[serde(default)]
    events: BTreeMap<String, String>,
    #[serde(default)]
    components: Vec<Component>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Component {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    /// Empty means the form. Otherwise the name of the panel that holds this widget.
    #[serde(default)]
    parent: String,
    #[serde(default)]
    props: BTreeMap<String, Value>,
    #[serde(default)]
    events: BTreeMap<String, String>,
}

fn default_bg() -> String {
    "#F7F8FA".into()
}
fn default_w() -> i32 {
    360
}
fn default_h() -> i32 {
    640
}

struct FormDoc {
    form: Form,
    form_path: PathBuf,
    source_path: PathBuf,
    source_rel: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Design,
    Code,
    Explore,
}

#[derive(Clone)]
enum DragKind {
    Move,
    Resize,
}

#[derive(Clone)]
struct Drag {
    name: String,
    kind: DragKind,
    origin: egui::Pos2,
    phone: egui::Pos2,
    x0: i32,
    y0: i32,
    scale: f32,
}

struct Job {
    kind: String,
    rx: Receiver<JobOut>,
}

struct JobOut {
    code: i32,
    stdout: String,
    stderr: String,
}

struct Gesture {
    select: Option<String>,
    handler: Option<String>,
    drag: Option<Drag>,
    preview_click: Option<String>,
}

struct Studio {
    repo: PathBuf,
    project: PathBuf,
    project_name: String,
    error: Option<String>,
    forms: Vec<FormDoc>,
    buffers: BTreeMap<String, String>,
    form_index: usize,
    open_rel: String,
    mode: Mode,
    selected: Option<String>,
    pinned: bool,
    dirty: bool,
    status: String,
    filter: String,
    /// Project-wide text search on the Explore page.
    file_query: String,
    db: Value,
    focus: Option<i64>,
    drag: Option<Drag>,
    preview: bool,
    preview_text: BTreeMap<String, String>,
    preview_check: BTreeMap<String, bool>,
    preview_num: BTreeMap<String, i32>,
    log: String,
    show_log: bool,
    job: Option<Job>,
    analyzed: bool,
    /// File, 1-based line, and 1-based column to show the next time that file is the code view.
    jump: Option<(String, i32, i32)>,
    /// Caller or callee selected this frame. Applied on a later frame so a double-click still hits the same row.
    arm_focus: Option<i64>,
    /// Focused entity, and every caller or callee whose next call hop is open.
    call_open: Option<(i64, BTreeSet<i64>)>,
    find_on: bool,
    find_query: String,
    find_at: Option<usize>,
    find_origin: usize,
    find_focus: bool,
    pending_select: Option<(String, usize, usize)>,
    caret: usize,
    sel: Option<(usize, usize)>,
    quick_on: bool,
    quick_query: String,
    quick_index: usize,
    quick_focus: bool,
}

impl Studio {
    fn new(cc: &eframe::CreationContext<'_>, project: PathBuf) -> Self {
        cc.egui_ctx.all_styles_mut(|style| {
            style.visuals = egui::Visuals::light();
            style.visuals.panel_fill = PAPER;
            style.visuals.window_fill = PAPER;
            style.visuals.extreme_bg_color = Color32::from_rgb(0xFB, 0xF9, 0xF4);
            style.visuals.selection.bg_fill = Color32::from_rgb(0xD5, 0xE4, 0xF2);
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        });
        let repo = find_repo();
        match load_project(&project) {
            Ok(loaded) => Self {
                repo,
                project: loaded.path,
                project_name: loaded.project_name,
                error: None,
                forms: loaded.forms,
                buffers: loaded.buffers,
                form_index: loaded.form_index,
                open_rel: loaded.open_rel,
                mode: Mode::Design,
                selected: None,
                pinned: false,
                dirty: false,
                status: "Opening…".into(),
                filter: String::new(),
                file_query: String::new(),
                db: json!({"entities": [], "refs": [], "cfgs": {}}),
                focus: None,
                drag: None,
                preview: false,
                preview_text: BTreeMap::new(),
                preview_check: BTreeMap::new(),
                preview_num: BTreeMap::new(),
                log: String::new(),
                show_log: false,
                job: None,
                analyzed: false,
                jump: None,
                arm_focus: None,
                call_open: None,
                find_on: false,
                find_query: String::new(),
                find_at: None,
                find_origin: 0,
                find_focus: false,
                pending_select: None,
                caret: 0,
                sel: None,
                quick_on: false,
                quick_query: String::new(),
                quick_index: 0,
                quick_focus: false,
            },
            Err(err) => Self {
                repo,
                project,
                project_name: "Droide".into(),
                error: Some(err),
                forms: Vec::new(),
                buffers: BTreeMap::new(),
                form_index: 0,
                open_rel: String::new(),
                mode: Mode::Design,
                selected: None,
                pinned: false,
                dirty: false,
                status: "No project".into(),
                filter: String::new(),
                file_query: String::new(),
                db: json!({"entities": [], "refs": [], "cfgs": {}}),
                focus: None,
                drag: None,
                preview: false,
                preview_text: BTreeMap::new(),
                preview_check: BTreeMap::new(),
                preview_num: BTreeMap::new(),
                log: String::new(),
                show_log: false,
                job: None,
                analyzed: true,
                jump: None,
                arm_focus: None,
                call_open: None,
                find_on: false,
                find_query: String::new(),
                find_at: None,
                find_origin: 0,
                find_focus: false,
                pending_select: None,
                caret: 0,
                sel: None,
                quick_on: false,
                quick_query: String::new(),
                quick_index: 0,
                quick_focus: false,
            },
        }
    }

    fn form_name(&self) -> &str {
        self.forms
            .get(self.form_index)
            .map(|f| f.form.name.as_str())
            .unwrap_or("—")
    }

    fn mode_name(&self) -> &'static str {
        match self.mode {
            Mode::Design => "design",
            Mode::Code => "code",
            Mode::Explore => "explore",
        }
    }

    fn inspector_open(&self) -> bool {
        self.pinned || self.selected.is_some()
    }

    fn current_form(&self) -> Option<&Form> {
        self.forms.get(self.form_index).map(|f| &f.form)
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let save = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
        let compile = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F9));
        let run = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F9));
        let toggle = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F12));
        let explore = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F4));
        let preview = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F8));
        let pin = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F11));
        let find = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
        let quick = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::P));
        let explore_word = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F3));
        let goto = !self.quick_on && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        let duplicate = self.mode == Mode::Design
            && !self.quick_on
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D));
        let escape = (self.quick_on || self.find_on)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if save {
            self.save();
        }
        if compile {
            self.begin_job("build");
        }
        if run {
            self.begin_job("run");
        }
        if toggle {
            self.mode = if self.mode == Mode::Code { Mode::Design } else { Mode::Code };
        }
        if explore {
            self.mode = Mode::Explore;
        }
        if preview {
            self.open_preview();
        }
        if pin {
            self.pinned = !self.pinned;
        }
        if escape {
            if self.quick_on {
                self.quick_on = false;
            } else {
                self.find_on = false;
                self.find_at = None;
            }
        }
        if find {
            self.open_find();
        }
        if quick {
            self.open_quick();
        }
        if explore_word && self.mode == Mode::Code {
            self.explore_caret();
        }
        if goto && self.mode == Mode::Code {
            self.go_to_declaration_at_caret();
        }
        if duplicate {
            self.duplicate_selected();
        }
        if self.quick_on {
            let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
            let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
            let explore = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
            let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let files: Vec<String> = self.buffers.keys().cloned().collect();
            let entities = self.entities();
            let count = nav::quick_items(&self.quick_query, &files, &entities).len();
            if down && count > 0 {
                self.quick_index = (self.quick_index + 1).min(count - 1);
            }
            if up {
                self.quick_index = self.quick_index.saturating_sub(1);
            }
            if explore {
                self.activate_quick(self.quick_index, true);
            } else if enter {
                self.activate_quick(self.quick_index, false);
            }
        }
    }

    fn apply_drag(&mut self, ctx: &egui::Context) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let (pos, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
        if !down {
            if matches!(drag.kind, DragKind::Move) {
                if let Some(pos) = pos {
                    let dx = ((pos.x - drag.origin.x) / drag.scale).round() as i32;
                    let dy = ((pos.y - drag.origin.y) / drag.scale).round() as i32;
                    if (dx != 0 || dy != 0) && drag.scale > 0.0 {
                        let x = ((pos.x - drag.phone.x) / drag.scale).round() as i32;
                        let y = ((pos.y - drag.phone.y) / drag.scale).round() as i32;
                        let fi = self.form_index;
                        if let Some(index) = self.forms[fi]
                            .form
                            .components
                            .iter()
                            .position(|c| c.name == drag.name)
                        {
                            let next = drop_parent(&self.forms[fi].form.components, index, x, y);
                            let current = self.forms[fi].form.components[index].parent.clone();
                            if !next.eq_ignore_ascii_case(&current) {
                                assign_parent(&mut self.forms[fi].form.components, index, &next);
                            }
                        }
                    }
                }
            }
            self.drag = None;
            self.dirty = true;
            return;
        }
        let Some(pos) = pos else {
            return;
        };
        let dx = ((pos.x - drag.origin.x) / drag.scale).round() as i32;
        let dy = ((pos.y - drag.origin.y) / drag.scale).round() as i32;
        let fi = self.form_index;
        let Some(comp) = self.forms[fi].form.components.iter_mut().find(|c| c.name == drag.name) else {
            return;
        };
        match drag.kind {
            DragKind::Move => {
                comp.left = snap(drag.x0 + dx).max(0);
                comp.top = snap(drag.y0 + dy).max(0);
            }
            DragKind::Resize => {
                comp.width = snap(drag.x0 + dx).max(24);
                comp.height = snap(drag.y0 + dy).max(16);
            }
        }
    }

    fn save(&mut self) -> bool {
        if let Err(err) = save_project(&self.forms, &self.buffers) {
            self.status = err;
            return false;
        }
        self.dirty = false;
        self.status = "Saved".into();
        true
    }

    fn begin_job(&mut self, kind: &str) {
        if self.job.is_some() {
            self.status = "Busy".into();
            return;
        }
        if self.error.is_some() {
            return;
        }
        if self.dirty && !self.save() {
            return;
        }
        let mut args = vec![kind.to_string(), self.project.display().to_string()];
        if kind == "analyze" {
            args = vec![
                "analyze".into(),
                "--json".into(),
                self.project.display().to_string(),
            ];
        }
        let repo = self.repo.clone();
        self.status = match kind {
            "build" => "Compiling…".into(),
            "run" => "Launching…".into(),
            _ => "Analyzing…".into(),
        };
        self.job = Some(Job {
            kind: kind.into(),
            rx: spawn_job(repo, args),
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if !self.analyzed && self.job.is_none() && self.error.is_none() {
            self.analyzed = true;
            self.begin_job("analyze");
        }
        let Some(job) = self.job.take() else {
            return;
        };
        match job.rx.try_recv() {
            Ok(out) => self.finish_job(job.kind, out),
            Err(TryRecvError::Empty) => {
                self.job = Some(job);
                ctx.request_repaint_after(Duration::from_millis(80));
            }
            Err(TryRecvError::Disconnected) => {
                self.status = "The compiler job stopped.".into();
            }
        }
    }

    fn finish_job(&mut self, kind: String, out: JobOut) {
        let text = format!("{}{}", out.stdout, out.stderr);
        if kind == "analyze" {
            match serde_json::from_str::<Value>(out.stdout.trim()) {
                Ok(payload) => {
                    self.db = payload.get("db").cloned().unwrap_or(json!({}));
                    let ok = payload.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                    if ok {
                        self.status = "Indexed".into();
                    } else {
                        self.status = "Analyze found errors".into();
                        self.log = format_errors(payload.get("errors"));
                        self.show_log = true;
                    }
                    if self.focus.is_none() {
                        self.focus = self
                            .entities()
                            .into_iter()
                            .find(|e| jstr(e, "qname") == "native.fib")
                            .map(|e| jnum(&e, "id"));
                    }
                }
                Err(_) => {
                    self.status = "Analyze failed".into();
                    self.log = text;
                    self.show_log = true;
                }
            }
            return;
        }
        self.log = text.trim().to_string();
        if out.code == 0 {
            let apk = out.stdout.lines().find(|line| line.starts_with("apk ")).unwrap_or("");
            self.status = if kind == "run" {
                "Run finished".into()
            } else if apk.is_empty() {
                "APK ready".into()
            } else {
                apk.to_string()
            };
        } else {
            self.show_log = true;
            self.status = if kind == "run" { "Run failed".into() } else { "Build failed".into() };
        }
    }

    fn entities(&self) -> Vec<Value> {
        self.db
            .get("entities")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    fn refs(&self) -> Vec<Value> {
        self.db
            .get("refs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    fn entity(&self, id: i64) -> Option<Value> {
        self.entities().into_iter().find(|e| jnum(e, "id") == id)
    }

    fn add_widget(&mut self, spec: &Kind) {
        let fi = self.form_index;
        let parent = self
            .forms
            .get(fi)
            .map(|doc| host_panel(&doc.form, self.selected.as_deref()))
            .unwrap_or_default();
        let Some(doc) = self.forms.get_mut(fi) else {
            return;
        };
        let name = unique_name(&doc.form, spec.name);
        let n = doc.form.components.len() as i32;
        let (left, top) = if parent.is_empty() {
            (16, (24 + n * 12).min(560))
        } else {
            (8, 8)
        };
        let comp = Component {
            kind: spec.name.into(),
            name: name.clone(),
            left,
            top,
            width: spec.w,
            height: spec.h,
            parent,
            props: default_props(spec.name),
            events: BTreeMap::new(),
        };
        doc.form.components.push(comp);
        self.selected = Some(name);
        self.dirty = true;
    }

    fn duplicate_selected(&mut self) {
        if self.mode != Mode::Design {
            self.status = "Duplicate works on the form".into();
            return;
        }
        let Some(sel) = self.selected.clone() else {
            self.status = "Select a widget to duplicate".into();
            return;
        };
        let fi = self.form_index;
        let Some(copy) = self
            .forms
            .get(fi)
            .and_then(|doc| duplicate_component(&doc.form, &sel))
        else {
            self.status = "Select a widget to duplicate".into();
            return;
        };
        let name = copy.name.clone();
        self.forms[fi].form.components.push(copy);
        self.selected = Some(name.clone());
        self.dirty = true;
        self.status = format!("Duplicated {name}");
    }

    fn ensure_handler(&mut self, widget: &str) {
        let fi = self.form_index;
        let Some(rel) = self.forms.get(fi).map(|f| f.source_rel.clone()) else {
            return;
        };
        let Some(comp) = self.forms[fi].form.components.iter().find(|c| c.name == widget) else {
            return;
        };
        let kind = comp.kind.clone();
        let events = comp.events.clone();
        let source = self.buffers.get(&rel).cloned().unwrap_or_default();
        let plan = plan_handler(&kind, widget, &events, &source);
        if plan.changed {
            self.buffers.insert(rel.clone(), plan.source);
            if let Some(comp) = self.forms[fi].form.components.iter_mut().find(|c| c.name == widget) {
                comp.events.insert(plan.event, plan.handler.clone());
            }
            self.dirty = true;
        }
        self.open_rel = rel.clone();
        self.mode = Mode::Code;
        self.jump = Some((rel, plan.line, 1));
        self.pending_select = None;
        self.status = plan.handler;
    }

    fn open_preview(&mut self) {
        let rows: Vec<(String, String, bool, i32)> = self
            .current_form()
            .map(|form| {
                form.components
                    .iter()
                    .map(|comp| {
                        (
                            comp.name.clone(),
                            prop_s(comp, "Text"),
                            prop_bool(comp, "Checked", false),
                            numeric_prop(comp),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.preview = true;
        self.preview_text.clear();
        self.preview_check.clear();
        self.preview_num.clear();
        for (name, text, on, amount) in rows {
            self.preview_text.insert(name.clone(), text);
            self.preview_check.insert(name.clone(), on);
            self.preview_num.insert(name, amount);
        }
    }

    fn select_file(&mut self, rel: &str) {
        if self.jump.as_ref().is_some_and(|(path, _, _)| path != rel) {
            self.jump = None;
        }
        if self.pending_select.as_ref().is_some_and(|(path, _, _)| path != rel) {
            self.pending_select = None;
        }
        self.open_rel = rel.to_string();
        if let Some(i) = self.forms.iter().position(|f| f.source_rel == rel) {
            self.form_index = i;
            self.selected = None;
        }
    }

    fn open_at(&mut self, rel: &str, line: i32) {
        self.open_site(rel, line, 1);
    }

    fn open_site(&mut self, rel: &str, line: i32, col: i32) {
        let shown = line.max(1);
        let col = col.max(1);
        let Some((path, at)) = open_target(rel, i64::from(shown), &self.buffers) else {
            self.status = format!("{rel}:{shown}");
            return;
        };
        self.open_rel = path.clone();
        if let Some(i) = self.forms.iter().position(|f| f.source_rel == path) {
            self.form_index = i;
            self.selected = None;
        }
        self.mode = Mode::Code;
        self.jump = Some((path, at, col));
        self.pending_select = None;
        self.arm_focus = None;
        self.status = if col > 1 {
            format!("{rel}:{at}:{col}")
        } else {
            format!("{rel}:{at}")
        };
    }

    fn open_entity(&mut self, id: i64) {
        self.focus = Some(id);
        self.arm_focus = None;
        let Some(ent) = self.entity(id) else {
            return;
        };
        let file = jstr(&ent, "file").to_string();
        if file.is_empty() {
            return;
        }
        self.open_at(&file, jnum(&ent, "line") as i32);
    }

    fn settle_arm(&mut self) {
        if let Some(id) = self.arm_focus.take() {
            self.focus = Some(id);
        }
    }

    fn open_find(&mut self) {
        if self.quick_on {
            return;
        }
        self.mode = Mode::Code;
        let text = self.buffers.get(&self.open_rel).cloned().unwrap_or_default();
        let mut seeded = false;
        if let Some((start, end)) = self.sel {
            if end > start {
                let selected: String = text.chars().skip(start).take(end - start).collect();
                if !selected.is_empty() && !selected.contains('\n') {
                    self.find_query = selected;
                    seeded = true;
                }
            }
        }
        if !seeded && !self.find_on {
            if let Some((_, _, word)) = nav::word_at(&text, self.caret) {
                self.find_query = word;
            }
        }
        self.find_origin = self.sel.map(|(start, _)| start).unwrap_or(self.caret);
        self.find_on = true;
        self.find_at = None;
        self.find_focus = true;
    }

    fn open_quick(&mut self) {
        let seed = if self.mode == Mode::Code {
            self.buffers
                .get(&self.open_rel)
                .and_then(|text| nav::word_at(text, self.caret).map(|(_, _, word)| word))
                .unwrap_or_default()
        } else {
            self.selected.clone().unwrap_or_default()
        };
        self.quick_query = seed;
        self.quick_index = 0;
        self.quick_on = true;
        self.quick_focus = true;
    }

    fn find_bar(&mut self, ui: &mut egui::Ui, text: &str) {
        let mut query = self.find_query.clone();
        let mut moved = 0i32;
        let find_id = Id::unique("find-query");
        if ui.memory(|mem| mem.has_focus(find_id)) {
            let shift = ui.input(|input| input.modifiers.shift);
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                moved = if shift { -1 } else { 1 };
                self.find_focus = true;
            }
        }
        ui.horizontal(|ui| {
            if self.find_focus {
                ui.memory_mut(|mem| mem.request_focus(find_id));
                self.find_focus = false;
            }
            let output = TextEdit::singleline(&mut query).id(find_id).hint_text("Find").desired_width(240.0).show(ui);
            if output.response.changed() {
                self.find_at = None;
            }
            let shown = nav::find_spans(text, &query);
            let count = if query.is_empty() {
                String::new()
            } else if shown.is_empty() {
                "none".into()
            } else if let Some(at) = self.find_at {
                let idx = shown.iter().position(|(start, _)| *start == at).unwrap_or(0);
                format!("{} of {}", idx + 1, shown.len())
            } else {
                shown.len().to_string()
            };
            ui.label(count);
            if ui.button("Previous").clicked() {
                moved = -1;
            }
            if ui.button("Next").clicked() {
                moved = 1;
            }
        });
        if query != self.find_query {
            self.find_query = query;
            self.find_at = None;
        }
        if moved != 0 {
            self.move_find(moved);
        }
    }

    fn sync_find(&mut self, rel: &str, text: &str) {
        if !self.find_on {
            return;
        }
        let spans = nav::find_spans(text, &self.find_query);
        let valid = self.find_at.is_some_and(|at| spans.iter().any(|(start, _)| *start == at));
        if valid {
            return;
        }
        let Some(at) = nav::find_from(&spans, self.find_origin) else {
            self.find_at = None;
            return;
        };
        self.find_at = Some(at);
        self.arm_find(rel, text, at, &spans);
    }

    fn arm_find(&mut self, rel: &str, text: &str, at: usize, spans: &[(usize, usize)]) {
        let Some((_, end)) = spans.iter().find(|(start, _)| *start == at) else {
            return;
        };
        self.pending_select = Some((rel.to_string(), at, *end));
        self.jump = Some((rel.to_string(), highlight::line_of_char(text, at) as i32, 1));
    }

    fn move_find(&mut self, dir: i32) {
        let rel = self.open_rel.clone();
        let text = self.buffers.get(&rel).cloned().unwrap_or_default();
        let spans = nav::find_spans(&text, &self.find_query);
        let Some(at) = nav::find_move(&spans, self.find_at, dir) else {
            self.find_at = None;
            return;
        };
        self.find_at = Some(at);
        self.arm_find(&rel, &text, at, &spans);
    }

    fn go_to_declaration_at_caret(&mut self) {
        let Some(text) = self.buffers.get(&self.open_rel).cloned() else {
            return;
        };
        self.go_to_declaration(&text, self.caret);
    }

    fn go_to_declaration(&mut self, text: &str, index: usize) {
        let Some((_, _, word)) = nav::word_at(text, index) else {
            return;
        };
        let file = self.open_rel.clone();
        let line = highlight::line_of_char(text, index) as i64;
        let entities = self.entities();
        let refs = self.refs();
        match nav::declaration_id(&entities, &refs, &file, line, &word) {
            Some(id) => {
                if let Some(ent) = entities.into_iter().find(|ent| jnum(ent, "id") == id) {
                    self.open_declaration(&ent);
                }
            }
            None => self.status = format!("No declaration for {word}"),
        }
    }

    fn explore_caret(&mut self) {
        let Some(text) = self.buffers.get(&self.open_rel).cloned() else {
            return;
        };
        let line = highlight::line_of_char(&text, self.caret) as i64;
        let word = nav::word_at(&text, self.caret).map(|(_, _, word)| word).unwrap_or_default();
        let entities = self.entities();
        let refs = self.refs();
        if let Some(id) = nav::context_id(&entities, &refs, &self.open_rel, line, &word) {
            self.focus = Some(id);
            self.arm_focus = None;
            self.mode = Mode::Explore;
        } else {
            self.status = "Nothing under the caret".into();
        }
    }

    fn open_declaration(&mut self, ent: &Value) {
        let id = jnum(ent, "id");
        let kind = jstr(ent, "kind").to_string();
        let name = jstr(ent, "name").to_string();
        let file = jstr(ent, "file").to_string();
        if kind == "builtin" || file.is_empty() {
            self.status = format!("{name} is built in");
            return;
        }
        self.focus = Some(id);
        self.arm_focus = None;
        if kind == "widget" {
            if let Some(i) = self.forms.iter().position(|form| form.source_rel == file) {
                self.form_index = i;
                self.open_rel = file;
                self.selected = Some(name.clone());
                self.mode = Mode::Design;
                self.jump = None;
                self.pending_select = None;
                self.status = name;
                return;
            }
        }
        self.open_at(&file, jnum(ent, "line") as i32);
    }

    fn activate_quick(&mut self, index: usize, explore: bool) {
        let files: Vec<String> = self.buffers.keys().cloned().collect();
        let entities = self.entities();
        let items = nav::quick_items(&self.quick_query, &files, &entities);
        let Some(item) = items.get(index).cloned() else {
            self.quick_on = false;
            return;
        };
        self.quick_on = false;
        if explore {
            if let Some(id) = item.entity_id {
                self.focus = Some(id);
                self.arm_focus = None;
                self.mode = Mode::Explore;
                return;
            }
        }
        if item.kind == "file" {
            self.select_file(&item.file);
            self.mode = Mode::Code;
            return;
        }
        if let Some(id) = item.entity_id {
            if let Some(ent) = self.entity(id) {
                self.open_declaration(&ent);
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Droide").strong().color(INK));
            ui.label(RichText::new(&self.project_name).color(INK));
            for (i, doc) in self.forms.iter().enumerate() {
                let on = i == self.form_index && self.mode == Mode::Design;
                if ui.selectable_label(on, &doc.form.name).clicked() {
                    self.form_index = i;
                    self.open_rel = doc.source_rel.clone();
                    self.selected = None;
                    self.mode = Mode::Design;
                    self.jump = None;
                    self.arm_focus = None;
                }
            }
            ui.separator();
            if ui.selectable_label(self.mode == Mode::Design, "Design").clicked() {
                self.mode = Mode::Design;
            }
            if ui.selectable_label(self.mode == Mode::Code, "Code").clicked() {
                self.mode = Mode::Code;
            }
            if ui.selectable_label(self.mode == Mode::Explore, "Explore").clicked() {
                self.mode = Mode::Explore;
            }
            ui.separator();
            if ui
                .button("Duplicate")
                .on_hover_text("Ctrl+D copies the selected widget on the form")
                .clicked()
            {
                self.duplicate_selected();
            }
            if ui.button("Compile").on_hover_text("Ctrl+F9").clicked() {
                self.begin_job("build");
            }
            if ui.button("Preview").on_hover_text("F8").clicked() {
                self.open_preview();
            }
            if ui
                .button("Run")
                .on_hover_text("F9 builds the APK and launches it. With no device, it starts the emulator.")
                .clicked()
            {
                self.begin_job("run");
            }
        });
    }

    fn palette(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("Widgets").small().weak());
            let mut last = "";
            for spec in KINDS {
                if spec.group != last {
                    ui.add_space(8.0);
                    ui.label(RichText::new(spec.group).small().weak());
                    last = spec.group;
                }
                if ui
                    .add_sized(Vec2::new(ui.available_width(), 28.0), Button::new(spec.name))
                    .clicked()
                {
                    self.add_widget(spec);
                }
            }
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui, design: bool) {
        let Some(form) = self.current_form().cloned() else {
            ui.label("This project has no form.");
            return;
        };
        let scale = fit_scale(ui.available_size(), form.width, form.height);
        let selected = self.selected.clone();
        let mut preview_text = self.preview_text.clone();
        let mut preview_check = self.preview_check.clone();
        let mut preview_num = self.preview_num.clone();
        let gesture = paint_phone(
            ui,
            &form,
            scale,
            design,
            selected.as_deref(),
            &mut preview_text,
            &mut preview_check,
            &mut preview_num,
        );
        if !design {
            self.preview_text = preview_text;
            self.preview_check = preview_check;
            self.preview_num = preview_num;
        }
        if let Some(name) = gesture.select {
            self.selected = Some(name);
        }
        if let Some(drag) = gesture.drag {
            self.drag = Some(drag);
        }
        if let Some(name) = gesture.handler {
            self.ensure_handler(&name);
        }
        if let Some(name) = gesture.preview_click {
            self.status = format!("{name}");
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Inspector").small().weak());
                let Some(sel) = self.selected.clone() else {
                    self.form_fields(ui);
                    return;
                };
                let fi = self.form_index;
                let Some(pos) = self.forms[fi].form.components.iter().position(|c| c.name == sel) else {
                    self.selected = None;
                    return;
                };
                let mut name = self.forms[fi].form.components[pos].name.clone();
                ui.label("Name");
                if ui.text_edit_singleline(&mut name).changed() && !name.is_empty() {
                    self.forms[fi].form.components[pos].name = name.clone();
                    self.selected = Some(name);
                    self.dirty = true;
                }
                let kind = self.forms[fi].form.components[pos].kind.clone();
                ui.label(RichText::new(&kind).small().weak());
                let parent_now = self.forms[fi].form.components[pos].parent.clone();
                let choices = parent_choices(&self.forms[fi].form.components, pos);
                let shown = if parent_now.is_empty() {
                    "(form)".to_string()
                } else {
                    parent_now.clone()
                };
                let mut picked = None;
                ui.label("Parent");
                egui::ComboBox::from_id_salt(("widget-parent", sel.clone()))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(parent_now.is_empty(), "(form)").clicked() {
                            picked = Some(String::new());
                        }
                        for name in &choices {
                            if ui.selectable_label(parent_now.eq_ignore_ascii_case(name), name).clicked() {
                                picked = Some(name.clone());
                            }
                        }
                    });
                if let Some(name) = picked {
                    if name.is_empty() {
                        if !parent_now.is_empty() {
                            assign_parent(&mut self.forms[fi].form.components, pos, "");
                            self.dirty = true;
                        }
                    } else if !name.eq_ignore_ascii_case(&parent_now) {
                        assign_parent(&mut self.forms[fi].form.components, pos, &name);
                        self.dirty = true;
                    }
                }
                let mut dirty = false;
                {
                    let comp = &mut self.forms[fi].form.components[pos];
                    ui.label("Place");
                    ui.horizontal(|ui| {
                        ui.label("L");
                        if ui.add(DragValue::new(&mut comp.left).speed(1.0)).changed() {
                            dirty = true;
                        }
                        ui.label("T");
                        if ui.add(DragValue::new(&mut comp.top).speed(1.0)).changed() {
                            dirty = true;
                        }
                        ui.label("W");
                        if ui.add(DragValue::new(&mut comp.width).speed(1.0)).changed() {
                            dirty = true;
                        }
                        ui.label("H");
                        if ui.add(DragValue::new(&mut comp.height).speed(1.0)).changed() {
                            dirty = true;
                        }
                    });
                    let props = kind_props(&kind);
                    for prop in ["Text", "Hint", "Format", "TextOn", "TextOff", "Html", "Url", "Items"] {
                        if props.contains(&prop) {
                            dirty |= edit_string(ui, comp, prop);
                        }
                    }
                    for prop in [
                        "TextSize", "Max", "Progress", "Stars", "Rating", "MinValue", "MaxValue", "Value",
                        "Hour", "Minute", "Year", "Month", "Day", "Selection",
                    ] {
                        if props.contains(&prop) {
                            dirty |= edit_number(ui, comp, prop);
                        }
                    }
                    if props.contains(&"TextColor") {
                        dirty |= edit_string(ui, comp, "TextColor");
                    }
                    if props.contains(&"Color") {
                        dirty |= edit_string(ui, comp, "Color");
                    }
                    if props.contains(&"Direction") {
                        let mut dir = prop_s(comp, "Direction");
                        if dir.is_empty() {
                            dir = "Vertical".into();
                        }
                        let mut next_dir = None;
                        ui.label("Direction");
                        egui::ComboBox::from_id_salt(("widget-direction", comp.name.clone()))
                            .selected_text(&dir)
                            .show_ui(ui, |ui| {
                                for choice in ["Vertical", "Horizontal"] {
                                    if ui.selectable_label(dir.eq_ignore_ascii_case(choice), choice).clicked() {
                                        next_dir = Some(choice);
                                    }
                                }
                            });
                        if let Some(choice) = next_dir {
                            comp.props.insert("Direction".into(), json!(choice));
                            dirty = true;
                        }
                    }
                    if props.contains(&"Checked") {
                        dirty |= edit_flag(ui, comp, "Checked", false);
                    }
                    if props.contains(&"Wrap") {
                        dirty |= edit_flag(ui, comp, "Wrap", true);
                    }
                    if props.contains(&"Is24Hour") {
                        dirty |= edit_flag(ui, comp, "Is24Hour", true);
                    }
                    if props.contains(&"Enabled") {
                        dirty |= edit_flag(ui, comp, "Enabled", true);
                    }
                    if props.contains(&"Visible") {
                        dirty |= edit_flag(ui, comp, "Visible", true);
                    }
                    let mut event_names: Vec<String> =
                        kind_events(&kind).iter().map(|name| (*name).to_string()).collect();
                    for key in comp.events.keys() {
                        if !event_names.iter().any(|name| name == key) {
                            event_names.push(key.clone());
                        }
                    }
                    for event in event_names {
                        ui.push_id(&event, |ui| {
                            ui.label(&event);
                            let mut text = comp.events.get(&event).cloned().unwrap_or_default();
                            if ui.text_edit_singleline(&mut text).changed() {
                                if text.is_empty() {
                                    comp.events.remove(&event);
                                } else {
                                    comp.events.insert(event.clone(), text);
                                }
                                dirty = true;
                            }
                        });
                    }
                }
                if dirty {
                    self.dirty = true;
                }
        });
    }

    fn form_fields(&mut self, ui: &mut egui::Ui) {
        let fi = self.form_index;
        if self.forms.get(fi).is_none() {
            return;
        }
        let mut dirty = false;
        {
            let doc = &mut self.forms[fi];
            ui.label("Form");
            ui.label("Title");
            if ui.text_edit_singleline(&mut doc.form.title).changed() {
                dirty = true;
            }
            ui.label("OnCreate");
            let mut create = doc.form.events.get("OnCreate").cloned().unwrap_or_default();
            if ui.text_edit_singleline(&mut create).changed() {
                if create.is_empty() {
                    doc.form.events.remove("OnCreate");
                } else {
                    doc.form.events.insert("OnCreate".into(), create);
                }
                dirty = true;
            }
        }
        if dirty {
            self.dirty = true;
        }
    }

    fn files(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("Files").small().weak());
            let rels: Vec<String> = self.buffers.keys().cloned().collect();
            for rel in rels {
                if ui.selectable_label(rel == self.open_rel, &rel).clicked() {
                    self.select_file(&rel);
                }
            }
        });
    }

    fn routine_bar(&mut self, ui: &mut egui::Ui, rel: &str, text: &str) {
        let routines = nav::routines(text);
        let line = highlight::line_of_char(text, self.caret) as i32;
        let current = nav::routine_index(&routines, line);
        let selected = current
            .map(|index| routines[index].name.as_str())
            .unwrap_or("(declarations)");
        let mut jump_line = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Procedure").small().weak());
            egui::ComboBox::from_id_salt("procedure")
                .selected_text(selected)
                .width(240.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "(declarations)").clicked() {
                        jump_line = Some(1);
                    }
                    for (index, routine) in routines.iter().enumerate() {
                        if ui.selectable_label(current == Some(index), &routine.name).clicked() {
                            jump_line = Some(routine.line);
                        }
                    }
                });
        });
        if let Some(line) = jump_line {
            self.jump = Some((rel.to_string(), line, 1));
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let rel = self.open_rel.clone();
        let mut text = self.buffers.get(&rel).cloned().unwrap_or_default();
        self.routine_bar(ui, &rel, &text);
        let lang = highlight::source_lang(&rel);
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let row_h = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
        let ppp = ui.pixels_per_point();
        let line_h = (row_h * ppp).round().max(1.0) / ppp;
        if self.find_on {
            self.find_bar(ui, &text);
        }
        self.sync_find(&rel, &text);
        let find_spans = if self.find_on {
            nav::find_spans(&text, &self.find_query)
        } else {
            Vec::new()
        };
        let hit_bytes: Vec<(usize, usize)> = find_spans
            .iter()
            .map(|(start, end)| (highlight::byte_of_char(&text, *start), highlight::byte_of_char(&text, *end)))
            .collect();
        let current_hit = self.find_at.and_then(|at| {
            find_spans.iter().find(|(start, _)| *start == at).map(|(start, end)| {
                (highlight::byte_of_char(&text, *start), highlight::byte_of_char(&text, *end))
            })
        });
        let full = ui.available_rect_before_wrap();
        let line_count = text.split('\n').count().max(1);
        let gutter_w = highlight::gutter_width(ui, &font_id, line_count).min(full.width() * 0.34);
        let gutter_rect = Rect::from_min_size(full.left_top(), egui::vec2(gutter_w, full.height()));
        let editor_rect = Rect::from_min_max(egui::pos2(gutter_rect.right(), full.top()), full.right_bottom());
        let code_bg = Color32::from_rgb(0xFB, 0xF9, 0xF4);
        ui.painter().rect_filled(full, 0.0, code_bg);

        let pointer = ui.input(|input| input.pointer.hover_pos());
        let wheel_y = ui.input(|input| input.smooth_scroll_delta().y);
        let gutter_hot = pointer.is_some_and(|pos| gutter_rect.contains(pos));
        let picked = match self.pending_select.clone() {
            Some((path, start, end)) if path == rel => {
                self.pending_select = None;
                Some((start, end))
            }
            _ => None,
        };
        let jump_to = match self.jump.clone() {
            Some((path, line, col)) if path == rel => {
                self.jump = None;
                Some((line, col))
            }
            _ => None,
        };

        let mut editor_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(editor_rect)
                .layout(egui::Layout::top_down(egui::Align::Min))
                .id_salt(("code-view", rel.clone())),
        );
        let edit_id = Id::unique(("source", rel.clone()));
        let mut area = ScrollArea::both()
            .id_salt(("code-scroll", rel.clone()))
            .auto_shrink([false, false]);
        let scroll_line = picked
            .map(|(start, _)| highlight::line_of_char(&text, start) as i32)
            .or_else(|| jump_to.map(|(line, _)| line));
        if let Some(line) = scroll_line {
            area = area.vertical_scroll_offset(jump_scroll_y(line, line_h));
            let mut state = egui::widgets::text_edit::TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
            let range = if let Some((start, end)) = picked {
                egui::text::CCursorRange::two(egui::text::CCursor::new(start), egui::text::CCursor::new(end))
            } else {
                let (_, col) = jump_to.unwrap_or((line, 1));
                let index = highlight::char_index_at(&text, line.max(1) as usize, col.max(1) as usize);
                egui::text::CCursorRange::one(egui::text::CCursor::new(index))
            };
            state.cursor.set_char_range(Some(range));
            state.store(ui.ctx(), edit_id);
            if picked.is_none() {
                ui.ctx().memory_mut(|mem| mem.request_focus(edit_id));
            }
        } else if gutter_hot && wheel_y != 0.0 {
            let scroll_id = editor_ui.make_persistent_id(egui::IdSalt::new(("code-scroll", rel.clone())));
            if let Some(state) = egui::scroll_area::State::load(editor_ui.ctx(), scroll_id) {
                let y = (state.clamped_offset().y - wheel_y).max(0.0);
                area = area.vertical_scroll_offset(y);
            }
        }
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
            let job = highlight::layout_job_marked(buf.as_str(), lang, font_id.clone(), line_h, &hit_bytes, current_hit);
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let shown = area.show(&mut editor_ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let width = ui.available_width();
            let width = if width.is_finite() { width.max(24.0) } else { editor_rect.width().max(24.0) };
            TextEdit::multiline(&mut text)
                .code_editor()
                .frame(
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .fill(code_bg),
                )
                .desired_width(width)
                .id(edit_id)
                .layouter(&mut layouter)
                .show(ui)
        });
        ui.advance_cursor_after_rect(full);

        let output = shown.inner;
        let mut cursor_line = output
            .cursor_range
            .as_ref()
            .map(|range| highlight::line_of_char(&text, range.primary.index.0));
        if cursor_line.is_none() {
            cursor_line = picked
                .map(|(start, _)| highlight::line_of_char(&text, start))
                .or_else(|| jump_to.map(|(line, _)| line.max(1) as usize));
        }
        highlight::paint_gutter(
            &ui.painter().with_clip_rect(gutter_rect),
            gutter_rect,
            &output.galley,
            output.galley_pos,
            &text,
            lang,
            cursor_line,
            font_id,
        );
        if let Some(range) = output.cursor_range.as_ref() {
            let primary = range.primary.index.0;
            let secondary = range.secondary.index.0;
            self.caret = primary;
            self.sel = if primary == secondary {
                None
            } else {
                Some((primary.min(secondary), primary.max(secondary)))
            };
        }
        // The menu covers the editor, so this click would also move the caret.
        if let Some((line, col)) = jump_to {
            let index = highlight::char_index_at(&text, line.max(1) as usize, col.max(1) as usize);
            let mut state = egui::widgets::text_edit::TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(index),
            )));
            state.store(ui.ctx(), edit_id);
            self.caret = index;
            self.sel = None;
        }
        if output.response.changed() {
            self.buffers.insert(rel.clone(), text.clone());
            self.dirty = true;
        }
        if output.response.clicked() && ui.input(|input| input.modifiers.command) {
            self.go_to_declaration(&text, self.caret);
        }
    }

    fn index(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Index").small().weak());
        ui.text_edit_singleline(&mut self.filter);
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let groups = nav::index_groups(&self.entities(), &self.filter);
            for group in groups {
                egui::CollapsingHeader::new(&group.file)
                    .default_open(true)
                    .show(ui, |ui| {
                    for entry in group.entries {
                        let weight = matches!(entry.kind.as_str(), "procedure" | "function" | "native");
                        let label = if weight {
                            format!("{}  {}   c{}  in{}", entry.kind, entry.name, entry.complexity, entry.fan_in)
                        } else {
                            format!("{}  {}", entry.kind, entry.name)
                        };
                        let response = ui
                            .selectable_label(self.focus == Some(entry.id), label)
                            .on_hover_text("Double-click to open");
                        if response.double_clicked() {
                            self.arm_focus = None;
                            self.open_entity(entry.id);
                        } else if response.clicked() {
                            self.arm_focus = None;
                            self.focus = Some(entry.id);
                        }
                    }
                });
            }
        });
    }

    fn explore_detail(&mut self, ui: &mut egui::Ui) {
        let mut opened = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("In files").small().weak());
            ui.add(
                egui::TextEdit::singleline(&mut self.file_query)
                    .desired_width(240.0)
                    .hint_text("search the project"),
            );
        });
        let hits = nav::project_hits(&self.buffers, &self.file_query);
        if self.file_query.trim().chars().count() >= 2 {
            if hits.is_empty() {
                ui.label("No matches.");
            }
            for hit in &hits {
                let label = format!("{}:{}   {}", hit.file, hit.line, hit.context);
                if ui.selectable_label(false, label).on_hover_text("Open this line").clicked() {
                    opened = Some((hit.file.clone(), hit.line));
                }
            }
            if hits.len() == 40 {
                ui.label(RichText::new("40 shown").small().weak());
            }
        }
        if let Some((file, line)) = opened {
            self.open_at(&file, line);
            return;
        }
        let Some(id) = self.focus else {
            ui.label("Select a routine, widget, or form.");
            self.settle_arm();
            return;
        };
        let Some(ent) = self.entity(id) else {
            ui.label("That entity is gone.");
            self.settle_arm();
            return;
        };
        let nav = ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            let mut nav = None;
            ui.heading(jstr(&ent, "qname"));
            let file = jstr(&ent, "file").to_string();
            let line = jnum(&ent, "line").max(1) as i32;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(format!("{} · ", jstr(&ent, "kind")));
                if ui.link(format!("{file}:{line}")).on_hover_text("Open this line").clicked() {
                    nav = Some(Nav::At(file.clone(), line));
                }
                let params = jnum(&ent, "params");
                let param_word = if params == 1 { "param" } else { "params" };
                ui.label(format!(
                    " · {} lines · complexity {} · depth {} · {params} {param_word} · {} in · {} out",
                    jnum(&ent, "loc"),
                    jnum(&ent, "complexity"),
                    jnum(&ent, "depth"),
                    jnum(&ent, "fanIn"),
                    jnum(&ent, "fanOut"),
                ));
            });
            let refs = self.refs();
            let mut callers = Vec::new();
            let mut callees = Vec::new();
            let mut self_call = false;
            for r in &refs {
                if jstr(r, "kind") != "call" {
                    continue;
                }
                let src = jnum(r, "src");
                let dst = jnum(r, "dst");
                if src == id && dst == id {
                    self_call = true;
                } else if src == id {
                    if let Some(other) = self.entity(dst) {
                        callees.push(other);
                    }
                } else if dst == id {
                    if let Some(other) = self.entity(src) {
                        callers.push(other);
                    }
                }
            }
            callers.sort_by(|a, b| jstr(a, "qname").cmp(jstr(b, "qname")));
            callees.sort_by(|a, b| jstr(a, "qname").cmp(jstr(b, "qname")));
            callers.dedup_by(|a, b| jnum(a, "id") == jnum(b, "id"));
            callees.dedup_by(|a, b| jnum(a, "id") == jnum(b, "id"));
            let names: BTreeMap<i64, String> = self
                .entities()
                .into_iter()
                .map(|ent| (jnum(&ent, "id"), jstr(&ent, "qname").to_string()))
                .collect();
            let open_ids = self
                .call_open
                .as_ref()
                .filter(|(focus, _)| *focus == id)
                .map(|(_, set)| set.clone())
                .unwrap_or_default();
            let mut toggle = None;
            ui.add_space(8.0);
            ui.label(RichText::new("Call graph").strong());
            let graph = ui
                .horizontal_top(|ui| {
                    let down = egui::Layout::top_down(egui::Align::Min);
                    let callers_hit = ui
                        .allocate_ui_with_layout(Vec2::new(260.0, 8.0), down, |ui| {
                            ui.label(RichText::new("Callers").small().weak());
                            if callers.is_empty() {
                                ui.label("—");
                            }
                            paint_graph_rows(ui, &callers, id, &open_ids, &refs, &names, &mut toggle)
                        })
                        .inner;
                    ui.allocate_ui_with_layout(Vec2::new(220.0, 8.0), down, |ui| {
                        ui.label(RichText::new("Focus").small().weak());
                        ui.label(jstr(&ent, "qname"));
                        if self_call {
                            ui.label("calls itself");
                        }
                    });
                    let callees_hit = ui
                        .allocate_ui_with_layout(Vec2::new(260.0, 8.0), down, |ui| {
                            ui.label(RichText::new("Callees").small().weak());
                            if callees.is_empty() {
                                ui.label("—");
                            }
                            paint_graph_rows(ui, &callees, id, &open_ids, &refs, &names, &mut toggle)
                        })
                        .inner;
                    callers_hit.or(callees_hit)
                })
                .inner;
            if graph.is_some() {
                nav = graph;
            }
            let calls = nav::sites(id, "call", true, &refs, &names, &self.buffers);
            let called = nav::sites(id, "call", false, &refs, &names, &self.buffers);
            let reads = nav::sites(id, "read", true, &refs, &names, &self.buffers);
            let writes = nav::sites(id, "write", true, &refs, &names, &self.buffers);
            let read_by = nav::sites(id, "read", false, &refs, &names, &self.buffers);
            let written_by = nav::sites(id, "write", false, &refs, &names, &self.buffers);
            let uses = nav::sites(id, "uses", true, &refs, &names, &self.buffers);
            let used_by = nav::sites(id, "uses", false, &refs, &names, &self.buffers);
            for (title, rows) in [("Calls", calls), ("Called by", called)] {
                let (hit, opened) = paint_call_sites(ui, title, &rows, id, &open_ids, &refs, &names);
                if hit.is_some() {
                    nav = hit;
                }
                if opened.is_some() {
                    toggle = opened;
                }
            }
            for (title, rows) in [
                ("Reads", reads),
                ("Writes", writes),
                ("Read by", read_by),
                ("Written by", written_by),
                ("Uses", uses),
                ("Used by", used_by),
            ] {
                if let Some(hit) = paint_sites(ui, title, &rows) {
                    nav = Some(hit);
                }
            }
            if let Some(other) = toggle {
                let mut set = self
                    .call_open
                    .take()
                    .filter(|(focus, _)| *focus == id)
                    .map(|(_, set)| set)
                    .unwrap_or_default();
                if !set.remove(&other) {
                    set.insert(other);
                }
                self.call_open = Some((id, set));
            }
            if let Some(cfg) = self.db.get("cfgs").and_then(|c| c.get(&id.to_string())) {
                let (entry, steps) = nav::flow_steps(cfg);
                if !steps.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Control flow").strong());
                    if let Some(hit) = paint_flow(ui, &file, entry, &steps) {
                        nav = Some(hit);
                    }
                }
            }
            nav
        });
        match nav.inner {
            Some(Nav::Focus(id)) => self.arm_focus = Some(id),
            Some(Nav::Open(id)) => {
                self.arm_focus = None;
                self.open_entity(id);
            }
            Some(Nav::At(file, line)) => {
                self.arm_focus = None;
                self.open_at(&file, line);
            }
            None => self.settle_arm(),
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.preview_window(ctx);
        self.log_window(ctx);
        self.quick_window(ctx);
    }

    fn quick_window(&mut self, ctx: &egui::Context) {
        if !self.quick_on {
            return;
        }
        let files: Vec<String> = self.buffers.keys().cloned().collect();
        let entities = self.entities();
        let items = nav::quick_items(&self.quick_query, &files, &entities);
        if self.quick_index >= items.len() {
            self.quick_index = items.len().saturating_sub(1);
        }
        let mut open = true;
        let mut query = self.quick_query.clone();
        let picked = std::cell::Cell::new(None::<(usize, bool)>);
        egui::Window::new("Open")
            .open(&mut open)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .default_width(520.0)
            .show(ctx, |ui| {
                let id = Id::unique("quick-query");
                if self.quick_focus {
                    ui.memory_mut(|mem| mem.request_focus(id));
                    self.quick_focus = false;
                }
                let output = TextEdit::singleline(&mut query)
                    .id(id)
                    .hint_text("File, routine, or widget")
                    .desired_width(ui.available_width())
                    .show(ui);
                if output.response.changed() {
                    self.quick_index = 0;
                }
                ui.label(RichText::new("Enter opens. Ctrl+Enter explores.").small().weak());
                ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    if items.is_empty() {
                        ui.label("Nothing matches.");
                        return;
                    }
                    for (i, item) in items.iter().enumerate() {
                        let label = format!("{:<10} {}   {}", item.kind, item.label, item.detail);
                        let response = ui.selectable_label(i == self.quick_index, label);
                        if response.clicked() {
                            picked.set(Some((i, ui.input(|input| input.modifiers.command))));
                        }
                    }
                });
            });
        self.quick_query = query;
        self.quick_on = open;
        if let Some((index, explore)) = picked.get() {
            self.activate_quick(index, explore);
        }
    }

    fn preview_window(&mut self, ctx: &egui::Context) {
        if !self.preview {
            return;
        }
        let Some(form) = self.current_form().cloned() else {
            self.preview = false;
            return;
        };
        let title = if form.title.is_empty() { form.name.clone() } else { form.title.clone() };
        let mut open = true;
        let mut preview_text = self.preview_text.clone();
        let mut preview_check = self.preview_check.clone();
        let mut preview_num = self.preview_num.clone();
        let mut gesture = Gesture::none();
        egui::Window::new(format!("{title} — egui"))
            .open(&mut open)
            .default_size([420.0, 760.0])
            .show(ctx, |ui| {
                ui.label("These are the egui widgets this studio is built with. Run compiles the same form for Android.");
                gesture = paint_phone(
                    ui,
                    &form,
                    1.0,
                    false,
                    None,
                    &mut preview_text,
                    &mut preview_check,
                    &mut preview_num,
                );
            });
        self.preview = open;
        self.preview_text = preview_text;
        self.preview_check = preview_check;
        self.preview_num = preview_num;
        if let Some(name) = gesture.preview_click {
            self.status = format!("{name}");
        }
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        if !self.show_log {
            return;
        }
        let mut open = true;
        let log = self.log.clone();
        let mut jump = None;
        egui::Window::new("Messages")
            .open(&mut open)
            .default_size([640.0, 280.0])
            .show(ctx, |ui| {
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if log.is_empty() {
                        ui.label("No messages.");
                        return;
                    }
                    for line in log.lines() {
                        if line.is_empty() {
                            ui.add_space(6.0);
                            continue;
                        }
                        if let Some((file, at, col)) = message_site(line, &self.buffers) {
                            if ui
                                .link(RichText::new(line).monospace())
                                .on_hover_text("Open this line")
                                .clicked()
                            {
                                jump = Some((file, at, col));
                            }
                        } else {
                            ui.monospace(line);
                        }
                    }
                });
            });
        self.show_log = open;
        if let Some((file, at, col)) = jump {
            self.open_site(&file, at, col);
        }
    }
}

impl Gesture {
    fn none() -> Self {
        Self {
            select: None,
            handler: None,
            drag: None,
            preview_click: None,
        }
    }
}

impl eframe::App for Studio {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.handle_keys(&ctx);
        self.apply_drag(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("Droide — {}", self.project_name)));
        if let Some(err) = &self.error {
            ui.label(RichText::new(err).color(Color32::from_rgb(0x8C, 0x2F, 0x2F)));
            return;
        }
        self.shell(ui);
        self.windows(&ctx);
    }
}

impl Studio {
    fn shell(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("bar").resizable(false).show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").resizable(false).show(ui, |ui| self.status_line(ui));
        match self.mode {
            Mode::Design => {
                egui::Panel::left("palette")
                    .exact_size(168.0)
                    .resizable(false)
                    .show(ui, |ui| self.palette(ui));
                if self.inspector_open() {
                    egui::Panel::right("inspector")
                        .exact_size(276.0)
                        .resizable(false)
                        .show(ui, |ui| self.inspector(ui));
                }
                CentralPanel::default_margins().show(ui, |ui| self.canvas(ui, true));
            }
            Mode::Code => {
                egui::Panel::left("files")
                    .exact_size(240.0)
                    .resizable(false)
                    .show(ui, |ui| self.files(ui));
                CentralPanel::default_margins().show(ui, |ui| self.editor(ui));
            }
            Mode::Explore => {
                egui::Panel::left("index")
                    .exact_size(320.0)
                    .resizable(false)
                    .show(ui, |ui| self.index(ui));
                CentralPanel::default_margins().show(ui, |ui| self.explore_detail(ui));
            }
        }
    }

    fn status_line(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mark = if self.dirty { " · unsaved" } else { "" };
            ui.label(format!("{} · {}{mark}", self.form_name(), self.mode_name()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(&self.status);
            });
        });
    }
}

fn paint_phone(
    ui: &mut egui::Ui,
    form: &Form,
    scale: f32,
    design: bool,
    selected: Option<&str>,
    preview_text: &mut BTreeMap<String, String>,
    preview_check: &mut BTreeMap<String, bool>,
    preview_num: &mut BTreeMap<String, i32>,
) -> Gesture {
    let mut gesture = Gesture::none();
    let pw = form.width as f32 * scale;
    let ph = form.height as f32 * scale;
    ui.add_space(8.0);
    let avail_x = ui.available_width();
    let pad = ((avail_x - pw) * 0.5).max(0.0);
    let mut phone = Rect::NOTHING;
    ui.horizontal(|ui| {
        ui.add_space(pad);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(pw, ph), Sense::hover());
        phone = rect;
        let prev_clip = ui.clip_rect();
        ui.set_clip_rect(prev_clip.intersect(rect));
        ui.painter().rect_filled(rect, 10, color_of(&form.background));
        ui.painter().rect_stroke(
            rect,
            10,
            Stroke::new(1.0, Color32::from_rgb(0xC8, 0xC4, 0xBC)),
            egui::StrokeKind::Inside,
        );
        for (index, comp) in form.components.iter().enumerate() {
            let (x, y) = component_origin(&form.components, index);
            let r = Rect::from_min_size(
                rect.min + egui::vec2(x as f32 * scale, y as f32 * scale),
                egui::vec2(comp.width as f32 * scale, comp.height as f32 * scale),
            );
            let mut shown = if design {
                prop_s(comp, "Text")
            } else {
                preview_text.get(&comp.name).cloned().unwrap_or_else(|| prop_s(comp, "Text"))
            };
            let mut on = if design {
                prop_bool(comp, "Checked", false)
            } else {
                preview_check.get(&comp.name).copied().unwrap_or(false)
            };
            let mut amount = if design {
                numeric_prop(comp)
            } else {
                preview_num.get(&comp.name).copied().unwrap_or_else(|| numeric_prop(comp))
            };
            let response = paint_widget(ui, comp, r, !design, &mut shown, &mut on, &mut amount);
            if !design {
                preview_text.insert(comp.name.clone(), shown);
                preview_check.insert(comp.name.clone(), on);
                preview_num.insert(comp.name.clone(), amount);
                if response.as_ref().map(|resp| resp.clicked()).unwrap_or(false) {
                    gesture.preview_click = Some(comp.name.clone());
                }
                continue;
            }
            let hit = ui.interact(r, Id::unique(("hit", &form.name, &comp.name)), Sense::click_and_drag());
            let was = selected == Some(comp.name.as_str());
            if hit.double_clicked() {
                gesture.handler = Some(comp.name.clone());
            }
            if hit.clicked() || hit.drag_started() {
                gesture.select = Some(comp.name.clone());
            }
            if hit.drag_started() {
                if let Some(pos) = hit.interact_pointer_pos() {
                    let corner = was && pos.x >= r.right() - 14.0 && pos.y >= r.bottom() - 14.0;
                    gesture.drag = Some(Drag {
                        name: comp.name.clone(),
                        kind: if corner { DragKind::Resize } else { DragKind::Move },
                        origin: pos,
                        phone: rect.min,
                        x0: if corner { comp.width } else { comp.left },
                        y0: if corner { comp.height } else { comp.top },
                        scale,
                    });
                }
            }
            if was {
                ui.painter().rect_stroke(r, 4, Stroke::new(1.5, SELECT), egui::StrokeKind::Inside);
                let handle = Rect::from_min_size(r.right_bottom() - egui::vec2(8.0, 8.0), Vec2::splat(8.0));
                ui.painter().rect_filled(handle, 2, SELECT);
            }
        }
        ui.set_clip_rect(prev_clip);
    });
    let _ = phone;
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.label(
            RichText::new(format!("{}×{} dp", form.width, form.height))
                .small()
                .weak(),
        );
    });
    gesture
}

fn paint_widget(
    ui: &mut egui::Ui,
    comp: &Component,
    rect: Rect,
    live: bool,
    text: &mut String,
    checked: &mut bool,
    value: &mut i32,
) -> Option<egui::Response> {
    let size = (prop_i(comp, "TextSize").unwrap_or(16) as f32).clamp(10.0, 36.0);
    let color = color_of(&prop_s(comp, "TextColor"));
    match comp.kind.as_str() {
        "Button" => {
            let rich = RichText::new(text.clone()).size(size);
            let mut button = Button::new(rich).min_size(rect.size());
            if !live {
                button = button.sense(Sense::hover());
            }
            Some(ui.put(rect, button))
        }
        "Toggle" => {
            let label = if *checked {
                prop_or(comp, "TextOn", "On")
            } else {
                prop_or(comp, "TextOff", "Off")
            };
            let rich = RichText::new(label).size(size);
            let mut button = Button::new(rich).min_size(rect.size());
            if *checked {
                button = button.fill(SELECT);
            }
            if !live {
                button = button.sense(Sense::hover());
            }
            let response = ui.put(rect, button);
            if live && response.clicked() {
                *checked = !*checked;
            }
            Some(response)
        }
        "Edit" | "Number" | "Email" | "Phone" => {
            let hint = prop_s(comp, "Hint");
            let edit = TextEdit::singleline(text)
                .hint_text(hint)
                .desired_width(rect.width())
                .interactive(live);
            Some(ui.put(rect, edit))
        }
        "Password" => {
            let hint = prop_s(comp, "Hint");
            let edit = TextEdit::singleline(text)
                .password(true)
                .hint_text(hint)
                .desired_width(rect.width())
                .interactive(live);
            Some(ui.put(rect, edit))
        }
        "Memo" => {
            let hint = prop_s(comp, "Hint");
            let edit = TextEdit::multiline(text)
                .hint_text(hint)
                .desired_width(rect.width())
                .interactive(live);
            Some(ui.put(rect, edit))
        }
        "Check" | "Switch" => {
            let label = RichText::new(prop_s(comp, "Text")).size(size).color(color);
            Some(ui.put(rect, Checkbox::new(checked, label)))
        }
        "Radio" => {
            let label = RichText::new(prop_s(comp, "Text")).size(size).color(color);
            if live {
                let response = ui.put(rect, RadioButton::new(*checked, label));
                if response.clicked() {
                    *checked = !*checked;
                }
                Some(response)
            } else {
                paint_radio(ui, rect, *checked, &prop_s(comp, "Text"), size, color);
                None
            }
        }
        "Seek" => {
            let max = prop_i(comp, "Max").unwrap_or(100).max(1);
            *value = (*value).clamp(0, max);
            if live {
                Some(ui.put(rect, Slider::new(value, 0..=max).trailing_fill(true).show_value(false)))
            } else {
                paint_seek(ui, rect, *value, max);
                None
            }
        }
        "Progress" => {
            let max = prop_i(comp, "Max").unwrap_or(100).max(1) as f32;
            let frac = (*value as f32 / max).clamp(0.0, 1.0);
            Some(ui.put(
                rect,
                ProgressBar::new(frac)
                    .desired_width(rect.width())
                    .desired_height(rect.height().max(8.0))
                    .fill(SELECT),
            ))
        }
        "Busy" => {
            paint_busy(ui, rect);
            None
        }
        "Picker" => {
            let min = prop_i(comp, "MinValue").unwrap_or(0);
            let max = prop_i(comp, "MaxValue").unwrap_or(min).max(min);
            *value = (*value).clamp(min, max);
            paint_picker(ui, rect, *value);
            if live {
                let hit = ui.interact(rect, Id::unique(("pick", &comp.name)), Sense::click());
                if hit.clicked() {
                    if let Some(pos) = hit.interact_pointer_pos() {
                        let wrap = prop_bool(comp, "Wrap", true);
                        if pos.y < rect.center().y {
                            *value = if *value >= max {
                                if wrap { min } else { max }
                            } else {
                                *value + 1
                            };
                        } else {
                            *value = if *value <= min {
                                if wrap { max } else { min }
                            } else {
                                *value - 1
                            };
                        }
                    }
                }
                Some(hit)
            } else {
                None
            }
        }
        "Time" => {
            paint_frame(ui, rect);
            let hour = prop_i(comp, "Hour").unwrap_or(12).clamp(0, 23);
            let minute = prop_i(comp, "Minute").unwrap_or(0).clamp(0, 59);
            let shown = if prop_bool(comp, "Is24Hour", true) {
                format!("{hour:02}:{minute:02}")
            } else {
                let suffix = if hour < 12 { "AM" } else { "PM" };
                let h12 = match hour % 12 {
                    0 => 12,
                    h => h,
                };
                format!("{h12}:{minute:02} {suffix}")
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                shown,
                egui::FontId::proportional(18.0),
                INK,
            );
            None
        }
        "Date" => {
            paint_frame(ui, rect);
            let year = prop_i(comp, "Year").unwrap_or(2026);
            let month = prop_i(comp, "Month").unwrap_or(1).clamp(1, 12);
            let day = prop_i(comp, "Day").unwrap_or(1).clamp(1, 31);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{year:04}-{month:02}-{day:02}"),
                egui::FontId::proportional(18.0),
                INK,
            );
            None
        }
        "Rating" => {
            let stars = prop_i(comp, "Stars").unwrap_or(5).clamp(1, 10);
            *value = (*value).clamp(0, stars);
            paint_stars(ui, rect, *value, stars);
            if live {
                let hit = ui.interact(rect, Id::unique(("stars", &comp.name)), Sense::click());
                if hit.clicked() {
                    if let Some(pos) = hit.interact_pointer_pos() {
                        let t = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
                        *value = (t * stars as f32).round() as i32;
                    }
                }
                Some(hit)
            } else {
                None
            }
        }
        "Clock" => {
            let shown = prop_or(comp, "Format", "HH:mm");
            let rich = RichText::new(shown).size(size).color(color);
            Some(ui.put(rect, Label::new(rich).selectable(false)))
        }
        "Panel" => {
            let fill = color_of(&prop_or(comp, "Color", "#E4E8EE"));
            ui.painter().rect_filled(rect, 6, fill);
            None
        }
        "List" => {
            let fill = color_of(&prop_or(comp, "Color", "#FBF9F4"));
            ui.painter().rect_filled(rect, 6, fill);
            ui.painter().rect_stroke(
                rect,
                6,
                Stroke::new(1.0, Color32::from_rgb(0xC4, 0xBE, 0xB4)),
                egui::StrokeKind::Inside,
            );
            let items = prop_s(comp, "Items");
            let lines: Vec<&str> = items
                .split('\n')
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(6)
                .collect();
            let font = egui::FontId::proportional(14.0);
            let ink = Color32::from_rgb(0x1C, 0x24, 0x30);
            let rule = Color32::from_rgb(0xE4, 0xDD, 0xD4);
            if lines.is_empty() {
                ui.painter().text(
                    rect.left_top() + egui::vec2(12.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    "List",
                    font,
                    Color32::from_rgb(0x7A, 0x73, 0x6A),
                );
            } else {
                for (i, line) in lines.iter().enumerate() {
                    let y = rect.top() + 8.0 + i as f32 * 28.0;
                    if y + 16.0 > rect.bottom() {
                        break;
                    }
                    ui.painter().text(
                        egui::pos2(rect.left() + 12.0, y),
                        egui::Align2::LEFT_TOP,
                        *line,
                        font.clone(),
                        ink,
                    );
                    let rule_y = y + 22.0;
                    if rule_y < rect.bottom() - 4.0 {
                        ui.painter().line_segment(
                            [
                                egui::pos2(rect.left() + 8.0, rule_y),
                                egui::pos2(rect.right() - 8.0, rule_y),
                            ],
                            Stroke::new(1.0, rule),
                        );
                    }
                }
            }
            None
        }
        "Spinner" => {
            let fill = color_of(&prop_or(comp, "Color", "#FBF9F4"));
            ui.painter().rect_filled(rect, 6, fill);
            ui.painter().rect_stroke(
                rect,
                6,
                Stroke::new(1.0, Color32::from_rgb(0xC4, 0xBE, 0xB4)),
                egui::StrokeKind::Inside,
            );
            let items = prop_s(comp, "Items");
            let lines: Vec<&str> = items
                .split('\n')
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect();
            let index = prop_i(comp, "Selection").unwrap_or(0);
            let shown = if index >= 0 {
                lines.get(index as usize).copied()
            } else {
                None
            }
            .or_else(|| lines.first().copied())
            .unwrap_or("Spinner");
            let font = egui::FontId::proportional(15.0);
            let ink = Color32::from_rgb(0x1C, 0x24, 0x30);
            ui.painter().text(
                egui::pos2(rect.left() + 12.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                shown,
                font.clone(),
                ink,
            );
            ui.painter().text(
                egui::pos2(rect.right() - 12.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                "v",
                font,
                Color32::from_rgb(0x5C, 0x56, 0x4E),
            );
            None
        }
        "Web" => {
            ui.painter().rect_filled(rect, 6, Color32::from_rgb(0x1C, 0x24, 0x30));
            ui.painter().rect_stroke(
                rect,
                6,
                Stroke::new(1.0, Color32::from_rgb(0xC4, 0xBE, 0xB4)),
                egui::StrokeKind::Inside,
            );
            let url = prop_s(comp, "Url");
            let html = prop_s(comp, "Html");
            let body = if !url.is_empty() {
                url
            } else if !html.is_empty() {
                html.chars().filter(|c| !c.is_control()).take(42).collect()
            } else {
                "Web".into()
            };
            let font = egui::FontId::proportional(13.0);
            ui.painter().text(
                rect.left_top() + egui::vec2(10.0, 8.0),
                egui::Align2::LEFT_TOP,
                "Web",
                font.clone(),
                Color32::from_rgb(0xF4, 0xF0, 0xE8),
            );
            ui.painter().text(
                rect.left_top() + egui::vec2(10.0, 28.0),
                egui::Align2::LEFT_TOP,
                body,
                font,
                Color32::from_rgb(0xC8, 0xD0, 0xD8),
            );
            None
        }
        _ => {
            let shown = if text.is_empty() { " ".to_string() } else { text.clone() };
            let rich = RichText::new(shown).size(size).color(color);
            Some(ui.put(rect, Label::new(rich).selectable(false)))
        }
    }
}

fn paint_seek(ui: &mut egui::Ui, rect: Rect, value: i32, max: i32) {
    let max = max.max(1);
    let t = (value as f32 / max as f32).clamp(0.0, 1.0);
    let mid = rect.center().y;
    let track = Rect::from_center_size(egui::pos2(rect.center().x, mid), egui::vec2(rect.width(), 4.0));
    ui.painter().rect_filled(track, 2, Color32::from_rgb(0xD9, 0xD4, 0xCC));
    let fill = Rect::from_min_size(track.min, egui::vec2(track.width() * t, track.height()));
    ui.painter().rect_filled(fill, 2, SELECT);
    let thumb = egui::pos2(track.left() + track.width() * t, mid);
    ui.painter().circle_filled(thumb, 7.0, Color32::WHITE);
    ui.painter().circle_stroke(thumb, 7.0, Stroke::new(1.0, SELECT));
}

fn paint_radio(ui: &mut egui::Ui, rect: Rect, on: bool, text: &str, size: f32, color: Color32) {
    let center = egui::pos2(rect.left() + 10.0, rect.center().y);
    ui.painter().circle_stroke(center, 7.0, Stroke::new(1.0, Color32::from_rgb(0x8A, 0x84, 0x7C)));
    if on {
        ui.painter().circle_filled(center, 4.0, SELECT);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(size),
        color,
    );
}

fn paint_stars(ui: &mut egui::Ui, rect: Rect, rating: i32, stars: i32) {
    let stars = stars.clamp(1, 10);
    let rating = rating.clamp(0, stars);
    let text = "★".repeat(rating as usize) + &"☆".repeat((stars - rating) as usize);
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(18.0),
        Color32::from_rgb(0xC4, 0x8A, 0x1A),
    );
}

fn paint_frame(ui: &mut egui::Ui, rect: Rect) {
    ui.painter().rect_filled(rect, 4.0, Color32::WHITE);
    ui.painter().rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_rgb(0xDD, 0xD9, 0xD2)),
        egui::StrokeKind::Inside,
    );
}

fn paint_picker(ui: &mut egui::Ui, rect: Rect, value: i32) {
    paint_frame(ui, rect);
    let muted = Color32::from_rgb(0x8A, 0x84, 0x7C);
    ui.painter().text(
        egui::pos2(rect.center().x, rect.top() + 14.0),
        egui::Align2::CENTER_CENTER,
        "▲",
        egui::FontId::proportional(12.0),
        muted,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        value.to_string(),
        egui::FontId::proportional(18.0),
        INK,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 14.0),
        egui::Align2::CENTER_CENTER,
        "▼",
        egui::FontId::proportional(12.0),
        muted,
    );
}

fn paint_busy(ui: &mut egui::Ui, rect: Rect) {
    let center = rect.center();
    let radius = (rect.width().min(rect.height()) * 0.32).clamp(6.0, 16.0);
    ui.painter()
        .circle_stroke(center, radius, Stroke::new(3.0, Color32::from_rgb(0xD9, 0xD4, 0xCC)));
    let tip = center + egui::vec2(0.0, -radius);
    ui.painter().circle_filled(tip, 3.5, SELECT);
}

fn numeric_prop(comp: &Component) -> i32 {
    match comp.kind.as_str() {
        "Rating" => prop_i(comp, "Rating").unwrap_or(0),
        "Picker" => prop_i(comp, "Value").unwrap_or(0),
        _ => prop_i(comp, "Progress").unwrap_or(0),
    }
}

fn edit_string(ui: &mut egui::Ui, comp: &mut Component, key: &str) -> bool {
    ui.label(key);
    let mut text = prop_s(comp, key);
    if ui.text_edit_singleline(&mut text).changed() {
        comp.props.insert(key.into(), Value::String(text));
        true
    } else {
        false
    }
}

fn edit_number(ui: &mut egui::Ui, comp: &mut Component, key: &str) -> bool {
    ui.label(key);
    let mut text = prop_i(comp, key).map(|n| n.to_string()).unwrap_or_default();
    if ui.text_edit_singleline(&mut text).changed() {
        if let Ok(n) = text.trim().parse::<i64>() {
            comp.props.insert(key.into(), json!(n));
        } else if text.trim().is_empty() {
            comp.props.remove(key);
        }
        true
    } else {
        false
    }
}

fn edit_flag(ui: &mut egui::Ui, comp: &mut Component, key: &str, default_on: bool) -> bool {
    let mut on = prop_bool(comp, key, default_on);
    if ui.checkbox(&mut on, key).changed() {
        if key == "Wrap" || key == "Is24Hour" {
            comp.props.insert(key.into(), Value::Bool(on));
        } else {
            let store = if key == "Checked" { on } else { !on };
            if store {
                comp.props.insert(key.into(), Value::Bool(on));
            } else {
                comp.props.remove(key);
            }
        }
        true
    } else {
        false
    }
}

fn prop_s(comp: &Component, key: &str) -> String {
    match comp.props.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn prop_or(comp: &Component, key: &str, fallback: &str) -> String {
    let text = prop_s(comp, key);
    if text.is_empty() { fallback.into() } else { text }
}

fn prop_i(comp: &Component, key: &str) -> Option<i32> {
    comp.props.get(key).and_then(|v| v.as_i64()).map(|n| n as i32)
}

fn prop_bool(comp: &Component, key: &str, default: bool) -> bool {
    match comp.props.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(Value::String(s)) => matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        _ => default,
    }
}

fn default_props(kind: &str) -> BTreeMap<String, Value> {
    let mut props = BTreeMap::new();
    match kind {
        "Label" => {
            props.insert("Text".into(), json!("Label"));
            props.insert("TextSize".into(), json!(18));
            props.insert("TextColor".into(), json!("#1A1A1A"));
        }
        "Clock" => {
            props.insert("Format".into(), json!("HH:mm"));
            props.insert("TextSize".into(), json!(18));
            props.insert("TextColor".into(), json!("#1A1A1A"));
        }
        "Button" => {
            props.insert("Text".into(), json!("Button"));
            props.insert("TextSize".into(), json!(15));
        }
        "Toggle" => {
            props.insert("TextOn".into(), json!("On"));
            props.insert("TextOff".into(), json!("Off"));
            props.insert("Checked".into(), json!(false));
            props.insert("TextSize".into(), json!(15));
        }
        "Edit" => {
            props.insert("Hint".into(), json!("Type here"));
            props.insert("Text".into(), json!(""));
            props.insert("TextSize".into(), json!(16));
        }
        "Memo" => {
            props.insert("Hint".into(), json!("Notes"));
            props.insert("Text".into(), json!(""));
            props.insert("TextSize".into(), json!(16));
        }
        "Number" => {
            props.insert("Hint".into(), json!("0"));
            props.insert("Text".into(), json!(""));
            props.insert("TextSize".into(), json!(16));
        }
        "Password" => {
            props.insert("Hint".into(), json!("Password"));
            props.insert("Text".into(), json!(""));
            props.insert("TextSize".into(), json!(16));
        }
        "Email" => {
            props.insert("Hint".into(), json!("name@example.com"));
            props.insert("Text".into(), json!(""));
            props.insert("TextSize".into(), json!(16));
        }
        "Phone" => {
            props.insert("Hint".into(), json!("Phone"));
            props.insert("Text".into(), json!(""));
            props.insert("TextSize".into(), json!(16));
        }
        "Check" | "Switch" | "Radio" => {
            props.insert("Text".into(), json!(kind));
            props.insert("TextSize".into(), json!(15));
        }
        "Seek" | "Progress" => {
            props.insert("Max".into(), json!(100));
            props.insert("Progress".into(), json!(40));
        }
        "Busy" => {}
        "Rating" => {
            props.insert("Stars".into(), json!(5));
            props.insert("Rating".into(), json!(3));
        }
        "Picker" => {
            props.insert("MinValue".into(), json!(0));
            props.insert("MaxValue".into(), json!(10));
            props.insert("Value".into(), json!(3));
            props.insert("Wrap".into(), json!(true));
        }
        "Time" => {
            props.insert("Is24Hour".into(), json!(true));
            props.insert("Hour".into(), json!(12));
            props.insert("Minute".into(), json!(0));
        }
        "Date" => {
            props.insert("Year".into(), json!(2026));
            props.insert("Month".into(), json!(10));
            props.insert("Day".into(), json!(24));
        }
        "Panel" => {
            props.insert("Direction".into(), json!("Vertical"));
            props.insert("Color".into(), json!("#E4E8EE"));
        }
        "List" => {
            props.insert("Items".into(), json!("One\nTwo\nThree"));
            props.insert("Color".into(), json!("#FBF9F4"));
        }
        "Spinner" => {
            props.insert("Items".into(), json!("One\nTwo\nThree"));
            props.insert("Selection".into(), json!(0));
            props.insert("Color".into(), json!("#FBF9F4"));
        }
        "Web" => {
            props.insert("Html".into(), json!(""));
            props.insert("Url".into(), json!(""));
        }
        _ => {}
    }
    props
}

/// A second copy of the selected widget. The name is new. The place steps
/// 16dp down and right, and stays on the phone. Text and the click handler
/// stay with the copy, so both widgets call the same procedure.
fn duplicate_component(form: &Form, selected: &str) -> Option<Component> {
    let src = form.components.iter().find(|c| c.name == selected)?;
    let name = unique_name(form, &src.kind);
    let left = snap((src.left + 16).min(form.width - src.width).max(0));
    let top = snap((src.top + 16).min(form.height - src.height).max(0));
    Some(Component {
        kind: src.kind.clone(),
        name,
        left,
        top,
        width: src.width,
        height: src.height,
        parent: src.parent.clone(),
        props: src.props.clone(),
        events: src.events.clone(),
    })
}

/// Where a widget is drawn. A child stacks inside its panel. `left` and `top`
/// are the margins of that stack. A widget with no parent uses them as form coordinates.
fn component_origin(components: &[Component], index: usize) -> (i32, i32) {
    let mut stack = Vec::new();
    origin_at(components, index, &mut stack)
}

fn origin_at(components: &[Component], index: usize, stack: &mut Vec<usize>) -> (i32, i32) {
    if index >= components.len() {
        return (0, 0);
    }
    if stack.contains(&index) {
        let comp = &components[index];
        return (comp.left, comp.top);
    }
    let parent_name = components[index].parent.trim();
    if parent_name.is_empty() {
        return (components[index].left, components[index].top);
    }
    let Some(pidx) = components
        .iter()
        .position(|comp| comp.name.eq_ignore_ascii_case(parent_name))
    else {
        return (components[index].left, components[index].top);
    };
    stack.push(index);
    let (px, py) = origin_at(components, pidx, stack);
    stack.pop();
    let horizontal = direction_horizontal(&components[pidx]);
    let parent_key = components[pidx].name.clone();
    let mut x = px;
    let mut y = py;
    for (i, sib) in components.iter().enumerate() {
        if i == index {
            break;
        }
        if !sib.parent.eq_ignore_ascii_case(&parent_key) {
            continue;
        }
        if horizontal {
            x += sib.left + sib.width;
        } else {
            y += sib.top + sib.height;
        }
    }
    let comp = &components[index];
    if horizontal {
        (x + comp.left, py + comp.top)
    } else {
        (px + comp.left, y + comp.top)
    }
}

fn direction_horizontal(comp: &Component) -> bool {
    comp.props
        .get("Direction")
        .and_then(|value| value.as_str())
        .is_some_and(|text| text.trim().eq_ignore_ascii_case("Horizontal"))
}

/// Keep the widget where it is drawn when its parent changes.
fn assign_parent(components: &mut [Component], index: usize, new_parent: &str) {
    let (ax, ay) = component_origin(components, index);
    components[index].parent = new_parent.to_string();
    components[index].left = 0;
    components[index].top = 0;
    let (cx, cy) = component_origin(components, index);
    components[index].left = (ax - cx).max(0);
    components[index].top = (ay - cy).max(0);
}

fn contains_widget(components: &[Component], ancestor: &str, name: &str) -> bool {
    let mut current = name.to_string();
    for _ in 0..=components.len() {
        let Some(comp) = components.iter().find(|c| c.name.eq_ignore_ascii_case(&current)) else {
            return false;
        };
        if comp.parent.is_empty() {
            return false;
        }
        if comp.parent.eq_ignore_ascii_case(ancestor) {
            return true;
        }
        current = comp.parent.clone();
    }
    false
}

fn parent_choices(components: &[Component], index: usize) -> Vec<String> {
    let name = &components[index].name;
    components
        .iter()
        .filter(|comp| {
            comp.kind == "Panel"
                && !comp.name.eq_ignore_ascii_case(name)
                && !contains_widget(components, name, &comp.name)
        })
        .map(|comp| comp.name.clone())
        .collect()
}

/// The panel under a drop point, or the form when the point misses every panel.
fn drop_parent(components: &[Component], index: usize, x: i32, y: i32) -> String {
    let name = &components[index].name;
    for i in (0..components.len()).rev() {
        if i == index {
            continue;
        }
        let cand = &components[i];
        if cand.kind != "Panel" {
            continue;
        }
        if contains_widget(components, name, &cand.name) {
            continue;
        }
        let (px, py) = component_origin(components, i);
        if x >= px && y >= py && x < px + cand.width && y < py + cand.height {
            return cand.name.clone();
        }
    }
    String::new()
}

fn host_panel(form: &Form, selected: Option<&str>) -> String {
    let Some(name) = selected else {
        return String::new();
    };
    let Some(comp) = form.components.iter().find(|c| c.name == name) else {
        return String::new();
    };
    if comp.kind == "Panel" {
        return comp.name.clone();
    }
    if comp.parent.is_empty() {
        return String::new();
    }
    form.components
        .iter()
        .find(|c| c.kind == "Panel" && c.name.eq_ignore_ascii_case(&comp.parent))
        .map(|c| c.name.clone())
        .unwrap_or_default()
}

fn unique_name(form: &Form, kind: &str) -> String {
    let mut n = 1;
    loop {
        let name = format!("{kind}{n}");
        if !form.components.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
            return name;
        }
        n += 1;
    }
}

/// A double-click opens the handler already bound to the widget. It writes a
/// procedure only when that event is empty or the procedure is missing, and it
/// does not retarget a handler that already has a name.
fn plan_handler(
    kind: &str,
    widget: &str,
    events: &BTreeMap<String, String>,
    source: &str,
) -> HandlerPlan {
    let (event, existing) = bound_event(kind, events);
    let handler = if existing.is_empty() {
        format!("{widget}{}", event_suffix(&event))
    } else {
        existing
    };
    let mut source = source.to_string();
    let mut changed = events.get(&event).map(|text| text.trim().is_empty()).unwrap_or(true);
    if procedure_line(&source, &handler).is_none() {
        if !source.is_empty() && !source.ends_with('\n') {
            source.push('\n');
        }
        let params = if event == "OnItemClick" || event == "OnItemSelected" {
            "Row: String"
        } else {
            "Sender: View"
        };
        source.push_str(&format!("\nprocedure {handler}({params});\nbegin\nend;\n"));
        changed = true;
    }
    let line = procedure_line(&source, &handler).unwrap_or(1);
    HandlerPlan {
        event,
        handler,
        source,
        changed,
        line,
    }
}

fn bound_event(kind: &str, events: &BTreeMap<String, String>) -> (String, String) {
    let mut names: Vec<&str> = kind_events(kind).to_vec();
    for extra in ["OnClick", "OnItemClick"] {
        if !names.contains(&extra) {
            names.push(extra);
        }
    }
    for name in &names {
        if let Some(handler) = events.get(*name).map(|text| text.trim()).filter(|text| !text.is_empty()) {
            return ((*name).to_string(), handler.to_string());
        }
    }
    for (name, handler) in events {
        let handler = handler.trim();
        if !handler.is_empty() {
            return (name.clone(), handler.to_string());
        }
    }
    (kind_events(kind).first().copied().unwrap_or("OnClick").to_string(), String::new())
}

fn event_suffix(event: &str) -> &str {
    event.strip_prefix("On").unwrap_or(event)
}

fn procedure_line(source: &str, name: &str) -> Option<i32> {
    let head = format!("procedure {}", name.to_ascii_lowercase());
    for (i, line) in source.lines().enumerate() {
        let trimmed = line.trim().to_ascii_lowercase();
        if let Some(rest) = trimmed.strip_prefix(&head) {
            if rest.is_empty()
                || rest.starts_with('(')
                || rest.starts_with(';')
                || rest.starts_with(':')
                || rest.starts_with(|c: char| c.is_whitespace())
            {
                return Some((i as i32) + 1);
            }
        }
    }
    None
}

struct HandlerPlan {
    event: String,
    handler: String,
    source: String,
    changed: bool,
    line: i32,
}

fn snap(v: i32) -> i32 {
    if v >= 0 { ((v + 2) / 4) * 4 } else { v }
}

fn fit_scale(avail: Vec2, w: i32, h: i32) -> f32 {
    let sx = (avail.x - 24.0) / w as f32;
    let sy = (avail.y - 36.0) / h as f32;
    sx.min(sy).clamp(0.45, 1.15)
}

fn color_of(text: &str) -> Color32 {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(n) = u32::from_str_radix(hex, 16) {
            return Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
        }
    }
    Color32::from_rgb(0xF7, 0xF8, 0xFA)
}

fn jstr<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn jnum(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0)
}

enum Nav {
    Focus(i64),
    Open(i64),
    At(String, i32),
}

fn paint_flow(ui: &mut egui::Ui, file: &str, entry: usize, steps: &[nav::FlowStep]) -> Option<Nav> {
    let (nodes, links) = nav::flow_layout(entry, steps);
    if nodes.is_empty() {
        return None;
    }
    const BOX_W: f32 = 220.0;
    const BOX_H: f32 = 56.0;
    const GAP_X: f32 = 40.0;
    const GAP_Y: f32 = 44.0;
    let max_col = nodes.iter().map(|node| node.col).max().unwrap_or(0);
    let max_row = nodes.iter().map(|node| node.row).max().unwrap_or(0);
    let back = links.iter().any(|link| link.back);
    let self_loop = links.iter().any(|link| link.back && link.from == link.to);
    let gutter = if back { 32.0 } else { 4.0 };
    let top_pad = if self_loop { 18.0 } else { 0.0 };
    let width = (max_col as f32 + 1.0) * BOX_W + max_col as f32 * GAP_X + gutter;
    let height = top_pad + (max_row as f32 + 1.0) * BOX_H + max_row as f32 * GAP_Y + 8.0;
    let (area, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter_at(area);
    let row_shift = |row: i32| {
        let count = nodes.iter().filter(|node| node.row == row).count() as f32;
        let row_w = count * BOX_W + (count - 1.0).max(0.0) * GAP_X;
        (width - gutter - row_w).max(0.0) / 2.0
    };
    let rect_of = |node: &nav::FlowNode| {
        let x = area.left() + row_shift(node.row) + node.col as f32 * (BOX_W + GAP_X);
        let y = area.top() + top_pad + node.row as f32 * (BOX_H + GAP_Y);
        Rect::from_min_size(egui::pos2(x, y), Vec2::new(BOX_W, BOX_H))
    };
    let mut rects: BTreeMap<usize, Rect> = BTreeMap::new();
    for node in &nodes {
        rects.insert(node.id, rect_of(node));
    }
    let stroke = Stroke::new(1.25, INK);
    let right = area.left() + width - gutter + 18.0;
    for link in &links {
        let (Some(from), Some(to)) = (rects.get(&link.from), rects.get(&link.to)) else {
            continue;
        };
        if link.back {
            let start = from.right_center();
            let lane = right;
            if link.from == link.to {
                let end = from.center_top();
                let above = from.top() - 12.0;
                painter.line_segment([start, egui::pos2(lane, start.y)], stroke);
                painter.line_segment([egui::pos2(lane, start.y), egui::pos2(lane, above)], stroke);
                painter.line_segment([egui::pos2(lane, above), egui::pos2(end.x, above)], stroke);
                painter.line_segment([egui::pos2(end.x, above), end], stroke);
                flow_arrow(&painter, end, Vec2::new(0.0, 1.0));
            } else {
                let end = to.right_center();
                painter.line_segment([start, egui::pos2(lane, start.y)], stroke);
                painter.line_segment([egui::pos2(lane, start.y), egui::pos2(lane, end.y)], stroke);
                painter.line_segment([egui::pos2(lane, end.y), end + Vec2::new(8.0, 0.0)], stroke);
                flow_arrow(&painter, end, Vec2::new(-1.0, 0.0));
            }
        } else {
            let start = from.center_bottom();
            let end = to.center_top();
            let mid_y = start.y + (end.y - start.y) * 0.45;
            painter.line_segment([start, egui::pos2(start.x, mid_y)], stroke);
            painter.line_segment([egui::pos2(start.x, mid_y), egui::pos2(end.x, mid_y)], stroke);
            painter.line_segment([egui::pos2(end.x, mid_y), end], stroke);
            flow_arrow(&painter, end, Vec2::new(0.0, 1.0));
        }
    }
    let font = egui::FontId::proportional(13.0);
    let small = egui::FontId::proportional(11.0);
    for node in &nodes {
        let rect = rects[&node.id];
        let entry_box = node.id == entry;
        painter.rect_filled(rect, 6.0, Color32::from_rgb(0xFB, 0xF9, 0xF4));
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(if entry_box { 2.0 } else { 1.0 }, if entry_box { SELECT } else { INK }),
            egui::StrokeKind::Inside,
        );
        let title = format!("{} · {}", node.label, node.line);
        painter.text(
            rect.left_top() + Vec2::new(8.0, 6.0),
            egui::Align2::LEFT_TOP,
            title,
            small.clone(),
            if entry_box { SELECT } else { INK },
        );
        painter.text(
            rect.left_top() + Vec2::new(8.0, 26.0),
            egui::Align2::LEFT_TOP,
            flow_caption(&node.text),
            font.clone(),
            INK,
        );
    }
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            for node in &nodes {
                if rects[&node.id].contains(pos) {
                    return Some(Nav::At(file.to_string(), node.line));
                }
            }
        }
    }
    None
}

fn flow_caption(text: &str) -> String {
    let first = text.split(" · ").next().unwrap_or("").trim();
    let mut clipped: String = first.chars().take(32).collect();
    if first.chars().count() > 32 || text.contains(" · ") {
        if !clipped.ends_with('…') {
            clipped.push('…');
        }
    }
    clipped
}

fn flow_arrow(painter: &egui::Painter, tip: egui::Pos2, dir: Vec2) {
    let len = dir.length();
    if len < 0.001 {
        return;
    }
    let dir = dir / len;
    let side = Vec2::new(-dir.y, dir.x);
    let base = tip - dir * 8.0;
    let stroke = Stroke::new(1.25, INK);
    painter.line_segment([tip, base + side * 4.5], stroke);
    painter.line_segment([tip, base - side * 4.5], stroke);
}

fn call_mark(open: bool) -> &'static str {
    // The UI font has no triangle glyphs, so a missing mark drew as an empty box.
    if open { "v" } else { ">" }
}

fn call_toggle(ui: &mut egui::Ui, open: bool) -> egui::Response {
    ui.add(egui::Button::selectable(open, call_mark(open)).min_size(Vec2::new(18.0, 18.0)))
}

fn chain_pad(depth: usize) -> f32 {
    22.0 + depth.saturating_sub(1) as f32 * 16.0
}

fn paint_sides(
    ui: &mut egui::Ui,
    sides: &nav::CallSides,
    depth: usize,
    toggle: &mut Option<i64>,
) -> Option<Nav> {
    let mut nav = None;
    for (title, rows) in [("calls", &sides.calls), ("called by", &sides.called_by)] {
        if let Some(hit) = paint_branch_rows(ui, title, rows, depth, toggle) {
            nav = Some(hit);
        }
    }
    nav
}

fn paint_branch_rows(
    ui: &mut egui::Ui,
    title: &str,
    rows: &[nav::CallBranch],
    depth: usize,
    toggle: &mut Option<i64>,
) -> Option<Nav> {
    let mut nav = None;
    let pad = chain_pad(depth);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.label(RichText::new(title).small().weak());
    });
    if rows.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.label("—");
        });
        return None;
    }
    for branch in rows {
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.spacing_mut().item_spacing.x = 4.0;
            if branch.expandable
                && call_toggle(ui, branch.open)
                    .on_hover_text("Show the next calls in this chain")
                    .clicked()
            {
                *toggle = Some(branch.id);
            }
            if ui.link(&branch.name).on_hover_text("Focus this entity").clicked() {
                nav = Some(Nav::Focus(branch.id));
            }
        });
        if branch.open {
            if let Some(hit) = paint_branch_rows(ui, "calls", &branch.calls, depth + 1, toggle) {
                nav = Some(hit);
            }
            if let Some(hit) = paint_branch_rows(ui, "called by", &branch.called_by, depth + 1, toggle) {
                nav = Some(hit);
            }
        }
    }
    nav
}

fn opened_sides(
    focus: i64,
    other: i64,
    open: &BTreeSet<i64>,
    refs: &[Value],
    names: &BTreeMap<i64, String>,
) -> nav::CallSides {
    nav::call_branches(other, open, refs, names, 1, &[focus, other])
}

fn paint_graph_rows(
    ui: &mut egui::Ui,
    rows: &[Value],
    focus: i64,
    open: &BTreeSet<i64>,
    refs: &[Value],
    names: &BTreeMap<i64, String>,
    toggle: &mut Option<i64>,
) -> Option<Nav> {
    let mut hit = None;
    for other in rows {
        let oid = jnum(other, "id");
        let is_open = open.contains(&oid);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if call_toggle(ui, is_open)
                .on_hover_text("Show the next calls in this chain")
                .clicked()
            {
                *toggle = Some(oid);
            }
            let response = ui
                .selectable_label(false, jstr(other, "qname"))
                .on_hover_text("Click to focus. Double-click to open.");
            if response.double_clicked() {
                hit = Some(Nav::Open(oid));
            } else if response.clicked() {
                hit = Some(Nav::Focus(oid));
            }
        });
        if is_open {
            let sides = opened_sides(focus, oid, open, refs, names);
            if let Some(next) = paint_sides(ui, &sides, 1, toggle) {
                hit = Some(next);
            }
        }
    }
    hit
}

fn paint_call_sites(
    ui: &mut egui::Ui,
    title: &str,
    sites: &[nav::Site],
    focus: i64,
    open: &BTreeSet<i64>,
    refs: &[Value],
    names: &BTreeMap<i64, String>,
) -> (Option<Nav>, Option<i64>) {
    if sites.is_empty() {
        return (None, None);
    }
    let mut nav = None;
    let mut toggle = None;
    ui.add_space(8.0);
    ui.label(RichText::new(format!("{title}  {}", sites.len())).strong());
    for site in sites {
        let is_open = open.contains(&site.other_id);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if call_toggle(ui, is_open)
                .on_hover_text("Show the next calls in this chain")
                .clicked()
            {
                toggle = Some(site.other_id);
            }
            if ui.link(&site.other_name).on_hover_text("Focus this entity").clicked() {
                nav = Some(Nav::Focus(site.other_id));
            }
            if !site.context.is_empty() {
                ui.label(RichText::new(&site.context).monospace());
            }
            let place = format!("{}:{}", site.file, site.line);
            if ui.link(place).on_hover_text("Open this line").clicked() {
                nav = Some(Nav::At(site.file.clone(), site.line));
            }
        });
        if is_open {
            let sides = opened_sides(focus, site.other_id, open, refs, names);
            if let Some(next) = paint_sides(ui, &sides, 1, &mut toggle) {
                nav = Some(next);
            }
        }
    }
    (nav, toggle)
}

fn paint_sites(ui: &mut egui::Ui, title: &str, sites: &[nav::Site]) -> Option<Nav> {
    if sites.is_empty() {
        return None;
    }
    let mut nav = None;
    ui.add_space(8.0);
    ui.label(RichText::new(format!("{title}  {}", sites.len())).strong());
    for site in sites {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui
                .link(&site.other_name)
                .on_hover_text("Focus this entity")
                .clicked()
            {
                nav = Some(Nav::Focus(site.other_id));
            }
            if !site.context.is_empty() {
                ui.label(RichText::new(&site.context).monospace());
            }
            let place = format!("{}:{}", site.file, site.line);
            if ui.link(place).on_hover_text("Open this line").clicked() {
                nav = Some(Nav::At(site.file.clone(), site.line));
            }
        });
    }
    nav
}

/// A compiler line names a buffered file. `file:line: message` has column 1.
/// `file:line:col: message` keeps the column. The longest project path wins.
fn message_site(line: &str, buffers: &BTreeMap<String, String>) -> Option<(String, i32, i32)> {
    let mut found: Option<(String, i32, i32)> = None;
    for rel in buffers.keys() {
        if rel.is_empty() {
            continue;
        }
        let needle = format!("{rel}:");
        let Some(at) = line.find(&needle) else {
            continue;
        };
        let before_ok = at == 0 || matches!(line.as_bytes()[at - 1], b'/' | b' ' | b'\t');
        if !before_ok {
            continue;
        }
        let rest = &line[at + needle.len()..];
        let Some((num, after)) = rest.split_once(':') else {
            continue;
        };
        let Ok(n) = num.parse::<i32>() else {
            continue;
        };
        if n < 1 || open_target(rel, i64::from(n), buffers).is_none() {
            continue;
        }
        let col = after
            .trim_start()
            .split_once(':')
            .and_then(|(col, _)| {
                if !col.is_empty() && col.bytes().all(|b| b.is_ascii_digit()) {
                    col.parse::<i32>().ok()
                } else {
                    None
                }
            })
            .filter(|col| *col >= 1)
            .unwrap_or(1);
        let replace = match &found {
            None => true,
            Some((prev, _, _)) => rel.len() > prev.len(),
        };
        if replace {
            found = Some((rel.clone(), n, col));
        }
    }
    found
}

/// A jump needs a buffered file with text. Line 0 becomes line 1.
fn open_target(file: &str, line: i64, buffers: &BTreeMap<String, String>) -> Option<(String, i32)> {
    let text = buffers.get(file)?;
    if text.is_empty() {
        return None;
    }
    Some((file.to_string(), line.max(1) as i32))
}

/// Scroll so `line` sits three rows below the top of the code view.
fn jump_scroll_y(line: i32, line_h: f32) -> f32 {
    let line = line.max(1) as f32;
    ((line - 1.0) * line_h - line_h * 3.0).max(0.0)
}

fn format_errors(errors: Option<&Value>) -> String {
    let Some(rows) = errors.and_then(|v| v.as_array()) else {
        return String::new();
    };
    rows.iter()
        .map(|e| {
            format!(
                "{}:{}: {}",
                e.get("file").and_then(|v| v.as_str()).unwrap_or(""),
                e.get("line").and_then(|v| v.as_i64()).unwrap_or(1),
                e.get("message").and_then(|v| v.as_str()).unwrap_or("error")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Loaded {
    path: PathBuf,
    project_name: String,
    forms: Vec<FormDoc>,
    buffers: BTreeMap<String, String>,
    form_index: usize,
    open_rel: String,
}

fn load_project(path: &Path) -> Result<Loaded, String> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let meta_text = fs::read_to_string(path.join("project.json")).map_err(|e| e.to_string())?;
    let meta: Value = serde_json::from_str(&meta_text).map_err(|e| e.to_string())?;
    let project_name = meta.get("name").and_then(|v| v.as_str()).unwrap_or("Droide").to_string();
    let main_form = meta.get("mainForm").and_then(|v| v.as_str()).unwrap_or("Main").to_string();
    let mut forms = Vec::new();
    let mut buffers = BTreeMap::new();
    let forms_dir = path.join("forms");
    if forms_dir.is_dir() {
        let mut jsons: Vec<PathBuf> = fs::read_dir(&forms_dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.ends_with(".form.json"))
                    .unwrap_or(false)
            })
            .collect();
        jsons.sort();
        for jp in jsons {
            let text = fs::read_to_string(&jp).map_err(|e| e.to_string())?;
            let form: Form = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", jp.display()))?;
            let source_rel = format!("forms/{}.droid", form.name);
            let source_path = forms_dir.join(format!("{}.droid", form.name));
            buffers.insert(source_rel.clone(), fs::read_to_string(&source_path).unwrap_or_default());
            forms.push(FormDoc { form, form_path: jp, source_path, source_rel });
        }
    }
    for sub in ["units", "native"] {
        let dir = path.join(sub);
        if !dir.is_dir() {
            continue;
        }
        let mut files: Vec<_> = fs::read_dir(&dir).map_err(|e| e.to_string())?.filter_map(|e| e.ok()).collect();
        files.sort_by_key(|e| e.file_name());
        for ent in files {
            let name = ent.file_name().to_string_lossy().to_string();
            let ext = Path::new(&name).extension().and_then(|ext| ext.to_str());
            let keep = (sub == "units" && ext == Some("droid"))
                || (sub == "native" && matches!(ext, Some("nc" | "c" | "cc" | "cpp")));
            if !keep {
                continue;
            }
            buffers.insert(format!("{sub}/{name}"), fs::read_to_string(ent.path()).unwrap_or_default());
        }
    }
    let form_index = forms
        .iter()
        .position(|f| f.form.name.eq_ignore_ascii_case(&main_form))
        .unwrap_or(0);
    let open_rel = forms.get(form_index).map(|f| f.source_rel.clone()).unwrap_or_default();
    Ok(Loaded { path, project_name, forms, buffers, form_index, open_rel })
}

fn save_project(forms: &[FormDoc], buffers: &BTreeMap<String, String>) -> Result<(), String> {
    for doc in forms {
        let text = serde_json::to_string_pretty(&doc.form).map_err(|e| e.to_string())?;
        fs::write(&doc.form_path, text + "\n").map_err(|e| e.to_string())?;
        if let Some(src) = buffers.get(&doc.source_rel) {
            fs::write(&doc.source_path, src).map_err(|e| e.to_string())?;
        }
    }
    for (rel, text) in buffers {
        if forms.iter().any(|f| &f.source_rel == rel) {
            continue;
        }
        let path = forms
            .first()
            .map(|f| f.form_path.parent().and_then(|p| p.parent()).unwrap_or(Path::new(".")).join(rel))
            .unwrap_or_else(|| PathBuf::from(rel));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(path, text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn find_repo() -> PathBuf {
    let mut starts = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        let mut cur = start;
        loop {
            if cur.join("droide").join("__main__.py").is_file() {
                return cur;
            }
            if !cur.pop() {
                break;
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn compiler_command(repo: &std::path::Path) -> Command {
    for rel in ["compiler/target/release/droide", "compiler/target/debug/droide"] {
        let path = repo.join(rel);
        if path.is_file() {
            return Command::new(path);
        }
    }
    let mut python = Command::new("python3");
    python.arg("-m").arg("droide");
    python
}

fn spawn_job(repo: PathBuf, args: Vec<String>) -> Receiver<JobOut> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = compiler_command(&repo)
            .args(&args)
            .current_dir(&repo)
            .output();
        let out = match result {
            Ok(out) => JobOut {
                code: out.status.code().unwrap_or(1),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            },
            Err(err) => JobOut {
                code: 1,
                stdout: String::new(),
                stderr: err.to_string(),
            },
        };
        let _ = tx.send(out);
    });
    rx
}

fn project_from_args() -> PathBuf {
    let arg = std::env::args().nth(1);
    let path = match arg {
        Some(p) => {
            let path = PathBuf::from(p);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
            }
        }
        None => find_repo().join("examples").join("Hello"),
    };
    path
}

fn main() -> eframe::Result {
    let project = project_from_args();
    let mut options = eframe::NativeOptions::default();
    options.renderer = eframe::Renderer::Glow;
    options.viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 820.0])
        .with_title("Droide");
    eframe::run_native(
        "droide",
        options,
        Box::new(|cc| Ok(Box::new(Studio::new(cc, project)))),
    )
}

#[cfg(test)]
mod nav_tests {
    use std::collections::BTreeMap;

    use super::{jump_scroll_y, kind_events, message_site, open_target, plan_handler};

    #[test]
    fn open_target_needs_text_in_the_buffer() {
        let mut buffers = BTreeMap::new();
        buffers.insert("native/mix.c".into(), "int x;\n".into());
        buffers.insert("native/empty.c".into(), String::new());
        assert_eq!(open_target("native/mix.c", 0, &buffers), Some(("native/mix.c".into(), 1)));
        assert_eq!(open_target("native/mix.c", 5, &buffers), Some(("native/mix.c".into(), 5)));
        assert_eq!(open_target("native/empty.c", 1, &buffers), None);
        assert_eq!(open_target("missing.droid", 3, &buffers), None);
    }

    #[test]
    fn message_site_reads_compiler_lines() {
        let mut buffers = BTreeMap::new();
        buffers.insert("forms/Main.droid".into(), "begin\nend\n".into());
        buffers.insert("mix.c".into(), "int x;\n".into());
        buffers.insert("native/mix.c".into(), "int x;\n".into());
        buffers.insert("native/empty.c".into(), String::new());
        assert_eq!(
            message_site("forms/Main.droid:12: expected end", &buffers),
            Some(("forms/Main.droid".into(), 12, 1))
        );
        assert_eq!(
            message_site("/tmp/proj/native/mix.c:6:10: error: boom", &buffers),
            Some(("native/mix.c".into(), 6, 10))
        );
        assert_eq!(
            message_site("native/mix.c:5: bad type", &buffers),
            Some(("native/mix.c".into(), 5, 1))
        );
        assert_eq!(message_site("apk build/app.apk (6910 bytes)", &buffers), None);
        assert_eq!(message_site("Run finished", &buffers), None);
        assert_eq!(message_site("native/empty.c:1: missing", &buffers), None);
        assert_eq!(message_site(":1: NDK clang failed", &buffers), None);
    }

    #[test]
    fn jump_keeps_three_lines_of_context() {
        let mut kept = BTreeMap::new();
        kept.insert("OnClick".into(), "OkClick".into());
        let source = "procedure OkClick(Sender: View);\nbegin\n  Log('hi')\nend;\n";
        let plan = plan_handler("Button", "OkButton", &kept, source);
        assert_eq!(plan.handler, "OkClick");
        assert_eq!(plan.event, "OnClick");
        assert!(!plan.changed);
        assert_eq!(plan.source, source);
        assert_eq!(plan.line, 1);

        let fresh = plan_handler("Button", "Button1", &BTreeMap::new(), "procedure Other();\nbegin\nend;\n");
        assert_eq!(fresh.handler, "Button1Click");
        assert!(fresh.changed);
        assert!(fresh.source.contains("procedure Other();"));
        assert!(fresh.source.contains("procedure Button1Click(Sender: View);"));
        assert_eq!(fresh.line, 5);

        let list = plan_handler("List", "Names", &BTreeMap::new(), "");
        assert_eq!(list.event, "OnItemClick");
        assert_eq!(list.handler, "NamesItemClick");
        assert!(list.source.contains("procedure NamesItemClick(Row: String);"));

        let missing = plan_handler("Button", "OkButton", &kept, "");
        assert_eq!(missing.handler, "OkClick");
        assert!(missing.source.contains("procedure OkClick(Sender: View);"));
        assert_eq!(jump_scroll_y(1, 16.0), 0.0);
        assert_eq!(jump_scroll_y(4, 16.0), 0.0);
        assert_eq!(jump_scroll_y(5, 16.0), 16.0);
        assert_eq!(jump_scroll_y(0, 16.0), 0.0);
    }

    #[test]
    fn kind_events_keep_click_first() {
        assert_eq!(kind_events("Edit"), &["OnClick", "OnLongClick", "OnChange"]);
        assert_eq!(kind_events("List"), &["OnItemClick", "OnItemLongClick"]);
        assert_eq!(kind_events("Button"), &["OnClick", "OnLongClick"]);
        assert_eq!(kind_events("Spinner"), &["OnItemSelected"]);
        let plan = plan_handler("Edit", "Edit1", &BTreeMap::new(), "");
        assert_eq!(plan.event, "OnClick");
        assert_eq!(plan.handler, "Edit1Click");
        let spin = plan_handler("Spinner", "Kind1", &BTreeMap::new(), "");
        assert_eq!(spin.event, "OnItemSelected");
        assert_eq!(spin.handler, "Kind1ItemSelected");
        assert!(spin.source.contains("procedure Kind1ItemSelected(Row: String);"));
    }
}

#[cfg(test)]
mod design_tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{
        assign_parent, component_origin, drop_parent, duplicate_component, host_panel, parent_choices,
        Component, Form,
    };

    fn button(name: &str, left: i32, top: i32) -> Component {
        Component {
            kind: "Button".into(),
            name: name.into(),
            left,
            top,
            width: 88,
            height: 36,
            parent: String::new(),
            props: BTreeMap::from([("Text".into(), json!("Go"))]),
            events: BTreeMap::from([("OnClick".into(), "Button1Click".into())]),
        }
    }

    fn form_with(components: Vec<Component>) -> Form {
        Form {
            name: "Main".into(),
            title: String::new(),
            background: "#F7F8FA".into(),
            width: 360,
            height: 640,
            events: BTreeMap::new(),
            components,
        }
    }

    #[test]
    fn duplicate_steps_and_keeps_the_handler() {
        let form = form_with(vec![button("Button1", 16, 24)]);
        let copy = duplicate_component(&form, "Button1").expect("copy");
        assert_eq!(copy.name, "Button2");
        assert_eq!(copy.kind, "Button");
        assert_eq!((copy.left, copy.top), (32, 40));
        assert_eq!(copy.props.get("Text").and_then(|v| v.as_str()), Some("Go"));
        assert_eq!(
            copy.events.get("OnClick").map(String::as_str),
            Some("Button1Click")
        );
        assert!(duplicate_component(&form, "Missing").is_none());

        let mut next = form;
        next.components.push(copy);
        let again = duplicate_component(&next, "Button1").expect("again");
        assert_eq!(again.name, "Button3");
        assert_eq!((again.left, again.top), (32, 40));
    }

    #[test]
    fn duplicate_stays_on_the_phone() {
        let form = form_with(vec![button("Button1", 300, 620)]);
        let copy = duplicate_component(&form, "Button1").expect("copy");
        assert!(copy.left >= 0 && copy.left + copy.width <= form.width);
        assert!(copy.top >= 0 && copy.top + copy.height <= form.height);
        assert_ne!((copy.left, copy.top), (300, 620));
    }

    fn panel(name: &str, left: i32, top: i32, width: i32, height: i32) -> Component {
        let mut comp = button(name, left, top);
        comp.kind = "Panel".into();
        comp.width = width;
        comp.height = height;
        comp.props.insert("Direction".into(), json!("Vertical"));
        comp
    }

    #[test]
    fn children_stack_inside_the_panel() {
        let mut box_ = panel("Box", 10, 20, 200, 160);
        box_.props.insert("Direction".into(), json!("Vertical"));
        let mut first = button("One", 4, 6);
        first.parent = "Box".into();
        first.width = 80;
        first.height = 36;
        let mut second = button("Two", 4, 0);
        second.parent = "Box".into();
        second.width = 80;
        second.height = 36;
        let form = form_with(vec![box_, first, second]);
        assert_eq!(component_origin(&form.components, 0), (10, 20));
        assert_eq!(component_origin(&form.components, 1), (14, 26));
        assert_eq!(component_origin(&form.components, 2), (14, 62));
    }

    #[test]
    fn a_horizontal_panel_stacks_across() {
        let mut row = panel("Row", 10, 20, 240, 80);
        row.props.insert("Direction".into(), json!("Horizontal"));
        let mut first = button("One", 4, 6);
        first.parent = "Row".into();
        first.width = 80;
        first.height = 36;
        let mut second = button("Two", 0, 6);
        second.parent = "Row".into();
        second.width = 40;
        second.height = 36;
        let form = form_with(vec![row, first, second]);
        assert_eq!(component_origin(&form.components, 1), (14, 26));
        assert_eq!(component_origin(&form.components, 2), (94, 26));
    }

    #[test]
    fn a_nested_panel_keeps_its_own_stack() {
        let outer = panel("Outer", 10, 20, 220, 180);
        let mut inner = panel("Inner", 8, 4, 120, 80);
        inner.parent = "Outer".into();
        let mut child = button("Go", 2, 2);
        child.parent = "Inner".into();
        child.width = 40;
        child.height = 20;
        let form = form_with(vec![outer, inner, child]);
        assert_eq!(component_origin(&form.components, 1), (18, 24));
        assert_eq!(component_origin(&form.components, 2), (20, 26));
    }

    #[test]
    fn assigning_a_parent_keeps_the_drawn_place() {
        let box_ = panel("Box", 10, 20, 200, 160);
        let go = button("Go", 40, 70);
        let mut form = form_with(vec![box_, go]);
        assert_eq!(component_origin(&form.components, 1), (40, 70));
        assign_parent(&mut form.components, 1, "Box");
        assert_eq!(form.components[1].parent, "Box");
        assert_eq!(component_origin(&form.components, 1), (40, 70));
        assign_parent(&mut form.components, 1, "");
        assert!(form.components[1].parent.is_empty());
        assert_eq!((form.components[1].left, form.components[1].top), (40, 70));
    }

    #[test]
    fn duplicate_keeps_the_parent() {
        let box_ = panel("Box", 10, 20, 200, 160);
        let mut go = button("Go", 8, 8);
        go.parent = "Box".into();
        let form = form_with(vec![box_, go]);
        let copy = duplicate_component(&form, "Go").expect("copy");
        assert_eq!(copy.parent, "Box");
        assert_eq!(host_panel(&form, Some("Box")), "Box");
        assert_eq!(host_panel(&form, Some("Go")), "Box");
        assert_eq!(host_panel(&form, None), "");
    }

    #[test]
    fn a_drop_on_a_panel_names_that_panel() {
        let box_ = panel("Box", 10, 20, 200, 160);
        let go = button("Go", 40, 70);
        let form = form_with(vec![box_, go]);
        assert_eq!(drop_parent(&form.components, 1, 50, 80), "Box");
        assert_eq!(drop_parent(&form.components, 1, 4, 4), "");
        assert!(parent_choices(&form.components, 0).is_empty());
        assert_eq!(parent_choices(&form.components, 1), vec!["Box".to_string()]);
    }
}
