//! The GL 2.1 budget, measured on GLSL source rather than trusted to the driver.
//!
//! A driver does not fail a shader that uses 900 uniform components; the old ATI compiler
//! either drops the excess silently (values become zero, the world turns black) or fails the
//! link with a message that does not say which limit. And llvmpipe, where this port is
//! developed, has no such limit at all, so nothing here would ever be noticed until the game
//! ran on the target.
//!
//! So the same numbers `tools/gl21_glsl.py` counts for the shaders that ship are counted here
//! from the sources themselves, before a program is created:
//!
//! * **512 uniform components per stage.** On a 2.1 context there are no uniform blocks;
//!   every member is a separate location, and each stage has its own 512. Arrays count in
//!   full: `uniform vec4 lights[16]` is 64 components whether or not the shader reads them.
//! * **44 varying floats** per stage.
//! * **16 samplers**, and **0 texture units in the vertex stage**.
//! * **16 vertex attributes**.
//! * **Loop bounds known at compile time**: GLSL 1.20 on SM 3.0 cannot compile a loop whose
//!   count comes from a uniform.
//!
//! The measurement is lexical, which is enough because this path's GLSL keeps its
//! declarations at the top level; the tests pin that, and `tools/gl21_glsl.py` measures the
//! same files independently so one implementation cannot hide a mistake from the other.

use crate::caps::Limits;
use crate::error::{Error, Result};

/// The two shader stages a 2.1 context has. No geometry stage: the target has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Vertex => "vertex",
            Stage::Fragment => "fragment",
        }
    }

    /// The suffix a source file of this stage carries.
    pub fn suffix(self) -> &'static str {
        self.extension()
    }

    fn extension(self) -> &'static str {
        match self {
            Stage::Vertex => ".vert",
            Stage::Fragment => ".frag",
        }
    }
}

/// A GLSL 1.20 type, for counting what it costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Float,
    Int,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    IVec2,
    IVec3,
    IVec4,
    BVec2,
    BVec3,
    BVec4,
    Mat2,
    Mat3,
    Mat4,
    Sampler2D,
    SamplerCube,
    /// Any other sampler kind.
    Sampler(&'static str),
    /// Unknown, counted as 0 and reported, so a typo does not look like a pass.
    Unknown,
}

impl Type {
    /// 4-byte components it occupies, as a driver counts them.
    pub fn components(self) -> u32 {
        match self {
            Type::Float | Type::Int | Type::Bool => 1,
            Type::Vec2 | Type::IVec2 | Type::BVec2 => 2,
            Type::Vec3 | Type::IVec3 | Type::BVec3 => 3,
            Type::Vec4 | Type::IVec4 | Type::BVec4 => 4,
            Type::Mat2 => 4,
            Type::Mat3 => 9,
            Type::Mat4 => 16,
            // A sampler occupies a unit, not a component.
            Type::Sampler2D | Type::SamplerCube | Type::Sampler(_) => 0,
            Type::Unknown => 0,
        }
    }

    pub fn is_sampler(self) -> bool {
        matches!(self, Type::Sampler2D | Type::SamplerCube | Type::Sampler(_))
    }

    pub fn parse(word: &str) -> Type {
        match word {
            "float" => Type::Float,
            "int" => Type::Int,
            "bool" => Type::Bool,
            "vec2" => Type::Vec2,
            "vec3" => Type::Vec3,
            "vec4" => Type::Vec4,
            "ivec2" => Type::IVec2,
            "ivec3" => Type::IVec3,
            "ivec4" => Type::IVec4,
            "bvec2" => Type::BVec2,
            "bvec3" => Type::BVec3,
            "bvec4" => Type::BVec4,
            "mat2" => Type::Mat2,
            "mat3" => Type::Mat3,
            "mat4" => Type::Mat4,
            "sampler2D" => Type::Sampler2D,
            "samplerCube" => Type::SamplerCube,
            "sampler1D" => Type::Sampler("sampler1D"),
            "sampler3D" => Type::Sampler("sampler3D"),
            "sampler2DShadow" => Type::Sampler("sampler2DShadow"),
            _ => Type::Unknown,
        }
    }

    /// For a message: `vec4`, not `Vec4`.
    pub fn glsl_name(self) -> &'static str {
        match self {
            Type::Float => "float",
            Type::Int => "int",
            Type::Bool => "bool",
            Type::Vec2 => "vec2",
            Type::Vec3 => "vec3",
            Type::Vec4 => "vec4",
            Type::IVec2 => "ivec2",
            Type::IVec3 => "ivec3",
            Type::IVec4 => "ivec4",
            Type::BVec2 => "bvec2",
            Type::BVec3 => "bvec3",
            Type::BVec4 => "bvec4",
            Type::Mat2 => "mat2",
            Type::Mat3 => "mat3",
            Type::Mat4 => "mat4",
            Type::Sampler2D => "sampler2D",
            Type::SamplerCube => "samplerCube",
            Type::Sampler(name) => name,
            Type::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeclKind {
    Uniform,
    Varying,
    Attribute,
}

/// One `uniform`/`varying`/`attribute` declaration, with its array count already applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    pub kind: DeclKind,
    pub ty: Type,
    pub name: String,
    /// Elements: 1, or the declared array length (which the parser cannot always see; a
    /// named constant's length counts as 1 and is reported, never silently dropped).
    pub count: u32,
}

/// What a stage's source declares, and what it costs against [`Limits`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    pub uniforms: Vec<Decl>,
    pub varyings: Vec<Decl>,
    pub attributes: Vec<Decl>,
    pub uniform_components: u32,
    pub varying_floats: u32,
    pub samplers: u32,
    /// Loops whose bound is not a literal or an ALL_CAPS constant.
    pub dynamic_loops: Vec<String>,
    /// Texture samples in a vertex source: legal elsewhere, fatal on the target.
    pub vertex_texture_samples: Vec<String>,
    /// Types the parser did not know; a budget pass over them would be a guess.
    pub unknown_types: Vec<String>,
    /// Array sizes the parser could not read because they are constant names.
    pub unknown_array_sizes: Vec<String>,
}

impl Measured {
    pub fn uniform(&self, name: &str) -> Option<&Decl> {
        self.uniforms.iter().find(|d| d.name == name)
    }
}

/// Reads the declarations out of one stage's GLSL 1.20 source.
pub fn measure(source: &str, stage: Stage) -> Measured {
    let code = strip_directives(&strip_comments(source));
    let mut out = Measured::default();

    for statement in code.split(';') {
        let words: Vec<&str> = statement
            .split_whitespace()
            .filter(|w| !is_qualifier(w) && !is_precision(w))
            .collect();
        let Some(kind) = statement_kind(statement) else {
            continue;
        };
        let mut iter = words.iter().peekable();
        // The kind word is still in `words`; step over it.
        while let Some(w) = iter.peek() {
            if **w == kind_word(kind) {
                iter.next();
            } else {
                break;
            }
        }
        let Some(ty_word) = iter.next() else { continue };
        let ty = Type::parse(ty_word);
        if ty == Type::Unknown {
            out.unknown_types.push((*ty_word).to_string());
        }
        let rest: String = iter.copied().collect::<Vec<_>>().join(" ");
        for part in rest.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (name, count, named_size) = parse_name_and_size(part);
            if named_size {
                out.unknown_array_sizes.push(part.to_string());
            }
            let decl = Decl {
                kind,
                ty,
                name,
                count,
            };
            let components = ty.components() * count;
            match kind {
                DeclKind::Uniform => {
                    out.uniform_components += components;
                    if ty.is_sampler() {
                        out.samplers += count;
                    }
                    out.uniforms.push(decl);
                }
                DeclKind::Varying => {
                    out.varying_floats += components;
                    out.varyings.push(decl);
                }
                DeclKind::Attribute => out.attributes.push(decl),
            }
        }
    }

    out.dynamic_loops = find_dynamic_loops(&code);
    if stage == Stage::Vertex {
        for name in TEXTURE_FUNCTIONS {
            if code.contains(name) {
                out.vertex_texture_samples.push(name.to_string());
            }
        }
    }
    out
}

/// Every texture fetch a 1.20 vertex shader might make.
const TEXTURE_FUNCTIONS: &[&str] = &[
    "texture2D",
    "texture2DProj",
    "texture2DLod",
    "texture2DGradARB",
    "textureCube",
    "textureCubeLod",
    "texture1D",
    "texture3D",
];

fn is_qualifier(word: &str) -> bool {
    matches!(
        word,
        "uniform"
            | "varying"
            | "attribute"
            | "const"
            | "flat"
            | "smooth"
            | "invariant"
            | "centroid"
    )
}

fn is_precision(word: &str) -> bool {
    matches!(word, "highp" | "mediump" | "lowp")
}

fn kind_word(kind: DeclKind) -> &'static str {
    match kind {
        DeclKind::Uniform => "uniform",
        DeclKind::Varying => "varying",
        DeclKind::Attribute => "attribute",
    }
}

fn statement_kind(statement: &str) -> Option<DeclKind> {
    let first = statement.split_whitespace().next()?;
    match first {
        "uniform" => Some(DeclKind::Uniform),
        "varying" => Some(DeclKind::Varying),
        "attribute" => Some(DeclKind::Attribute),
        _ => None,
    }
}

/// `lights[16]` -> ("lights", 16, false); `lights[N]` -> ("lights", 1, true);
/// `uv` -> ("uv", 1, false).
fn parse_name_and_size(part: &str) -> (String, u32, bool) {
    let Some(open) = part.find('[') else {
        return (part.to_string(), 1, false);
    };
    let name = part[..open].trim().to_string();
    let inner = part[open + 1..].split(']').next().unwrap_or("").trim();
    match inner.parse::<u32>() {
        Ok(size) => (name, size.max(1), false),
        Err(_) => (name, 1, true),
    }
}

/// Removes `//` and `/* */` comments while keeping line numbers.
/// Drops preprocessor lines, whole.
///
/// The statements are split on `;`, so a line like `#define HORIZON_STAGE_FRAGMENT 1` lands in
/// the same chunk as the declaration after it -- and [`statement_kind`] then sees `#define` as
/// the chunk's first word and skips a real `uniform vec4 tint;`. The composition this crate does
/// before compiling puts a `#version` and two `#define`s in front of every shader body, so that
/// was the *first* declaration of every shader, and the budget under-counted it in silence.
fn strip_directives(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        if line.trim_start().starts_with('#') {
            // Keep the newline, so a chunk never runs two declarations together.
            if line.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// The right-hand side of a loop condition's comparison: `i < 4` -> "4", `i <= N` -> "N".
/// Whitespace around the operator is optional, as GLSL allows.
fn comparison_bound(condition: &str) -> String {
    let Some(open) = condition.find(['<', '>']) else {
        return String::new();
    };
    let mut rest = &condition[open..];
    rest = rest.trim_start_matches(['<', '>', '=', ' ']);
    let token: String = rest
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    token
}

/// `for`/`while` loops whose count is not known at compile time.
fn find_dynamic_loops(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(pos) = rest.find("for") {
        let after = rest[pos + 3..].trim_start();
        let Some(header) = after.strip_prefix('(') else {
            // The word "for" inside a comment or an identifier: skip and keep looking.
            rest = &rest[pos + 3..];
            continue;
        };
        let header = header.split(')').next().unwrap_or("").trim();
        let condition = header.split(';').nth(1).unwrap_or("").trim();
        if !condition.is_empty() {
            let token = comparison_bound(condition);
            let literal = !token.is_empty() && token.chars().all(|c| c.is_ascii_digit());
            let constant = !token.is_empty()
                && token
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            if !literal && !constant {
                found.push(format!("for ({header})"));
            }
        }
        rest = &rest[pos + 3..];
    }
    if code.contains("while") {
        found.push("while".to_string());
    }
    found
}

/// Checks one measured stage against the limits; the error names the budget and the count.
pub fn check(measured: &Measured, stage: Stage, limits: &Limits) -> Result<()> {
    let uniform_limit = match stage {
        Stage::Vertex => limits.max_vertex_uniform_components,
        Stage::Fragment => limits.max_fragment_uniform_components,
    };
    if measured.uniform_components > uniform_limit {
        return Err(Error::budget(
            format!("{} uniforms", stage.name()),
            format!(
                "{} components, the limit is {uniform_limit}: move arrays into a texture",
                measured.uniform_components
            ),
        ));
    }
    if measured.varying_floats > limits.max_varying_floats {
        return Err(Error::budget(
            format!("{} varyings", stage.name()),
            format!(
                "{} floats, the limit is {}: pack the tint into one u32 and the flags into \
                 one float",
                measured.varying_floats, limits.max_varying_floats
            ),
        ));
    }
    if measured.samplers > limits.max_texture_units {
        return Err(Error::budget(
            format!("{} samplers", stage.name()),
            format!(
                "{} samplers, the limit is {}",
                measured.samplers, limits.max_texture_units
            ),
        ));
    }
    if stage == Stage::Vertex
        && !measured.vertex_texture_samples.is_empty()
        && limits.max_vertex_texture_units == 0
    {
        return Err(Error::budget(
            "vertex texture fetch",
            format!(
                "{} in a vertex shader, and this GPU has 0 vertex texture units: compute the \
                 value on the CPU",
                measured.vertex_texture_samples.join(", ")
            ),
        ));
    }
    if stage == Stage::Vertex && measured.attributes.len() as u32 > limits.max_vertex_attribs {
        return Err(Error::budget(
            "vertex attributes",
            format!(
                "{} attributes, the limit is {}",
                measured.attributes.len(),
                limits.max_vertex_attribs
            ),
        ));
    }
    if !measured.dynamic_loops.is_empty() {
        return Err(Error::budget(
            format!("{} loops", stage.name()),
            format!(
                "{}: GLSL 1.20 on SM 3.0 needs compile-time bounds",
                measured.dynamic_loops.join("; ")
            ),
        ));
    }
    if !measured.unknown_types.is_empty() {
        return Err(Error::budget(
            format!("{} types", stage.name()),
            format!(
                "the budget check does not know {}: teach budget::Type about it",
                measured.unknown_types.join(", ")
            ),
        ));
    }
    if !measured.unknown_array_sizes.is_empty() {
        return Err(Error::budget(
            format!("{} array sizes", stage.name()),
            format!(
                "{}: write the length as a number so the budget can count it",
                measured.unknown_array_sizes.join(", ")
            ),
        ));
    }
    Ok(())
}

/// A name declared in both stages must have the same type: old drivers only warn, then read
/// undefined values.
pub fn check_linked(vertex: &Measured, fragment: &Measured) -> Result<()> {
    for v in &vertex.uniforms {
        let Some(f) = fragment.uniform(&v.name) else { continue };
        if f.ty == v.ty && f.count == v.count {
            continue;
        }
        return Err(Error::budget(
            "linked uniforms",
            format!(
                "`{}` is {} in the vertex stage and {} in the fragment stage",
                v.name,
                describe(v),
                describe(f)
            ),
        ));
    }
    Ok(())
}

fn describe(decl: &Decl) -> String {
    if decl.count > 1 {
        format!("{}[{}]", decl.ty.glsl_name(), decl.count)
    } else {
        decl.ty.glsl_name().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_uniform_components_the_way_the_driver_does() {
        let src = r#"
            uniform mat4 view_proj;
            uniform mat4 inverse_view;
            uniform vec4 fog;
            uniform vec3 sun;
            uniform vec4 lights[16];
            uniform float alpha;
        "#;
        let m = measure(src, Stage::Vertex);
        // 16 + 16 + 4 + 3 + 64 + 1
        assert_eq!(m.uniform_components, 104);
        assert_eq!(m.uniforms.len(), 6);
        assert_eq!(m.uniform("lights").unwrap().count, 16);
    }

    #[test]
    fn samplers_are_units_not_components() {
        let src = "uniform sampler2D sprites; uniform sampler2D lightmap; uniform vec4 tint;";
        let m = measure(src, Stage::Fragment);
        assert_eq!(m.samplers, 2);
        assert_eq!(m.uniform_components, 4);
    }

    #[test]
    fn varyings_cost_their_components() {
        let src = "varying vec2 uv; varying vec4 color; varying float fog; varying mat2 m;";
        let m = measure(src, Stage::Vertex);
        assert_eq!(m.varying_floats, 2 + 4 + 1 + 4);
        assert_eq!(m.varyings.len(), 4);
    }

    #[test]
    fn comments_are_not_declarations() {
        let src = r#"
            // uniform vec4 this_is_not_real;
            /* varying vec4 neither_is_this; */
            uniform vec4 real;
        "#;
        let m = measure(src, Stage::Fragment);
        assert_eq!(m.uniforms.len(), 1);
        assert_eq!(m.uniform_components, 4);
    }

    #[test]
    fn a_precision_qualifier_does_not_confuse_the_type() {
        let m = measure("uniform highp vec4 tint;", Stage::Fragment);
        assert_eq!(m.uniforms.len(), 1);
        assert_eq!(m.uniforms[0].ty, Type::Vec4);
        assert!(m.unknown_types.is_empty());
    }

    #[test]
    fn a_declaration_list_shares_the_type() {
        let m = measure("uniform vec2 a, b, c[4];", Stage::Fragment);
        assert_eq!(m.uniform_components, 2 + 2 + 8);
        assert_eq!(m.uniforms.len(), 3);
    }

    #[test]
    fn an_array_sized_by_a_constant_name_is_reported_not_guessed() {
        let m = measure("uniform vec4 lights[MAX_LIGHTS];", Stage::Fragment);
        assert_eq!(m.unknown_array_sizes, vec!["lights[MAX_LIGHTS]"]);
        assert!(check(&m, Stage::Fragment, &Limits::X1650).is_err());
    }

    #[test]
    fn a_directive_before_a_declaration_does_not_hide_it() {
        // What the renderer compiles: directives, then the body. Before `strip_directives` the
        // statement split on `;` made `#define ... uniform vec4 tint` one chunk whose first
        // word was `#define`, so the tint was not counted at all.
        let src = "#version 120\n#define HORIZON_STAGE_FRAGMENT 1\n\
                   uniform vec4 tint;\nuniform sampler2D atlas;\nvoid main() { }\n";
        let m = measure(src, Stage::Fragment);
        let names: Vec<&str> = m.uniforms.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, vec!["tint", "atlas"]);
        // And a directive *between* two declarations does not glue them together either.
        let src = "uniform vec4 a;\n#define X 1\nuniform vec4 b;\n";
        let m = measure(src, Stage::Fragment);
        assert_eq!(m.uniforms.len(), 2);
    }

    #[test]
    fn vertex_texture_fetch_is_a_budget_failure_not_a_syntax_one() {
        let src = "uniform sampler2D cloud; varying vec2 uv; \
                   void main() { gl_Position = texture2D(cloud, uv); }";
        let m = measure(src, Stage::Vertex);
        assert_eq!(m.vertex_texture_samples, vec!["texture2D"]);
        let err = check(&m, Stage::Vertex, &Limits::X1650).unwrap_err();
        assert!(err.to_string().contains("vertex texture"), "{err}");
        // The same call in a fragment shader is fine.
        let m = measure(src, Stage::Fragment);
        assert!(check(&m, Stage::Fragment, &Limits::X1650).is_ok());
    }

    #[test]
    fn const_bounded_loops_pass_and_data_bounded_loops_do_not() {
        for ok in [
            "void main() { for (int i = 0; i < MAX_LIGHTS; i++) { } }",
            "void main() { for (int i = 0; i < 4; i++) { } }",
            "void main() { for (int i = 0; i <= 7; i++) { } }",
        ] {
            assert!(measure(ok, Stage::Fragment).dynamic_loops.is_empty(), "{ok}");
        }
        let bad = "void main() { for (int i = 0; i < light_count; i++) { } }";
        let m = measure(bad, Stage::Fragment);
        assert_eq!(m.dynamic_loops.len(), 1);
        assert!(check(&m, Stage::Fragment, &Limits::X1650).is_err());
        assert!(measure("void main() { while (true) { } }", Stage::Fragment)
            .dynamic_loops
            .contains(&"while".to_string()));
    }

    #[test]
    fn more_attributes_than_the_target_has_is_a_budget_failure() {
        let mut src = String::new();
        for i in 0..17 {
            src.push_str(&format!("attribute vec4 a{i};\n"));
        }
        let m = measure(&src, Stage::Vertex);
        assert_eq!(m.attributes.len(), 17);
        assert!(check(&m, Stage::Vertex, &Limits::X1650).is_err());
        let ok = measure(
            "attribute vec3 a_position;\nattribute vec2 a_texcoord;\nattribute vec4 a_color;\n\
             attribute vec2 a_light;\nattribute vec4 a_extra;\n",
            Stage::Vertex,
        );
        assert!(check(&ok, Stage::Vertex, &Limits::X1650).is_ok());
    }

    #[test]
    fn the_targets_budget_refuses_what_llvmpipe_would_accept() {
        // The shape of terrain.wgsl's Globals on the wgpu side: two matrices, four `vec4`s,
        // then four arrays (16, 32, 128, 32 entries of four components). 16 + 16 + 16 + 64 + 128
        // + 512 + 128 = 880 components against the target's 512 -- the comment here said 936,
        // which is 56 more than the expression below adds up to, and the assertion copied it.
        let mut src = String::from("uniform mat4 view_proj; uniform mat4 inverse_view;\n");
        src.push_str("uniform vec4 fog; uniform vec4 sky;\n");
        src.push_str("uniform vec4 misc; uniform vec4 sunrise;\n");
        src.push_str("uniform vec4 dynamic_lights[16]; uniform vec4 colored_lights[32];\n");
        src.push_str("uniform vec4 emissive_layers[128]; uniform vec4 sunrise_fan[32];\n");
        let m = measure(&src, Stage::Vertex);
        assert_eq!(m.uniform_components, 880);
        assert!(check(&m, Stage::Vertex, &Limits::X1650).is_err());
        // The same source is fine on a 3.3 driver, which is why the check has to run here.
        assert!(check(&m, Stage::Vertex, &Limits::GL33).is_ok());
    }

    #[test]
    fn linked_programs_must_agree_on_shared_uniforms() {
        let v = measure("uniform vec4 tint;", Stage::Vertex);
        assert!(check_linked(&v, &measure("uniform vec3 tint;", Stage::Fragment)).is_err());
        assert!(check_linked(&v, &measure("uniform vec4 tint;", Stage::Fragment)).is_ok());
        // Same type, different array length is still a mismatch.
        let v = measure("uniform vec4 lights[4];", Stage::Vertex);
        assert!(check_linked(&v, &measure("uniform vec4 lights[8];", Stage::Fragment)).is_err());
    }

    #[test]
    fn an_unknown_type_is_a_failure_not_a_silent_zero() {
        let m = measure("uniform sampler2DArray sprites;", Stage::Fragment);
        assert!(m.unknown_types.contains(&"sampler2DArray".to_string()));
        assert!(check(&m, Stage::Fragment, &Limits::X1650).is_err());
    }

    #[test]
    fn type_names_read_as_glsl() {
        assert_eq!(Type::Vec4.glsl_name(), "vec4");
        assert_eq!(Type::Sampler2D.glsl_name(), "sampler2D");
        assert_eq!(describe(&Decl {
            kind: DeclKind::Uniform,
            ty: Type::Vec4,
            name: "lights".into(),
            count: 16
        }), "vec4[16]");
    }

    #[test]
    fn stage_names_and_suffixes() {
        assert_eq!(Stage::Vertex.suffix(), ".vert");
        assert_eq!(Stage::Fragment.suffix(), ".frag");
        assert_eq!(Stage::Vertex.name(), "vertex");
    }
}
