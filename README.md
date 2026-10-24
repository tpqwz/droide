# Droide

Droide is a self-contained RAD IDE for Android. The studio is a native binary, `studio/target/debug/droide-studio`, built with Rust and egui. The form on the canvas uses those same egui widgets, and Preview opens that form inside the IDE. Compile, Run, and Analyze call the Rust compiler, `compiler/target/debug/droide`, which writes a real DEX APK. Signing still calls `openssl`. The browser IDE and the Qt fallback stay on Python. It does not need the Android SDK, `android.jar`, or an interpreter. A device or the emulator is used only when you launch.

```sh
./droide.sh                  # native studio
./droide.sh ide --web        # browser studio at http://127.0.0.1:8765/
./droide.sh build examples/Hello
./droide.sh run examples/Hello
./droide.sh analyze examples/Hello
./droide.sh build examples/Search
./droide.sh build examples/Library
./droide.sh new apps/Demo --name Demo --package com.example.demo
```

History is a Jujutsu repository. `RESTORE.md` describes the `restore-point` bookmark and the commands that put the tree back.

The designer opens on the form. A short palette sits beside it. The inspector appears when a widget is selected, and `F11` pins it. Preview (`F8`) shows the form with the egui widgets the studio itself uses. Project files, compile, and shortcuts live under the project name.

In the native studio, `F9` builds and launches, `Ctrl+F9` compiles, `F12` switches design and code, `F4` opens Explore, `F8` previews the form, `F11` pins the inspector, and `Ctrl+S` saves. On the form, Duplicate or `Ctrl+D` copies the selected widget 16dp down and right, with a new name and the same text and click handler. `Ctrl+F` finds in the current file, `Ctrl+P` opens a file or routine, `Ctrl+Click` and `Ctrl+Enter` go to the declaration, and `F3` opens the name under the caret in Explore. Explore groups entities by file. A routine shows its complexity and fan-in. In files searches every form and native buffer and opens the matching line. The page for one entity shows the call graph, each call and use with the source line, a control-flow graph, and architecture metrics. Opening a caller or callee shows that name's own calls and callers, and each opened name can be opened again. A cycle stops, and the chain stops after four steps. A block in that graph opens at its line. Double-click an entity to open it at its line. A reference opens the line that mentions it. A compiler message in Messages does the same, including the column when the line has one.

The code view has a procedure list above the editor. It names each `procedure`, `function`, and `fun`. The list follows the caret. Choosing a name moves the caret to that header. Above the first header it reads `(declarations)`. Code colors keywords, strings, comments, and numbers. Droide and `.nc` use their own keywords. C and C++ use C keywords, and a preprocessor line is a comment, so a brace in a macro does not open a block. The left margin numbers the lines and draws the open block: `begin`/`end` in Droide, and `{`/`}` in `.nc` and C. A comment or a string does not count as a block.

The browser studio (`ide --web`) colors Droide and `.nc` the same way, with the same gutter, and also has `Ctrl+F`, `Ctrl+P`, and `Ctrl+Click` to a declaration.

## Project

```
project.json
forms/<Name>.form.json    designer
forms/<Name>.droid        form code
units/<Name>.droid        static routines
native/<Name>.nc          i32 functions, no libc
native/<Name>.c           NDK C
native/<Name>.cc          NDK C++
build/app.apk             output
```

Widgets are real Android classes on a `FrameLayout`, placed in dp: Label, Clock, Button, Toggle, Edit, Memo, Number, Password, Email, Phone, Check, Radio, Switch, Seek, Progress, Busy, Rating, Picker, Time, Date, Panel, List, Spinner, and Web. Web is an `android.webkit.WebView` with `Html` and `Url`. List is an `android.widget.ListView`. `Items` is one row per line, drawn with the platform row layout. Spinner is an `android.widget.Spinner` with those same rows and a `Selection` index; `OnItemSelected` passes the row text, the index, or the row view, and it is attached after the index is set so the first value does not fire. Radio buttons are independent. Number uses a signed numeric keyboard. Email and Phone request those keyboards. Picker is a number wheel (`MinValue`, `MaxValue`, `Value`). Time is hours and minutes. Date is a calendar whose month is 1–12. Busy spins with no progress value. Panel is an `android.widget.LinearLayout`. A component's `parent` names the panel that holds it. Children stack vertically, or horizontally when `Direction` is Horizontal. `left` and `top` on a child are the margins of that stack. A panel with no children stays on the form as a colored rectangle. Events are `OnClick`, `OnLongClick`, `OnItemClick`, `OnItemLongClick`, `OnItemSelected`, `OnChange`, and `OnCreate`. List uses `OnItemClick` and `OnItemLongClick`. Spinner uses `OnItemSelected`. The other widgets use `OnClick` and `OnLongClick`. A long-press handler takes no parameters or one `View`, and the listener returns true so the press is consumed. `OnItemClick` and `OnItemLongClick` pass the row text to a `String` parameter, the index to an `Integer`, or the row itself to a `View`. `OnChange` is on Edit, Memo, Number, Password, Email, Phone, Check, Radio, Switch, Toggle, Seek, Rating, Picker, Time, and Date. It passes the new text as a `String`, a `Boolean` for Check, Radio, Switch, and Toggle, and an `Integer` for Seek, Rating, and Picker. Time and Date take no parameters, and the handler reads Hour, Minute, Year, Month, and Day. A change listener is attached after the initial properties, so the first value does not fire it. Date uses `DatePicker.init` when `OnChange` is set, which keeps the listener on API 24. A double-click opens the handler already on the widget. It inserts `procedure NameClick(Sender: View)` only when `OnClick` is empty, `procedure NameItemClick(Row: String)` for a List, and `procedure NameItemSelected(Row: String)` for a Spinner. A body that is already there stays.

## Language

Units, forms, `procedure` / `function`, `var`, `begin`/`end`, `if`/`then`/`else`, `while`/`do`, `Exit`, and `Result`. Types are `Integer`, `String`, `Boolean`, and `View`. Semicolons are optional. Names are case-insensitive.

Built-ins: `Toast`, `Message`, `Question`, `Log`, `OpenForm('Form')`, `Finish`, `IntToStr`, `StrToInt`, `IsInt`, `Length`, `WriteText`, `ReadText`, `Where`, `OpenCamera`, `Share`, `CopyText`, `PasteText`, `Search`. `Message` shows the text in an alert with OK. `Question` shows the text with Yes and No, and Yes calls a procedure that takes no parameters. `Share` sends the text through an `ACTION_SEND` chooser, and does nothing when no app handles it. `CopyText` puts the text on the clipboard. `PasteText` returns that plain text, or an empty string when the clipboard is empty or not text. `IsInt` is true for an optional sign and one to nine digits. `WriteText` and `ReadText` use the app's private files. `Where` returns the last known location. `Search` returns matching notes, one per line. An empty query, a device below API 31, or a failed session returns five built-in notes. A non-empty query uses AppSearch when `SDK_INT` is at least 31 and the `app_search` service is present. An empty AppSearch result stays empty. `Ingest` stores a title, a body, and a kind (`txt`, `md`, `html`, `csv`, or `json`) in a SQLite FTS4 table. `Lookup` lists that library. A blank query lists every document through FTS4. A word uses AppSearch when the device has it, and FTS4 when the session is missing or fails. The FTS4 match reads `offsets()` and keeps every hit on its own row. Each hit is marked and keeps 256 characters of the column on either side, wider than the 64-token `snippet()` window. AppSearch uses that same window from each match range and reads every page. `/` and `div` are both integer division.

Native functions are pure `i32` with at most four parameters:

```
fun fib(n: i32): i32 {
  if n < 2 { return n; }
  return fib(n - 1) + fib(n - 2);
}
```

Each `.nc` function is a C ABI symbol plus a JNI stub `Java_<package>_Native_<name>` in `libdroide.so`. That library has no libc. The generated `Native` class loads it. `div` and `mod` are operators on the form side, so a native quotient used from a form is named `quo` in the sample.

A `DROIDE_EXPORT` function in `native/*.c`, `native/*.cc`, or `native/*.cpp` is the same Integer call, with at most four `int32_t` parameters. The body is C or C++ and may use pointers and the C library. NDK clang compiles it for `arm64-v8a` and `x86_64` into `libdroide_ndk.so`, and the `Native` class loads that library too. Put the marker on the definition:

```c
#include <string.h>
DROIDE_EXPORT int32_t mix(int32_t n) {
    return n + (int32_t)strlen("ndk");
}
```

## What the APK contains

`AndroidManifest.xml` (binary XML, no `resources.arsc`), `classes.dex` (DEX 035 calling the framework directly), `libdroide.so` when a `.nc` file has functions, and `libdroide_ndk.so` when a C or C++ file exports a function. Both ABIs are `arm64-v8a` and `x86_64`. Signing is APK Signature Scheme v2 with a debug key in `~/.local/share/droide/`. Minimum SDK 24, target SDK 28. `extractNativeLibs` is true, so the libraries may be stored compressed. The studio and `./droide.sh build` run this compiler. The browser IDE and the Qt fallback stay on Python, which reads `.nc` files.

Class names `DroideRt`, `DroideSearch`, `DroideIndex`, and `Native` are reserved. A form or unit cannot use them.

`./droide.sh run` installs with `adb` and starts the main activity. If nothing is in the `device` state, it boots the first emulator image and waits until Android is up. A crash in the first second is written into the launch log.

## Samples

`examples/Library` is a document index. Find calls `Lookup`. An empty library is filled with five sample documents, and Add pastes another as plain text, Markdown, HTML, CSV, or JSON. A word is searched with AppSearch when the device has that service, and with SQLite FTS4 otherwise. Each match is its own row, with the text around it. AppSearch does the same from match ranges and reads every page. `examples/Search` lists five notes. Find calls `Search`, which uses AppSearch when the device has that service and the same notes otherwise. The list starts with those notes. `examples/Hello` is a two-screen app: a greeting, `OpenForm`, and `fib` clamped through `Utils.Clamp`. `examples/Swatch` mixes red, green, blue, and light on a panel; `native/pack.c` scales the channels and packs the Android color word, and `Tint.HexByte` prints it. `examples/Device` saves a private note, reads it back, asks `Where`, opens the camera, and loads a page in a `Web` view. `examples/Shelf` shows a `List`, reports the tapped row, and replaces the rows from a memo. `examples/NativeCalc` calls `add`, `sub`, `mul`, `quo`, `rem`, `abs`, `fib`, `sumto`, and `dist`. `examples/Ndk` calls `.nc` `echo` and NDK `mix` and `span`.

## Limits

`.nc` code is `i32` arithmetic: no pointers, no libc, no heap. The register window is 16, so a Droide routine cannot grow past a handful of locals. There is no resource table, so images and custom row layouts stay out. A `List` uses the platform row. Document pickers and the camera's returned photo need an activity result, which is still ahead. `Search` uses AppSearch when `SDK_INT` is at least 31 and `getSystemService("app_search")` returns a service. Below that, it returns the built-in notes. `Lookup` uses the same gate and keeps every document in FTS4, which Android has had since well before API 24. FTS4 `offsets()` plus the runtime's own walk returns every match on its own row, with its snippet. AppSearch walks each match range the same way and reads every page. `snippet()` itself stops at 64 tokens and one window, on FTS4 and on FTS5. The target SDK stays 28. PDF and Word files need a parser this runtime does not include, and choosing a file still needs an activity result. Division by zero is undefined in `.nc` code (`idiv` faults on x86_64; ARM `sdiv` returns 0). A Droide call into NDK code is still an `int32_t` with at most four parameters. The C body may use pointers and libc. The export itself is written in the `.c` or `.cc` file, not only in a header.
