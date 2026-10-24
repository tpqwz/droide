# Restore

This directory is a Jujutsu repository with a colocated Git store (`.jj` and `.git`). Use `jj` for history. The Git directory is the storage backend.

The named snapshot is the bookmark `restore-point`. It is the tree from 2026-10-24: the Rust studio, the Rust compiler in `compiler/`, the Python browser IDE, the Qt fallback, the examples, and the tests. Build outputs are not in the snapshot (`studio/target/`, `compiler/target/`, `build/`, and `*.apk`).

The change id is filled in below after the bookmark is created. `jj show restore-point` prints the commit id.

## Look at the snapshot

```sh
jj bookmark list
jj log
jj show restore-point
```

## Put files back

`jj restore` copies file contents. It leaves the current change where it is.

```sh
jj restore --from restore-point
```

That makes every tracked file in the working copy (`@`) match `restore-point`. To bring back one path:

```sh
jj restore --from restore-point compiler/src/build.rs
```

## Start a new change from the snapshot

```sh
jj new restore-point
```

The new change is empty and its parent is `restore-point`. Earlier changes stay in `jj log`.

## Undo the last `jj` command

The operation log is a second way back. It restores repository state, including bookmark moves, not just file contents.

```sh
jj op log
jj undo
```

`jj undo` walks backward one operation at a time. `jj redo` walks forward again. To return to a specific operation id from `jj op log`:

```sh
jj op restore <operation-id>
```

## What a restore does not bring back

- `studio/target/` and `compiler/target/`. Rebuild them:

```sh
cargo build --offline --manifest-path compiler/Cargo.toml --bin droide
cargo build --offline --manifest-path studio/Cargo.toml
```

- `build/` and any `*.apk`
- The debug signing key in `~/.local/share/droide/`
- A running studio process. Start `./droide.sh` again after the studio binary exists.

## Keep the bookmark where it is

`restore-point` does not move when you edit files. `jj new` and ordinary edits create new changes on top of it.

These commands retarget or rewrite it. Run them only when you mean to replace the snapshot:

```sh
jj bookmark set restore-point
jj bookmark move restore-point --to @
jj edit restore-point
```

On this machine the repo config marks that commit immutable, so `jj` refuses to rewrite it:

```sh
jj config set --repo 'revset-aliases."immutable_heads()"' 'builtin_immutable_heads() | restore-point'
```

Repo config lives outside the directory (`jj config path --repo`). A fresh clone does not have that line until you set it again. The bookmark itself is in the repository.

## Change id

`restore-point` change id: `syzyvzyqtyrpwrtzttnyywzwrprrvqqo`
