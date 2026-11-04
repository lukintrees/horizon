#!/usr/bin/env python3
"""Check the GL 2.1 crate's call sites against glow's and its own real signatures, without a
Rust compiler.

Why this exists
---------------
The machine this crate was written on has no rustc (docs/gl21-dev-env.md: every Rust mirror is
blocked), so the first `cargo check` happens in CI. That is a slow way to find a call passing
four arguments to a function that takes three. This script reads the crate's sources with
tree-sitter and glow's sources from a checkout, and reports a call site when:

  * it calls a method that glow's `HasContext` trait does not declare;
  * it calls one of the crate's own functions with the wrong number of arguments;
  * it calls a glow or crate method with the wrong number of arguments;
  * it names a `glow::CONSTANT` that glow does not define;
  * it passes a literal of the wrong kind (`3` into a `bool`, `"x"` into a `u32`).

It is a lint, not a compiler. It resolves no generics, traits, lifetimes or inference: a call
is only checked when the checker can name the function and, for a method, the receiver's type
(`gl` is glow's context, `ctx` / `buffer` / `program` are this crate's). Everything it reports
is a real error, because a false positive would train the reader to ignore it — so a call it
cannot attribute is skipped in silence.

Usage
-----
    python3 tools/gl21_typecheck.py --glow /path/to/glow
    python3 tools/gl21_typecheck.py --glow /path/to/glow --json --verbose
    python3 tools/gl21_typecheck.py --self-test        # the checker's own fixtures

`--glow` needs `src/lib.rs` and, for glow's native types, `src/native.rs`.

Exit status: 0 nothing found, 1 something found, 2 usage or setup error.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    import tree_sitter
    import tree_sitter_rust
except ImportError:  # pragma: no cover - the message is the point
    sys.exit(
        "gl21_typecheck: the tree-sitter Rust grammar is not importable.\n"
        "  python3 tools/gl21_rustsyntax.py --install\n"
        "  or: pip install tree-sitter tree-sitter-rust"
    )

CRATE = Path("crates/horizon-client-gl")

# Literal node types, and what they are, for the argument kind check.
LITERAL_KINDS = {
    "string_literal": "&str",
    "integer_literal": "integer",
    "float_literal": "float",
    "char_literal": "char",
    "true": "bool",
    "false": "bool",
}

# The parameter types a literal can be compared against directly.
PLAIN_PARAMS = {
    "&str": {"&str", "String"},
    "String": {"&str", "String"},
    "bool": {"bool"},
    "u8": {"integer"},
    "i8": {"integer"},
    "u16": {"integer"},
    "i16": {"integer"},
    "u32": {"integer"},
    "i32": {"integer"},
    "u64": {"integer"},
    "i64": {"integer"},
    "usize": {"integer"},
    "isize": {"integer"},
    "f32": {"float", "integer"},
    "f64": {"float", "integer"},
}

# The crate's own types whose methods the checker reads: the context and the wrappers around
# GL objects. A receiver of any other type could be anything, so its methods are not checked.
CRATE_TYPES = {
    "Context",
    "Buffer",
    "Program",
    "Texture",
    "Target",
    "StateCache",
    "Executor",
    "GlLibrary",
    "Caps",
    "Limits",
    "Measured",
    "Decl",
    "Resources",
    "VertexFormat",
    "FrameStats",
}

GLOW_CONTEXT = "glow::Context"


@dataclass
class Param:
    ty: str
    optional: bool = False
    variadic: bool = False


@dataclass
class Function:
    name: str
    params: list[Param]
    ret: str = ""
    module: str = ""
    path: str = ""
    line: int = 0


@dataclass
class Finding:
    path: str
    line: int
    column: int
    message: str

    def as_dict(self) -> dict:
        return {
            "path": self.path,
            "line": self.line,
            "column": self.column,
            "message": self.message,
        }


# --------------------------------------------------------------------------------------------
# tree-sitter plumbing
# --------------------------------------------------------------------------------------------


def parser() -> "tree_sitter.Parser":
    lang = tree_sitter.Language(tree_sitter_rust.language())
    return tree_sitter.Parser(lang)


def node_text(node) -> str:
    return node.text.decode("utf-8", "replace") if node.text else ""


def walk(node):
    """Every node, depth first, without recursion (a 12 000-line file would blow the stack)."""
    stack = [node]
    while stack:
        current = stack.pop()
        yield current
        stack.extend(reversed(current.children))


def normalize_type(raw: str) -> str:
    """`&'a mut glow::Context<'x>` -> `glow::Context`: what the checker compares."""
    raw = re.sub(r"&\s*'\w+\s*", "&", raw)
    raw = raw.replace("&", "").replace("mut ", "")
    raw = re.sub(r"<.*>$", "", raw, flags=re.S)
    raw = re.sub(r"\s+", "", raw)
    return raw


def parameter_type(node) -> Param:
    ty_node = node.child_by_field_name("type")
    raw = normalize_type(node_text(ty_node)) if ty_node is not None else ""
    return Param(ty=raw, optional=raw.startswith("Option<"))


def return_type(node) -> str:
    ty_node = node.child_by_field_name("return_type")
    return normalize_type(node_text(ty_node)) if ty_node is not None else "()"


def function_of(node, module: str) -> Function | None:
    """A `function_item` as a `Function`, or None when it is not one."""
    name_node = node.child_by_field_name("name")
    if name_node is None:
        return None
    params: list[Param] = []
    params_node = node.child_by_field_name("parameters")
    if params_node is not None:
        for child in params_node.children:
            if child.type == "parameter":
                params.append(parameter_type(child))
            elif child.type == "variadic_parameter":
                params.append(Param(ty="...", variadic=True))
    return Function(
        name=node_text(name_node),
        params=params,
        ret=return_type(node),
        module=module,
        line=node.start_point[0] + 1,
    )


def file_module(crate_root: Path, path: Path) -> str:
    """The module a file is: `src/lib.rs` is `crate`, `src/gl/texture.rs` is `crate::gl::texture`.

    The grammar cannot know this — a file is only a module because some `mod` item declares it,
    and the checker does not follow those. The filesystem is the mapping rustc uses for the
    files in this crate, and getting it wrong makes two files' `find` helpers collide.
    """
    relative = path.relative_to(crate_root).with_suffix("")
    parts = list(relative.parts)
    if parts and parts[-1] == "lib":
        parts = parts[:-1]
    if parts and parts[-1] == "mod":
        parts = parts[:-1]
    return "::".join(["crate"] + parts)


def module_path(node) -> str:
    """The `mod` chain a node sits in: `crate`, `crate::gl`, `crate::gl::tests`."""
    names = []
    current = node.parent
    while current is not None:
        if current.type == "mod_item":
            name_node = current.child_by_field_name("name")
            if name_node is not None:
                names.append(node_text(name_node))
        current = current.parent
    return "::".join(["crate"] + list(reversed(names)))


def glob_imports_super(node) -> bool:
    """Whether the enclosing module has `use super::*`: how a test module sees its parent's
    items, and the only case in which an unqualified name resolves outwards."""
    current = node.parent
    while current is not None:
        if current.type == "mod_item":
            body = current.child_by_field_name("body")
            if body is not None:
                for child in body.children:
                    if child.type == "use_declaration":
                        raw = node_text(child).replace(" ", "")
                        if raw in ("usesuper::*;", "usesuper::{*};"):
                            return True
            return False
        current = current.parent
    return False


# --------------------------------------------------------------------------------------------
# Reading a crate: module-scoped functions and inherent methods
# --------------------------------------------------------------------------------------------


@dataclass
class CrateFacts:
    # module path -> function name -> Function, for free functions only (not impl members).
    functions: dict[str, dict[str, Function]]
    # function name -> the file that defines it. A test module's `use super::*` makes an
    # unqualified name cross into a file that also has that name, so the file has to agree.
    origins: dict[str, set[str]]
    # module path of a function -> does that module `use super::*`?
    glob_super: dict[str, bool]
    # type name -> method name -> Function, for inherent `impl Type` blocks.
    methods: dict[str, dict[str, Function]]
    files: list[Path]
    root: Path


def read_functions_in(
    path: Path, crate_root: Path
) -> tuple[dict[str, dict[str, Function]], dict[str, bool]]:
    """Free functions of one file, keyed by module, plus which modules glob-import `super`."""
    p = parser()
    source = path.read_bytes()
    tree = p.parse(source)
    functions: dict[str, dict[str, Function]] = {}
    glob_super: dict[str, bool] = {}
    file_mod = file_module(crate_root, path)
    for node in walk(tree.root_node):
        if node.type != "function_item":
            continue
        parent = node.parent
        if parent is not None and parent.type in ("impl_item", "trait_item"):
            continue  # methods are read by `read_methods_in`
        inner = module_path(node)
        module = file_mod if not inner else file_mod + inner[len("crate"):]
        fn = function_of(node, module)
        if fn is None:
            continue
        functions.setdefault(module, {})[fn.name] = fn
        glob_super.setdefault(module, False)
        glob_super[module] |= glob_imports_super(node)
    return functions, glob_super


def read_methods_in(path: Path) -> dict[str, dict[str, Function]]:
    """Inherent `impl Type { fn a() }` methods, keyed by the receiver's type name."""
    p = parser()
    source = path.read_bytes()
    tree = p.parse(source)
    out: dict[str, dict[str, Function]] = {}
    for node in walk(tree.root_node):
        if node.type != "impl_item":
            continue
        ty_node = node.child_by_field_name("type")
        body = node.child_by_field_name("body")
        if ty_node is None or body is None:
            continue
        ty = normalize_type(node_text(ty_node))
        table: dict[str, Function] = {}
        for child in body.children:
            if child.type != "function_item":
                continue
            fn = function_of(child, module_path(node))
            if fn is not None:
                table[fn.name] = fn
        if table:
            out.setdefault(ty, {}).update(table)
    return out


def read_crate(root: Path) -> CrateFacts:
    functions: dict[str, dict[str, Function]] = {}
    origins: dict[str, set[str]] = {}
    glob_super: dict[str, bool] = {}
    methods: dict[str, dict[str, Function]] = {}
    files = sorted(root.rglob("*.rs"))
    for path in files:
        file_functions, file_glob = read_functions_in(path, root)
        for module, table in file_functions.items():
            functions.setdefault(module, {}).update(table)
            for name in table:
                origins.setdefault(name, set()).add(str(path))
        glob_super.update(file_glob)
        for ty, table in read_methods_in(path).items():
            methods.setdefault(ty, {}).update(table)
    return CrateFacts(
        functions=functions,
        origins=origins,
        glob_super=glob_super,
        methods=methods,
        files=files,
        root=root,
    )


def read_glow(root: Path) -> tuple[dict[str, Function], set[str]]:
    """glow's `HasContext` methods and every constant it defines."""
    p = parser()
    lib = root / "src" / "lib.rs"
    source = lib.read_bytes()
    tree = p.parse(source)
    methods: dict[str, Function] = {}
    constants: set[str] = set()
    for node in walk(tree.root_node):
        if node.type == "trait_item":
            name_node = node.child_by_field_name("name")
            if name_node is None or node_text(name_node) != "HasContext":
                continue
            body = node.child_by_field_name("body")
            if body is None:
                continue
            for child in body.children:
                if child.type not in ("function_signature_item", "function_item"):
                    continue
                fn = function_of(child, "glow::HasContext")
                if fn is not None:
                    methods[fn.name] = fn
        elif node.type in ("const_item", "static_item"):
            name_node = node.child_by_field_name("name")
            if name_node is not None:
                constants.add(node_text(name_node))
    native = root / "src" / "native.rs"
    if native.is_file():
        tree = p.parse(native.read_bytes())
        for node in walk(tree.root_node):
            if node.type in ("const_item", "static_item"):
                name_node = node.child_by_field_name("name")
                if name_node is not None:
                    constants.add(node_text(name_node))
    return methods, constants


# --------------------------------------------------------------------------------------------
# One file's call sites
# --------------------------------------------------------------------------------------------


def argument_list(node) -> list[tuple[str, str]]:
    """(source text, literal kind) for each argument, kind '' when it is not a literal."""
    args_node = node.child_by_field_name("arguments")
    if args_node is None:
        return []
    out: list[tuple[str, str]] = []
    for child in args_node.children:
        if not child.is_named or child.type.endswith("comment"):
            continue
        kind = LITERAL_KINDS.get(child.type, "")
        if not kind and child.type == "unary_expression" and child.children:
            kind = LITERAL_KINDS.get(child.children[-1].type, "")
        if not kind and child.type == "reference_expression":
            value = child.child_by_field_name("value")
            if value is not None and value.type == "string_literal":
                kind = "&str"
        out.append((node_text(child).strip(), kind))
    return out


def check_callee(fn: Function, args: list[tuple[str, str]]) -> str | None:
    """An arity or literal-kind complaint about one call, or None."""
    required = [p for p in fn.params if not p.optional]
    variadic = any(p.variadic for p in fn.params)
    exact = len(required) == len(fn.params)
    if len(args) < len(required):
        if exact:
            return f"{fn.name} takes {len(fn.params)} argument(s), the call passes {len(args)}"
        return (
            f"{fn.name} takes at least {len(required)} argument(s), the call passes {len(args)}"
        )
    if len(args) > len(fn.params) and not variadic:
        return f"{fn.name} takes {len(fn.params)} argument(s), the call passes {len(args)}"
    for param, (_, kind) in zip(fn.params, args):
        if not kind or param.optional:
            continue
        allowed = PLAIN_PARAMS.get(param.ty)
        if allowed is not None and kind not in allowed:
            return f"{fn.name} expects {param.ty} here, the call passes {kind}"
    return None


def typed_locals(function) -> dict[str, str]:
    """Typed bindings of one function: its parameters, and `let` lines the checker can type."""
    out: dict[str, str] = {}
    params = function.child_by_field_name("parameters")
    if params is not None:
        for node in params.children:
            if node.type != "parameter":
                continue
            name_node = node.child_by_field_name("pattern")
            ty_node = node.child_by_field_name("type")
            if name_node is not None and name_node.type == "identifier" and ty_node is not None:
                out[node_text(name_node)] = normalize_type(node_text(ty_node))
    body = function.child_by_field_name("body")
    if body is None:
        return out
    for node in walk(body):
        if node.type != "let_declaration":
            continue
        pattern = node.child_by_field_name("pattern")
        if pattern is None or pattern.type != "identifier":
            continue
        name = node_text(pattern)
        ty_node = node.child_by_field_name("type")
        if ty_node is not None:
            out[name] = normalize_type(node_text(ty_node))
            continue
        value = node.child_by_field_name("value")
        if value is not None and value.type == "call_expression":
            fn_node = value.child_by_field_name("function")
            if fn_node is not None and fn_node.type == "field_expression":
                method = node_text(fn_node.child_by_field_name("field"))
                if method == "gl":
                    out[name] = GLOW_CONTEXT
    return out


def infer_type(node, locals_: dict[str, str], facts: CrateFacts) -> str:
    """The type of an expression, as far as the checker needs it."""
    if node is None:
        return ""
    if node.type == "identifier":
        return locals_.get(node_text(node), "")
    if node.type == "reference_expression":
        return infer_type(node.child_by_field_name("value"), locals_, facts)
    if node.type == "unary_expression" and node.children:
        return infer_type(node.children[-1], locals_, facts)
    if node.type == "parenthesized_expression" and node.named_children:
        return infer_type(node.named_children[0], locals_, facts)
    if node.type == "self":
        return ""
    return ""


def check_file(path: Path, facts: CrateFacts, glow: dict[str, Function], constants: set[str]):
    p = parser()
    source = path.read_bytes()
    tree = p.parse(source)
    findings: list[Finding] = []
    rel = str(path)

    def add(node, message):
        findings.append(
            Finding(
                path=rel,
                line=node.start_point[0] + 1,
                column=node.start_point[1] + 1,
                message=message,
            )
        )

    file_mod = file_module(facts.root, path)
    for function in walk(tree.root_node):
        if function.type != "function_item":
            continue
        inner = module_path(function)
        module = file_mod if not inner else file_mod + inner[len("crate"):]
        locals_ = typed_locals(function)
        for node in walk(function):
            if node.type != "call_expression":
                continue
            fn_node = node.child_by_field_name("function")
            if fn_node is None:
                continue
            args = argument_list(node)
            if fn_node.type == "identifier":
                name = node_text(fn_node)
                target = resolve_free(name, module, rel, facts)
                if target is not None:
                    problem = check_callee(target, args)
                    if problem:
                        add(node, problem)
            elif fn_node.type == "field_expression":
                field = fn_node.child_by_field_name("field")
                if field is None:
                    continue
                method = node_text(field)
                receiver = infer_type(
                    fn_node.child_by_field_name("value"), locals_, facts
                )
                if receiver == GLOW_CONTEXT:
                    if method not in glow:
                        add(node, f"glow's HasContext has no `{method}` method")
                        continue
                    problem = check_callee(glow[method], args)
                    if problem:
                        add(node, problem)
                elif receiver in facts.methods and method in facts.methods[receiver]:
                    problem = check_callee(facts.methods[receiver][method], args)
                    if problem:
                        add(node, problem)
                # Any other receiver: the checker cannot name the type, so it says nothing.
            elif fn_node.type == "scoped_identifier":
                raw = node_text(fn_node)
                if raw.startswith("glow::"):
                    name = raw.split("::")[-1]
                    if is_constant_name(name) and name not in constants:
                        add(node, f"glow::{name} is not a constant glow defines")

    for node in walk(tree.root_node):
        if node.type != "scoped_identifier":
            continue
        raw = node_text(node)
        if not raw.startswith("glow::"):
            continue
        name = raw.split("::")[-1]
        if is_constant_name(name) and name not in constants:
            add(node, f"glow::{name} is not a constant glow defines")
    return findings


def is_constant_name(name: str) -> bool:
    """`FRAMEBUFFER_COMPLETE` is a constant; `HasContext`, `Slice`, `Program` are types."""
    return bool(name) and name[0].isalpha() and name.upper() == name


def resolve_free(
    name: str, module: str, file: str, facts: CrateFacts
) -> Function | None:
    """The free function `name` means at this module, if the checker can tell.

    Three rules, in order, each of them something rustc does too:
      1. a function in this module wins;
      2. `use super::*` (a test module, essentially always) reaches the parent's functions,
         but only in this same file: that is what `super` means;
      3. `find`, `check` and `replace` exist in several modules at once, so a name that only
         resolves through a glob is checked against the definitions in the file it came from.
    """
    table = facts.functions.get(module)
    if table and name in table:
        return table[name]
    if not facts.glob_super.get(module):
        return None
    parts = module.split("::")
    while len(parts) > 1:
        parts.pop()
        parent = "::".join(parts)
        table = facts.functions.get(parent)
        if table and name in table:
            if table[name].module and file in facts.origins.get(name, set()):
                return table[name]
            return None
    return None


# --------------------------------------------------------------------------------------------
# Entry point
# --------------------------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        prog="gl21_typecheck",
        description="Check the GL 2.1 crate's call sites against glow and against itself.",
    )
    ap.add_argument("--glow", type=Path, help="a glow checkout: contains src/lib.rs")
    ap.add_argument(
        "--glow-cargo",
        action="store_true",
        help="take glow's sources from the registry copy Cargo.lock pins (needs `cargo metadata`)",
    )
    ap.add_argument("--crate", type=Path, default=CRATE, help="the crate to check")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--verbose", action="store_true", help="report what was read")
    ap.add_argument(
        "--max-findings", type=int, default=0, help="0 means any finding fails the run"
    )
    ap.add_argument("--self-test", action="store_true", help="check the checker")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    if args.glow_cargo:
        found = glow_from_cargo()
        if found is None:
            print(
                "gl21_typecheck: --glow-cargo could not find glow's sources; run `cargo fetch`\n"
                "  first, or pass --glow <a checkout>",
                file=sys.stderr,
            )
            return 2
        args.glow = found
    if args.glow is None:
        print(
            "gl21_typecheck: --glow or --glow-cargo is required\n"
            "  --glow-cargo uses the registry copy Cargo.lock pins; tools/gl21_preflight.sh\n"
            "  --glow fetches one through a mirror; --self-test needs neither",
            file=sys.stderr,
        )
        return 2
    if not (args.glow / "src" / "lib.rs").is_file():
        print(f"gl21_typecheck: no glow sources at {args.glow / 'src' / 'lib.rs'}", file=sys.stderr)
        return 2
    if not args.crate.is_dir():
        print(f"gl21_typecheck: no crate at {args.crate}", file=sys.stderr)
        return 2

    glow_methods, constants = read_glow(args.glow)
    facts = read_crate(args.crate)
    findings: list[Finding] = []
    for path in facts.files:
        findings.extend(check_file(path, facts, glow_methods, constants))

    if args.verbose:
        print(
            f"read {len(glow_methods)} HasContext methods, {len(constants)} glow constants, "
            f"{sum(len(t) for t in facts.functions.values())} free functions and "
            f"{sum(len(t) for t in facts.methods.values())} methods in {len(facts.files)} file(s)"
        )
    if args.json:
        print(json.dumps([f.as_dict() for f in findings], indent=2))
    else:
        for f in findings:
            print(f"{f.path}:{f.line}:{f.column}: {f.message}")
        if not findings:
            print(f"gl21_typecheck: {len(facts.files)} file(s), 0 findings")
    allowed = args.max_findings
    return 0 if len(findings) <= allowed else 1


def glow_from_cargo() -> Path | None:
    """glow's source directory as Cargo resolved it: the exact version Cargo.lock pins.

    No network: `cargo metadata` answers from the registry cache that `cargo fetch`, or any
    earlier build, already filled. That is why this is the mode CI uses: it cannot drift from
    the version the crate is compiled against.
    """
    import json
    import subprocess

    # `--offline` first: CI has run `cargo fetch`, so every source is in the cache and the
    # metadata comes from it without touching the network. The online attempt is the fallback
    # for a developer who has not fetched yet, not the other way round.
    raw = None
    for extra in (["--offline"], []):
        try:
            raw = subprocess.check_output(
                ["cargo", "metadata", "--format-version", "1", *extra],
                stderr=subprocess.DEVNULL,
                timeout=300,
            )
            break
        except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
            continue
    if raw is None:
        print(
            "gl21_typecheck: cargo metadata failed (is cargo installed, and has cargo fetch run?)",
            file=sys.stderr,
        )
        return None
    try:
        metadata = json.loads(raw)
    except json.JSONDecodeError as error:
        print(f"gl21_typecheck: cargo metadata is not JSON: {error}", file=sys.stderr)
        return None
    for package in metadata.get("packages", []):
        if package.get("name") != "glow":
            continue
        manifest = Path(package.get("manifest_path", ""))
        if manifest.is_file():
            return manifest.parent
    return None


def self_test() -> int:
    """Fixtures the checker must and must not report on."""
    import tempfile

    glow_src = """
pub trait HasContext {
    unsafe fn enable(&self, param: u32);
    unsafe fn clear(&self, mask: u32);
    unsafe fn clear_color(&self, r: f32, g: f32, b: f32, a: f32);
}
pub const DEPTH_TEST: u32 = 0x0B71;
pub const COLOR_BUFFER_BIT: u32 = 0x4000;
"""
    own_src = """
pub struct Thing { n: u32 }
impl Thing {
    pub fn two(&self, a: u32, b: &str) -> u32 { a }
    pub fn one(&mut self) {}
}
fn takes_two(a: u32, b: &str) -> u32 { a }
fn takes_one(a: bool) {}

pub mod good {
    use super::*;
    fn good(gl: &glow::Context, t: &Thing) {
        gl.enable(glow::DEPTH_TEST);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        takes_two(3, "x");
        takes_one(true);
        t.two(1, "y");
    }
}

pub mod bad {
    use super::*;
    fn bad(gl: &glow::Context) {
        gl.disable(glow::DEPTH_TEST);
        gl.clear_color(0.0, 0.0, 0.0);
        gl.enable(glow::NOT_A_CONSTANT);
        takes_two(3);
        takes_two(3, "x", "y");
        takes_one(3);
    }
}

pub mod outer {
    fn takes_two(a: u32, b: &str) -> u32 { a }
    pub mod tests {
        use super::*;
        fn a_child_module_sees_its_parents_functions() {
            takes_two(1, "ok");
        }
    }
}
"""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        glow = root / "glow"
        (glow / "src").mkdir(parents=True)
        (glow / "src" / "lib.rs").write_text(glow_src)
        crate = root / "crate" / "src"
        crate.mkdir(parents=True)
        (crate / "lib.rs").write_text(own_src)

        glow_methods, constants = read_glow(glow)
        facts = read_crate(root / "crate")
        # One file, so which function is which module's is the module path's job, not the
        # file's: `good` and `bad` are submodules of the file's own module.
        found = check_file(crate / "lib.rs", facts, glow_methods, constants)

    markers = {}
    for name in ("pub mod good", "pub mod bad", "pub mod outer"):
        offset = own_src.index(name)
        markers[name.split()[-1]] = own_src[:offset].count("\n") + 1

    def module_of(finding) -> str:
        if finding.line >= markers["outer"]:
            return "outer"
        if finding.line >= markers["bad"]:
            return "bad"
        if finding.line >= markers["good"]:
            return "good"
        return "root"

    good = [f for f in found if module_of(f) == "good"]
    bad = [f for f in found if module_of(f) == "bad"]
    scoped = [f for f in found if module_of(f) == "outer"]

    problems = []
    if len(found) < 6:
        problems.append(f"only {len(found)} finding(s) in the fixture file: {[ (f.line, f.message) for f in found ]}")
    if good:
        problems.append(f"the good fixture reported {len(good)}: {[f.message for f in good]}")
    if scoped:
        problems.append(f"a module-scoped call was misattributed: {[f.message for f in scoped]}")
    messages = " | ".join(f.message for f in bad)
    expected = [
        "glow's HasContext has no `disable` method",
        "clear_color takes 4 argument(s), the call passes 3",
        "glow::NOT_A_CONSTANT is not a constant glow defines",
        "takes_two takes 2 argument(s), the call passes 1",
        "takes_two takes 2 argument(s), the call passes 3",
        "takes_one expects bool here, the call passes integer",
    ]
    for want in expected:
        if want not in messages:
            problems.append(f"the bad fixture should have reported `{want}`\n  got: {messages}")
    if len(bad) != len(expected):
        problems.append(f"the bad fixture reported {len(bad)} findings, expected {len(expected)}")
    if problems:
        for problem in problems:
            print(f"self-test: {problem}", file=sys.stderr)
        return 1
    print(f"self-test: 4 fixtures, {len(bad)} finding(s) as expected, 0 false positives")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
