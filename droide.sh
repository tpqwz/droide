#!/bin/sh
cd "$(dirname "$0")"
ROOT=$(pwd)

# The studio and the compiler are native binaries. ide --web and the Qt fallback stay on Python.
want_studio=0
if [ $# -eq 0 ]; then
  want_studio=1
elif [ "$1" = "ide" ]; then
  want_studio=1
  for arg in "$@"; do
    if [ "$arg" = "--web" ]; then
      want_studio=0
    fi
  done
fi

if [ "$want_studio" = 1 ]; then
  STUDIO=""
  if [ -x "$ROOT/studio/target/release/droide-studio" ]; then
    STUDIO="$ROOT/studio/target/release/droide-studio"
  elif [ -x "$ROOT/studio/target/debug/droide-studio" ]; then
    STUDIO="$ROOT/studio/target/debug/droide-studio"
  fi
  if [ -n "$STUDIO" ]; then
    if [ "${1:-}" = "ide" ]; then
      shift
    fi
    exec "$STUDIO" "$@"
  fi
fi

# build, run, analyze, and new use the Rust compiler when that binary exists.
if [ "$want_studio" = 0 ]; then
  case "${1:-}" in
    build|run|analyze|new)
      COMPILER=""
      if [ -x "$ROOT/compiler/target/release/droide" ]; then
        COMPILER="$ROOT/compiler/target/release/droide"
      elif [ -x "$ROOT/compiler/target/debug/droide" ]; then
        COMPILER="$ROOT/compiler/target/debug/droide"
      fi
      if [ -n "$COMPILER" ]; then
        exec "$COMPILER" "$@"
      fi
      ;;
  esac
fi

# Qt 6 on X11 needs libxcb-cursor. Use a user copy when the system package is absent.
LIB="${HOME}/.local/share/droide/lib"
if [ -e "$LIB/libxcb-cursor.so.0" ]; then
  LD_LIBRARY_PATH="$LIB${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  export LD_LIBRARY_PATH
fi
exec python3 -m droide "$@"
