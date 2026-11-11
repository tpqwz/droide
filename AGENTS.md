# Droide

Two Rust crates, not one workspace. The toolchain is `rust-toolchain.toml`: 1.99.1.

```sh
cargo test --locked --manifest-path compiler/Cargo.toml
cargo test --locked --manifest-path studio/Cargo.toml
cargo build --release --locked --manifest-path compiler/Cargo.toml
cargo build --release --locked --manifest-path studio/Cargo.toml
```

Point rust-analyzer at `compiler/` or `studio/`, the directory of the crate you are changing. `~/code` and this repo root are not Cargo workspaces.

`.nc` code is `i32` arithmetic: no pointers, no libc, no heap. A routine has a 16-register window. NDK bodies in `.c` or `.cc` may use pointers. Do not widen that split unless the user asked.

`./droide.sh` uses the release binaries when they exist. The studio needs X11 and OpenGL on Linux. The compiler calls `openssl` when it signs an APK.
