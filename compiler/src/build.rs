//! Compile a project, sign the APK, and launch it. Same steps as the Python driver.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::apk::pack_apk;
use crate::ast::NFun;
use crate::axml::build_manifest;
use crate::check::check_model;
use crate::codegen::compile_dex;
use crate::dex::check_dex;
use crate::error::CompileError;
use crate::native::compile_library;
use crate::project::{class_ok, valid_ident, valid_package, Project};
use crate::vm::verify_image;

#[derive(Clone, Debug)]
pub struct JobResult {
    pub ok: bool,
    pub errors: Vec<Value>,
    pub db: Value,
    pub meta: Value,
    pub root: PathBuf,
    pub listing: String,
    pub apk: Option<PathBuf>,
    pub size: u64,
    pub verify: String,
    pub manifest: String,
    pub launched: bool,
    pub device: String,
    pub log: String,
}

impl JobResult {
    fn from_compile(root: PathBuf, meta: Value, errors: Vec<Value>, db: Value, listing: String) -> Self {
        Self {
            ok: errors.is_empty(),
            errors,
            db,
            meta,
            root,
            listing,
            apk: None,
            size: 0,
            verify: String::new(),
            manifest: String::new(),
            launched: false,
            device: String::new(),
            log: String::new(),
        }
    }
}

fn empty_db() -> Value {
    json!({"entities": [], "refs": [], "cfgs": {}})
}

fn err_value(message: impl Into<String>, file: &str, line: i32, col: i32) -> Value {
    json!({"message": message.into(), "line": line, "col": col, "file": file})
}

fn has_form(forms: &impl FormMap, main: &str) -> bool {
    forms.form_names().iter().any(|name| name == main)
}

/// The checker stores forms, units, and natives under their own structs.
/// These helpers read the fields the driver needs.
trait FormMap {
    fn form_names(&self) -> Vec<String>;
    fn form_rows(&self) -> Vec<FormRow>;
    fn unit_rows(&self) -> Vec<UnitRow>;
    fn native_rows(&self) -> Vec<NativeRow>;
}

struct FormRow {
    pub name: String,
    pub file: String,
    pub widgets: Vec<String>,
}

struct UnitRow {
    pub name: String,
    pub file: String,
}

struct NativeRow {
    pub name: String,
    pub file: String,
    pub line: i32,
    pub col: i32,
    pub fun: NFun,
}

impl FormMap for crate::check::Checked {
    fn form_names(&self) -> Vec<String> {
        self.forms.values().map(|form| form.name.clone()).collect()
    }

    fn form_rows(&self) -> Vec<FormRow> {
        self.forms
            .values()
            .map(|form| FormRow {
                name: form.name.clone(),
                file: form.file.clone(),
                widgets: form.widgets.values().map(|widget| widget.name.clone()).collect(),
            })
            .collect()
    }

    fn unit_rows(&self) -> Vec<UnitRow> {
        self.units
            .values()
            .map(|unit| UnitRow { name: unit.name.clone(), file: unit.file.clone() })
            .collect()
    }

    fn native_rows(&self) -> Vec<NativeRow> {
        self.natives
            .values()
            .map(|native| NativeRow {
                name: native.name.clone(),
                file: native.file.clone(),
                line: native.fun.line,
                col: native.fun.col,
                fun: native.fun.clone(),
            })
            .collect()
    }
}

fn structural(meta: &Value, checked: &crate::check::Checked) -> Vec<Value> {
    let mut errors = Vec::new();
    let package = meta.get("package").and_then(|v| v.as_str()).unwrap_or("");
    if !valid_package(package) {
        errors.push(err_value(format!("bad package {package}"), "project.json", 1, 1));
    }
    let main = meta.get("mainForm").and_then(|v| v.as_str()).unwrap_or("");
    if !has_form(checked, main) {
        errors.push(err_value(format!("mainForm {main} is not a form"), "project.json", 1, 1));
    }
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut clash = |name: &str, file: &str, errors: &mut Vec<Value>| {
        let key = name.to_ascii_lowercase();
        if let Some((_, other)) = seen.iter().find(|(k, _)| k == &key) {
            errors.push(err_value(format!("{name} clashes with {other}"), file, 1, 1));
        }
        seen.push((key, name.to_string()));
    };
    for form in checked.form_rows() {
        if !class_ok(&form.name) {
            errors.push(err_value(
                format!("form name {} cannot be an Android class", form.name),
                &form.file,
                1,
                1,
            ));
        }
        clash(&form.name, &form.file, &mut errors);
        for widget in &form.widgets {
            if !valid_ident(widget) {
                errors.push(err_value(
                    format!("component name {widget} is not an identifier"),
                    &form.file,
                    1,
                    1,
                ));
            }
        }
    }
    for unit in checked.unit_rows() {
        if !class_ok(&unit.name) {
            errors.push(err_value(
                format!("unit name {} cannot be an Android class", unit.name),
                &unit.file,
                1,
                1,
            ));
        }
        clash(&unit.name, &unit.file, &mut errors);
    }
    for native in checked.native_rows() {
        if !valid_ident(&native.name) {
            errors.push(err_value(
                format!("native function {} is not an identifier", native.name),
                &native.file,
                native.line,
                native.col,
            ));
        }
    }
    errors
}

struct Compiled {
    listing: String,
    apk: Vec<u8>,
}

fn emit_apk(meta: &Value, checked: &crate::check::Checked) -> Result<Compiled, CompileError> {
    let package = meta.get("package").and_then(|v| v.as_str()).unwrap_or("");
    let compiled = compile_dex(package, checked)?;
    check_dex(&compiled.bytes)?;
    let image = compiled
        .dex
        .image
        .as_ref()
        .ok_or_else(|| CompileError::bare("dex image missing"))?;
    verify_image(image)?;
    let listing = compiled.dex.listing.join("\n");
    let funs: Vec<NFun> = checked.native_rows().into_iter().map(|row| row.fun).collect();
    let lib = if funs.is_empty() { None } else { compile_library(package, &funs)? };
    let mut names = checked.form_names();
    names.sort_by(|a, b| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
    let main = meta.get("mainForm").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(pos) = names.iter().position(|name| name == main) {
        let name = names.remove(pos);
        names.insert(0, name);
    }
    let manifest = build_manifest(meta, &names);
    let mut entries = vec![
        ("AndroidManifest.xml".to_string(), manifest),
        ("classes.dex".to_string(), compiled.bytes),
    ];
    if let Some(lib) = lib {
        entries.push(("lib/arm64-v8a/libdroide.so".to_string(), lib.arm));
        entries.push(("lib/x86_64/libdroide.so".to_string(), lib.x86));
    }
    let apk = pack_apk(&entries)?;
    Ok(Compiled { listing, apk })
}

pub fn analyze(root: &Path) -> JobResult {
    let project = match Project::open(root) {
        Ok(project) => project,
        Err(exc) => {
            return JobResult::from_compile(root.to_path_buf(), json!({}), vec![exc.to_value()], empty_db(), String::new());
        }
    };
    let (meta, mut model, mut errors) = project.load(&std::collections::BTreeMap::new());
    let checked = check_model(&mut model);
    errors.extend(checked.errors.clone());
    errors.extend(structural(&meta, &checked));
    let db = if checked.db.is_null() { empty_db() } else { checked.db.clone() };
    JobResult::from_compile(project.root, meta, errors, db, String::new())
}

pub fn build(root: &Path) -> Result<JobResult, CompileError> {
    let project = Project::open(root)?;
    let (meta, mut model, mut errors) = project.load(&std::collections::BTreeMap::new());
    let checked = check_model(&mut model);
    errors.extend(checked.errors.clone());
    errors.extend(structural(&meta, &checked));
    let db = if checked.db.is_null() { empty_db() } else { checked.db.clone() };
    if !errors.is_empty() {
        return Ok(JobResult::from_compile(project.root, meta, errors, db, String::new()));
    }
    let compiled = match emit_apk(&meta, &checked) {
        Ok(compiled) => compiled,
        Err(exc) => {
            errors.push(exc.to_value());
            return Ok(JobResult::from_compile(project.root, meta, errors, db, String::new()));
        }
    };
    let out = project.root.join("build");
    fs::create_dir_all(&out).map_err(|e| CompileError::bare(e.to_string()))?;
    let apk_path = out.join("app.apk");
    fs::write(&apk_path, &compiled.apk).map_err(|e| CompileError::bare(e.to_string()))?;
    if !compiled.listing.is_empty() {
        fs::write(out.join("classes.lst"), &compiled.listing).map_err(|e| CompileError::bare(e.to_string()))?;
    }
    let mut result = JobResult::from_compile(project.root, meta, errors, db, compiled.listing);
    result.apk = Some(apk_path.clone());
    result.size = fs::metadata(&apk_path).map(|m| m.len()).unwrap_or(compiled.apk.len() as u64);
    if let Some(signer) = tool("apksigner") {
        let (_code, text) = run_cmd(&[
            signer.display().to_string(),
            "verify".into(),
            "--min-sdk-version".into(),
            "24".into(),
            "-v".into(),
            apk_path.display().to_string(),
        ]);
        result.verify = text;
    }
    if let Some(aapt) = tool("aapt") {
        let (_code, text) = run_cmd(&[
            aapt.display().to_string(),
            "dump".into(),
            "xmltree".into(),
            apk_path.display().to_string(),
            "AndroidManifest.xml".into(),
        ]);
        result.manifest = text;
    }
    Ok(result)
}

pub fn run(root: &Path) -> Result<JobResult, CompileError> {
    let mut result = build(root)?;
    result.launched = false;
    result.device.clear();
    result.log.clear();
    if !result.ok {
        result.log = "build failed".into();
        return Ok(result);
    }
    let Some(adb) = adb_path() else {
        result.log = format!("adb was not found. APK is at {}", result.apk.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
        return Ok(result);
    };
    let (serial, note) = ensure_serial(&adb);
    result.log = note;
    let Some(serial) = serial else {
        result.log.push_str(&format!("APK is at {}\n", result.apk.as_ref().map(|p| p.display().to_string()).unwrap_or_default()));
        return Ok(result);
    };
    result.device = serial.clone();
    let apk = result.apk.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let (code, install) = run_cmd(&[
        adb.display().to_string(),
        "-s".into(),
        serial.clone(),
        "install".into(),
        "-r".into(),
        apk,
    ]);
    result.log.push_str(&install);
    if code != 0 || !install.contains("Success") {
        return Ok(result);
    }
    let package = result.meta.get("package").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let main = result.meta.get("mainForm").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let _ = run_cmd(&[adb.display().to_string(), "-s".into(), serial.clone(), "logcat".into(), "-c".into()]);
    let (code, started) = run_cmd(&[
        adb.display().to_string(),
        "-s".into(),
        serial.clone(),
        "shell".into(),
        "am".into(),
        "start".into(),
        "-W".into(),
        "-n".into(),
        format!("{package}/.{main}"),
    ]);
    result.log.push_str(&started);
    if let Some(crash) = launch_failure(&adb, &serial, &package) {
        result.log.push('\n');
        result.log.push_str(&crash);
        result.log.push('\n');
        return Ok(result);
    }
    result.launched = code == 0 && !started.contains("Error");
    Ok(result)
}

pub fn print_result(result: &JobResult, verbose: bool) -> i32 {
    for err in &result.errors {
        let file = err.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let line = err.get("line").and_then(|v| v.as_i64()).unwrap_or(1);
        let message = err.get("message").and_then(|v| v.as_str()).unwrap_or("");
        println!("{file}:{line}: {message}");
    }
    if result.ok {
        if let Some(apk) = &result.apk {
            println!("apk {} ({} bytes)", apk.display(), result.size);
        }
    }
    if verbose && !result.verify.is_empty() {
        println!("{}", result.verify.trim_end());
    }
    if !result.log.is_empty() {
        println!("{}", result.log.trim_end());
    }
    if result.ok { 0 } else { 1 }
}

pub fn print_metrics(db: &Value) {
    let Some(entities) = db.get("entities").and_then(|v| v.as_array()) else {
        return;
    };
    let mut rows: Vec<&Value> = entities
        .iter()
        .filter(|ent| matches!(ent.get("kind").and_then(|v| v.as_str()), Some("procedure" | "function" | "native")))
        .collect();
    rows.sort_by(|a, b| {
        let ca = a.get("complexity").and_then(|v| v.as_i64()).unwrap_or(0);
        let cb = b.get("complexity").and_then(|v| v.as_i64()).unwrap_or(0);
        let qa = a.get("qname").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
        let qb = b.get("qname").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
        cb.cmp(&ca).then(qa.cmp(&qb))
    });
    for ent in rows {
        let kind = ent.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let qname = ent.get("qname").and_then(|v| v.as_str()).unwrap_or("");
        let loc = ent.get("loc").and_then(|v| v.as_i64()).unwrap_or(0);
        let cc = ent.get("complexity").and_then(|v| v.as_i64()).unwrap_or(0);
        let fan_in = ent.get("fanIn").and_then(|v| v.as_i64()).unwrap_or(0);
        let fan_out = ent.get("fanOut").and_then(|v| v.as_i64()).unwrap_or(0);
        println!("{kind:<10} {qname:<24} loc {loc:<4} cc {cc:<3} in {fan_in:<3} out {fan_out}");
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(env) = std::env::var_os("ANDROID_HOME").or_else(|| std::env::var_os("ANDROID_SDK_ROOT")) {
        roots.push(PathBuf::from(env));
    }
    roots.push(home().join(".android-sdk"));
    roots
}

fn tool(name: &str) -> Option<PathBuf> {
    for root in sdk_roots() {
        let tools = root.join("build-tools");
        let Ok(rd) = fs::read_dir(&tools) else { continue };
        let mut versions: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        for version in versions {
            let candidate = version.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    which(name)
}

fn adb_path() -> Option<PathBuf> {
    if let Some(found) = which("adb") {
        return Some(found);
    }
    let candidate = home().join(".android-sdk/platform-tools/adb");
    if candidate.is_file() { Some(candidate) } else { None }
}

fn emulator_path() -> Option<PathBuf> {
    for root in sdk_roots() {
        let candidate = root.join("emulator/emulator");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    which("emulator")
}

fn run_cmd(args: &[String]) -> (i32, String) {
    if args.is_empty() {
        return (1, "empty command".into());
    }
    match Command::new(&args[0]).args(&args[1..]).output() {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&out.stderr));
            (out.status.code().unwrap_or(1), text)
        }
        Err(err) => (1, err.to_string()),
    }
}

fn online(text: &str) -> Vec<String> {
    let mut serials = Vec::new();
    for line in text.lines() {
        if !line.contains("\tdevice") {
            continue;
        }
        let serial = line.split('\t').next().unwrap_or("").trim();
        if !serial.is_empty() {
            serials.push(serial.to_string());
        }
    }
    serials
}

fn avds(emulator: &Path) -> Vec<String> {
    let (code, text) = run_cmd(&[emulator.display().to_string(), "-list-avds".into()]);
    if code != 0 {
        return Vec::new();
    }
    text.lines()
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.contains(' ') && !name.starts_with("INFO") && !name.starts_with("ERROR") && !name.starts_with("WARNING"))
        .map(str::to_string)
        .collect()
}

fn booted(adb: &Path, serial: &str) -> bool {
    let (code, out) = run_cmd(&[
        adb.display().to_string(),
        "-s".into(),
        serial.into(),
        "shell".into(),
        "getprop".into(),
        "sys.boot_completed".into(),
    ]);
    code == 0 && out.trim().replace('\r', "") == "1"
}

fn wait_boot(adb: &Path, serial: &str, timeout: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(timeout);
    while Instant::now() < deadline {
        if booted(adb, serial) {
            return true;
        }
        thread::sleep(Duration::from_secs(2));
    }
    false
}

fn emulator_args(emulator: &Path, avd: &str, window: bool) -> Vec<String> {
    let mut cmd = vec![
        emulator.display().to_string(),
        "-avd".into(),
        avd.into(),
        "-no-audio".into(),
        "-no-boot-anim".into(),
    ];
    if !window {
        cmd.extend(["-no-window".into(), "-gpu".into(), "swiftshader_indirect".into()]);
    }
    cmd
}

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
    fn killpg(pid: i32, sig: i32) -> i32;
}

fn start_and_wait(adb: &Path, emulator: &Path, avd: &str, window: bool, timeout: u64) -> (Option<String>, String) {
    let args = emulator_args(emulator, avd, window);
    let log_path = std::env::temp_dir().join("droide-emulator.log");
    let note = format!("starting emulator {avd}\n");
    let log = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(file) => file,
        Err(err) => return (None, format!("{note}{err}\n")),
    };
    let err_log = match log.try_clone() {
        Ok(file) => file,
        Err(err) => return (None, format!("{note}{err}\n")),
    };
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]).stdout(Stdio::from(log)).stderr(Stdio::from(err_log));
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            if setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return (None, format!("{note}{err}\n")),
    };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut tail = String::new();
                if let Ok(mut file) = fs::File::open(&log_path) {
                    let mut all = String::new();
                    let _ = file.read_to_string(&mut all);
                    let start = all.len().saturating_sub(1200);
                    tail = all[start..].to_string();
                }
                return (None, format!("{note}emulator exited ({})\n{tail}", status.code().unwrap_or(1)));
            }
            Ok(None) => {
                let (code, text) = run_cmd(&[adb.display().to_string(), "devices".into()]);
                if code == 0 {
                    for serial in online(&text) {
                        if booted(adb, &serial) {
                            return (Some(serial), note);
                        }
                    }
                }
                thread::sleep(Duration::from_secs(2));
            }
            Err(_) => break,
        }
    }
    #[cfg(unix)]
    unsafe {
        killpg(child.id() as i32, 15);
    }
    let _ = child.kill();
    (None, format!("{note}emulator did not finish booting\n"))
}

fn ensure_serial(adb: &Path) -> (Option<String>, String) {
    let (code, text) = run_cmd(&[adb.display().to_string(), "devices".into()]);
    let found = if code == 0 { online(&text) } else { Vec::new() };
    if let Some(serial) = found.into_iter().next() {
        if booted(adb, &serial) || wait_boot(adb, &serial, 60) {
            return (Some(serial), String::new());
        }
        return (None, format!("{serial} did not finish booting\n"));
    }
    let Some(emulator) = emulator_path() else {
        return (None, format!("no Android device is attached, and the emulator was not found.\n{text}"));
    };
    let images = avds(&emulator);
    if images.is_empty() {
        return (None, format!("no Android device is attached, and no emulator image is installed.\n{text}"));
    }
    let window = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    let (mut serial, mut note) = start_and_wait(adb, &emulator, &images[0], window, 150);
    if serial.is_none() && window {
        let (again_serial, again_note) = start_and_wait(adb, &emulator, &images[0], false, 150);
        note.push_str("the window did not open, so Droide started it headless.\n");
        note.push_str(&again_note);
        serial = again_serial;
    }
    (serial, note)
}

fn launch_failure(adb: &Path, serial: &str, package: &str) -> Option<String> {
    thread::sleep(Duration::from_secs(1));
    let (_code, fatal) = run_cmd(&[
        adb.display().to_string(),
        "-s".into(),
        serial.into(),
        "logcat".into(),
        "-d".into(),
        "-t".into(),
        "80".into(),
        "AndroidRuntime:E".into(),
        "*:S".into(),
    ]);
    if fatal.contains("FATAL EXCEPTION") && fatal.contains(package) {
        Some(fatal.trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::emulate_aarch64;
    use crate::vm::run_static;
    use std::collections::BTreeMap;

    fn copy_hello(tag: &str) -> PathBuf {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/Hello");
        let dst = std::env::temp_dir().join(format!("droide-rust-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dst);
        fs::create_dir_all(dst.join("forms")).unwrap();
        fs::create_dir_all(dst.join("units")).unwrap();
        fs::create_dir_all(dst.join("native")).unwrap();
        for rel in [
            "project.json",
            "forms/Main.droid",
            "forms/Main.form.json",
            "forms/About.droid",
            "forms/About.form.json",
            "units/Utils.droid",
            "native/fib.nc",
        ] {
            fs::copy(src.join(rel), dst.join(rel)).unwrap_or_else(|err| panic!("copy {rel}: {err}"));
        }
        dst
    }

    fn insn_len(op: u16) -> usize {
        match op {
            0x00 | 0x0E | 0x0F | 0x11 | 0x12 | 0x01 | 0x07 | 0x0A | 0x0C | 0x7B | 0x82 | 0x87 => 1,
            0x13 | 0x1A | 0x1C | 0x22 | 0x29 | 0x32 | 0x33 | 0x34 | 0x35 | 0x36 | 0x37 | 0x38
            | 0x39 | 0x52 | 0x54 | 0x59 | 0x5B | 0x90 | 0x91 | 0x92 | 0x93 | 0x94 | 0xA6 | 0xA7
            | 0xA8 | 0xA9 => 2,
            0x14 | 0x6E | 0x6F | 0x70 | 0x71 | 0x72 => 3,
            other => panic!("unknown opcode {other:#x}"),
        }
    }

    #[test]
    fn hello_executes_and_verifies() {
        let dst = copy_hello("hello");
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let package = meta.get("package").and_then(|v| v.as_str()).unwrap();
        let compiled = compile_dex(package, &checked).unwrap();
        let image = compiled.dex.image.as_ref().expect("image");
        verify_image(image).unwrap();
        let init = image.classes["Lcom/droide/hello/Main;"]
            .methods
            .get(&("<init>".into(), "V".into(), Vec::new()))
            .expect("init");
        let invoke = init.words[0];
        assert_eq!(invoke & 0xFF, 0x70);
        assert_eq!((invoke >> 12) & 0xF, 1);
        let utils = "Lcom/droide/hello/Utils;";
        assert_eq!(run_static(image, utils, "Clamp", &[5, 1, 3]).unwrap(), 3);
        assert_eq!(run_static(image, utils, "Clamp", &[0, 1, 3]).unwrap(), 1);
        assert_eq!(run_static(image, utils, "SumTo", &[10]).unwrap(), 55);
        assert_eq!(run_static(image, utils, "InRange", &[2, 1, 3]).unwrap(), 1);
        assert_eq!(run_static(image, utils, "InRange", &[9, 1, 3]).unwrap(), 0);
        let funs: Vec<_> = checked.natives.values().map(|native| native.fun.clone()).collect();
        let lib = compile_library(package, &funs).unwrap().expect("native lib");
        assert_eq!(emulate_aarch64(&lib.arm, &lib.arm_symbols, "fib", &[10]).unwrap(), 55);
        let report = analyze(&dst);
        assert!(report.ok, "{:?}", report.errors);
        let entities = report.db.get("entities").and_then(|v| v.as_array()).expect("entities");
        let fib = entities.iter().find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("native.fib")).expect("fib");
        assert!(report.db["cfgs"].get(fib["id"].to_string()).is_some() || report.db["cfgs"].get(fib["id"].as_i64().unwrap().to_string()).is_some());
        let click = entities.iter().find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("Main.FibClick")).expect("FibClick");
        assert_eq!(click.get("fanOut").and_then(|v| v.as_i64()), Some(7));
        let result = build(&dst).unwrap();
        assert!(result.ok, "{:?}", result.errors);
        assert!(result.verify.contains("Verified using v2 scheme"), "{}", result.verify);
        let bytes = fs::read(result.apk.expect("apk")).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.windows(16).any(|window| window == b"APK Sig Block 42"));
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn widget_listing_matches_catalog() {
        let dst = std::env::temp_dir().join(format!("droide-rust-kit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dst);
        Project::create(&dst, "Kit", "com.example.kit", None).unwrap();
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        let mut comps = form["components"].as_array().cloned().unwrap_or_default();
        let mut top = 160;
        for kind in [
            "Memo", "Number", "Password", "Radio", "Seek", "Progress", "Rating", "Clock", "Toggle",
            "Email", "Phone", "Picker", "Time", "Date", "Busy",
        ] {
            comps.push(crate::widgets::new_component(kind, &format!("{kind}1"), 16, top));
            top += 8;
        }
        form["components"] = Value::Array(comps);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        let mut source = fs::read_to_string(dst.join("forms/Main.droid")).unwrap();
        source.push_str(
            "\nprocedure ReadKit(Sender: View);\nvar\n  N, M, R, S: Integer;\n  T, F: String;\nbegin\n  T := Number1.Text;\n  T := Memo1.Text;\n  T := Password1.Text;\n  if Radio1.Checked then\n    N := 1;\n  N := Seek1.Progress;\n  N := Progress1.Progress;\n  M := Progress1.Max;\n  R := Rating1.Rating;\n  S := Rating1.Stars;\n  F := Clock1.Format;\n  Seek1.Progress := N;\n  Progress1.Max := M;\n  Rating1.Rating := R;\n  Rating1.Stars := S;\n  Clock1.Format := F;\n  T := Toggle1.TextOn;\n  T := Toggle1.TextOff;\n  if Toggle1.Checked then\n    N := 1;\n  T := Email1.Text;\n  T := Phone1.Text;\n  N := Picker1.MinValue;\n  N := Picker1.MaxValue;\n  N := Picker1.Value;\n  if Picker1.Wrap then\n    N := 1;\n  N := Time1.Hour;\n  N := Time1.Minute;\n  if Time1.Is24Hour then\n    N := 1;\n  N := Date1.Year;\n  N := Date1.Month;\n  N := Date1.Day;\n  Toggle1.TextOn := T;\n  Toggle1.Checked := Toggle1.Checked;\n  Picker1.MaxValue := N;\n  Picker1.MinValue := N;\n  Picker1.Value := N;\n  Picker1.Wrap := Picker1.Wrap;\n  Time1.Is24Hour := Time1.Is24Hour;\n  Time1.Hour := N;\n  Time1.Minute := N;\n  Date1.Year := N;\n  Date1.Month := N;\n  Date1.Day := N;\n  Log(T);\n  Log(IntToStr(N + M + R + S));\n  Log(F)\nend;\n",
        );
        fs::write(dst.join("forms/Main.droid"), source).unwrap();
        let picker = crate::widgets::widget("Picker").unwrap();
        assert_eq!(&picker.props[..4], ["MinValue", "MaxValue", "Value", "Wrap"]);
        assert_eq!(crate::widgets::default_props("Date")["Month"], 10);
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let compiled = compile_dex("com.example.kit", &checked).unwrap();
        let listing = compiled.dex.listing.join("\n");
        for needle in [
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
            "16842872",
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
        ] {
            assert!(listing.contains(needle), "missing {needle}");
        }
        let max_at = listing.find("NumberPicker;->setMaxValue").expect("setMaxValue");
        let min_at = listing.find("NumberPicker;->setMinValue").expect("setMinValue");
        assert!(max_at < min_at);
        let image = compiled.dex.image.as_ref().unwrap();
        verify_image(image).unwrap();
        let method = image.classes["Lcom/example/kit/Main;"]
            .methods
            .get(&("onCreate".into(), "V".into(), vec!["Landroid/os/Bundle;".into()]))
            .expect("onCreate");
        assert!(method.outs >= 4);
        let mut found = false;
        let mut found_busy = false;
        let mut found_date = false;
        let mut pc = 0;
        while pc < method.words.len() {
            let op = method.words[pc] & 0xFF;
            if op == 0x6E || op == 0x70 {
                let method_ref = &image.methods[method.words[pc + 1] as usize];
                let count = (method.words[pc] >> 12) & 0xF;
                if op == 0x70 && method_ref.owner == "Landroid/widget/ProgressBar;" && method_ref.name == "<init>" {
                    if method_ref.params == ["Landroid/content/Context;", "Landroid/util/AttributeSet;", "I"] {
                        assert_eq!(count, 4);
                        found = true;
                    } else if method_ref.params == ["Landroid/content/Context;"] {
                        assert_eq!(count, 2);
                        found_busy = true;
                    }
                }
                if op == 0x6E && method_ref.owner == "Landroid/widget/DatePicker;" && method_ref.name == "updateDate" {
                    assert_eq!(count, 4);
                    assert_eq!(method_ref.params, ["I", "I", "I"]);
                    found_date = true;
                }
            }
            pc += insn_len(op);
        }
        assert!(found && found_busy && found_date);
        let _ = fs::remove_dir_all(&dst);
    }
}
