//! Project layout on disk: project.json, forms, units, native files.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::check::{FormSource, NativeSource, SourceModel, UnitSource};
use crate::error::CompileError;
use crate::parse::{parse_droid, parse_native};

fn write_pretty(path: &Path, value: &Value) -> Result<(), CompileError> {
    let text = serde_json::to_string_pretty(value).map_err(|e| CompileError::bare(e.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| CompileError::bare(e.to_string()))?;
    }
    fs::write(path, format!("{text}\n")).map_err(|e| CompileError::bare(e.to_string()))
}

const JAVA_WORDS: &[&str] = &[
    "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char", "class", "const",
    "continue", "default", "do", "double", "else", "enum", "extends", "final", "finally", "float",
    "for", "goto", "if", "implements", "import", "instanceof", "int", "interface", "long", "native",
    "new", "package", "private", "protected", "public", "return", "short", "static", "strictfp",
    "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try", "void",
    "volatile", "while", "true", "false", "null",
];

pub fn valid_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    !JAVA_WORDS.iter().any(|w| w.eq_ignore_ascii_case(name))
}

pub fn valid_package(package: &str) -> bool {
    let parts: Vec<&str> = package.split('.').collect();
    !parts.is_empty() && parts.iter().all(|part| valid_ident(part))
}

pub fn class_ok(name: &str) -> bool {
    valid_ident(name) && !matches!(name.to_ascii_lowercase().as_str(), "droidert" | "native")
}

#[derive(Clone, Debug)]
pub struct FileInfo {
    pub kind: String,
    pub name: String,
    pub source: String,
    pub form: Option<String>,
}

pub struct Project {
    pub root: PathBuf,
    pub meta: Value,
}

impl Project {
    pub fn create(root: impl AsRef<Path>, name: &str, package: &str, title: Option<&str>) -> Result<Self, CompileError> {
        let root = root.as_ref();
        if root.join("project.json").exists() {
            return Err(CompileError::bare(format!("{} already has a project.json", root.display())));
        }
        if !class_ok(name) {
            return Err(CompileError::bare(format!("bad project name {name}")));
        }
        if !valid_package(package) {
            return Err(CompileError::bare(format!("bad package {package}")));
        }
        let title = title.unwrap_or(name);
        fs::create_dir_all(root.join("forms")).map_err(|e| CompileError::bare(e.to_string()))?;
        fs::create_dir_all(root.join("units")).map_err(|e| CompileError::bare(e.to_string()))?;
        fs::create_dir_all(root.join("native")).map_err(|e| CompileError::bare(e.to_string()))?;
        let meta = json!({
            "name": name,
            "package": package,
            "title": title,
            "versionCode": 1,
            "versionName": "1.0",
            "mainForm": "Main",
            "minSdk": 24,
            "targetSdk": 28,
            "permissions": [],
        });
        write_pretty(&root.join("project.json"), &meta)?;
        let mut form = crate::widgets::blank_form("Main", Some(title));
        let mut label = crate::widgets::new_component("Label", "TitleLabel", 16, 24);
        label["props"]["Text"] = json!(title);
        label["props"]["TextSize"] = json!(22);
        label["width"] = json!(328);
        let mut button = crate::widgets::new_component("Button", "OkButton", 16, 88);
        button["props"]["Text"] = json!("Click");
        button["events"]["OnClick"] = json!("OkClick");
        form["components"] = json!([label, button]);
        write_pretty(&root.join("forms/Main.form.json"), &form)?;
        fs::write(
            root.join("forms/Main.droid"),
            "procedure OkClick(Sender: View);\nbegin\n  Toast('Hello from Droide')\nend;\n",
        )
        .map_err(|e| CompileError::bare(e.to_string()))?;
        fs::write(
            root.join("units/Utils.droid"),
            "unit Utils;\n\nfunction Clamp(Value, Lo, Hi: Integer): Integer;\nbegin\n  if Value < Lo then\n    Result := Lo\n  else if Value > Hi then\n    Result := Hi\n  else\n    Result := Value\nend;\n",
        )
        .map_err(|e| CompileError::bare(e.to_string()))?;
        fs::write(
            root.join("native/app.nc"),
            "fun echo(n: i32): i32 {\n  return n;\n}\n",
        )
        .map_err(|e| CompileError::bare(e.to_string()))?;
        Self::open(root)
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self, CompileError> {
        let root = root.as_ref().to_path_buf();
        let text = fs::read_to_string(root.join("project.json")).unwrap_or_default();
        let meta = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
        Ok(Self { root, meta })
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel).unwrap_or_else(|_| self.root.join(rel))).unwrap_or_default()
    }

    pub fn path(&self, rel: &str) -> Result<PathBuf, CompileError> {
        let rel = rel.replace('\\', "/");
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() || rel.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
            return Err(CompileError::bare(format!("bad project path {rel}")));
        }
        let path = self.root.join(rel);
        Ok(path)
    }

    fn text(&self, rel: &str, buffers: &BTreeMap<String, String>) -> String {
        buffers.get(rel).cloned().unwrap_or_else(|| self.read(rel))
    }

    pub fn list_files(&self, buffers: &BTreeMap<String, String>) -> Vec<FileInfo> {
        let mut found: BTreeMap<String, FileInfo> = BTreeMap::new();
        let mut add = |info: FileInfo| {
            found.insert(info.source.clone(), info);
        };
        let forms = self.root.join("forms");
        if forms.is_dir() {
            if let Ok(rd) = fs::read_dir(&forms) {
                let mut names: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".form.json"))
                    .collect();
                names.sort();
                for file_name in names {
                    let name = file_name.trim_end_matches(".form.json").to_string();
                    add(FileInfo {
                        kind: "form".into(),
                        name: name.clone(),
                        source: format!("forms/{name}.droid"),
                        form: Some(format!("forms/{name}.form.json")),
                    });
                }
            }
        }
        for rel in buffers.keys() {
            if let Some(name) = rel.strip_prefix("forms/").and_then(|r| r.strip_suffix(".form.json")) {
                add(FileInfo {
                    kind: "form".into(),
                    name: name.into(),
                    source: format!("forms/{name}.droid"),
                    form: Some(rel.clone()),
                });
            }
        }
        let units = self.root.join("units");
        if units.is_dir() {
            if let Ok(rd) = fs::read_dir(&units) {
                let mut names: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".droid"))
                    .collect();
                names.sort();
                for file_name in names {
                    let name = file_name.trim_end_matches(".droid").to_string();
                    add(FileInfo {
                        kind: "unit".into(),
                        name: name.clone(),
                        source: format!("units/{file_name}"),
                        form: None,
                    });
                }
            }
        }
        for rel in buffers.keys() {
            if rel.starts_with("units/") && rel.ends_with(".droid") {
                let name = Path::new(rel).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                add(FileInfo { kind: "unit".into(), name, source: rel.clone(), form: None });
            }
        }
        let native = self.root.join("native");
        if native.is_dir() {
            if let Ok(rd) = fs::read_dir(&native) {
                let mut names: Vec<String> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".nc"))
                    .collect();
                names.sort();
                for file_name in names {
                    let name = file_name.trim_end_matches(".nc").to_string();
                    add(FileInfo {
                        kind: "native".into(),
                        name,
                        source: format!("native/{file_name}"),
                        form: None,
                    });
                }
            }
        }
        for rel in buffers.keys() {
            if rel.starts_with("native/") && rel.ends_with(".nc") {
                let name = Path::new(rel).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
                add(FileInfo { kind: "native".into(), name, source: rel.clone(), form: None });
            }
        }
        let mut files: Vec<FileInfo> = found.into_values().collect();
        files.sort_by(|a, b| {
            let rank = |k: &str| match k { "form" => 0, "unit" => 1, "native" => 2, _ => 9 };
            rank(&a.kind).cmp(&rank(&b.kind)).then(a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
        });
        files.push(FileInfo { kind: "project".into(), name: "project.json".into(), source: "project.json".into(), form: None });
        files
    }

    pub fn load(&self, buffers: &BTreeMap<String, String>) -> (Value, SourceModel, Vec<Value>) {
        let mut errors = Vec::new();
        let meta_text = self.text("project.json", buffers);
        let meta = match serde_json::from_str::<Value>(&meta_text) {
            Ok(v) => v,
            Err(exc) => {
                errors.push(json!({"message": format!("bad project.json: {exc}"), "line": 1, "col": 1, "file": "project.json"}));
                self.meta.clone()
            }
        };
        let mut forms = BTreeMap::new();
        let mut units = BTreeMap::new();
        let mut natives = BTreeMap::new();
        for info in self.list_files(buffers) {
            match info.kind.as_str() {
                "form" => {
                    let form_rel = info.form.clone().unwrap_or_default();
                    let raw = self.text(&form_rel, buffers);
                    let form = match serde_json::from_str::<Value>(&raw) {
                        Ok(v) => v,
                        Err(exc) => {
                            errors.push(json!({"message": format!("bad form json: {exc}"), "line": 1, "col": 1, "file": form_rel}));
                            continue;
                        }
                    };
                    if form.get("name").and_then(|v| v.as_str()) != Some(info.name.as_str()) {
                        errors.push(json!({
                            "message": format!("form name {:?} does not match {}", form.get("name"), info.name),
                            "line": 1, "col": 1, "file": form_rel,
                        }));
                    }
                    let src = self.text(&info.source, buffers);
                    let program = match parse_droid(&src, &info.source) {
                        Ok(p) => p,
                        Err(exc) => {
                            errors.push(exc.to_value());
                            parse_droid("", &info.source).unwrap_or_else(|_| crate::ast::Program {
                                line: 1, unit: None, uses: Vec::new(), routines: Vec::new(),
                            })
                        }
                    };
                    forms.insert(info.name, FormSource { program, form, file: info.source });
                }
                "unit" => {
                    let src = self.text(&info.source, buffers);
                    match parse_droid(&src, &info.source) {
                        Ok(program) => {
                            units.insert(info.name, UnitSource { program, file: info.source });
                        }
                        Err(exc) => errors.push(exc.to_value()),
                    }
                }
                "native" => {
                    let src = self.text(&info.source, buffers);
                    match parse_native(&src, &info.source) {
                        Ok(program) => {
                            natives.insert(info.source.clone(), NativeSource { program, file: info.source });
                        }
                        Err(exc) => errors.push(exc.to_value()),
                    }
                }
                _ => {}
            }
        }
        (meta, SourceModel { forms, units, natives }, errors)
    }
}
