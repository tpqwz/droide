//! NDK C and C++ beside the `.nc` sandbox.
//!
//! `DROIDE_EXPORT int32_t name(int32_t ...)` is the Droide call. The body is
//! ordinary C and may use the C library. NDK clang links it to bionic and
//! packs `libdroide_ndk.so`. `libdroide.so` stays the freestanding `.nc` image.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::CompileError;
use crate::native::jni_symbol;

#[derive(Clone, Debug)]
pub struct NdkExport {
    pub name: String,
    pub params: Vec<String>,
    pub line: i32,
    pub col: i32,
    pub end_line: i32,
}

#[derive(Clone, Debug)]
pub struct NdkFile {
    pub path: String,
    pub exports: Vec<NdkExport>,
    pub failed: bool,
}

pub struct NdkScan {
    pub exports: Vec<NdkExport>,
    pub error: Option<CompileError>,
}

pub struct NdkLib {
    pub arm: Vec<u8>,
    pub x86: Vec<u8>,
}

const HEADER: &str = "\
#ifndef DROIDE_H
#define DROIDE_H
#include <stdint.h>
#ifdef __cplusplus
#define DROIDE_EXPORT extern \"C\"
#else
#define DROIDE_EXPORT
#endif
#endif
";

#[derive(Clone, Debug)]
struct Tok {
    text: String,
    line: i32,
    col: i32,
    kind: TokKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokKind {
    Ident,
    Punct,
    Number,
}

pub fn scan_exports(source: &str, file: &str) -> NdkScan {
    let tokens = tokenize(source);
    let mut exports = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        if tokens[i].kind == TokKind::Ident && tokens[i].text == "DROIDE_EXPORT" {
            match parse_export(&tokens, i, file) {
                Ok((export, next)) => {
                    exports.push(export);
                    i = next;
                }
                Err(error) => {
                    return NdkScan {
                        exports,
                        error: Some(error),
                    };
                }
            }
        } else {
            i += 1;
        }
    }
    NdkScan {
        exports,
        error: None,
    }
}

fn parse_export(tokens: &[Tok], start: usize, file: &str) -> Result<(NdkExport, usize), CompileError> {
    let marker = &tokens[start];
    let mut i = start + 1;
    let Some(ty) = tokens.get(i) else {
        return Err(err_at(file, Some(marker), "DROIDE_EXPORT is followed by int32_t"));
    };
    if ty.kind != TokKind::Ident || ty.text != "int32_t" {
        return Err(err_at(file, Some(ty), "DROIDE_EXPORT is followed by int32_t"));
    }
    i += 1;
    if tokens.get(i).is_some_and(|tok| tok.text == "*") {
        return Err(err_at(file, tokens.get(i), "a Droide export returns int32_t"));
    }
    let Some(name_tok) = tokens.get(i) else {
        return Err(err_at(file, Some(ty), "a Droide export needs a name"));
    };
    if name_tok.kind != TokKind::Ident {
        return Err(err_at(file, Some(name_tok), "a Droide export needs a name"));
    }
    let name = name_tok.text.clone();
    let line = name_tok.line;
    let col = name_tok.col;
    i += 1;
    let Some(paren) = tokens.get(i) else {
        return Err(err_at(file, Some(name_tok), "a Droide export needs a parameter list"));
    };
    if paren.text != "(" {
        return Err(err_at(file, Some(paren), "a Droide export needs a parameter list"));
    }
    i += 1;
    let mut params = Vec::new();
    if tokens.get(i).is_some_and(|tok| tok.text == ")") {
        i += 1;
    } else if tokens.get(i).is_some_and(|tok| tok.kind == TokKind::Ident && tok.text == "void")
        && tokens.get(i + 1).is_some_and(|tok| tok.text == ")")
    {
        i += 2;
    } else {
        loop {
            let Some(pty) = tokens.get(i) else {
                return Err(err_at(file, tokens.get(i.saturating_sub(1)), "a Droide export parameter is int32_t"));
            };
            if pty.kind != TokKind::Ident || pty.text != "int32_t" {
                return Err(err_at(file, Some(pty), "a Droide export parameter is int32_t"));
            }
            i += 1;
            if tokens.get(i).is_some_and(|tok| tok.text == "*") {
                return Err(err_at(file, tokens.get(i), "a Droide export parameter is int32_t"));
            }
            let pname = if tokens.get(i).is_some_and(|tok| tok.kind == TokKind::Ident) {
                let text = tokens[i].text.clone();
                i += 1;
                text
            } else {
                format!("arg{}", params.len())
            };
            params.push(pname);
            if params.len() > 4 {
                return Err(err_at(
                    file,
                    Some(name_tok),
                    "a Droide export takes at most 4 parameters",
                ));
            }
            let Some(sep) = tokens.get(i) else {
                return Err(err_at(file, Some(name_tok), "a Droide export needs a closing ')'"));
            };
            if sep.text == "," {
                i += 1;
                continue;
            }
            if sep.text == ")" {
                i += 1;
                break;
            }
            return Err(err_at(file, Some(sep), "a Droide export parameter is int32_t"));
        }
    }
    let Some(end) = tokens.get(i) else {
        return Err(err_at(file, Some(name_tok), "a Droide export ends with ';' or '{'"));
    };
    if end.text != ";" && end.text != "{" {
        return Err(err_at(file, Some(end), "a Droide export ends with ';' or '{'"));
    }
    let end_line = end.line;
    i += 1;
    Ok((
        NdkExport {
            name,
            params,
            line,
            col,
            end_line,
        },
        i,
    ))
}

fn err_at(file: &str, tok: Option<&Tok>, message: &str) -> CompileError {
    let (line, col) = tok.map(|tok| (tok.line, tok.col)).unwrap_or((1, 1));
    CompileError::new(message, line, col, file)
}

fn tokenize(source: &str) -> Vec<Tok> {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0usize;
    let mut line = 1i32;
    let mut col = 1i32;
    let mut line_start = true;
    let mut tokens = Vec::new();
    if chars.first() == Some(&'\u{feff}') {
        i = 1;
    }
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            col = 1;
            i += 1;
            line_start = true;
            continue;
        }
        if c == ' ' || c == '\t' || c == '\r' {
            col += 1;
            i += 1;
            continue;
        }
        if line_start && c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\n' {
                    i += 2;
                    line += 1;
                    col = 1;
                    continue;
                }
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            line_start = false;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            col += 2;
            line_start = false;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                if chars[i] == '\n' {
                    line += 1;
                    col = 1;
                    i += 1;
                    line_start = true;
                } else {
                    if !chars[i].is_whitespace() {
                        line_start = false;
                    }
                    col += 1;
                    i += 1;
                }
            }
            if i + 1 < chars.len() {
                i += 2;
                col += 2;
                line_start = false;
            } else {
                i = chars.len();
            }
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            col += 1;
            line_start = false;
            while i < chars.len() && chars[i] != '\n' {
                if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] != '\n' {
                    i += 2;
                    col += 2;
                    continue;
                }
                if chars[i] == quote {
                    i += 1;
                    col += 1;
                    break;
                }
                col += 1;
                i += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start_line = line;
            let start_col = col;
            let mut text = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                text.push(chars[i]);
                i += 1;
                col += 1;
            }
            line_start = false;
            tokens.push(Tok {
                text,
                line: start_line,
                col: start_col,
                kind: TokKind::Ident,
            });
            continue;
        }
        if c.is_ascii_digit() {
            let start_line = line;
            let start_col = col;
            let mut text = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                text.push(chars[i]);
                i += 1;
                col += 1;
            }
            line_start = false;
            tokens.push(Tok {
                text,
                line: start_line,
                col: start_col,
                kind: TokKind::Number,
            });
            continue;
        }
        tokens.push(Tok {
            text: c.to_string(),
            line,
            col,
            kind: TokKind::Punct,
        });
        line_start = false;
        i += 1;
        col += 1;
    }
    tokens
}

pub fn compile_ndk(root: &Path, package: &str, min_sdk: u32, files: &[NdkFile]) -> Result<NdkLib, CompileError> {
    if files.is_empty() {
        return Err(CompileError::bare("no native C files"));
    }
    // Clang runs with the project as its cwd. A relative root would make
    // `root.join("build/...")` point outside that cwd.
    let root = fs::canonicalize(root).map_err(|err| CompileError::bare(err.to_string()))?;
    let root = root.as_path();
    let exports = unique_exports(files)?;
    if exports.is_empty() {
        return Err(CompileError::bare("native C files have no DROIDE_EXPORT function"));
    }
    for export in &exports {
        c_name(&export.name)?;
    }
    let tools = find_tools(min_sdk)?;
    let ndk_dir = root.join("build/ndk");
    fs::create_dir_all(&ndk_dir).map_err(|err| CompileError::bare(err.to_string()))?;
    let header = ndk_dir.join("droide.h");
    fs::write(&header, HEADER).map_err(|err| CompileError::bare(err.to_string()))?;
    let glue = ndk_dir.join("jni_glue.c");
    fs::write(&glue, glue_source(package, &exports)).map_err(|err| CompileError::bare(err.to_string()))?;
    let cpp = files.iter().any(|file| {
        matches!(
            Path::new(&file.path).extension().and_then(|ext| ext.to_str()),
            Some("cc") | Some("cpp")
        )
    });
    let native_dir = root.join("native");
    let arm = link_one(
        root,
        &ndk_dir.join("arm64-v8a"),
        &tools.arm_cc,
        &tools.arm_cxx,
        cpp,
        &header,
        &native_dir,
        &glue,
        files,
    )?;
    let x86 = link_one(
        root,
        &ndk_dir.join("x86_64"),
        &tools.x86_cc,
        &tools.x86_cxx,
        cpp,
        &header,
        &native_dir,
        &glue,
        files,
    )?;
    Ok(NdkLib { arm, x86 })
}

fn unique_exports(files: &[NdkFile]) -> Result<Vec<&NdkExport>, CompileError> {
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut unique = Vec::new();
    for export in files.iter().flat_map(|file| file.exports.iter()) {
        if let Some(arity) = seen.get(&export.name) {
            if *arity != export.params.len() {
                return Err(CompileError::new(
                    format!("duplicate native function {}", export.name),
                    export.line,
                    export.col,
                    "",
                ));
            }
            continue;
        }
        seen.insert(export.name.clone(), export.params.len());
        unique.push(export);
    }
    Ok(unique)
}

fn c_name(name: &str) -> Result<(), CompileError> {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => {
            return Err(CompileError::bare(format!(
                "native function {name} is not a C identifier"
            )))
        }
    }
    if chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        Ok(())
    } else {
        Err(CompileError::bare(format!(
            "native function {name} is not a C identifier"
        )))
    }
}

fn glue_source(package: &str, exports: &[&NdkExport]) -> String {
    let mut out = String::from("#include <jni.h>\n#include <stdint.h>\n\n");
    for export in exports {
        let args = c_params(export.params.len());
        out.push_str(&format!("extern int32_t {}({});\n", export.name, args));
    }
    out.push('\n');
    for export in exports {
        let symbol = jni_symbol(package, &export.name);
        let mut formals = vec!["JNIEnv *env".to_string(), "jclass clazz".to_string()];
        let mut call = Vec::new();
        for index in 0..export.params.len() {
            formals.push(format!("jint a{index}"));
            call.push(format!("(int32_t)a{index}"));
        }
        out.push_str(&format!(
            "__attribute__((visibility(\"default\")))\njint JNICALL {symbol}({}) {{\n  (void)env;\n  (void)clazz;\n  return (jint){}({});\n}}\n\n",
            formals.join(", "),
            export.name,
            call.join(", ")
        ));
    }
    out
}

fn c_params(count: usize) -> String {
    if count == 0 {
        return "void".into();
    }
    (0..count)
        .map(|index| format!("int32_t a{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

struct Tools {
    arm_cc: PathBuf,
    arm_cxx: PathBuf,
    x86_cc: PathBuf,
    x86_cxx: PathBuf,
}

fn link_one(
    root: &Path,
    out_dir: &Path,
    cc: &Path,
    cxx: &Path,
    cpp: bool,
    header: &Path,
    native_dir: &Path,
    glue: &Path,
    files: &[NdkFile],
) -> Result<Vec<u8>, CompileError> {
    fs::create_dir_all(out_dir).map_err(|err| CompileError::bare(err.to_string()))?;
    let mut objects = Vec::new();
    for file in files {
        let obj = out_dir.join(format!("{}.o", file.path.replace('/', "_")));
        let driver = if is_cpp(&file.path) { cxx } else { cc };
        compile_one(root, driver, header, native_dir, Path::new(&file.path), &obj)?;
        objects.push(obj);
    }
    let glue_obj = out_dir.join("jni_glue.o");
    compile_one(root, cc, header, native_dir, glue, &glue_obj)?;
    objects.push(glue_obj);
    let output = out_dir.join("libdroide_ndk.so");
    let linker = if cpp { cxx } else { cc };
    let mut args = vec![
        "-shared".to_string(),
        "-fPIC".into(),
        "-Wl,-soname,libdroide_ndk.so".into(),
        "-Wl,--no-undefined".into(),
    ];
    if cpp {
        args.push("-static-libstdc++".into());
    }
    for obj in &objects {
        args.push(obj.display().to_string());
    }
    args.push("-o".into());
    args.push(output.display().to_string());
    run_clang(root, linker, &args)?;
    let bytes = fs::read(&output).map_err(|err| CompileError::bare(format!("{}: {err}", output.display())))?;
    if bytes.len() < 4 || &bytes[..4] != b"\x7fELF" {
        return Err(CompileError::bare(format!(
            "{} is not an ELF library",
            output.display()
        )));
    }
    Ok(bytes)
}

fn is_cpp(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|ext| ext.to_str()),
        Some("cc") | Some("cpp")
    )
}

fn compile_one(
    root: &Path,
    clang: &Path,
    header: &Path,
    native_dir: &Path,
    source: &Path,
    object: &Path,
) -> Result<(), CompileError> {
    let header_dir = header.parent().unwrap_or(header).display().to_string();
    let args = vec![
        "-c".to_string(),
        "-fPIC".into(),
        "-O2".into(),
        "-fvisibility=hidden".into(),
        "-include".into(),
        header.display().to_string(),
        "-I".into(),
        header_dir,
        "-I".into(),
        native_dir.display().to_string(),
        "-o".into(),
        object.display().to_string(),
        source.display().to_string(),
    ];
    run_clang(root, clang, &args)
}

fn run_clang(root: &Path, clang: &Path, args: &[String]) -> Result<(), CompileError> {
    let compiled = Command::new(clang)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|err| CompileError::bare(format!("NDK clang {}: {err}", clang.display())))?;
    if compiled.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&compiled.stderr),
        String::from_utf8_lossy(&compiled.stdout)
    );
    Err(CompileError::bare(format!(
        "NDK compile failed: {}\n{}",
        clang.display(),
        tail(&text)
    )))
}

fn tail(text: &str) -> &str {
    const MAX: usize = 4000;
    if text.len() <= MAX {
        return text;
    }
    let mut start = text.len() - MAX;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

fn find_tools(min_sdk: u32) -> Result<Tools, CompileError> {
    let mut tried = Vec::new();
    for root in ndk_candidates() {
        let Some(bin) = prebuilt_bin(&root) else {
            tried.push(root.display().to_string());
            continue;
        };
        let arm_cc = bin.join(format!("aarch64-linux-android{min_sdk}-clang"));
        let arm_cxx = bin.join(format!("aarch64-linux-android{min_sdk}-clang++"));
        let x86_cc = bin.join(format!("x86_64-linux-android{min_sdk}-clang"));
        let x86_cxx = bin.join(format!("x86_64-linux-android{min_sdk}-clang++"));
        if arm_cc.is_file() && arm_cxx.is_file() && x86_cc.is_file() && x86_cxx.is_file() {
            return Ok(Tools {
                arm_cc,
                arm_cxx,
                x86_cc,
                x86_cxx,
            });
        }
        tried.push(bin.display().to_string());
    }
    let looked = if tried.is_empty() {
        "(no NDK directory)".to_string()
    } else {
        tried.join(", ")
    };
    Err(CompileError::bare(format!(
        "NDK clang for android{min_sdk} was not found. Set ANDROID_NDK_HOME. Looked in: {looked}"
    )))
}

fn ndk_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for key in ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"] {
        if let Some(value) = std::env::var_os(key) {
            let path = PathBuf::from(value);
            if path.is_dir() {
                out.push(path);
            }
        }
    }
    let mut sdks = Vec::new();
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = std::env::var_os(key) {
            sdks.push(PathBuf::from(value));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        sdks.push(PathBuf::from(home).join(".android-sdk"));
    }
    for sdk in sdks {
        let mut versions: Vec<PathBuf> = fs::read_dir(sdk.join("ndk"))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|ent| ent.ok())
            .map(|ent| ent.path())
            .filter(|path| path.is_dir())
            .collect();
        versions.sort();
        versions.reverse();
        out.extend(versions);
    }
    out
}

fn prebuilt_bin(ndk: &Path) -> Option<PathBuf> {
    let root = ndk.join("toolchains/llvm/prebuilt");
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .filter_map(|ent| ent.ok())
        .map(|ent| ent.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    let host = dirs
        .iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("linux-x86_64"))
        })
        .or_else(|| dirs.first())?;
    Some(host.join("bin"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_an_export_and_skips_comments() {
        let src = r#"
#include <stdint.h>
#include <string.h>
#define DROIDE_EXPORT ignored
/* DROIDE_EXPORT int32_t hidden(int32_t n); */
// DROIDE_EXPORT int32_t also(int32_t n);
const char *note = "DROIDE_EXPORT int32_t nope(int32_t n);";
DROIDE_EXPORT int32_t mix(int32_t n) {
    return n + (int32_t)strlen("ndk");
}
"#;
        let scan = scan_exports(src, "native/mix.c");
        assert!(scan.error.is_none(), "{:?}", scan.error);
        assert_eq!(scan.exports.len(), 1);
        assert_eq!(scan.exports[0].name, "mix");
        assert_eq!(scan.exports[0].params, vec!["n".to_string()]);
    }

    #[test]
    fn scans_void_split_and_two_params() {
        let src = "DROIDE_EXPORT int32_t\nadd(int32_t a, int32_t b) {\n  return a + b;\n}\nDROIDE_EXPORT int32_t tick(void);\n";
        let scan = scan_exports(src, "native/calc.c");
        assert!(scan.error.is_none(), "{:?}", scan.error);
        assert_eq!(scan.exports[0].name, "add");
        assert_eq!(scan.exports[0].params, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(scan.exports[0].line, 2);
        assert!(scan.exports[1].params.is_empty());
    }

    #[test]
    fn rejects_pointers_and_a_fifth_parameter() {
        let star = scan_exports("DROIDE_EXPORT int32_t *bad(int32_t n);\n", "native/bad.c");
        assert!(star.error.unwrap().message.contains("returns int32_t"));
        let param = scan_exports("DROIDE_EXPORT int32_t bad(int32_t *p);\n", "native/bad.c");
        assert!(param.error.unwrap().message.contains("parameter is int32_t"));
        let many = scan_exports(
            "DROIDE_EXPORT int32_t bad(int32_t a, int32_t b, int32_t c, int32_t d, int32_t e);\n",
            "native/bad.c",
        );
        assert!(many.error.unwrap().message.contains("at most 4"));
    }
}
