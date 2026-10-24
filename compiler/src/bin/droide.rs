//! Compiler commands. The studio and `ide --web` stay on their own programs.
//! `droide build|run|analyze|new` is the native entry those commands will call.

use std::path::PathBuf;
use std::process::ExitCode;

use droide_compiler::build::{analyze, build, print_metrics, print_result, run};
use droide_compiler::project::Project;

const USAGE: &str = "\
droide 0.1.0

  droide build PATH [-v]       write PATH/build/app.apk
  droide run PATH [-v]         build, install, and start on a device
  droide analyze PATH [--json] typecheck and print the explorer index
  droide new PATH --name NAME --package PKG [--title TITLE]

The IDE is still `./droide.sh` (native window) or `./droide.sh ide --web`.
";

struct Args {
    cmd: String,
    path: String,
    verbose: bool,
    json: bool,
    name: Option<String>,
    package: Option<String>,
    title: Option<String>,
}

fn parse(argv: &[String]) -> Result<Args, String> {
    if argv.is_empty() || argv.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Err(USAGE.to_string());
    }
    let cmd = argv[0].clone();
    if !matches!(cmd.as_str(), "build" | "run" | "analyze" | "new") {
        return Err(format!("unknown command {cmd}\n{USAGE}"));
    }
    let mut path = None;
    let mut verbose = false;
    let mut json = false;
    let mut name = None;
    let mut package = None;
    let mut title = None;
    let mut i = 1;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
            "--json" => json = true,
            "--name" | "--package" | "--title" => {
                let Some(value) = argv.get(i + 1) else {
                    return Err(format!("{arg} needs a value\n{USAGE}"));
                };
                match arg.as_str() {
                    "--name" => name = Some(value.clone()),
                    "--package" => package = Some(value.clone()),
                    _ => title = Some(value.clone()),
                }
                i += 1;
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other}\n{USAGE}")),
            other => {
                if path.is_some() {
                    return Err(format!("unexpected argument {other}\n{USAGE}"));
                }
                path = Some(other.to_string());
            }
        }
        i += 1;
    }
    let Some(path) = path else {
        return Err(format!("{cmd} needs a project path\n{USAGE}"));
    };
    Ok(Args { cmd, path, verbose, json, name, package, title })
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|arg| arg == "--version") {
        println!("droide 0.1.0");
        return ExitCode::SUCCESS;
    }
    let args = match parse(&argv) {
        Ok(args) => args,
        Err(message) => {
            eprint!("{message}");
            if !message.ends_with('\n') {
                eprintln!();
            }
            return ExitCode::from(1);
        }
    };
    match dispatch(args) {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn dispatch(args: Args) -> Result<u8, String> {
    if args.cmd == "new" {
        let name = args.name.ok_or_else(|| format!("new needs --name\n{USAGE}"))?;
        let package = args.package.ok_or_else(|| format!("new needs --package\n{USAGE}"))?;
        let project = Project::create(&args.path, &name, &package, args.title.as_deref()).map_err(|err| err.to_string())?;
        println!("{}", project.root.display());
        return Ok(0);
    }
    let root = PathBuf::from(&args.path);
    if args.cmd == "analyze" {
        let result = analyze(&root);
        if args.json {
            let payload = serde_json::json!({
                "ok": result.ok,
                "errors": result.errors,
                "db": result.db,
            });
            println!("{}", serde_json::to_string(&payload).map_err(|err| err.to_string())?);
            return Ok(if result.ok { 0 } else { 1 });
        }
        let code = print_result(&result, false);
        print_metrics(&result.db);
        return Ok(code as u8);
    }
    if args.cmd == "run" {
        let result = run(&root).map_err(|err| err.to_string())?;
        return Ok(print_result(&result, args.verbose) as u8);
    }
    let result = build(&root).map_err(|err| err.to_string())?;
    Ok(print_result(&result, args.verbose) as u8)
}
