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
from dataclasses import dataclass, field
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
            elif child.type == "self_parameter":
                params.append(Param(ty="self"))
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
class Shape:
    """A struct's or enum variant's fields, for checking a literal against its definition."""

    fields: dict[str, str]
    tuple_fields: int = 0
    # Associated constants declared inside the type's `impl` blocks (`Feature::ALL`,
    # `Limits::X1650`): a path in a value position may name one.
    constants: set[str] = field(default_factory=set)


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
    # type name -> its fields
    structs: dict[str, Shape]
    # enum name -> variant name -> its fields (empty fields for a unit variant)
    enums: dict[str, dict[str, Shape]]
    # type name -> constants declared in its `impl` blocks (`Feature::ALL`, `Limits::X1650`)
    constants: dict[str, set[str]]


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


# The methods a `#[derive(Trait)]` adds: enough to know `Type::default()` exists.
DERIVED_METHODS = {
    "Default": "default",
    "Clone": "clone",
    "Copy": "clone",
    "Debug": "fmt",
    "PartialEq": "eq",
    "Eq": "eq",
    "Hash": "hash",
    "PartialOrd": "partial_cmp",
    "Ord": "cmp",
}


def derived_methods(node) -> list[str]:
    """The methods a node's `#[derive(..)]` attribute adds."""
    out: list[str] = []
    previous = node.prev_named_sibling
    while previous is not None and previous.type == "attribute_item":
        raw = node_text(previous)
        if "derive" in raw:
            inside = raw[raw.find("(") + 1 : raw.rfind(")")]
            for name in inside.split(","):
                name = name.strip()
                if name in DERIVED_METHODS:
                    out.append(DERIVED_METHODS[name])
        previous = previous.prev_named_sibling
    return out


def read_derives_in(path: Path) -> dict[str, list[str]]:
    """Type name -> the methods its `#[derive(..)]` adds, read where the derive is written."""
    p = parser()
    source = path.read_bytes()
    tree = p.parse(source)
    out: dict[str, list[str]] = {}
    for node in walk(tree.root_node):
        if node.type not in ("struct_item", "enum_item"):
            continue
        name_node = node.child_by_field_name("name")
        if name_node is None:
            continue
        names = derived_methods(node)
        if names:
            out[node_text(name_node)] = names
    return out


def read_shapes_in(path: Path) -> tuple[dict[str, Shape], dict[str, dict[str, Shape]]]:
    """Structs and enums of one file, with their fields and variants."""
    p = parser()
    source = path.read_bytes()
    tree = p.parse(source)
    structs: dict[str, Shape] = {}
    enums: dict[str, dict[str, Shape]] = {}
    for node in walk(tree.root_node):
        if node.type == "struct_item":
            name_node = node.child_by_field_name("name")
            body = node.child_by_field_name("body")
            if name_node is None:
                continue
            shape = Shape(fields={})
            if body is None:
                # `struct Unit;` has no fields at all, which a literal cannot fill.
                pass
            elif body.type == "field_declaration_list":
                for child in body.children:
                    if child.type != "field_declaration":
                        continue
                    fname = child.child_by_field_name("name")
                    fty = child.child_by_field_name("type")
                    if fname is not None:
                        shape.fields[node_text(fname)] = (
                            normalize_type(node_text(fty)) if fty is not None else ""
                        )
            elif body.type == "ordered_field_declaration_list":
                shape.tuple_fields = len([c for c in body.children if c.is_named])
            structs[node_text(name_node)] = shape
        elif node.type == "enum_item":
            name_node = node.child_by_field_name("name")
            body = node.child_by_field_name("body")
            if name_node is None or body is None:
                continue
            variants: dict[str, Shape] = {}
            for child in body.children:
                if child.type != "enum_variant":
                    continue
                vname = child.child_by_field_name("name")
                if vname is None:
                    continue
                shape = Shape(fields={})
                # A variant's payload is an unnamed child of the variant node.
                payload = next(
                    (
                        c
                        for c in child.children
                        if c.type
                        in ("field_declaration_list", "ordered_field_declaration_list")
                    ),
                    None,
                )
                if payload is not None:
                    if payload.type == "field_declaration_list":
                        for field in payload.children:
                            if field.type != "field_declaration":
                                continue
                            fname = field.child_by_field_name("name")
                            fty = field.child_by_field_name("type")
                            if fname is not None:
                                shape.fields[node_text(fname)] = (
                                    normalize_type(node_text(fty)) if fty is not None else ""
                                )
                    elif payload.type == "ordered_field_declaration_list":
                        shape.tuple_fields = len(
                            [c for c in payload.children if c.is_named]
                        )
                variants[node_text(vname)] = shape
            enums[node_text(name_node)] = variants
    return structs, enums


def read_methods_in(
    path: Path, associated_constants: dict[str, set[str]] | None = None
) -> dict[str, dict[str, Function]]:
    """Inherent `impl Type { fn a() }` methods, keyed by the receiver's type name.

    `associated_constants` is filled in as a side effect: `Type::NAME` in a value position is
    legal when NAME is one, and only the impl block says so.
    """
    if associated_constants is None:
        associated_constants = {}
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
        constants: set[str] = set()
        for child in body.children:
            if child.type == "const_item":
                name_node = child.child_by_field_name("name")
                if name_node is not None:
                    constants.add(node_text(name_node))
                continue
            if child.type != "function_item":
                continue
            fn = function_of(child, module_path(node))
            if fn is not None:
                table[fn.name] = fn
        if constants:
            associated_constants.setdefault(ty.split("::")[-1], set()).update(constants)
        if table:
            out.setdefault(ty, {}).update(table)
    return out


def read_crate(root: Path) -> CrateFacts:
    functions: dict[str, dict[str, Function]] = {}
    origins: dict[str, set[str]] = {}
    glob_super: dict[str, bool] = {}
    methods: dict[str, dict[str, Function]] = {}
    structs: dict[str, Shape] = {}
    enums: dict[str, dict[str, Shape]] = {}
    constants: dict[str, set[str]] = {}
    derived: dict[str, list[str]] = {}
    files = sorted(root.rglob("*.rs"))
    for path in files:
        file_structs, file_enums = read_shapes_in(path)
        structs.update(file_structs)
        enums.update(file_enums)
        derived.update(read_derives_in(path))
        file_functions, file_glob = read_functions_in(path, root)
        for module, table in file_functions.items():
            functions.setdefault(module, {}).update(table)
            for name in table:
                origins.setdefault(name, set()).add(str(path))
        glob_super.update(file_glob)
        for ty, table in read_methods_in(path, constants).items():
            methods.setdefault(ty, {}).update(table)
    for ty, names in derived.items():
        table = methods.setdefault(ty, {})
        for name in names:
            table.setdefault(name, Function(name=name, params=[], ret=ty, module=ty, line=0))
    for ty, names in constants.items():
        if ty in structs:
            structs[ty].constants |= names
    return CrateFacts(
        functions=functions,
        origins=origins,
        glob_super=glob_super,
        methods=methods,
        files=files,
        root=root,
        structs=structs,
        enums=enums,
        constants=constants,
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
                    # Every `gl.*` call passes the context implicitly: the trait's `&self` is
                    # not an argument at the call site.
                    fn.params = [p for p in fn.params if p.ty != "self"]
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


def closure_names(function) -> set[str]:
    """Names bound to a closure in one function: `let has = |name| ..` is not the method `has`."""
    out: set[str] = set()
    body = function.child_by_field_name("body")
    if body is None:
        return out
    for node in walk(body):
        if node.type != "let_declaration":
            continue
        value = node.child_by_field_name("value")
        pattern = node.child_by_field_name("pattern")
        if value is None or pattern is None or pattern.type != "identifier":
            continue
        if value.type == "closure_expression":
            out.add(node_text(pattern))
    return out


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
        shadows = closure_names(function)
        for node in walk(function):
            if node.type != "call_expression":
                continue
            fn_node = node.child_by_field_name("function")
            if fn_node is None:
                continue
            args = argument_list(node)
            if fn_node.type == "identifier":
                name = node_text(fn_node)
                target = None if name in shadows else resolve_free(name, module, rel, facts)
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
                else:
                    table = find_method_table(facts.methods, receiver) if receiver else None
                    if table is not None and method in table:
                        # `value.method(..)`: the receiver is `self`, passed implicitly.
                        fn = table[method]
                        without_self = Function(
                            name=fn.name,
                            params=[p for p in fn.params if p.ty != "self"],
                            ret=fn.ret,
                            module=fn.module,
                            line=fn.line,
                        )
                        problem = check_callee(without_self, args)
                        if problem:
                            add(node, problem)
                # Any other receiver: the checker cannot name the type, so it says nothing.
            elif fn_node.type == "scoped_identifier":
                complain_about_glow_constant(node, fn_node, constants, add)

    # A struct literal: every field must exist on the struct, no field may repeat, and every
    # field the struct declares must be given (or the whole thing came from `..rest`).
    for node in walk(tree.root_node):
        if node.type != "struct_expression":
            continue
        name_node = node.child_by_field_name("name")
        body = node.child_by_field_name("body")
        if name_node is None or body is None:
            continue
        name = normalize_type(node_text(name_node)).split("::")[-1]
        shape = facts.structs.get(name)
        if shape is None:
            continue
        seen: list[str] = []
        spread = False
        for child in body.children:
            if child.type == "shorthand_field_initializer":
                seen.append(node_text(child))
            elif child.type in ("field_initializer", "field_initializer_shorthand"):
                field = child.child_by_field_name("field")
                if field is not None:
                    seen.append(node_text(field))
            elif child.type == "base_field_initializer":
                spread = True
        unknown = [field for field in seen if field not in shape.fields]
        for field in unknown:
            add(
                node,
                f"{name} has no field `{field}` (its fields are "
                + ", ".join(sorted(shape.fields))
                + ")",
            )
        if not spread and not unknown:
            missing = [f for f in shape.fields if f not in seen]
            if missing:
                add(
                    node,
                    f"{name} literal is missing " + ", ".join(f"`{m}`" for m in sorted(missing)),
                )

    # `Type::method(...)` and `Type::Variant(...)`: a path the checker can resolve by name.
    for node in walk(tree.root_node):
        if node.type != "call_expression":
            continue
        fn_node = node.child_by_field_name("function")
        if fn_node is None or fn_node.type != "scoped_identifier":
            continue
        raw = node_text(fn_node)
        if raw.startswith("glow::"):
            continue
        parts = raw.split("::")
        if len(parts) != 2:
            continue
        type_name, member = parts
        args = argument_list(node)
        table = find_method_table(facts.methods, type_name)
        if table is not None and member not in table and member[:1].islower():
            known = ", ".join(sorted(table))
            add(node, f"{type_name} has no method `{member}` (it has {known})")
            continue
        if table is not None and member in table:
            # `Type::method(value, ..)` is UFCS: the receiver is the first argument, and it is
            # there in the call, so the full parameter list applies.
            problem = check_callee(table[member], args)
            if problem:
                add(node, f"{type_name}::{problem}")
            continue
        variants = find_variants(facts.enums, type_name)
        if variants is None:
            continue
        if member not in variants:
            add(
                node,
                f"{type_name} has no variant `{member}` (it has "
                + ", ".join(sorted(variants))
                + ")",
            )
        elif variants[member].tuple_fields and len(args) != variants[member].tuple_fields:
            add(
                node,
                f"{type_name}::{member} takes {variants[member].tuple_fields} value(s), "
                f"the call passes {len(args)}",
            )

    # `Type::member` in a value position (a unit variant, an associated constant).
    for node in walk(tree.root_node):
        if node.type != "scoped_identifier":
            continue
        raw = node_text(node)
        if raw.startswith("glow::"):
            continue
        parts = raw.split("::")
        if len(parts) != 2:
            continue
        type_name, member = parts
        # A unit variant or an associated constant may stand alone as a value; nothing else
        # may. Only names that are supposed to be one of those are checked, so a module-level
        # `Type::thing` path cannot be reported by mistake.
        if not member[:1].isupper():
            continue
        variants = find_variants(facts.enums, type_name)
        associated = facts.constants.get(type_name, set())
        if variants is None and not associated:
            # A type this crate does not define as an enum and whose impl declares no
            # constants: only an associated function could be here, which the call check
            # covers. Nothing to say about a value position.
            continue
        variants = variants or {}
        if member not in variants and member not in associated:
            # A call's own path was already checked as a method; this pass would say "no
            # variant" about a method name, so skip a path that is a call's callee.
            if node.parent is not None and node.parent.type == "call_expression":
                continue
            names = sorted(set(variants) | associated)
            add(
                node,
                f"{type_name} has no variant or constant `{member}` (it has "
                + ", ".join(names)
                + ")",
            )

    for node in walk(tree.root_node):
        if node.type != "scoped_identifier":
            continue
        complain_about_glow_constant(node, node, constants, add)
    return findings


def complain_about_glow_constant(node, path_node, glow_constants: set[str], add) -> None:
    """`glow::NAME`: NAME must be a constant glow defines.

    The parameter is named `glow_constants` on purpose: a local called `constants` earlier in
    `check_file` once shadowed the set of glow's constants and turned 34 legal calls into
    findings, which is exactly the kind of false positive that makes a checker useless.
    """
    raw = node_text(path_node)
    if not raw.startswith("glow::"):
        return
    name = raw.split("::")[-1]
    if is_constant_name(name) and name not in glow_constants:
        add(node, f"glow::{name} is not a constant glow defines")


def find_method_table(
    methods: dict[str, dict[str, Function]], type_name: str
) -> dict[str, Function] | None:
    """The method table for `type_name`, however the `impl` spelled it.

    `impl Executor` keys on `Executor`, `impl<T> Foo<T>` normalises to `Foo`, and a call may
    write `crate::gl::Target::create`: the last segment is what all three agree on.
    """
    if type_name in methods:
        return methods[type_name]
    for key, table in methods.items():
        if key.split("::")[-1] == type_name.split("::")[-1]:
            return table
    return None


def find_variants(
    enums: dict[str, dict[str, Shape]], type_name: str
) -> dict[str, Shape] | None:
    """The variants of `type_name`: `find_method_table`'s rule, for enums."""
    short = type_name.split("::")[-1]
    if short in enums:
        return enums[short]
    return None


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
pub struct Thing {
    pub n: u32,
    pub name: &'static str,
}
impl Thing {
    pub fn two(&self, a: u32, b: &str) -> u32 { a }
    pub fn one(&mut self) {}
    pub const EMPTY: Thing = Thing { n: 0, name: "" };
}
fn takes_two(a: u32, b: &str) -> u32 { a }
fn takes_one(a: bool) {}

pub enum Shape2 { Point, Line(u32), Area { w: u32 } }

pub mod good {
    use super::*;
    fn good(gl: &glow::Context, t: &Thing) {
        gl.enable(glow::DEPTH_TEST);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        takes_two(3, "x");
        takes_one(true);
        t.two(1, "y");
        let _ = Thing { n: 1, name: "a" };
        let _ = Thing { n: 1, ..*t };
        let _ = Thing::EMPTY;
        let _ = Thing::two(t, 1, "z");
        let _ = Shape2::Point;
        let _ = Shape2::Line(4);
        let _ = Shape2::Area { w: 3 };
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
        let _ = Thing { n: 1, nam: "typo" };
        let _ = Thing { n: 1 };
        let _ = Shape2::Missing;
        let _ = Shape2::Line(1, 2);
        let _ = Thing::EMPTYY;
        let _ = Thing::onet(1);
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
        # calls
        "glow's HasContext has no `disable` method",
        "clear_color takes 4 argument(s), the call passes 3",
        "glow::NOT_A_CONSTANT is not a constant glow defines",
        "takes_two takes 2 argument(s), the call passes 1",
        "takes_two takes 2 argument(s), the call passes 3",
        "takes_one expects bool here, the call passes integer",
        # struct literals
        "Thing has no field `nam`",
        "Thing literal is missing `name`",
        # enums, tuple payloads and associated constants
        "Shape2 has no variant or constant `Missing`",
        "Shape2::Line takes 1 value(s), the call passes 2",
        "Thing has no variant or constant `EMPTYY`",
        "Thing has no method `onet`",
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
