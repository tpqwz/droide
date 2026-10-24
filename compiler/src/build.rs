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
use crate::ndk::compile_ndk;
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
    pub ndk: bool,
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
                ndk: native.ndk,
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

fn emit_apk(root: &Path, meta: &Value, checked: &crate::check::Checked) -> Result<Compiled, CompileError> {
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
    let funs: Vec<NFun> = checked
        .native_rows()
        .into_iter()
        .filter(|row| !row.ndk)
        .map(|row| row.fun)
        .collect();
    let lib = if funs.is_empty() { None } else { compile_library(package, &funs)? };
    let min_sdk = meta.get("minSdk").and_then(|v| v.as_u64()).unwrap_or(24) as u32;
    let ndk_lib = if checked.ndk.is_empty() {
        None
    } else {
        Some(compile_ndk(root, package, min_sdk, &checked.ndk)?)
    };
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
    if let Some(lib) = ndk_lib {
        entries.push(("lib/arm64-v8a/libdroide_ndk.so".to_string(), lib.arm));
        entries.push(("lib/x86_64/libdroide_ndk.so".to_string(), lib.x86));
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
    let compiled = match emit_apk(&project.root, &meta, &checked) {
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
            0x00 | 0x0E | 0x0F | 0x11 | 0x12 | 0x01 | 0x07 | 0x0A | 0x0B | 0x0C | 0x7B | 0x82 | 0x87 => 1,
            0x13 | 0x16 | 0x1A | 0x1C | 0x1F | 0x22 | 0x23 | 0x29 | 0x32 | 0x33 | 0x34 | 0x35
            | 0x36 | 0x37 | 0x38 | 0x39 | 0x4D | 0x52 | 0x54 | 0x59 | 0x5B | 0x60 | 0x90 | 0x91 | 0x92
            | 0x93 | 0x94 | 0xA6 | 0xA7 | 0xA8 | 0xA9 => 2,
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
        assert_eq!(click.get("fanOut").and_then(|v| v.as_i64()), Some(9));
        let result = build(&dst).unwrap();
        assert!(result.ok, "{:?}", result.errors);
        assert!(result.verify.contains("Verified using v2 scheme"), "{}", result.verify);
        let bytes = fs::read(result.apk.expect("apk")).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.windows(16).any(|window| window == b"APK Sig Block 42"));
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn search_sample_gates_appsearch() {
        assert!(!crate::project::class_ok("DroideSearch"));
        assert!(crate::project::class_ok("Main"));
        let dst = copy_tree("../examples/Search", "search");
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let package = meta.get("package").and_then(|v| v.as_str()).unwrap();
        assert_eq!(package, "com.droide.search");
        let compiled = compile_dex(package, &checked).unwrap();
        let image = compiled.dex.image.as_ref().expect("image");
        verify_image(image).unwrap();
        assert!(image.classes.contains_key("Lcom/droide/search/DroideSearch;"));
        assert!(image.classes.contains_key("Lcom/droide/search/DroideSearch$Cb;"));
        assert!(image.types.iter().any(|ty| ty == "Landroid/app/appsearch/BatchResultCallback;"));
        assert!(image.strings.iter().any(|text| text == "app_search"));
        let form: Value = serde_json::from_str(
            &fs::read_to_string(dst.join("forms/Main.form.json")).unwrap(),
        )
        .unwrap();
        let items = form["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|comp| comp["name"] == "Hits")
            .expect("Hits")["props"]["Items"]
            .as_str()
            .unwrap();
        let lines: Vec<&str> = items.split('\n').collect();
        assert_eq!(lines.len(), 5);
        for line in &lines {
            assert!(image.strings.iter().any(|text| text == line), "{line}");
        }
        let search = image.classes["Lcom/droide/search/DroideRt;"]
            .methods
            .get(&(
                "search".into(),
                "Ljava/lang/String;".into(),
                vec![
                    "Landroid/content/Context;".into(),
                    "Ljava/lang/String;".into(),
                ],
            ))
            .expect("search");
        let body = search.listing.join("\n");
        assert!(body.contains("SDK_INT"), "{body}");
        assert!(body.contains("\"app_search\""), "{body}");
        assert!(body.contains("DroideSearch;->query"), "{body}");
        assert!(!body.contains("Landroid/app/appsearch"), "{body}");
        let result = build(&dst).unwrap();
        assert!(result.ok, "{:?}", result.errors);
        assert!(result.verify.contains("Verified using v2 scheme"), "{}", result.verify);
        let entities = result.db["entities"].as_array().expect("entities");
        let click = entities
            .iter()
            .find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("Main.FindClick"))
            .expect("FindClick");
        let builtin = entities
            .iter()
            .find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("builtin.Search"))
            .expect("builtin.Search");
        assert_eq!(click.get("fanOut").and_then(|v| v.as_i64()), Some(1));
        let refs = result.db["refs"].as_array().expect("refs");
        let click_id = click["id"].as_i64().unwrap();
        let search_id = builtin["id"].as_i64().unwrap();
        assert!(refs.iter().any(|xref| {
            xref["src"].as_i64() == Some(click_id)
                && xref["dst"].as_i64() == Some(search_id)
                && xref["kind"] == "call"
        }));
        let bytes = fs::read(result.apk.expect("apk")).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.windows(16).any(|window| window == b"APK Sig Block 42"));
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn library_sample_uses_appsearch_or_fts4() {
        assert!(!crate::project::class_ok("DroideIndex"));
        assert!(crate::project::class_ok("Main"));
        let dst = copy_tree("../examples/Library", "library");
        let result = build(&dst).unwrap();
        assert!(result.ok, "{:?}", result.errors);
        assert!(result.verify.contains("Verified using v2 scheme"), "{}", result.verify);
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
        assert!(image.classes.contains_key("Lcom/droide/library/DroideIndex;"));
        assert!(image.classes.contains_key("Lcom/droide/library/DroideIndex$Cb;"));
        assert!(image.types.iter().any(|ty| ty == "Landroid/app/appsearch/BatchResultCallback;"));
        assert!(image.strings.iter().any(|text| text == "app_search"));
        assert!(image.strings.iter().any(|text| text.contains("fts4")));
        assert!(image.strings.iter().any(|text| {
            text == "Ink is a deep blue mixed on the swatch and kept in a glass jar."
        }));
        let lookup = image.classes["Lcom/droide/library/DroideRt;"]
            .methods
            .get(&(
                "lookup".into(),
                "Ljava/lang/String;".into(),
                vec![
                    "Landroid/content/Context;".into(),
                    "Ljava/lang/String;".into(),
                ],
            ))
            .expect("lookup");
        let body = lookup.listing.join("\n");
        assert!(body.contains("SDK_INT"), "{body}");
        assert!(body.contains("\"app_search\""), "{body}");
        assert!(body.contains("DroideIndex;->query"), "{body}");
        assert!(!body.contains("Landroid/app/appsearch"), "{body}");
        let rows = image.classes["Lcom/droide/library/DroideRt;"]
            .methods
            .get(&(
                "rows".into(),
                "Ljava/lang/String;".into(),
                vec![
                    "Landroid/database/sqlite/SQLiteDatabase;".into(),
                    "Ljava/lang/String;".into(),
                ],
            ))
            .expect("rows");
        let listed = rows.listing.join("\n");
        assert!(listed.contains("offsets(docs)"), "{listed}");
        assert!(listed.contains("DroideRt;->snip"), "{listed}");
        assert!(!listed.contains("substr(body"), "{listed}");
        let snip = image.classes["Lcom/droide/library/DroideRt;"]
            .methods
            .get(&(
                "snip".into(),
                "Ljava/lang/String;".into(),
                vec![
                    "Ljava/lang/String;".into(),
                    "Ljava/lang/String;".into(),
                    "I".into(),
                ],
            ))
            .expect("snip");
        let clip = snip.listing.join("\n");
        assert!(clip.contains("utfAt"), "{clip}");
        assert!(clip.contains("DroideRt;->span"), "{clip}");
        assert!(listed.contains("DroideRt;->pour"), "{listed}");
        let span = image.classes["Lcom/droide/library/DroideRt;"]
            .methods
            .get(&(
                "span".into(),
                "Ljava/lang/String;".into(),
                vec!["Ljava/lang/String;".into(), "I".into(), "I".into()],
            ))
            .expect("span");
        let window = span.listing.join("\n");
        assert!(window.contains("\"«\""), "{window}");
        let line = image.classes["Lcom/droide/library/DroideIndex;"]
            .methods
            .get(&(
                "line".into(),
                "Ljava/lang/String;".into(),
                vec!["Landroid/app/appsearch/SearchResult;".into()],
            ))
            .expect("line");
        let drawn = line.listing.join("\n");
        assert!(drawn.contains("getMatchInfos"), "{drawn}");
        assert!(drawn.contains("getExactMatchRange"), "{drawn}");
        assert!(drawn.contains("DroideRt;->span"), "{drawn}");
        let spec = image.classes["Lcom/droide/library/DroideIndex;"]
            .methods
            .get(&(
                "spec".into(),
                "Landroid/app/appsearch/SearchSpec;".into(),
                Vec::new(),
            ))
            .expect("spec");
        let shaped = spec.listing.join("\n");
        assert!(shaped.contains("setSnippetCount"), "{shaped}");
        assert!(shaped.contains("setResultCountPerPage"), "{shaped}");
        let accept = image.classes["Lcom/droide/library/DroideIndex$Cb;"]
            .methods
            .get(&(
                "accept".into(),
                "V".into(),
                vec!["Ljava/lang/Object;".into()],
            ))
            .expect("accept");
        let page = accept.listing.join("\n");
        assert!(page.contains("getNextPage"), "{page}");
        assert!(image.strings.iter().any(|text| text.contains("offsets(docs)")));
        assert!(!image.strings.iter().any(|text| text.contains("substr(body")));
        let entities = result.db["entities"].as_array().expect("entities");
        let click = entities
            .iter()
            .find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("Main.FindClick"))
            .expect("FindClick");
        let save = entities
            .iter()
            .find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("Add.SaveClick"))
            .expect("SaveClick");
        let lookup_ent = entities
            .iter()
            .find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("builtin.Lookup"))
            .expect("Lookup");
        let ingest_ent = entities
            .iter()
            .find(|ent| ent.get("qname").and_then(|v| v.as_str()) == Some("builtin.Ingest"))
            .expect("Ingest");
        assert_eq!(click.get("fanOut").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(save.get("fanOut").and_then(|v| v.as_i64()), Some(1));
        let refs = result.db["refs"].as_array().expect("refs");
        let click_id = click["id"].as_i64().unwrap();
        let save_id = save["id"].as_i64().unwrap();
        assert!(refs.iter().any(|xref| {
            xref["src"].as_i64() == Some(click_id)
                && xref["dst"].as_i64() == Some(lookup_ent["id"].as_i64().unwrap())
                && xref["kind"] == "call"
        }));
        assert!(refs.iter().any(|xref| {
            xref["src"].as_i64() == Some(save_id)
                && xref["dst"].as_i64() == Some(ingest_ent["id"].as_i64().unwrap())
                && xref["kind"] == "call"
        }));
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

    fn events_form(tag: &str, name: &str, package: &str) -> PathBuf {
        let dst = std::env::temp_dir().join(format!("droide-rust-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dst);
        Project::create(&dst, name, package, None).unwrap();
        dst
    }

    #[test]
    fn change_and_long_press_compile() {
        let dst = events_form("events", "Events", "com.example.events");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        let mut comps = Vec::new();
        let mut top = 16;
        for (kind, name, events) in [
            ("Button", "Press", json!({"OnClick": "PressClick", "OnLongClick": "PressLong"})),
            ("Edit", "Edit1", json!({"OnChange": "EditChange"})),
            ("Check", "Check1", json!({"OnChange": "CheckChange"})),
            ("Seek", "Seek1", json!({"OnChange": "SeekChange"})),
            ("Rating", "Rating1", json!({"OnChange": "RatingChange"})),
            ("Picker", "Picker1", json!({"OnChange": "PickerChange"})),
            ("Time", "Time1", json!({"OnChange": "TimeChange"})),
            ("Date", "Date1", json!({"OnChange": "DateChange"})),
            ("List", "Rows", json!({"OnItemClick": "RowClick", "OnItemLongClick": "RowLong"})),
        ] {
            let mut comp = crate::widgets::new_component(kind, name, 16, top);
            comp["events"] = events;
            comps.push(comp);
            top += 8;
        }
        form["components"] = Value::Array(comps);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "\
procedure PressClick(Sender: View);
begin
end;

procedure PressLong(Sender: View);
begin
end;

procedure EditChange(Text: String);
begin
end;

procedure CheckChange(On: Boolean);
begin
end;

procedure SeekChange(Value: Integer);
begin
end;

procedure RatingChange(Value: Integer);
begin
end;

procedure PickerChange(Value: Integer);
begin
end;

procedure TimeChange();
begin
end;

procedure DateChange();
begin
end;

procedure RowClick(Row: String);
begin
end;

procedure RowLong(Row: String);
begin
end;
",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let compiled = compile_dex("com.example.events", &checked).unwrap();
        let listing = compiled.dex.listing.join("\n");
        for needle in [
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
        ] {
            assert!(listing.contains(needle), "missing {needle}");
        }
        assert!(!listing.contains("updateDate"), "{listing}");
        let image = compiled.dex.image.as_ref().unwrap();
        verify_image(image).unwrap();
        let method = image.classes["Lcom/example/events/Main;"]
            .methods
            .get(&("onCreate".into(), "V".into(), vec!["Landroid/os/Bundle;".into()]))
            .expect("onCreate");
        assert!(method.outs >= 5);
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn change_handler_rejects_a_bad_parameter() {
        let dst = events_form("badevent", "Bad", "com.example.bad");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        let mut edit = crate::widgets::new_component("Edit", "Edit1", 16, 16);
        edit["events"] = json!({"OnChange": "EditChange"});
        let mut time = crate::widgets::new_component("Time", "Time1", 16, 80);
        time["events"] = json!({"OnChange": "TimeChange"});
        form["components"] = json!([edit, time]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "procedure EditChange(Value: Integer);\nbegin\nend;\n\nprocedure TimeChange(Value: Integer);\nbegin\nend;\n",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (_meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|err| err.get("message").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert!(
            messages.iter().any(|msg| msg.contains("EditChange") && msg.contains("String")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|msg| msg.contains("TimeChange") && msg.contains("takes no parameters")),
            "{messages:?}"
        );
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn spinner_compiles() {
        let dst = events_form("spin", "Spin", "com.example.spin");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        let mut rows = crate::widgets::new_component("List", "Rows", 16, 16);
        rows["props"]["Items"] = json!("One\nTwo");
        let mut kind = crate::widgets::new_component("Spinner", "Kind1", 16, 80);
        kind["props"]["Items"] = json!("Red\nGreen\nBlue");
        kind["props"]["Selection"] = json!(2);
        kind["events"] = json!({"OnItemSelected": "Pick"});
        form["components"] = json!([rows, kind]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "\
procedure Pick(Row: String);
var
  N: Integer;
begin
  N := Kind1.Selection;
  Kind1.Items := Row;
  Log(IntToStr(N))
end;
",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let compiled = compile_dex("com.example.spin", &checked).unwrap();
        let listing = compiled.dex.listing.join("\n");
        for needle in [
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
        ] {
            assert!(listing.contains(needle), "missing {needle}");
        }
        let main = class_body(&listing, "Lcom/example/spin/Main;");
        let spin_at = main.find("->setSpinner").expect("setSpinner");
        let lines_at = main.find("->setLines").expect("setLines");
        let sel_at = main.find("AdapterView;->setSelection").expect("setSelection");
        let listen_at = main.find("setOnItemSelectedListener").expect("listener");
        assert!(spin_at < sel_at && sel_at < listen_at, "{main}");
        assert!(main.contains("const/4 v4, #2"), "{main}");
        assert!(lines_at != spin_at);
        let image = compiled.dex.image.as_ref().unwrap();
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/widget/AbsSpinner;"
                && method.name == "setAdapter"
                && method.params == ["Landroid/widget/SpinnerAdapter;"]
        }));
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/widget/AbsListView;"
                && method.name == "setAdapter"
                && method.params == ["Landroid/widget/ListAdapter;"]
        }));
        verify_image(image).unwrap();
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn spinner_handler_rejects_a_bad_parameter() {
        let dst = events_form("badspin", "BadSpin", "com.example.badspin");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        let mut kind = crate::widgets::new_component("Spinner", "Kind1", 16, 16);
        kind["events"] = json!({"OnItemSelected": "Pick"});
        form["components"] = json!([kind]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "procedure Pick(On: Boolean);\nbegin\nend;\n",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (_meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|err| err.get("message").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert!(
            messages.iter().any(|msg| {
                msg.contains("Pick")
                    && msg.contains("takes no parameters, a String, an Integer, or the row View")
            }),
            "{messages:?}"
        );
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn message_and_question_compile() {
        let dst = events_form("ask", "Ask", "com.example.ask");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        form["components"] = json!([]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "\
procedure Show(Sender: View);
begin
  Message('Saved')
  if IsInt('1') then
    Question('Delete this?', 'yesclick')
  else begin
    Question('Keep it?', 'YesClick')
  end
end;

procedure YesClick();
begin
  Log('yes')
end;
",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let compiled = compile_dex("com.example.ask", &checked).unwrap();
        let listing = compiled.dex.listing.join("\n");
        for needle in [
            "Landroid/app/AlertDialog$Builder;",
            "setMessage",
            "setPositiveButton",
            "setNegativeButton",
            "AlertDialog$Builder;->show",
            "const-string v",
            "\"OK\"",
            "\"Yes\"",
            "\"No\"",
            "class Lcom/example/ask/Main$Ask$YesClick;",
            "onClick",
            "Main;->YesClick",
        ] {
            assert!(listing.contains(needle), "missing {needle}");
        }
        assert_eq!(listing.matches("class Lcom/example/ask/Main$Ask$YesClick;").count(), 1);
        let image = compiled.dex.image.as_ref().unwrap();
        for name in ["setPositiveButton", "setNegativeButton"] {
            assert!(image.methods.iter().any(|method| {
                method.owner == "Landroid/app/AlertDialog$Builder;"
                    && method.name == name
                    && method.params
                        == [
                            "Ljava/lang/CharSequence;",
                            "Landroid/content/DialogInterface$OnClickListener;",
                        ]
            }));
        }
        verify_image(image).unwrap();
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn question_rejects_a_bad_procedure() {
        let dst = events_form("badask", "BadAsk", "com.example.badask");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        form["components"] = json!([]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "\
procedure Show();
var
  Name: String;
begin
  Question('Hi?', Name)
  Question('Hi?', 'Missing')
  Question('Hi?', 'YesClick')
  Message(1)
end;

procedure YesClick(Sender: View);
begin
end;
",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (_meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|err| err.get("message").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        for needle in [
            "needs a string literal procedure name",
            "unknown procedure Missing",
            "YesClick takes no parameters",
            "Message expects String",
        ] {
            assert!(messages.iter().any(|msg| msg.contains(needle)), "{needle}: {messages:?}");
        }
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn share_and_clipboard_compile() {
        let dst = events_form("clip", "Clip", "com.example.clip");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        form["components"] = json!([]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "\
procedure Show(Sender: View);
var
  Text: String;
begin
  CopyText('Hello')
  Text := PasteText()
  Share(Text)
end;
",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let compiled = compile_dex("com.example.clip", &checked).unwrap();
        let listing = compiled.dex.listing.join("\n");
        for needle in [
            "android.intent.action.SEND",
            "android.intent.extra.TEXT",
            "text/plain",
            "createChooser",
            "queryIntentActivities",
            "\"clipboard\"",
            "Landroid/content/ClipboardManager;",
            "newPlainText",
            "setPrimaryClip",
            "getPrimaryClip",
            "hasPrimaryClip",
            "getText",
            "->share",
            "->copyText",
            "->pasteText",
        ] {
            assert!(listing.contains(needle), "missing {needle}");
        }
        let send_at = listing.find("android.intent.action.SEND").expect("SEND");
        let after = &listing[send_at..];
        let query_at = after.find("queryIntentActivities").expect("query");
        let chooser_at = after.find("createChooser").expect("chooser");
        let start_at = after.find("startActivity").expect("start");
        assert!(query_at < chooser_at && chooser_at < start_at, "{after}");
        let plain_at = listing.find("newPlainText").expect("plain");
        let set_at = listing[plain_at..].find("setPrimaryClip").expect("set");
        assert!(set_at > 0);
        let clip_at = listing.find("getPrimaryClip").expect("clip");
        let text_at = listing[clip_at..].find("getText").expect("text");
        assert!(text_at > 0);
        let image = compiled.dex.image.as_ref().unwrap();
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/content/Intent;"
                && method.name == "createChooser"
                && method.params
                    == [
                        "Landroid/content/Intent;",
                        "Ljava/lang/CharSequence;",
                    ]
        }));
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/content/Intent;"
                && method.name == "putExtra"
                && method.params == ["Ljava/lang/String;", "Ljava/lang/String;"]
        }));
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/content/ClipboardManager;"
                && method.name == "setPrimaryClip"
                && method.params == ["Landroid/content/ClipData;"]
        }));
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/content/ClipData$Item;"
                && method.name == "getText"
                && method.ret == "Ljava/lang/CharSequence;"
        }));
        verify_image(image).unwrap();
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn share_rejects_a_bad_argument() {
        let dst = events_form("badclip", "BadClip", "com.example.badclip");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        form["components"] = json!([]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "\
procedure Show();
begin
  Share(1)
  CopyText(1)
  PasteText('x')
end;
",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (_meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|err| err.get("message").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        for needle in [
            "Share expects String",
            "CopyText expects String",
            "PasteText expects 0 arguments",
        ] {
            assert!(messages.iter().any(|msg| msg.contains(needle)), "{needle}: {messages:?}");
        }
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn panel_holds_children() {
        let dst = events_form("panel", "Panel", "com.example.panel");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        form["components"] = json!([
            {
                "type": "Button",
                "name": "Early",
                "parent": "Box",
                "left": 4,
                "top": 2,
                "width": 80,
                "height": 40,
                "props": {"Text": "Early"},
                "events": {}
            },
            {
                "type": "Panel",
                "name": "Box",
                "left": 16,
                "top": 24,
                "width": 200,
                "height": 160,
                "props": {"Color": "#E4E8EE"},
                "events": {}
            },
            {
                "type": "Panel",
                "name": "Pad",
                "left": 16,
                "top": 200,
                "width": 80,
                "height": 40,
                "props": {"Color": "#E4E8EE"},
                "events": {}
            },
            {
                "type": "Panel",
                "name": "Row",
                "left": 16,
                "top": 260,
                "width": 240,
                "height": 64,
                "props": {"Direction": "Horizontal", "Color": "#E4E8EE"},
                "events": {}
            },
            {
                "type": "Button",
                "name": "Side",
                "parent": "Row",
                "left": 900,
                "top": 0,
                "width": 40,
                "height": 36,
                "props": {"Text": "Side"},
                "events": {}
            },
            {
                "type": "Panel",
                "name": "Inner",
                "parent": "Box",
                "left": 8,
                "top": 4,
                "width": 120,
                "height": 80,
                "props": {"Color": "#D8DEE8"},
                "events": {}
            },
            {
                "type": "Button",
                "name": "Go",
                "parent": "Inner",
                "left": 2,
                "top": 2,
                "width": 72,
                "height": 36,
                "props": {"Text": "Go"},
                "events": {}
            }
        ]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(
            dst.join("forms/Main.droid"),
            "\
procedure Turn();
begin
  Box.Direction := 'Horizontal'
end;
",
        )
        .unwrap();
        let project = Project::open(&dst).unwrap();
        let (meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        errors.extend(structural(&meta, &checked));
        assert!(errors.is_empty(), "{errors:?}");
        let compiled = compile_dex("com.example.panel", &checked).unwrap();
        let listing = compiled.dex.listing.join("\n");
        let create = method_body(&listing, "onCreate(");
        let frame = create.split("FrameLayout$LayoutParams").next().unwrap();
        assert!(frame.contains("#360"), "{frame}");
        assert!(frame.contains("#640"), "{frame}");
        assert!(!frame.contains("#900"), "{frame}");
        assert!(create.contains("#900"));
        assert!(create.contains("Landroid/widget/LinearLayout;"));
        assert!(create.contains("Landroid/widget/LinearLayout$LayoutParams;"));
        assert_eq!(create.matches("setOrientation").count(), 4);
        let early_put = create.find("iput-object v2, v6, Early").expect("early field");
        let box_put = create.find("iput-object v2, v6, Box").expect("box field");
        assert!(early_put < box_put, "a child listed first is still created before its panel");
        let early_get = create.find("iget-object v2, v6, Early").expect("load child");
        assert!(box_put < early_get, "the panel exists before the child is attached");
        let between = &create[box_put..early_get];
        assert!(
            between.contains("iget-object v3, v6, Box"),
            "{between}"
        );
        let mut saw_vertical = false;
        let mut saw_horizontal = false;
        let lines: Vec<&str> = create.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.contains("setOrientation") {
                continue;
            }
            let prev = lines[..i].iter().rev().find(|item| item.contains("const/")).unwrap();
            if prev.contains("#0") {
                saw_horizontal = true;
            }
            if prev.contains("#1") {
                saw_vertical = true;
            }
        }
        assert!(saw_vertical && saw_horizontal, "{create}");
        let turn = method_body(&listing, "Turn(");
        assert!(turn.contains("equalsIgnoreCase"));
        assert!(turn.contains("setOrientation"));
        let image = compiled.dex.image.as_ref().unwrap();
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/widget/LinearLayout;"
                && method.name == "setOrientation"
                && method.ret == "V"
                && method.params == ["I"]
        }));
        assert!(image.methods.iter().any(|method| {
            method.owner == "Landroid/widget/LinearLayout$LayoutParams;"
                && method.name == "<init>"
                && method.ret == "V"
                && method.params == ["I", "I"]
        }));
        verify_image(image).unwrap();
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn panel_rejects_a_bad_parent() {
        let dst = events_form("badpanel", "BadPanel", "com.example.badpanel");
        let form_path = dst.join("forms/Main.form.json");
        let mut form: Value = serde_json::from_str(&fs::read_to_string(&form_path).unwrap()).unwrap();
        form["components"] = json!([
            {
                "type": "Button", "name": "Early", "parent": "Missing",
                "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {}
            },
            {
                "type": "Label", "name": "Title",
                "left": 0, "top": 0, "width": 40, "height": 40,
                "props": {"Text": "T"}, "events": {}
            },
            {
                "type": "Button", "name": "Late", "parent": "Title",
                "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {}
            },
            {
                "type": "Panel", "name": "Box", "parent": "Box",
                "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {}
            },
            {
                "type": "Panel", "name": "A", "parent": "B",
                "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {}
            },
            {
                "type": "Panel", "name": "B", "parent": "A",
                "left": 0, "top": 0, "width": 40, "height": 40, "props": {}, "events": {}
            },
            {
                "type": "Panel", "name": "Wide",
                "left": 0, "top": 0, "width": 40, "height": 40,
                "props": {"Direction": "Sideways"}, "events": {}
            }
        ]);
        fs::write(&form_path, serde_json::to_string(&form).unwrap()).unwrap();
        fs::write(dst.join("forms/Main.droid"), "procedure Show();\nbegin\nend;\n").unwrap();
        let project = Project::open(&dst).unwrap();
        let (_meta, mut model, mut errors) = project.load(&BTreeMap::new());
        let checked = check_model(&mut model);
        errors.extend(checked.errors.iter().cloned());
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|err| err.get("message").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        for needle in [
            "Early parent Missing is unknown",
            "Late parent Title is not a Panel",
            "Box parents itself",
            "parent cycle",
            "Wide Direction must be Vertical or Horizontal",
        ] {
            assert!(messages.iter().any(|msg| msg.contains(needle)), "{needle}: {messages:?}");
        }
        let _ = fs::remove_dir_all(&dst);
    }

    fn method_body(listing: &str, name: &str) -> String {
        let header = format!("\n  {name}");
        let start = listing.find(&header).unwrap_or_else(|| panic!("missing {name}\n{listing}"));
        let rest = &listing[start + 1..];
        let mut out = String::new();
        for line in rest.lines().skip(1) {
            if line.starts_with("class ") || (line.starts_with("  ") && !line.starts_with("   ")) {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn class_body(listing: &str, desc: &str) -> String {
        let header = format!("class {desc}");
        let start = listing.find(&header).unwrap_or(0) + header.len();
        let rest = &listing[start..];
        let end = rest.find("\nclass ").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    fn copy_tree(rel: &str, tag: &str) -> PathBuf {
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
        let dst = std::env::temp_dir().join(format!("droide-rust-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dst);
        copy_dir(&src, &dst);
        dst
    }

    fn copy_dir(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for ent in fs::read_dir(src).unwrap() {
            let ent = ent.unwrap();
            let name = ent.file_name();
            if name == "build" || name == "target" {
                continue;
            }
            let to = dst.join(&name);
            if ent.path().is_dir() {
                copy_dir(&ent.path(), &to);
            } else {
                fs::copy(ent.path(), to).unwrap();
            }
        }
    }

    fn apk_entry(apk: &[u8], want: &str) -> Vec<u8> {
        let mut off = 0usize;
        while off + 30 <= apk.len() && &apk[off..off + 4] == b"PK\x03\x04" {
            let method = u16::from_le_bytes(apk[off + 8..off + 10].try_into().unwrap());
            let comp = u32::from_le_bytes(apk[off + 18..off + 22].try_into().unwrap()) as usize;
            let name_len = u16::from_le_bytes(apk[off + 26..off + 28].try_into().unwrap()) as usize;
            let extra_len = u16::from_le_bytes(apk[off + 28..off + 30].try_into().unwrap()) as usize;
            let name_at = off + 30;
            let name = &apk[name_at..name_at + name_len];
            let data_at = name_at + name_len + extra_len;
            let data = &apk[data_at..data_at + comp];
            if name == want.as_bytes() {
                if method == 0 {
                    return data.to_vec();
                }
                let mut decoder = flate2::read::DeflateDecoder::new(data);
                let mut out = Vec::new();
                std::io::Read::read_to_end(&mut decoder, &mut out).unwrap();
                return out;
            }
            off = data_at + comp;
        }
        panic!("missing {want}");
    }

    #[test]
    fn ndk_library_links_bionic_beside_nc() {
        let dst = copy_tree("../examples/Ndk", "ndk");
        let result = build(&dst).unwrap();
        assert!(result.ok, "{:?}", result.errors);
        assert!(result.listing.contains("const-string v0, \"droide\""));
        assert!(result.listing.contains("const-string v0, \"droide_ndk\""));
        assert!(result.listing.contains("mix('I',)I"));
        assert!(result.listing.contains("span('I',)I"));
        assert!(result.listing.contains("echo('I',)I"));
        assert!(result.verify.contains("Verified using v2 scheme"), "{}", result.verify);
        let entities = result.db["entities"].as_array().expect("entities");
        let mix = entities
            .iter()
            .find(|ent| ent["qname"] == "native.mix")
            .expect("mix");
        assert_eq!(mix["file"], "native/mix.c");
        assert_eq!(mix["kind"], "native");
        let click = entities
            .iter()
            .find(|ent| ent["qname"] == "Main.MixClick")
            .expect("MixClick");
        assert_eq!(click["fanOut"].as_i64(), Some(4));
        let apk = fs::read(result.apk.expect("apk")).unwrap();
        let nc = apk_entry(&apk, "lib/arm64-v8a/libdroide.so");
        let arm = apk_entry(&apk, "lib/arm64-v8a/libdroide_ndk.so");
        let x86 = apk_entry(&apk, "lib/x86_64/libdroide_ndk.so");
        assert!(nc.starts_with(b"\x7fELF"));
        assert!(arm.starts_with(b"\x7fELF"));
        assert!(x86.starts_with(b"\x7fELF"));
        assert!(!nc.windows(8).any(|window| window == b"libc.so\0"));
        for so in [&arm, &x86] {
            assert!(so.windows(8).any(|window| window == b"libc.so\0"));
            assert!(so.windows(b"Java_com_droide_ndk_Native_mix".len()).any(|window| window == b"Java_com_droide_ndk_Native_mix"));
            assert!(so.windows(b"Java_com_droide_ndk_Native_span".len()).any(|window| window == b"Java_com_droide_ndk_Native_span"));
            assert!(!so.windows(b"Java_com_droide_ndk_Native_echo".len()).any(|window| window == b"Java_com_droide_ndk_Native_echo"));
        }
        assert!(nc.windows(b"Java_com_droide_ndk_Native_echo".len()).any(|window| window == b"Java_com_droide_ndk_Native_echo"));
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn ndk_duplicate_and_missing_export_are_errors() {
        let dst = std::env::temp_dir().join(format!("droide-rust-ndk-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dst);
        Project::create(&dst, "Dup", "com.example.dup", None).unwrap();
        fs::write(
            dst.join("native/mix.c"),
            "DROIDE_EXPORT int32_t echo(int32_t n) { return n; }\n",
        )
        .unwrap();
        let result = build(&dst).unwrap();
        assert!(!result.ok);
        assert!(
            result.errors.iter().any(|err| err["message"].as_str().unwrap_or("").contains("duplicate native function")),
            "{:?}",
            result.errors
        );

        let bare = std::env::temp_dir().join(format!("droide-rust-ndk-bare-{}", std::process::id()));
        let _ = fs::remove_dir_all(&bare);
        Project::create(&bare, "Bare", "com.example.bare", None).unwrap();
        fs::write(bare.join("native/helper.c"), "int helper(int n) { return n; }\n").unwrap();
        fs::remove_file(bare.join("native/app.nc")).unwrap();
        let result = build(&bare).unwrap();
        assert!(!result.ok);
        assert!(
            result.errors.iter().any(|err| err["message"].as_str().unwrap_or("").contains("DROIDE_EXPORT")),
            "{:?}",
            result.errors
        );
        let _ = fs::remove_dir_all(&dst);
        let _ = fs::remove_dir_all(&bare);
    }
}
