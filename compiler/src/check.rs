//! Name resolution, types, and the explorer database.
//! Names are case-insensitive. The checker writes `ty`, `resolved`,
//! `string_add`, and `form_name` on the AST inside `SourceModel`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::ast::*;
use crate::error::CompileError;
use crate::widgets::{getter, prop_type, WidgetSpec, WIDGETS};

pub struct SourceModel {
    pub forms: BTreeMap<String, FormSource>,
    pub units: BTreeMap<String, UnitSource>,
    pub natives: BTreeMap<String, NativeSource>,
}

pub struct FormSource {
    pub program: Program,
    pub form: Value,
    pub file: String,
}

pub struct UnitSource {
    pub program: Program,
    pub file: String,
}

pub struct NativeSource {
    pub program: NProgram,
    pub file: String,
}

/// A form routine after type checking. `routine` already carries resolutions.
#[derive(Clone, Debug)]
pub struct RoutineInfo {
    pub routine: Routine,
    pub id: i64,
    pub ret: Option<String>,
    pub params: Vec<String>,
    pub is_static: bool,
    pub unit: String,
    pub file: String,
    pub index: usize,
}

#[derive(Clone, Debug)]
pub struct FormChecked {
    pub name: String,
    pub file: String,
    pub form: Value,
    pub widgets: BTreeMap<String, WidgetRef>,
    pub routines: BTreeMap<String, RoutineInfo>,
    pub id: i64,
}

#[derive(Clone, Debug)]
pub struct UnitChecked {
    pub name: String,
    pub file: String,
    pub routines: BTreeMap<String, RoutineInfo>,
    pub id: i64,
}

#[derive(Clone, Debug)]
pub struct NativeChecked {
    pub name: String,
    pub params: Vec<String>,
    pub id: i64,
    pub file: String,
    pub path: String,
    pub fun: NFun,
}

pub struct Checked {
    pub errors: Vec<Value>,
    pub forms: BTreeMap<String, FormChecked>,
    pub units: BTreeMap<String, UnitChecked>,
    pub natives: BTreeMap<String, NativeChecked>,
    pub db: Value,
}

pub fn check_model(model: &mut SourceModel) -> Checked {
    let mut checker = Checker::new();
    checker.parse_natives(model);
    checker.collect_signatures(model);
    checker.check_bodies(model);
    checker.into_checked(model)
}

fn low(name: &str) -> String {
    name.to_lowercase()
}

fn canon_type(name: &str) -> String {
    match low(name).as_str() {
        "integer" => "Integer".into(),
        "string" => "String".into(),
        "boolean" => "Boolean".into(),
        "view" => "View".into(),
        _ => name.to_string(),
    }
}

fn widget_exact(name: &str) -> Option<&'static WidgetSpec> {
    WIDGETS.iter().find(|spec| spec.name == name)
}

struct Builtin {
    args: &'static [&'static str],
    ret: Option<&'static str>,
    form_only: bool,
}

fn builtin_spec(name: &str) -> Option<Builtin> {
    let spec = match name {
        "Toast" => Builtin {
            args: &["String"],
            ret: None,
            form_only: true,
        },
        "Log" => Builtin {
            args: &["String"],
            ret: None,
            form_only: false,
        },
        "OpenForm" => Builtin {
            args: &["String"],
            ret: None,
            form_only: true,
        },
        "Finish" => Builtin {
            args: &[],
            ret: None,
            form_only: true,
        },
        "IntToStr" => Builtin {
            args: &["Integer"],
            ret: Some("String"),
            form_only: false,
        },
        "StrToInt" => Builtin {
            args: &["String"],
            ret: Some("Integer"),
            form_only: false,
        },
        "Length" => Builtin {
            args: &["String"],
            ret: Some("Integer"),
            form_only: false,
        },
        _ => return None,
    };
    Some(spec)
}

fn builtin_canon(lower: &str) -> Option<&'static str> {
    match lower {
        "toast" => Some("Toast"),
        "log" => Some("Log"),
        "openform" => Some("OpenForm"),
        "finish" => Some("Finish"),
        "inttostr" => Some("IntToStr"),
        "strtoint" => Some("StrToInt"),
        "length" => Some("Length"),
        _ => None,
    }
}

#[derive(Clone)]
struct Sig {
    id: i64,
    ret: Option<String>,
    params: Vec<String>,
    is_static: bool,
    unit: String,
    name: String,
    index: usize,
    line: i32,
    col: i32,
}

struct FormBuild {
    name: String,
    file: String,
    form: Value,
    widgets: BTreeMap<String, WidgetRef>,
    routines: BTreeMap<String, Sig>,
    id: i64,
    uses: Vec<String>,
}

struct UnitBuild {
    name: String,
    file: String,
    routines: BTreeMap<String, Sig>,
    id: i64,
    uses: Vec<String>,
}

struct FormCtx {
    widgets: BTreeMap<String, WidgetRef>,
    routines: BTreeMap<String, Sig>,
    uses: Vec<String>,
}

#[derive(Clone)]
struct Slot {
    kind: String,
    ty: String,
    spelling: String,
}

type Scope = BTreeMap<String, Slot>;

enum Found {
    Missing,
    Ambiguous,
    Builtin(&'static str),
    Native(NativeRef),
    Routine(Sig),
}

struct Over {
    params: Option<i32>,
    complexity: Option<i32>,
    depth: Option<i32>,
    loc: Option<i32>,
}

impl Over {
    fn none() -> Self {
        Self {
            params: None,
            complexity: None,
            depth: None,
            loc: None,
        }
    }

    fn metrics(params: i32, complexity: i32, depth: i32, loc: i32) -> Self {
        Self {
            params: Some(params),
            complexity: Some(complexity),
            depth: Some(depth),
            loc: Some(loc),
        }
    }

    fn params(params: i32) -> Self {
        Self {
            params: Some(params),
            complexity: None,
            depth: None,
            loc: None,
        }
    }
}

struct Ent {
    id: i64,
    kind: String,
    name: String,
    qname: String,
    file: String,
    line: i32,
    end_line: i32,
    parent: Option<i64>,
    type_name: String,
    loc: i32,
    complexity: i32,
    depth: i32,
    params: i32,
    fan_in: i32,
    fan_out: i32,
}

struct XRef {
    src: i64,
    dst: i64,
    kind: String,
    file: String,
    line: i32,
    col: i32,
}

struct Db {
    entities: Vec<Ent>,
    refs: Vec<XRef>,
    cfgs: BTreeMap<String, Value>,
}

impl Db {
    fn new() -> Self {
        Self {
            entities: Vec::new(),
            refs: Vec::new(),
            cfgs: BTreeMap::new(),
        }
    }

    fn add(
        &mut self,
        kind: &str,
        name: &str,
        qname: &str,
        file: &str,
        line: i32,
        end_line: Option<i32>,
        parent: Option<i64>,
        type_name: &str,
        over: Over,
    ) -> i64 {
        let id = self.entities.len() as i64;
        let end = end_line.unwrap_or(line);
        let mut loc = if end_line.is_some() {
            end - line + 1
        } else {
            1
        };
        let mut complexity = 1;
        let mut depth = 0;
        let mut params = 0;
        if let Some(value) = over.loc {
            loc = value;
        }
        if let Some(value) = over.complexity {
            complexity = value;
        }
        if let Some(value) = over.depth {
            depth = value;
        }
        if let Some(value) = over.params {
            params = value;
        }
        self.entities.push(Ent {
            id,
            kind: kind.into(),
            name: name.into(),
            qname: qname.into(),
            file: file.into(),
            line,
            end_line: end,
            parent,
            type_name: type_name.into(),
            loc,
            complexity,
            depth,
            params,
            fan_in: 0,
            fan_out: 0,
        });
        id
    }

    fn add_ref(&mut self, src: i64, dst: i64, kind: &str, file: &str, line: i32, col: i32) {
        self.refs.push(XRef {
            src,
            dst,
            kind: kind.into(),
            file: file.into(),
            line,
            col,
        });
    }

    fn finish(&mut self) {
        for xref in &self.refs {
            if xref.kind == "call" || xref.kind == "uses" {
                let src = xref.src as usize;
                let dst = xref.dst as usize;
                if let Some(ent) = self.entities.get_mut(src) {
                    ent.fan_out += 1;
                }
                if let Some(ent) = self.entities.get_mut(dst) {
                    ent.fan_in += 1;
                }
            }
        }
    }

    fn to_value(&self) -> Value {
        let entities: Vec<Value> = self
            .entities
            .iter()
            .map(|ent| {
                json!({
                    "id": ent.id,
                    "kind": ent.kind,
                    "name": ent.name,
                    "qname": ent.qname,
                    "file": ent.file,
                    "line": ent.line,
                    "endLine": ent.end_line,
                    "parent": ent.parent,
                    "type": ent.type_name,
                    "loc": ent.loc,
                    "complexity": ent.complexity,
                    "depth": ent.depth,
                    "params": ent.params,
                    "fanIn": ent.fan_in,
                    "fanOut": ent.fan_out,
                })
            })
            .collect();
        let refs: Vec<Value> = self
            .refs
            .iter()
            .map(|xref| {
                json!({
                    "src": xref.src,
                    "dst": xref.dst,
                    "kind": xref.kind,
                    "file": xref.file,
                    "line": xref.line,
                    "col": xref.col,
                })
            })
            .collect();
        json!({ "entities": entities, "refs": refs, "cfgs": self.cfgs })
    }
}

struct Checker {
    errors: Vec<CompileError>,
    db: Db,
    forms: BTreeMap<String, FormBuild>,
    units: BTreeMap<String, UnitBuild>,
    natives: BTreeMap<String, NativeChecked>,
    routine_index: BTreeMap<(String, String), Sig>,
    builtin_ids: BTreeMap<String, i64>,
}

impl Checker {
    fn new() -> Self {
        Self {
            errors: Vec::new(),
            db: Db::new(),
            forms: BTreeMap::new(),
            units: BTreeMap::new(),
            natives: BTreeMap::new(),
            routine_index: BTreeMap::new(),
            builtin_ids: BTreeMap::new(),
        }
    }

    fn error(&mut self, message: impl Into<String>, line: i32, col: i32, file: &str) {
        self.errors
            .push(CompileError::new(message, line, col, file));
    }

    fn parse_natives(&mut self, model: &SourceModel) {
        for (path, info) in &model.natives {
            let fid = self.db.add(
                "native-file",
                path,
                &format!("native:{path}"),
                &info.file,
                1,
                None,
                None,
                "",
                Over::none(),
            );
            for fun in &info.program.funs {
                let key = low(&fun.name);
                if self.natives.contains_key(&key) {
                    self.error(
                        format!("duplicate native function {}", fun.name),
                        fun.line,
                        fun.col,
                        &info.file,
                    );
                    continue;
                }
                if fun.params.len() > 4 {
                    self.error(
                        "native functions support at most 4 parameters",
                        fun.line,
                        fun.col,
                        &info.file,
                    );
                }
                let qn = format!("native.{}", fun.name);
                let complexity = 1 + n_complexity(&fun.body);
                let depth = n_depth(&fun.body, 1);
                let eid = self.db.add(
                    "native",
                    &fun.name,
                    &qn,
                    &info.file,
                    fun.line,
                    Some(fun.end_line),
                    Some(fid),
                    "i32",
                    Over::metrics(
                        fun.params.len() as i32,
                        complexity,
                        depth,
                        fun.end_line - fun.line + 1,
                    ),
                );
                // loc above is the same formula add() would use; metrics() also sets it.
                self.db.cfgs.insert(eid.to_string(), build_native_cfg(fun));
                for pname in &fun.params {
                    self.db.add(
                        "param",
                        pname,
                        &format!("{qn}.{pname}"),
                        &info.file,
                        fun.line,
                        Some(fun.line),
                        Some(eid),
                        "i32",
                        Over::none(),
                    );
                }
                self.natives.insert(
                    key,
                    NativeChecked {
                        name: fun.name.clone(),
                        params: fun.params.clone(),
                        id: eid,
                        file: info.file.clone(),
                        path: path.clone(),
                        fun: fun.clone(),
                    },
                );
                self.scan_native_stmts(&fun.body, eid, &info.file);
            }
        }
    }

    fn scan_native_stmts(&mut self, stmts: &[NStmt], eid: i64, file: &str) {
        for stmt in stmts {
            match stmt {
                NStmt::If(stmt) => {
                    self.scan_native_stmts(&stmt.then, eid, file);
                    self.scan_native_stmts(&stmt.else_, eid, file);
                }
                NStmt::While(stmt) => self.scan_native_stmts(&stmt.body, eid, file),
                NStmt::Block(stmt) => self.scan_native_stmts(&stmt.stmts, eid, file),
                NStmt::Return(stmt) => {
                    if let Some(expr) = &stmt.expr {
                        self.scan_native_expr(expr, eid, file);
                    }
                }
                NStmt::Var(stmt) => self.scan_native_expr(&stmt.expr, eid, file),
                NStmt::Assign(stmt) => self.scan_native_expr(&stmt.expr, eid, file),
            }
        }
    }

    fn scan_native_expr(&mut self, expr: &NExpr, eid: i64, file: &str) {
        match expr {
            NExpr::Call(call) => {
                if let Some(target) = self.natives.get(&low(&call.name)) {
                    let dst = target.id;
                    self.db.add_ref(eid, dst, "call", file, call.line, call.col);
                }
                for arg in &call.args {
                    self.scan_native_expr(arg, eid, file);
                }
            }
            NExpr::Binary(bin) => {
                self.scan_native_expr(&bin.left, eid, file);
                self.scan_native_expr(&bin.right, eid, file);
            }
            NExpr::Unary(un) => self.scan_native_expr(&un.expr, eid, file),
            NExpr::Ident(_) | NExpr::Lit(_) => {}
        }
    }

    fn collect_signatures(&mut self, model: &SourceModel) {
        for (name, info) in &model.forms {
            self.collect_form(name, info);
        }
        for (name, info) in &model.units {
            self.collect_unit(name, info);
        }
        self.collect_uses();
    }

    fn collect_form(&mut self, name: &str, info: &FormSource) {
        let file = info.file.clone();
        let fid = self.db.add(
            "form",
            name,
            name,
            &file,
            1,
            None,
            None,
            "Activity",
            Over::none(),
        );
        let mut widgets = BTreeMap::new();
        let comps = info.form.get("components").and_then(|v| v.as_array());
        if let Some(comps) = comps {
            for comp in comps {
                let cname = comp.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if cname.is_empty() {
                    self.error("component has no name", 1, 1, &file);
                    continue;
                }
                let key = low(cname);
                if widgets.contains_key(&key) {
                    self.error(format!("duplicate component {cname}"), 1, 1, &file);
                }
                let ctype = comp.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if widget_exact(ctype).is_none() {
                    self.error(format!("unknown component type {ctype}"), 1, 1, &file);
                    continue;
                }
                let qn = format!("{name}.{cname}");
                let cid = self.db.add(
                    "widget",
                    cname,
                    &qn,
                    &file,
                    1,
                    None,
                    Some(fid),
                    ctype,
                    Over::none(),
                );
                widgets.insert(
                    key,
                    WidgetRef {
                        name: cname.to_string(),
                        id: cid,
                        comp: comp.clone(),
                    },
                );
            }
        }
        let mut routines = BTreeMap::new();
        for (index, routine) in info.program.routines.iter().enumerate() {
            let key = low(&routine.name);
            if routines.contains_key(&key) || widgets.contains_key(&key) {
                self.error(
                    format!("duplicate name {}", routine.name),
                    routine.line,
                    routine.col,
                    &file,
                );
                continue;
            }
            if routine.params.len() > 4 {
                self.error(
                    format!("{} has too many parameters (max 4 on a form)", routine.name),
                    routine.line,
                    routine.col,
                    &file,
                );
            }
            let sig = self.add_routine(routine, &file, name, fid, false, index);
            self.routine_index
                .insert((low(name), key.clone()), sig.clone());
            routines.insert(key, sig);
        }
        self.forms.insert(
            name.to_string(),
            FormBuild {
                name: name.to_string(),
                file,
                form: info.form.clone(),
                widgets,
                routines,
                id: fid,
                uses: info.program.uses.clone(),
            },
        );
    }

    fn collect_unit(&mut self, name: &str, info: &UnitSource) {
        let file = info.file.clone();
        let declared = match &info.program.unit {
            Some(unit) if !unit.is_empty() => unit.clone(),
            _ => name.to_string(),
        };
        if let Some(unit) = &info.program.unit {
            if !unit.is_empty() && low(unit) != low(name) {
                self.error(
                    format!("unit name {unit} does not match file {name}"),
                    1,
                    1,
                    &file,
                );
            }
        }
        let fid = self.db.add(
            "unit",
            &declared,
            &declared,
            &file,
            1,
            None,
            None,
            "",
            Over::none(),
        );
        let mut routines = BTreeMap::new();
        for (index, routine) in info.program.routines.iter().enumerate() {
            let key = low(&routine.name);
            if routines.contains_key(&key) {
                self.error(
                    format!("duplicate routine {}", routine.name),
                    routine.line,
                    routine.col,
                    &file,
                );
                continue;
            }
            if routine.params.len() > 5 {
                self.error(
                    format!("{} has too many parameters (max 5 in a unit)", routine.name),
                    routine.line,
                    routine.col,
                    &file,
                );
            }
            let sig = self.add_routine(routine, &file, &declared, fid, true, index);
            self.routine_index
                .insert((low(&declared), key.clone()), sig.clone());
            routines.insert(key, sig);
        }
        self.units.insert(
            name.to_string(),
            UnitBuild {
                name: declared,
                file,
                routines,
                id: fid,
                uses: info.program.uses.clone(),
            },
        );
    }

    fn add_routine(
        &mut self,
        routine: &Routine,
        file: &str,
        unit: &str,
        parent: i64,
        is_static: bool,
        index: usize,
    ) -> Sig {
        let qn = format!("{unit}.{}", routine.name);
        let ret = routine.ret.as_ref().map(|ty| canon_type(ty));
        let kind = if routine.kind == "function" {
            "function"
        } else {
            "procedure"
        };
        let loc = routine.end_line - routine.line + 1;
        let eid = self.db.add(
            kind,
            &routine.name,
            &qn,
            file,
            routine.line,
            Some(routine.end_line),
            Some(parent),
            ret.as_deref().unwrap_or(""),
            Over::metrics(
                routine.params.len() as i32,
                1 + complexity(&routine.body),
                depth(&routine.body, 1),
                loc,
            ),
        );
        self.db.cfgs.insert(eid.to_string(), build_cfg(routine));
        let mut params = Vec::new();
        for param in &routine.params {
            let ptype = canon_type(&param.type_name);
            params.push(ptype.clone());
            self.db.add(
                "param",
                &param.name,
                &format!("{qn}.{}", param.name),
                file,
                param.line,
                Some(param.line),
                Some(eid),
                &ptype,
                Over::none(),
            );
        }
        for local in &routine.locals {
            let ptype = canon_type(&local.type_name);
            self.db.add(
                "local",
                &local.name,
                &format!("{qn}.{}", local.name),
                file,
                local.line,
                Some(local.line),
                Some(eid),
                &ptype,
                Over::none(),
            );
        }
        Sig {
            id: eid,
            ret,
            params,
            is_static,
            unit: unit.to_string(),
            name: routine.name.clone(),
            index,
            line: routine.line,
            col: routine.col,
        }
    }

    fn collect_uses(&mut self) {
        let mut edges = Vec::new();
        for form in self.forms.values() {
            for used in &form.uses {
                edges.push((form.id, used.clone(), form.file.clone()));
            }
        }
        for unit in self.units.values() {
            for used in &unit.uses {
                edges.push((unit.id, used.clone(), unit.file.clone()));
            }
        }
        for (src, used, file) in edges {
            if let Some(dst) = self.unit_id(&used) {
                self.db.add_ref(src, dst, "uses", &file, 1, 1);
            } else {
                self.error(format!("unknown unit {used}"), 1, 1, &file);
            }
        }
    }

    fn unit_id(&self, name: &str) -> Option<i64> {
        let key = low(name);
        for unit in self.units.values() {
            if low(&unit.name) == key {
                return Some(unit.id);
            }
        }
        for form in self.forms.values() {
            if low(&form.name) == key {
                return Some(form.id);
            }
        }
        None
    }

    fn check_bodies(&mut self, model: &mut SourceModel) {
        let form_names: Vec<String> = self.forms.keys().cloned().collect();
        for name in form_names {
            self.check_events(&name);
            let mut keys: Vec<(usize, String)> = self
                .forms
                .get(&name)
                .map(|form| {
                    form.routines
                        .iter()
                        .map(|(key, sig)| (sig.index, key.clone()))
                        .collect()
                })
                .unwrap_or_default();
            keys.sort_by_key(|(index, _)| *index);
            for (_, key) in keys {
                self.check_form_routine(model, &name, &key);
            }
        }
        let unit_names: Vec<String> = self.units.keys().cloned().collect();
        for name in unit_names {
            let mut keys: Vec<(usize, String)> = self
                .units
                .get(&name)
                .map(|unit| {
                    unit.routines
                        .iter()
                        .map(|(key, sig)| (sig.index, key.clone()))
                        .collect()
                })
                .unwrap_or_default();
            keys.sort_by_key(|(index, _)| *index);
            for (_, key) in keys {
                self.check_unit_routine(model, &name, &key);
            }
        }
        let natives: Vec<NativeChecked> = self.natives.values().cloned().collect();
        for native in natives {
            self.check_native(&native);
        }
    }

    fn check_events(&mut self, form_name: &str) {
        let file = self.forms[form_name].file.clone();
        let form_json = self.forms[form_name].form.clone();
        if let Some(handler) = event_name(form_json.get("events"), "OnCreate") {
            self.check_handler(form_name, &handler, "OnCreate", true, &file);
        }
        let comps = form_json
            .get("components")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for comp in comps {
            let ctype = comp
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let events = comp
                .get("events")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let spec_events: Vec<&str> = widget_exact(&ctype)
                .map(|spec| spec.events.to_vec())
                .unwrap_or_default();
            for (event, value) in events {
                let Some(handler) = value.as_str() else {
                    continue;
                };
                if handler.is_empty() {
                    continue;
                }
                if !spec_events.iter().any(|name| *name == event) {
                    self.error(format!("{ctype} has no {event} event"), 1, 1, &file);
                }
                self.check_handler(form_name, handler, &event, false, &file);
            }
        }
    }

    fn check_handler(
        &mut self,
        form_name: &str,
        handler: &str,
        event: &str,
        on_create: bool,
        file: &str,
    ) {
        let sig = self
            .forms
            .get(form_name)
            .and_then(|form| form.routines.get(&low(handler)))
            .cloned();
        let Some(sig) = sig else {
            if on_create {
                self.error(
                    format!("OnCreate handler {handler} is not declared"),
                    1,
                    1,
                    file,
                );
            } else {
                self.error(
                    format!("{event} handler {handler} is not declared"),
                    1,
                    1,
                    file,
                );
            }
            return;
        };
        if sig.ret.is_some() {
            if on_create {
                self.error(
                    "OnCreate handler must be a procedure",
                    sig.line,
                    sig.col,
                    file,
                );
            } else {
                self.error(
                    format!("{handler} must be a procedure"),
                    sig.line,
                    sig.col,
                    file,
                );
            }
        } else if sig.params.len() > 1 {
            if on_create {
                self.error(
                    "OnCreate handler takes no parameters, or one View",
                    sig.line,
                    sig.col,
                    file,
                );
            } else {
                self.error(
                    format!("{handler} takes no parameters, or one View"),
                    sig.line,
                    sig.col,
                    file,
                );
            }
        } else if sig.params.len() == 1 && sig.params[0] != "View" {
            if on_create {
                self.error(
                    "OnCreate handler parameter must be a View",
                    sig.line,
                    sig.col,
                    file,
                );
            } else {
                self.error(
                    format!("{handler} parameter must be a View"),
                    sig.line,
                    sig.col,
                    file,
                );
            }
        }
    }

    fn check_form_routine(&mut self, model: &mut SourceModel, form_name: &str, key: &str) {
        let (sig, file, ctx) = {
            let form = &self.forms[form_name];
            let sig = form.routines[key].clone();
            let ctx = FormCtx {
                widgets: form.widgets.clone(),
                routines: form.routines.clone(),
                uses: form.uses.clone(),
            };
            (sig, form.file.clone(), ctx)
        };
        let routine = &mut model.forms.get_mut(form_name).unwrap().program.routines[sig.index];
        self.check_routine(routine, &sig, &file, Some(&ctx));
    }

    fn check_unit_routine(&mut self, model: &mut SourceModel, unit_name: &str, key: &str) {
        let (sig, file) = {
            let unit = &self.units[unit_name];
            (unit.routines[key].clone(), unit.file.clone())
        };
        let routine = &mut model.units.get_mut(unit_name).unwrap().program.routines[sig.index];
        self.check_routine(routine, &sig, &file, None);
    }

    fn check_routine(
        &mut self,
        routine: &mut Routine,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
    ) {
        let mut scope = Scope::new();
        for param in &routine.params {
            scope.insert(
                low(&param.name),
                Slot {
                    kind: "local".into(),
                    ty: canon_type(&param.type_name),
                    spelling: param.name.clone(),
                },
            );
        }
        for local in &routine.locals {
            let key = low(&local.name);
            if scope.contains_key(&key) {
                self.error(
                    format!("duplicate local {}", local.name),
                    local.line,
                    local.col,
                    file,
                );
            }
            scope.insert(
                key,
                Slot {
                    kind: "local".into(),
                    ty: canon_type(&local.type_name),
                    spelling: local.name.clone(),
                },
            );
        }
        if routine.kind == "function" {
            let ty = canon_type(routine.ret.as_deref().unwrap_or(""));
            scope.insert(
                low("result"),
                Slot {
                    kind: "result".into(),
                    ty,
                    spelling: "Result".into(),
                },
            );
        }
        self.check_stmts(&mut routine.body, &scope, sig, file, form);
    }

    fn check_stmts(
        &mut self,
        stmts: &mut [Stmt],
        scope: &Scope,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
    ) {
        for stmt in stmts {
            self.check_stmt(stmt, scope, sig, file, form);
        }
    }

    fn check_stmt(
        &mut self,
        stmt: &mut Stmt,
        scope: &Scope,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
    ) {
        match stmt {
            Stmt::Block(block) => self.check_stmts(&mut block.stmts, scope, sig, file, form),
            Stmt::If(stmt) => {
                let ty = self.check_expr(&mut stmt.cond, scope, sig, file, form);
                if ty.as_deref().map(|ty| ty != "Boolean").unwrap_or(false) {
                    let line = stmt.cond.line();
                    let col = stmt.cond.col();
                    self.error("condition must be Boolean", line, col, file);
                }
                self.check_stmt(&mut stmt.then, scope, sig, file, form);
                self.check_stmt(&mut stmt.else_, scope, sig, file, form);
            }
            Stmt::While(stmt) => {
                let ty = self.check_expr(&mut stmt.cond, scope, sig, file, form);
                if ty.as_deref().map(|ty| ty != "Boolean").unwrap_or(false) {
                    let line = stmt.cond.line();
                    let col = stmt.cond.col();
                    self.error("condition must be Boolean", line, col, file);
                }
                self.check_stmt(&mut stmt.body, scope, sig, file, form);
            }
            Stmt::Exit(_) => {}
            Stmt::Assign(stmt) => self.check_assign(stmt, scope, sig, file, form),
            Stmt::Call(stmt) => {
                let line = stmt.line;
                let col = stmt.col;
                let name = stmt.name.clone();
                self.check_call(
                    &name,
                    &mut stmt.args,
                    &mut stmt.resolved,
                    &mut stmt.form_name,
                    None,
                    line,
                    col,
                    scope,
                    sig,
                    file,
                    form,
                    true,
                );
            }
        }
    }

    fn check_assign(
        &mut self,
        stmt: &mut Assign,
        scope: &Scope,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
    ) {
        let parts = stmt.target.parts.clone();
        if parts.len() == 1 {
            let key = low(&parts[0]);
            let Some(slot) = scope.get(&key).cloned() else {
                self.error(
                    format!("unknown name {}", parts[0]),
                    stmt.target.line,
                    stmt.target.col,
                    file,
                );
                self.check_expr(&mut stmt.expr, scope, sig, file, form);
                return;
            };
            if slot.kind == "result" || slot.kind == "local" {
                let expr_ty = self.check_expr(&mut stmt.expr, scope, sig, file, form);
                if let Some(got) = expr_ty {
                    if got != slot.ty {
                        let line = stmt.expr.line();
                        let col = stmt.expr.col();
                        self.error(
                            format!("cannot assign {got} to {}", slot.ty),
                            line,
                            col,
                            file,
                        );
                    }
                }
                stmt.target.resolved = Some(Resolved::SlotWrite {
                    kind: slot.kind,
                    ty: slot.ty,
                });
                return;
            }
            self.error(
                format!("cannot assign to {}", parts[0]),
                stmt.target.line,
                stmt.target.col,
                file,
            );
            return;
        }
        if parts.len() == 2 {
            if let Some(form) = form {
                let wkey = low(&parts[0]);
                let Some(widget) = form.widgets.get(&wkey).cloned() else {
                    self.error(
                        format!("unknown component {}", parts[0]),
                        stmt.target.line,
                        stmt.target.col,
                        file,
                    );
                    return;
                };
                let ctype = widget
                    .comp
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let Some(spec) = widget_exact(&ctype) else {
                    self.error(
                        format!("unknown component {}", parts[0]),
                        stmt.target.line,
                        stmt.target.col,
                        file,
                    );
                    return;
                };
                let Some(actual) = spec
                    .props
                    .iter()
                    .copied()
                    .find(|prop| prop.eq_ignore_ascii_case(&parts[1]))
                else {
                    self.error(
                        format!("{ctype} has no property {}", parts[1]),
                        stmt.target.line,
                        stmt.target.col,
                        file,
                    );
                    return;
                };
                let expr_ty = self.check_expr(&mut stmt.expr, scope, sig, file, Some(form));
                let expect = prop_type(actual).unwrap_or("");
                let color_ok = (actual == "TextColor" || actual == "Color")
                    && expr_ty.as_deref() == Some("Integer");
                if !color_ok {
                    if let Some(got) = &expr_ty {
                        if got != expect {
                            let line = stmt.expr.line();
                            let col = stmt.expr.col();
                            self.error(format!("{actual} expects {expect}"), line, col, file);
                        }
                    }
                }
                let wid = widget.id;
                let line = stmt.target.line;
                let col = stmt.target.col;
                stmt.target.resolved = Some(Resolved::Prop {
                    actual: actual.to_string(),
                    widget,
                });
                self.db.add_ref(sig.id, wid, "write", file, line, col);
                return;
            }
        }
        self.error(
            "invalid assignment target",
            stmt.target.line,
            stmt.target.col,
            file,
        );
    }

    fn check_expr(
        &mut self,
        expr: &mut Expr,
        scope: &Scope,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
    ) -> Option<String> {
        match expr {
            Expr::Literal(lit) => {
                let ty = lit.type_name.clone();
                lit.ty = Some(ty.clone());
                Some(ty)
            }
            Expr::Ident(ident) => self.resolve_ident(ident, scope, sig, file, form),
            Expr::Designator(des) => self.resolve_designator(des, sig, file, form),
            Expr::Unary(un) => {
                let ty = self.check_expr(&mut un.expr, scope, sig, file, form);
                if un.op == "not" {
                    if ty.as_deref().map(|ty| ty != "Boolean").unwrap_or(false) {
                        self.error("not expects Boolean", un.line, un.col, file);
                    }
                    un.ty = Some("Boolean".into());
                    Some("Boolean".into())
                } else if un.op == "-" {
                    if ty.as_deref().map(|ty| ty != "Integer").unwrap_or(false) {
                        self.error("unary minus expects Integer", un.line, un.col, file);
                    }
                    un.ty = Some("Integer".into());
                    Some("Integer".into())
                } else {
                    un.ty = ty.clone();
                    ty
                }
            }
            Expr::Binary(bin) => {
                let lt = self.check_expr(&mut bin.left, scope, sig, file, form);
                let rt = self.check_expr(&mut bin.right, scope, sig, file, form);
                let op = bin.op.clone();
                let line = bin.line;
                let col = bin.col;
                let (ty, string_add) = if op == "and" || op == "or" {
                    let bad = |ty: &Option<String>| {
                        ty.as_deref().map(|ty| ty != "Boolean").unwrap_or(false)
                    };
                    if bad(&lt) || bad(&rt) {
                        self.error(format!("{op} expects Boolean operands"), line, col, file);
                    }
                    (Some("Boolean".into()), false)
                } else if matches!(op.as_str(), "=" | "<>" | "<" | ">" | "<=" | ">=") {
                    if let (Some(left), Some(right)) = (&lt, &rt) {
                        if left != right {
                            self.error(
                                format!("cannot compare {left} and {right}"),
                                line,
                                col,
                                file,
                            );
                        }
                    }
                    if let Some(left) = &lt {
                        if left != "Integer" && left != "String" && left != "Boolean" {
                            self.error(format!("cannot compare {left}"), line, col, file);
                        }
                    }
                    (Some("Boolean".into()), false)
                } else if op == "+" {
                    if lt.as_deref() == Some("String") || rt.as_deref() == Some("String") {
                        let ok = |ty: &Option<String>| {
                            matches!(ty.as_deref(), None | Some("String") | Some("Integer"))
                        };
                        if !ok(&lt) || !ok(&rt) {
                            self.error(
                                "string concatenation needs strings or integers",
                                line,
                                col,
                                file,
                            );
                        }
                        (Some("String".into()), true)
                    } else {
                        let ok =
                            |ty: &Option<String>| matches!(ty.as_deref(), None | Some("Integer"));
                        if !ok(&lt) || !ok(&rt) {
                            self.error("+ expects integers or strings", line, col, file);
                        }
                        (Some("Integer".into()), false)
                    }
                } else if matches!(op.as_str(), "-" | "*" | "/" | "div" | "mod") {
                    let ok = |ty: &Option<String>| matches!(ty.as_deref(), None | Some("Integer"));
                    if !ok(&lt) || !ok(&rt) {
                        self.error(format!("{op} expects integers"), line, col, file);
                    }
                    (Some("Integer".into()), false)
                } else {
                    self.error(format!("unknown operator {op}"), line, col, file);
                    (None, false)
                };
                bin.ty = ty.clone();
                bin.string_add = string_add;
                ty
            }
            Expr::Call(call) => {
                let line = call.line;
                let col = call.col;
                let name = call.name.clone();
                self.check_call(
                    &name,
                    &mut call.args,
                    &mut call.resolved,
                    &mut call.form_name,
                    Some(&mut call.ty),
                    line,
                    col,
                    scope,
                    sig,
                    file,
                    form,
                    false,
                )
            }
        }
    }

    fn resolve_ident(
        &mut self,
        ident: &mut Ident,
        scope: &Scope,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
    ) -> Option<String> {
        let key = low(&ident.name);
        if let Some(slot) = scope.get(&key) {
            ident.ty = Some(slot.ty.clone());
            ident.resolved = Some(Resolved::Slot {
                kind: slot.kind.clone(),
                ty: slot.ty.clone(),
                spelling: slot.spelling.clone(),
            });
            return Some(slot.ty.clone());
        }
        if let Some(form) = form {
            if let Some(widget) = form.widgets.get(&key).cloned() {
                let id = widget.id;
                let line = ident.line;
                let col = ident.col;
                ident.ty = Some("View".into());
                ident.resolved = Some(Resolved::Widget(widget));
                self.db.add_ref(sig.id, id, "read", file, line, col);
                return Some("View".into());
            }
        }
        self.error(
            format!("unknown name {}", ident.name),
            ident.line,
            ident.col,
            file,
        );
        ident.ty = None;
        None
    }

    fn resolve_designator(
        &mut self,
        expr: &mut Designator,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
    ) -> Option<String> {
        let Some(form) = form else {
            self.error(
                format!("invalid name {}", expr.parts.join(".")),
                expr.line,
                expr.col,
                file,
            );
            expr.ty = None;
            return None;
        };
        if expr.parts.len() != 2 {
            self.error(
                format!("invalid name {}", expr.parts.join(".")),
                expr.line,
                expr.col,
                file,
            );
            expr.ty = None;
            return None;
        }
        let Some(widget) = form.widgets.get(&low(&expr.parts[0])).cloned() else {
            self.error(
                format!("unknown component {}", expr.parts[0]),
                expr.line,
                expr.col,
                file,
            );
            expr.ty = None;
            return None;
        };
        let ctype = widget
            .comp
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let Some(spec) = widget_exact(ctype) else {
            self.error(
                format!("invalid name {}", expr.parts.join(".")),
                expr.line,
                expr.col,
                file,
            );
            expr.ty = None;
            return None;
        };
        let actual = spec
            .props
            .iter()
            .copied()
            .find(|prop| prop.eq_ignore_ascii_case(&expr.parts[1]));
        let in_getters = actual.map(|name| getter(name).is_some()).unwrap_or(false);
        let in_types = actual
            .map(|name| prop_type(name).is_some())
            .unwrap_or(false);
        let Some(actual) = actual else {
            self.error(
                format!("cannot read {}", expr.parts[1]),
                expr.line,
                expr.col,
                file,
            );
            expr.ty = None;
            return None;
        };
        if !in_getters && !in_types {
            self.error(
                format!("cannot read {}", expr.parts[1]),
                expr.line,
                expr.col,
                file,
            );
            expr.ty = None;
            return None;
        }
        if !in_getters {
            self.error(
                format!("{actual} cannot be read"),
                expr.line,
                expr.col,
                file,
            );
            expr.ty = None;
            return None;
        }
        let ty = prop_type(actual).unwrap_or("").to_string();
        let id = widget.id;
        let line = expr.line;
        let col = expr.col;
        expr.ty = Some(ty.clone());
        expr.resolved = Some(Resolved::Prop {
            actual: actual.to_string(),
            widget,
        });
        self.db.add_ref(sig.id, id, "read", file, line, col);
        Some(ty)
    }

    fn lookup(&self, name: &str, sig: &Sig, form: Option<&FormCtx>) -> Found {
        let key = low(name);
        if let Some(form) = form {
            if let Some(hit) = form.routines.get(&key) {
                return Found::Routine(hit.clone());
            }
        }
        if let Some(hit) = self.routine_index.get(&(low(&sig.unit), key.clone())) {
            return Found::Routine(hit.clone());
        }
        let uses = self.uses_of(sig, form);
        let mut found = Vec::new();
        for ((unit, rname), candidate) in &self.routine_index {
            if rname == &key && uses.iter().any(|used| used == unit) {
                found.push(candidate.clone());
            }
        }
        if found.len() > 1 {
            return Found::Ambiguous;
        }
        if let Some(hit) = found.pop() {
            return Found::Routine(hit);
        }
        if let Some(native) = self.natives.get(&key) {
            return Found::Native(NativeRef {
                name: native.name.clone(),
                params: native.params.clone(),
                id: native.id,
            });
        }
        if let Some(canon) = builtin_canon(&key) {
            return Found::Builtin(canon);
        }
        Found::Missing
    }

    fn uses_of(&self, sig: &Sig, form: Option<&FormCtx>) -> Vec<String> {
        if let Some(form) = form {
            return form.uses.iter().map(|used| low(used)).collect();
        }
        for unit in self.units.values() {
            if low(&unit.name) == low(&sig.unit) {
                return unit.uses.iter().map(|used| low(used)).collect();
            }
        }
        Vec::new()
    }

    fn check_call(
        &mut self,
        name: &str,
        args: &mut [Expr],
        resolved: &mut Option<Resolved>,
        form_name: &mut Option<String>,
        mut ty_slot: Option<&mut Option<String>>,
        line: i32,
        col: i32,
        scope: &Scope,
        sig: &Sig,
        file: &str,
        form: Option<&FormCtx>,
        as_stmt: bool,
    ) -> Option<String> {
        let mut arg_types = Vec::new();
        for arg in args.iter_mut() {
            arg_types.push(self.check_expr(arg, scope, sig, file, form));
        }
        let target = self.lookup(name, sig, form);
        match target {
            Found::Missing => {
                self.error(format!("unknown routine {name}"), line, col, file);
                if let Some(slot) = ty_slot.as_mut() {
                    **slot = None;
                }
                None
            }
            Found::Ambiguous => {
                self.error(
                    format!("{name} is declared in more than one used unit"),
                    line,
                    col,
                    file,
                );
                if let Some(slot) = ty_slot.as_mut() {
                    **slot = None;
                }
                None
            }
            Found::Builtin(canon) => {
                let spec = builtin_spec(canon).expect("builtin");
                if spec.form_only && form.is_none() {
                    self.error(
                        format!("{canon} is only available on a form"),
                        line,
                        col,
                        file,
                    );
                }
                if arg_types.len() != spec.args.len() {
                    self.error(
                        format!("{canon} expects {} arguments", spec.args.len()),
                        line,
                        col,
                        file,
                    );
                } else {
                    for (got, expect) in arg_types.iter().zip(spec.args.iter()) {
                        if let Some(got) = got {
                            if got != expect {
                                self.error(
                                    format!("{canon} expects {expect}, got {got}"),
                                    line,
                                    col,
                                    file,
                                );
                            }
                        }
                    }
                }
                if canon == "OpenForm" {
                    let literal = args.first().and_then(|arg| match arg {
                        Expr::Literal(lit) => match &lit.value {
                            Lit::Str(text) => Some((text.clone(), lit.line, lit.col)),
                            _ => None,
                        },
                        _ => None,
                    });
                    if let Some((text, lit_line, lit_col)) = literal {
                        let found = self
                            .forms
                            .keys()
                            .find(|form_name| low(form_name) == low(&text))
                            .cloned();
                        if let Some(found) = found {
                            *form_name = Some(found);
                        } else {
                            self.error(format!("unknown form {text}"), lit_line, lit_col, file);
                        }
                    } else {
                        self.error("OpenForm needs a string literal form name", line, col, file);
                    }
                }
                *resolved = Some(Resolved::Builtin(canon.to_string()));
                let dst = self.builtin_id(canon, spec.args.len() as i32, spec.ret.unwrap_or(""));
                self.db.add_ref(sig.id, dst, "call", file, line, col);
                if as_stmt {
                    return None;
                }
                match spec.ret {
                    None => {
                        self.error(format!("{canon} does not return a value"), line, col, file);
                        if let Some(slot) = ty_slot.as_mut() {
                            **slot = None;
                        }
                        None
                    }
                    Some(ret) => {
                        if let Some(slot) = ty_slot.as_mut() {
                            **slot = Some(ret.to_string());
                        }
                        Some(ret.to_string())
                    }
                }
            }
            Found::Native(native) => {
                if arg_types.len() != native.params.len() {
                    self.error(
                        format!("{} expects {} arguments", native.name, native.params.len()),
                        line,
                        col,
                        file,
                    );
                }
                for got in &arg_types {
                    if let Some(got) = got {
                        if got != "Integer" {
                            self.error("native arguments must be Integer", line, col, file);
                        }
                    }
                }
                let dst = native.id;
                *resolved = Some(Resolved::Native(native));
                self.db.add_ref(sig.id, dst, "call", file, line, col);
                if as_stmt {
                    return None;
                }
                if let Some(slot) = ty_slot.as_mut() {
                    **slot = Some("Integer".into());
                }
                Some("Integer".into())
            }
            Found::Routine(target) => {
                if !target.is_static && (sig.is_static || low(&target.unit) != low(&sig.unit)) {
                    self.error(
                        format!("{name} is an instance routine of {}", target.unit),
                        line,
                        col,
                        file,
                    );
                }
                if arg_types.len() != target.params.len() {
                    self.error(
                        format!("{name} expects {} arguments", target.params.len()),
                        line,
                        col,
                        file,
                    );
                } else {
                    for (got, expect) in arg_types.iter().zip(target.params.iter()) {
                        if let Some(got) = got {
                            if got != expect && !(expect == "View" && got == "View") {
                                self.error(
                                    format!("expected {expect}, got {got}"),
                                    line,
                                    col,
                                    file,
                                );
                            }
                        }
                    }
                }
                let dst = target.id;
                let ret = target.ret.clone();
                *resolved = Some(Resolved::Routine(RoutineRef {
                    unit: target.unit,
                    name: target.name,
                    ret: target.ret,
                    params: target.params,
                    is_static: target.is_static,
                }));
                self.db.add_ref(sig.id, dst, "call", file, line, col);
                if as_stmt {
                    return None;
                }
                match ret {
                    None => {
                        self.error(format!("{name} does not return a value"), line, col, file);
                        if let Some(slot) = ty_slot.as_mut() {
                            **slot = None;
                        }
                        None
                    }
                    Some(ret) => {
                        if let Some(slot) = ty_slot.as_mut() {
                            **slot = Some(ret.clone());
                        }
                        Some(ret)
                    }
                }
            }
        }
    }

    fn builtin_id(&mut self, name: &str, params: i32, ret: &str) -> i64 {
        if let Some(id) = self.builtin_ids.get(name) {
            return *id;
        }
        let id = self.db.add(
            "builtin",
            name,
            &format!("builtin.{name}"),
            "",
            1,
            None,
            None,
            ret,
            Over::params(params),
        );
        self.builtin_ids.insert(name.to_string(), id);
        id
    }

    fn check_native(&mut self, native: &NativeChecked) {
        let mut defined: BTreeSet<String> = native.fun.params.iter().cloned().collect();
        self.check_native_stmts(&native.fun.body, &mut defined, &native.file);
    }

    fn check_native_stmts(&mut self, stmts: &[NStmt], defined: &mut BTreeSet<String>, file: &str) {
        for stmt in stmts {
            match stmt {
                NStmt::Var(stmt) => {
                    if !defined.insert(stmt.name.clone()) {
                        self.error(
                            format!("duplicate variable {}", stmt.name),
                            stmt.line,
                            stmt.col,
                            file,
                        );
                    }
                    self.check_native_expr(&stmt.expr, defined, file);
                }
                NStmt::Assign(stmt) => {
                    if !defined.contains(&stmt.name) {
                        self.error(
                            format!("unknown name {}", stmt.name),
                            stmt.line,
                            stmt.col,
                            file,
                        );
                    }
                    self.check_native_expr(&stmt.expr, defined, file);
                }
                NStmt::Return(stmt) => {
                    if let Some(expr) = &stmt.expr {
                        self.check_native_expr(expr, defined, file);
                    }
                }
                NStmt::If(stmt) => {
                    self.check_native_expr(&stmt.cond, defined, file);
                    self.check_native_stmts(&stmt.then, defined, file);
                    self.check_native_stmts(&stmt.else_, defined, file);
                }
                NStmt::While(stmt) => {
                    self.check_native_expr(&stmt.cond, defined, file);
                    self.check_native_stmts(&stmt.body, defined, file);
                }
                NStmt::Block(stmt) => self.check_native_stmts(&stmt.stmts, defined, file),
            }
        }
    }

    fn check_native_expr(&mut self, expr: &NExpr, defined: &BTreeSet<String>, file: &str) {
        match expr {
            NExpr::Lit(_) => {}
            NExpr::Ident(ident) => {
                if !defined.contains(&ident.name) {
                    self.error(
                        format!("unknown name {}", ident.name),
                        ident.line,
                        ident.col,
                        file,
                    );
                }
            }
            NExpr::Unary(un) => self.check_native_expr(&un.expr, defined, file),
            NExpr::Binary(bin) => {
                self.check_native_expr(&bin.left, defined, file);
                self.check_native_expr(&bin.right, defined, file);
            }
            NExpr::Call(call) => {
                match self.natives.get(&low(&call.name)) {
                    None => self.error(
                        format!("unknown native function {}", call.name),
                        call.line,
                        call.col,
                        file,
                    ),
                    Some(target) if call.args.len() != target.params.len() => {
                        self.error(
                            format!("{} expects {} arguments", call.name, target.params.len()),
                            call.line,
                            call.col,
                            file,
                        );
                    }
                    Some(_) => {}
                }
                for arg in &call.args {
                    self.check_native_expr(arg, defined, file);
                }
            }
        }
    }

    fn into_checked(mut self, model: &SourceModel) -> Checked {
        self.db.finish();
        let mut forms = BTreeMap::new();
        for (name, built) in &self.forms {
            let program = &model.forms[name].program;
            let mut routines = BTreeMap::new();
            for (key, sig) in &built.routines {
                routines.insert(
                    key.clone(),
                    RoutineInfo {
                        routine: program.routines[sig.index].clone(),
                        id: sig.id,
                        ret: sig.ret.clone(),
                        params: sig.params.clone(),
                        is_static: sig.is_static,
                        unit: sig.unit.clone(),
                        file: built.file.clone(),
                        index: sig.index,
                    },
                );
            }
            forms.insert(
                name.clone(),
                FormChecked {
                    name: built.name.clone(),
                    file: built.file.clone(),
                    form: built.form.clone(),
                    widgets: built.widgets.clone(),
                    routines,
                    id: built.id,
                },
            );
        }
        let mut units = BTreeMap::new();
        for (name, built) in &self.units {
            let program = &model.units[name].program;
            let mut routines = BTreeMap::new();
            for (key, sig) in &built.routines {
                routines.insert(
                    key.clone(),
                    RoutineInfo {
                        routine: program.routines[sig.index].clone(),
                        id: sig.id,
                        ret: sig.ret.clone(),
                        params: sig.params.clone(),
                        is_static: sig.is_static,
                        unit: sig.unit.clone(),
                        file: built.file.clone(),
                        index: sig.index,
                    },
                );
            }
            units.insert(
                name.clone(),
                UnitChecked {
                    name: built.name.clone(),
                    file: built.file.clone(),
                    routines,
                    id: built.id,
                },
            );
        }
        let errors = self.errors.iter().map(CompileError::to_value).collect();
        Checked {
            errors,
            forms,
            units,
            natives: self.natives,
            db: self.db.to_value(),
        }
    }
}

fn event_name(events: Option<&Value>, name: &str) -> Option<String> {
    let text = events?.as_object()?.get(name)?.as_str()?;
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn expr_decisions(expr: &Expr) -> i32 {
    match expr {
        Expr::Binary(bin) if bin.op == "and" || bin.op == "or" => {
            1 + expr_decisions(&bin.left) + expr_decisions(&bin.right)
        }
        Expr::Binary(bin) => expr_decisions(&bin.left) + expr_decisions(&bin.right),
        Expr::Unary(un) => expr_decisions(&un.expr),
        Expr::Call(call) => call.args.iter().map(expr_decisions).sum(),
        _ => 0,
    }
}

fn complexity(stmts: &[Stmt]) -> i32 {
    let mut total = 0;
    for stmt in stmts {
        match stmt {
            Stmt::If(stmt) => {
                total += 1 + expr_decisions(&stmt.cond);
                total += complexity(std::slice::from_ref(stmt.then.as_ref()));
                total += complexity(std::slice::from_ref(stmt.else_.as_ref()));
            }
            Stmt::While(stmt) => {
                total += 1 + expr_decisions(&stmt.cond);
                total += complexity(std::slice::from_ref(stmt.body.as_ref()));
            }
            Stmt::Block(stmt) => total += complexity(&stmt.stmts),
            Stmt::Assign(stmt) => total += expr_decisions(&stmt.expr),
            Stmt::Call(stmt) => {
                for arg in &stmt.args {
                    total += expr_decisions(arg);
                }
            }
            Stmt::Exit(_) => {}
        }
    }
    total
}

fn depth(stmts: &[Stmt], base: i32) -> i32 {
    let mut best = base;
    for stmt in stmts {
        match stmt {
            Stmt::If(stmt) => {
                best = best
                    .max(depth(std::slice::from_ref(stmt.then.as_ref()), base + 1))
                    .max(depth(std::slice::from_ref(stmt.else_.as_ref()), base + 1));
            }
            Stmt::While(stmt) => {
                best = best.max(depth(std::slice::from_ref(stmt.body.as_ref()), base + 1))
            }
            Stmt::Block(stmt) => best = best.max(depth(&stmt.stmts, base)),
            _ => {}
        }
    }
    best
}

fn n_complexity(stmts: &[NStmt]) -> i32 {
    let mut total = 0;
    for stmt in stmts {
        match stmt {
            NStmt::If(stmt) => total += 1 + n_complexity(&stmt.then) + n_complexity(&stmt.else_),
            NStmt::While(stmt) => total += 1 + n_complexity(&stmt.body),
            NStmt::Block(stmt) => total += n_complexity(&stmt.stmts),
            _ => {}
        }
    }
    total
}

fn n_depth(stmts: &[NStmt], base: i32) -> i32 {
    let mut best = base;
    for stmt in stmts {
        match stmt {
            NStmt::If(stmt) => {
                best = best
                    .max(n_depth(&stmt.then, base + 1))
                    .max(n_depth(&stmt.else_, base + 1));
            }
            NStmt::While(stmt) => best = best.max(n_depth(&stmt.body, base + 1)),
            NStmt::Block(stmt) => best = best.max(n_depth(&stmt.stmts, base)),
            _ => {}
        }
    }
    best
}

fn fmt_expr(expr: &Expr) -> String {
    match expr {
        Expr::Literal(lit) => match &lit.value {
            Lit::Str(text) => format!("'{}'", text.replace('\'', "''")),
            Lit::Bool(true) => "true".into(),
            Lit::Bool(false) => "false".into(),
            Lit::Int(value) => value.to_string(),
        },
        Expr::Ident(ident) => ident.name.clone(),
        Expr::Designator(des) => des.parts.join("."),
        Expr::Call(call) => {
            let args = call
                .args
                .iter()
                .map(fmt_expr)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({args})", call.name)
        }
        Expr::Unary(un) => format!("{} {}", un.op, fmt_expr(&un.expr)),
        Expr::Binary(bin) => format!(
            "({} {} {})",
            fmt_expr(&bin.left),
            bin.op,
            fmt_expr(&bin.right)
        ),
    }
}

fn fmt_stmt(stmt: &Stmt) -> String {
    match stmt {
        Stmt::Assign(stmt) => format!(
            "{} := {}",
            stmt.target.parts.join("."),
            fmt_expr(&stmt.expr)
        ),
        Stmt::Call(stmt) => {
            let args = stmt
                .args
                .iter()
                .map(fmt_expr)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({args})", stmt.name)
        }
        Stmt::Exit(_) => "Exit".into(),
        Stmt::If(stmt) => format!("if {}", fmt_expr(&stmt.cond)),
        Stmt::While(stmt) => format!("while {}", fmt_expr(&stmt.cond)),
        Stmt::Block(_) => "begin".into(),
    }
}

fn nfmt(expr: &NExpr) -> String {
    match expr {
        NExpr::Lit(lit) => lit.value.to_string(),
        NExpr::Ident(ident) => ident.name.clone(),
        NExpr::Unary(un) => format!("{}{}", un.op, nfmt(&un.expr)),
        NExpr::Binary(bin) => format!("({} {} {})", nfmt(&bin.left), bin.op, nfmt(&bin.right)),
        NExpr::Call(call) => {
            let args = call.args.iter().map(nfmt).collect::<Vec<_>>().join(", ");
            format!("{}({args})", call.name)
        }
    }
}

fn nfmt_stmt(stmt: &NStmt) -> String {
    match stmt {
        NStmt::Var(stmt) => format!("var {} = {}", stmt.name, nfmt(&stmt.expr)),
        NStmt::Assign(stmt) => format!("{} = {}", stmt.name, nfmt(&stmt.expr)),
        NStmt::Return(stmt) => match &stmt.expr {
            Some(expr) => {
                let text = nfmt(expr);
                if text.is_empty() {
                    "return".into()
                } else {
                    format!("return {text}")
                }
            }
            None => "return".into(),
        },
        NStmt::If(stmt) => format!("if {}", nfmt(&stmt.cond)),
        NStmt::While(stmt) => format!("while {}", nfmt(&stmt.cond)),
        NStmt::Block(_) => "NBlock".into(),
    }
}

struct Flow {
    blocks: Vec<FlowBlock>,
    exit_id: usize,
}

struct FlowBlock {
    label: String,
    text: Vec<String>,
    line: i32,
    succs: Vec<usize>,
}

impl Flow {
    fn new(exit_text: &str, end_line: i32) -> Self {
        let mut flow = Self {
            blocks: Vec::new(),
            exit_id: 0,
        };
        flow.exit_id = flow.push("exit", vec![exit_text.into()], end_line);
        flow
    }

    fn push(&mut self, label: &str, text: Vec<String>, line: i32) -> usize {
        let id = self.blocks.len();
        self.blocks.push(FlowBlock {
            label: label.into(),
            text,
            line,
            succs: Vec::new(),
        });
        id
    }

    fn link(&mut self, src: usize, dst: usize) {
        if !self.blocks[src].succs.contains(&dst) {
            self.blocks[src].succs.push(dst);
        }
    }

    fn to_value(&self, entry: usize) -> Value {
        let blocks: Vec<Value> = self
            .blocks
            .iter()
            .enumerate()
            .map(|(id, block)| {
                json!({
                    "id": id,
                    "label": block.label,
                    "text": block.text,
                    "line": block.line,
                    "succs": block.succs,
                })
            })
            .collect();
        json!({ "entry": entry, "blocks": blocks })
    }
}

fn cfg_split(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::If(_) | Stmt::While(_) | Stmt::Block(_) | Stmt::Exit(_)
    )
}

fn build_cfg(routine: &Routine) -> Value {
    let mut flow = Flow::new("exit", routine.end_line);
    let entry = flow.sequence(&routine.body, flow.exit_id);
    flow.to_value(entry)
}

impl Flow {
    fn sequence(&mut self, stmts: &[Stmt], cont: usize) -> usize {
        let mut idx = 0;
        while idx < stmts.len() && !cfg_split(&stmts[idx]) {
            idx += 1;
        }
        if idx == stmts.len() {
            if stmts.is_empty() {
                return cont;
            }
            let text = stmts.iter().map(fmt_stmt).collect();
            let line = stmts[0].line();
            let block = self.push("code", text, line);
            self.link(block, cont);
            return block;
        }
        if idx > 0 {
            let text = stmts[..idx].iter().map(fmt_stmt).collect();
            let line = stmts[0].line();
            let head = self.push("code", text, line);
            let next = self.one(&stmts[idx], &stmts[idx + 1..], cont);
            self.link(head, next);
            return head;
        }
        self.one(&stmts[idx], &stmts[idx + 1..], cont)
    }

    fn rest(&mut self, rest: &[Stmt], cont: usize) -> usize {
        if rest.is_empty() {
            cont
        } else {
            self.sequence(rest, cont)
        }
    }

    fn branch(&mut self, stmt: &Stmt, cont: usize) -> usize {
        match stmt {
            Stmt::Block(block) => self.sequence(&block.stmts, cont),
            other => self.sequence(std::slice::from_ref(other), cont),
        }
    }

    fn one(&mut self, stmt: &Stmt, rest: &[Stmt], cont: usize) -> usize {
        match stmt {
            Stmt::Block(block) => {
                let after = self.rest(rest, cont);
                self.sequence(&block.stmts, after)
            }
            Stmt::Exit(stmt) => {
                let _after = self.rest(rest, cont);
                let block = self.push("exit-jump", vec!["Exit".into()], stmt.line);
                self.link(block, self.exit_id);
                block
            }
            Stmt::If(stmt) => {
                let after = self.rest(rest, cont);
                let text = vec![format!("if {}", fmt_expr(&stmt.cond))];
                let cond = self.push("if", text, stmt.line);
                let then_b = self.branch(&stmt.then, after);
                let else_b = self.branch(&stmt.else_, after);
                self.link(cond, then_b);
                self.link(cond, else_b);
                cond
            }
            Stmt::While(stmt) => {
                let after = self.rest(rest, cont);
                let text = vec![format!("while {}", fmt_expr(&stmt.cond))];
                let cond = self.push("while", text, stmt.line);
                let body = self.branch(&stmt.body, cond);
                self.link(cond, body);
                self.link(cond, after);
                cond
            }
            other => {
                let after = self.rest(rest, cont);
                let block = self.push("code", vec![fmt_stmt(other)], other.line());
                self.link(block, after);
                block
            }
        }
    }
}

fn native_straight(stmt: &NStmt) -> bool {
    matches!(stmt, NStmt::Var(_) | NStmt::Assign(_))
}

fn build_native_cfg(fun: &NFun) -> Value {
    let mut flow = NativeFlow::new(fun.end_line);
    let entry = flow.sequence(&fun.body, flow.exit_id);
    flow.to_value(entry)
}

struct NativeFlow {
    blocks: Vec<FlowBlock>,
    exit_id: usize,
}

impl NativeFlow {
    fn new(end_line: i32) -> Self {
        let mut flow = Self {
            blocks: Vec::new(),
            exit_id: 0,
        };
        flow.exit_id = flow.push("exit", vec!["return".into()], end_line);
        flow
    }

    fn push(&mut self, label: &str, text: Vec<String>, line: i32) -> usize {
        let id = self.blocks.len();
        self.blocks.push(FlowBlock {
            label: label.into(),
            text,
            line,
            succs: Vec::new(),
        });
        id
    }

    fn link(&mut self, src: usize, dst: usize) {
        if !self.blocks[src].succs.contains(&dst) {
            self.blocks[src].succs.push(dst);
        }
    }

    fn to_value(&self, entry: usize) -> Value {
        let blocks: Vec<Value> = self
            .blocks
            .iter()
            .enumerate()
            .map(|(id, block)| {
                json!({
                    "id": id,
                    "label": block.label,
                    "text": block.text,
                    "line": block.line,
                    "succs": block.succs,
                })
            })
            .collect();
        json!({ "entry": entry, "blocks": blocks })
    }

    fn sequence(&mut self, stmts: &[NStmt], cont: usize) -> usize {
        let mut idx = 0;
        while idx < stmts.len() && native_straight(&stmts[idx]) {
            idx += 1;
        }
        if idx == stmts.len() {
            if stmts.is_empty() {
                return cont;
            }
            let text = stmts.iter().map(nfmt_stmt).collect();
            let line = stmts[0].line();
            let block = self.push("code", text, line);
            self.link(block, cont);
            return block;
        }
        if idx > 0 {
            let text = stmts[..idx].iter().map(nfmt_stmt).collect();
            let line = stmts[0].line();
            let head = self.push("code", text, line);
            let next = self.one(&stmts[idx], &stmts[idx + 1..], cont);
            self.link(head, next);
            return head;
        }
        self.one(&stmts[idx], &stmts[idx + 1..], cont)
    }

    fn rest(&mut self, rest: &[NStmt], cont: usize) -> usize {
        if rest.is_empty() {
            cont
        } else {
            self.sequence(rest, cont)
        }
    }

    fn one(&mut self, stmt: &NStmt, rest: &[NStmt], cont: usize) -> usize {
        match stmt {
            NStmt::Block(block) => {
                let after = self.rest(rest, cont);
                self.sequence(&block.stmts, after)
            }
            NStmt::Return(stmt) => {
                let _after = self.rest(rest, cont);
                let block = self.push(
                    "return",
                    vec![nfmt_stmt(&NStmt::Return(stmt.clone()))],
                    stmt.line,
                );
                self.link(block, self.exit_id);
                block
            }
            NStmt::If(stmt) => {
                let after = self.rest(rest, cont);
                let cond = self.push("if", vec![format!("if {}", nfmt(&stmt.cond))], stmt.line);
                let then_b = self.sequence(&stmt.then, after);
                let else_b = if stmt.else_.is_empty() {
                    after
                } else {
                    self.sequence(&stmt.else_, after)
                };
                self.link(cond, then_b);
                self.link(cond, else_b);
                cond
            }
            NStmt::While(stmt) => {
                let after = self.rest(rest, cont);
                let cond = self.push(
                    "while",
                    vec![format!("while {}", nfmt(&stmt.cond))],
                    stmt.line,
                );
                let body = self.sequence(&stmt.body, cond);
                self.link(cond, body);
                self.link(cond, after);
                cond
            }
            other => {
                let after = self.rest(rest, cont);
                let block = self.push("code", vec![nfmt_stmt(other)], other.line());
                self.link(block, after);
                block
            }
        }
    }
}
