#!/usr/bin/env bash
# Sets up (or inspects) the development environment for the OpenGL 2.1 port.
#
#   tools/gl21_preflight.sh                 # report what is here and what is missing
#   tools/gl21_preflight.sh --install       # install the Python-side gates into .local/gl21-env
#   tools/gl21_preflight.sh --mirror URL    # use a PyPI mirror (also: PIP_INDEX_URL=...)
#   tools/gl21_preflight.sh --json out.json # machine-readable report
#
# It never installs system packages and never writes outside .local/: on a machine
# without apt rights (containers, CI, this sandbox) that is the difference between a
# script that helps and one that fails half way through with a half-built toolchain.
#
# Two environments matter, and they are not the same:
#
#   * the build machine (this one): needs rustc/cargo and, to *draw*, a GL driver
#     (Mesa llvmpipe is enough: `LIBGL_ALWAYS_SOFTWARE=1`) plus a display for a window
#     (Xvfb, or EGL surfaceless for `--offscreen`);
#   * the target machine: a Windows 7 x86 box with a Radeon X1650 and Catalyst 9.3,
#     where only `i686-win7-windows-msvc` (nightly + -Z build-std) or import shims run.
#
# Exit status: 0 when everything the current phase needs is present, 1 otherwise.
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
ENVDIR="$ROOT/.local/gl21-env"
JSON=""
INSTALL=0
MIRROR="${PIP_INDEX_URL:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --install) INSTALL=1 ;;
    --mirror) MIRROR="${2:-}"; shift ;;
    --mirror=*) MIRROR="${1#*=}" ;;
    --json) JSON="${2:-}"; shift ;;
    --json=*) JSON="${1#*=}" ;;
    -h|--help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

have() { command -v "$1" >/dev/null 2>&1; }
ver() { "$@" 2>/dev/null | head -1; }

report=""
add() { # add <status> <name> <detail>
  report="$report$1|$2|$3
"
  printf '%-6s %-26s %s\n' "$1" "$2" "$3"
}

MISSING_HARD=0
MISSING_SOFT=0

echo "horizon gl21 preflight — $ROOT"
echo

# --- compilers -------------------------------------------------------------
if have rustc; then
  add ok rustc "$(ver rustc --version)"
else
  add MISS rustc "no rustc: the Rust crates cannot be built here (see docs/gl21-dev-env.md)"
  MISSING_HARD=1
fi
if have cargo; then
  add ok cargo "$(ver cargo --version)"
  if have rustup; then
    add ok rustup "$(ver rustup --version)"
    for t in i686-win7-windows-msvc x86_64-win7-windows-msvc; do
      if rustup target list --installed 2>/dev/null | grep -qx "$t"; then
        add ok "target $t" "installed"
      else
        add MISS "target $t" "rustup target add $t (tier 3: needs a pinned nightly and -Z build-std)"
        MISSING_SOFT=1
      fi
    done
  fi
else
  add MISS cargo "no cargo: crates cannot be built here"
  MISSING_HARD=1
fi

# --- OpenGL: the driver the port is developed against ----------------------
libgl=$(ldconfig -p 2>/dev/null | awk '/libGL\.so/{print $NF; exit}')
if [ -n "$libgl" ]; then
  add ok libGL "$libgl"
else
  add MISS libGL "no libGL: nothing can create a GL context here"
  MISSING_HARD=1
fi
libegl=$(ldconfig -p 2>/dev/null | awk '/libEGL\.so/{print $NF; exit}')
if [ -n "$libegl" ]; then
  add ok libEGL "$libegl"
else
  add warn libEGL "no libEGL: use WGL/GLX or install Mesa's EGL (needed for --offscreen)"
  MISSING_SOFT=1
fi
if [ -d /dev/dri ]; then
  add ok /dev/dri "$(ls /dev/dri | tr '\n' ' ')"
else
  add warn /dev/dri "no DRM nodes: Mesa must run software (LIBGL_ALWAYS_SOFTWARE=1, llvmpipe)"
fi
dri=$(ls -d /usr/lib/*/dri 2>/dev/null | head -1)
if [ -n "$dri" ] && ls "$dri" 2>/dev/null | grep -q swrast; then
  add ok llvmpipe "swrast_dri.so in $dri"
elif [ -n "$dri" ]; then
  add warn llvmpipe "$dri has no swrast_dri: software GL unavailable"
  MISSING_SOFT=1
else
  add MISS llvmpipe "no DRI drivers: install Mesa (mesa-utils libgl1-mesa-dri libegl-mesa0)"
  MISSING_HARD=1
fi
if have glxinfo; then
  add ok glxinfo "$(glxinfo -B 2>/dev/null | awk -F': ' '/OpenGL version/{print $2; exit}')"
else
  add warn glxinfo "no glxinfo: cannot print the GL version (mesa-utils)"
fi
if have Xvfb || have xvfb-run; then
  add ok Xvfb "present"
else
  add warn Xvfb "no virtual display: windowed runs need one (xvfb-run), offscreen runs do not"
fi

# --- gates that run without a compiler or a GPU ----------------------------
if have python3; then
  add ok python3 "$(ver python3 --version)"
  if python3 - "$ROOT" <<'PY' >/dev/null 2>&1
import sys
sys.path.insert(0, sys.argv[1])
from tools import gl21_budget
gl21_budget.scan_shaders()
PY
  then
    add ok budget-gate "tools/gl21_budget.py runs"
  else
    add MISS budget-gate "tools/gl21_budget.py fails to run"
    MISSING_HARD=1
  fi
else
  add MISS python3 "the Python gates need it"
  MISSING_HARD=1
fi

if python3 -c 'import tree_sitter, tree_sitter_rust' >/dev/null 2>&1 \
   || [ -n "$(ls "$ENVDIR"/tree_sitter* 2>/dev/null)" ]; then
  add ok tree-sitter "Rust syntax gate available"
else
  add warn tree-sitter "no tree_sitter/tree_sitter_rust: tools/gl21_rustsyntax.py cannot run"
  MISSING_SOFT=1
fi

if have glslangValidator; then
  add ok glslangValidator "$(glslangValidator --version 2>/dev/null | sed -n 2p)"
elif have glslang; then
  add ok glslang "present"
else
  add warn glslang "no glslangValidator: shaders are checked by the lexical gate only"
fi

echo
if [ "$INSTALL" = 1 ]; then
  echo "installing the Python gates into .local/gl21-env"
  mkdir -p "$ENVDIR"
  pip_args="--target $ENVDIR --upgrade --quiet"
  if [ -n "$MIRROR" ]; then
    pip_args="$pip_args --index-url $MIRROR"
  fi
  # shellcheck disable=SC2086
  python3 -m pip install $pip_args tree-sitter tree-sitter-rust \
    && echo "installed: tree-sitter, tree-sitter-rust"
  echo "add to PYTHONPATH: $ENVDIR"
fi

if [ -n "$JSON" ]; then
  mkdir -p "$(dirname "$JSON")"
  {
    printf '{\n  "root": "%s",\n  "tools": [\n' "$ROOT"
    first=1
    printf '%s' "$report" | while IFS='|' read -r status name detail; do
      [ -z "${name:-}" ] && continue
      if [ "$first" = 1 ]; then first=0; else printf ',\n'; fi
      printf '    {"status": "%s", "name": "%s", "detail": "%s"}' \
        "$status" "$name" "$(printf '%s' "$detail" | sed 's/"/\\"/g')"
    done
    printf '\n  ]\n}\n'
  } > "$JSON"
  echo "written: $JSON"
fi

echo
if [ "$MISSING_HARD" = 1 ]; then
  echo "RESULT: cannot build or draw here yet — see docs/gl21-dev-env.md for the two commands"
  exit 1
fi
if [ "$MISSING_SOFT" = 1 ]; then
  echo "RESULT: can build and draw; some optional gates are missing"
  exit 0
fi
echo "RESULT: ready"
