//! Native Droide studio. The window is egui. Compile, run, and analyze shell out
//! to the Python compiler in this repository.

mod highlight;

use std::collections::BTreeMap;
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
];

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
        "Panel" => &["Color", "Enabled", "Visible"],
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
    }

    fn apply_drag(&mut self, ctx: &egui::Context) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let (pos, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
        if !down {
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
        let Some(doc) = self.forms.get_mut(fi) else {
            return;
        };
        let name = unique_name(&doc.form, spec.name);
        let n = doc.form.components.len() as i32;
        let comp = Component {
            kind: spec.name.into(),
            name: name.clone(),
            left: 16,
            top: (24 + n * 12).min(560),
            width: spec.w,
            height: spec.h,
            props: default_props(spec.name),
            events: BTreeMap::new(),
        };
        doc.form.components.push(comp);
        self.selected = Some(name);
        self.dirty = true;
    }

    fn ensure_handler(&mut self, widget: &str) {
        let proc_name = format!("{widget}Click");
        let fi = self.form_index;
        let Some(rel) = self.forms.get(fi).map(|f| f.source_rel.clone()) else {
            return;
        };
        let needle = format!("procedure {}", proc_name.to_ascii_lowercase());
        let missing = !self
            .buffers
            .get(&rel)
            .map(|s| s.to_ascii_lowercase().contains(&needle))
            .unwrap_or(false);
        if missing {
            let source = self.buffers.entry(rel.clone()).or_default();
            if !source.is_empty() && !source.ends_with('\n') {
                source.push('\n');
            }
            source.push_str(&format!("\nprocedure {proc_name}(Sender: View);\nbegin\nend;\n"));
        }
        if let Some(comp) = self.forms[fi].form.components.iter_mut().find(|c| c.name == widget) {
            comp.events.insert("OnClick".into(), proc_name);
        }
        self.open_rel = rel;
        self.mode = Mode::Code;
        self.dirty = true;
        self.status = format!("{widget}Click");
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
        self.open_rel = rel.to_string();
        if let Some(i) = self.forms.iter().position(|f| f.source_rel == rel) {
            self.form_index = i;
            self.selected = None;
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
                    for prop in ["Text", "Hint", "Format", "TextOn", "TextOff"] {
                        if props.contains(&prop) {
                            dirty |= edit_string(ui, comp, prop);
                        }
                    }
                    for prop in [
                        "TextSize", "Max", "Progress", "Stars", "Rating", "MinValue", "MaxValue", "Value",
                        "Hour", "Minute", "Year", "Month", "Day",
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
                    ui.label("OnClick");
                    let mut click = comp.events.get("OnClick").cloned().unwrap_or_default();
                    if ui.text_edit_singleline(&mut click).changed() {
                        if click.is_empty() {
                            comp.events.remove("OnClick");
                        } else {
                            comp.events.insert("OnClick".into(), click);
                        }
                        dirty = true;
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

    fn editor(&mut self, ui: &mut egui::Ui) {
        let rel = self.open_rel.clone();
        let mut text = self.buffers.get(&rel).cloned().unwrap_or_default();
        let native = rel.ends_with(".nc");
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let row_h = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
        let ppp = ui.pixels_per_point();
        let line_h = (row_h * ppp).round().max(1.0) / ppp;
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

        let mut editor_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(editor_rect)
                .layout(egui::Layout::top_down(egui::Align::Min))
                .id_salt(("code-view", rel.clone())),
        );
        let mut area = ScrollArea::both()
            .id_salt(("code-scroll", rel.clone()))
            .auto_shrink([false, false]);
        if gutter_hot && wheel_y != 0.0 {
            let scroll_id = editor_ui.make_persistent_id(egui::IdSalt::new(("code-scroll", rel.clone())));
            if let Some(state) = egui::scroll_area::State::load(editor_ui.ctx(), scroll_id) {
                let y = (state.clamped_offset().y - wheel_y).max(0.0);
                area = area.vertical_scroll_offset(y);
            }
        }
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
            let job = highlight::layout_job(buf.as_str(), native, font_id.clone(), line_h);
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
                .id(Id::unique(("source", rel.clone())))
                .layouter(&mut layouter)
                .show(ui)
        });
        ui.advance_cursor_after_rect(full);

        let output = shown.inner;
        let cursor_line = output.cursor_range.as_ref().map(|range| highlight::line_of_char(&text, range.primary.index.0));
        highlight::paint_gutter(
            &ui.painter().with_clip_rect(gutter_rect),
            gutter_rect,
            &output.galley,
            output.galley_pos,
            &text,
            native,
            cursor_line,
            font_id,
        );
        if output.response.changed() {
            self.buffers.insert(rel, text);
            self.dirty = true;
        }
    }

    fn index(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Index").small().weak());
        ui.text_edit_singleline(&mut self.filter);
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let query = self.filter.to_ascii_lowercase();
            let rows: Vec<Value> = self
                .entities()
                .into_iter()
                .filter(|e| listed(jstr(e, "kind"), &query, jstr(e, "name"), jstr(e, "qname")))
                .collect();
            for ent in rows {
                let id = jnum(&ent, "id");
                let label = format!("{}  {}", jstr(&ent, "kind"), jstr(&ent, "qname"));
                if ui.selectable_label(self.focus == Some(id), label).clicked() {
                    self.focus = Some(id);
                }
            }
        });
    }

    fn explore_detail(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.focus else {
            ui.label("Select a routine, widget, or form.");
            return;
        };
        let Some(ent) = self.entity(id) else {
            ui.label("That entity is gone.");
            return;
        };
        ScrollArea::vertical().show(ui, |ui| {
            ui.heading(jstr(&ent, "qname"));
            ui.label(format!(
                "{} · {}:{} · {} lines · complexity {} · {} in · {} out",
                jstr(&ent, "kind"),
                jstr(&ent, "file"),
                jnum(&ent, "line"),
                jnum(&ent, "loc"),
                jnum(&ent, "complexity"),
                jnum(&ent, "fanIn"),
                jnum(&ent, "fanOut"),
            ));
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
            ui.add_space(8.0);
            ui.label(RichText::new("Call graph").strong());
            ui.horizontal_top(|ui| {
                let down = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(Vec2::new(220.0, 8.0), down, |ui| {
                    ui.label(RichText::new("Callers").small().weak());
                    if callers.is_empty() {
                        ui.label("—");
                    }
                    for other in &callers {
                        ui.label(jstr(other, "qname"));
                    }
                });
                ui.allocate_ui_with_layout(Vec2::new(220.0, 8.0), down, |ui| {
                    ui.label(RichText::new("Focus").small().weak());
                    ui.label(jstr(&ent, "qname"));
                    if self_call {
                        ui.label("calls itself");
                    }
                });
                ui.allocate_ui_with_layout(Vec2::new(220.0, 8.0), down, |ui| {
                    ui.label(RichText::new("Callees").small().weak());
                    if callees.is_empty() {
                        ui.label("—");
                    }
                    for other in &callees {
                        ui.label(jstr(other, "qname"));
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(RichText::new("References").strong());
            let mut shown = 0;
            for r in &refs {
                let src = jnum(r, "src");
                let dst = jnum(r, "dst");
                if src != id && dst != id {
                    continue;
                }
                let other_id = if src == id { dst } else { src };
                let other = self.entity(other_id);
                let name = other.as_ref().map(|e| jstr(e, "qname")).unwrap_or("?");
                ui.label(format!(
                    "{} {}  {}:{}",
                    jstr(r, "kind"),
                    name,
                    jstr(r, "file"),
                    jnum(r, "line")
                ));
                shown += 1;
                if shown >= 24 {
                    break;
                }
            }
            if shown == 0 {
                ui.label("No references.");
            }
            if let Some(cfg) = self.db.get("cfgs").and_then(|c| c.get(&id.to_string())) {
                ui.add_space(8.0);
                ui.label(RichText::new("Control flow").strong());
                if let Some(blocks) = cfg.get("blocks").and_then(|b| b.as_array()) {
                    for block in blocks {
                        let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        if text.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(text).monospace());
                    }
                }
            }
        });
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.preview_window(ctx);
        self.log_window(ctx);
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
        egui::Window::new("Messages")
            .open(&mut open)
            .default_size([640.0, 280.0])
            .show(ctx, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    ui.monospace(&log);
                });
            });
        self.show_log = open;
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
        for comp in &form.components {
            let r = Rect::from_min_size(
                rect.min + egui::vec2(comp.left as f32 * scale, comp.top as f32 * scale),
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
            props.insert("Color".into(), json!("#E4E8EE"));
        }
        _ => {}
    }
    props
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

fn listed(kind: &str, query: &str, name: &str, qname: &str) -> bool {
    if kind == "native-file" {
        return false;
    }
    if query.is_empty() {
        return matches!(kind, "procedure" | "function" | "native" | "widget" | "form" | "unit");
    }
    let name = name.to_ascii_lowercase();
    let qname = qname.to_ascii_lowercase();
    name.contains(query) || qname.contains(query)
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
            let keep = (sub == "units" && name.ends_with(".droid")) || (sub == "native" && name.ends_with(".nc"));
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
