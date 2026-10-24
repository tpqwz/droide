# Droide

Droide is a self-contained RAD IDE for Android. The studio is a native binary, `studio/target/debug/droide-studio`, built with Rust and egui. The form on the canvas uses those same egui widgets, and Preview opens that form inside the IDE. Compile, Run, and Analyze call the Rust compiler, `compiler/target/debug/droide`, which writes a real DEX APK. Signing still calls `openssl`. The browser IDE and the Qt fallback stay on Python. It does not need the Android SDK, `android.jar`, or an interpreter. A device or the emulator is used only when you launch.

```sh
./droide.sh                  # native studio
./droide.sh ide --web        # browser studio at http://127.0.0.1:8765/
./droide.sh build examples/Hello
./droide.sh run examples/Hello
./droide.sh analyze examples/Hello
./droide.sh new apps/Demo --name Demo --package com.example.demo
```

History is a Jujutsu repository. `RESTORE.md` describes the `restore-point` bookmark and the commands that put the tree back.

The designer opens on the form. A short palette sits beside it. The inspector appears when a widget is selected, and `F11` pins it. Preview (`F8`) shows the form with the egui widgets the studio itself uses. Project files, compile, and shortcuts live under the project name.

In the native studio, `F9` builds and launches, `Ctrl+F9` compiles, `F12` switches design and code, `F4` opens Explore, `F8` previews the form, `F11` pins the inspector, and `Ctrl+S` saves. Explore lists the entities and shows the call graph, control flow, references, and architecture metrics.

Code colors keywords, strings, comments, and numbers. The left margin numbers the lines and draws the open block: `begin`/`end` in Droide, and `{`/`}` in native code. A comment or a string does not count as a block.

The browser studio (`ide --web`) uses that same coloring and gutter, and also has `Ctrl+F`, `Ctrl+P`, and `Ctrl+Click` to a declaration.

## Project

```
project.json
forms/<Name>.form.json    designer
forms/<Name>.droid        form code
units/<Name>.droid        static routines
native/<Name>.nc          i32 functions
build/app.apk             output
```

Widgets are real Android classes on a `FrameLayout`, placed in dp: Label, Clock, Button, Toggle, Edit, Memo, Number, Password, Email, Phone, Check, Radio, Switch, Seek, Progress, Busy, Rating, Picker, Time, Date, and Panel. Radio buttons are independent. Number uses a signed numeric keyboard. Email and Phone request those keyboards. Picker is a number wheel (`MinValue`, `MaxValue`, `Value`). Time is hours and minutes. Date is a calendar whose month is 1–12. Busy spins with no progress value. Events are `OnClick` and `OnCreate`. A double-click in the designer inserts `procedure NameClick(Sender: View)`.

## Language

Units, forms, `procedure` / `function`, `var`, `begin`/`end`, `if`/`then`/`else`, `while`/`do`, `Exit`, and `Result`. Types are `Integer`, `String`, `Boolean`, and `View`. Semicolons are optional. Names are case-insensitive.

Built-ins: `Toast`, `Log`, `OpenForm('Form')`, `Finish`, `IntToStr`, `StrToInt`, `Length`. `/` and `div` are both integer division.

Native functions are pure `i32` with at most four parameters:

```
fun fib(n: i32): i32 {
  if n < 2 { return n; }
  return fib(n - 1) + fib(n - 2);
}
```

Each one is a C ABI symbol plus a JNI stub `Java_<package>_Native_<name>` in `libdroide.so`. The generated `Native` class loads that library. `div` and `mod` are operators on the form side, so a native quotient used from a form is named `quo` in the sample.

## What the APK contains

`AndroidManifest.xml` (binary XML, no `resources.arsc`), `classes.dex` (DEX 035 calling the framework directly), and the two native libraries when a `.nc` file has functions. Signing is APK Signature Scheme v2 with a debug key in `~/.local/share/droide/`. Minimum SDK 24, target SDK 28. `extractNativeLibs` is true, so the libraries may be stored compressed.

Class names `DroideRt` and `Native` are reserved. A form or unit cannot use either.

`./droide.sh run` installs with `adb` and starts the main activity. If nothing is in the `device` state, it boots the first emulator image and waits until Android is up. A crash in the first second is written into the launch log.

## Samples

`examples/Hello` is a two-screen app: a greeting, `OpenForm`, and `fib` clamped through `Utils.Clamp`. `examples/NativeCalc` calls `add`, `sub`, `mul`, `quo`, `rem`, `abs`, `fib`, `sumto`, and `dist`.

## Limits

Native code is `i32` arithmetic only: no pointers, no libc, no heap. The register window is 16, so a Droide routine cannot grow past a handful of locals. There is no resource table, so images, adapters, and a web view are out of scope. Division by zero is undefined in native code (`idiv` faults on x86_64; ARM `sdiv` returns 0).
