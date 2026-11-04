//! The GL 2.1 budget, measured on GLSL source rather than trusted to the driver.
//!
//! A driver does not fail a shader that uses 900 uniform components; the old ATI compiler
//! either drops the excess silently (values become zero, the world turns black) or fails the
//! link with a message that does not say which limit. And llvmpipe, where this port is
//! developed, has no such limit at all, so nothing here would ever be noticed until the game
//! ran on the target.
//!
//! So the same numbers `tools/gl21_budget.py` counts for the WGSL side are counted here for
//! the GLSL side, from the sources themselves, before a program is created:
//!
//! * **512 uniform components per stage.** On a 2.1 context there are no uniform blocks;
//!   every member is a separate location, and each stage has its own 512 (the target
//!   reports `Max Uniform Vertex Components 512`). Arrays count in full: `uniform vec4
//!   lights[16]` is 64 components whether or not the shader reads them all.
//! * **44 varying floats.** The target's `Max Varying Floats`. `flat` and `smooth`
//!   qualifiers change nothing; a `vec3` costs three, a `vec4` four, an `int`/`float` one.
//! * **16 texture units, 0 in the vertex shader.** The vertex stage may not sample. This is
//!   the limit that changes the design, not just the shader.
//! * **Static loops.** GLSL 1.20 on SM 3.0 needs compile-time loop bounds; a loop whose
//!   bound comes from a uniform fails to compile on the target and works everywhere else.
//!
//! The measurement is deliberately lexical: a preprocessor pass is not needed because the
//! renderer's own GLSL keeps its declarations at the top level, which the tests pin.

use crate::caps::Limits;
use crate::error::{Error, Result};

/// The three shader stages a 2.1 context has. No geometry stage: the target has none.
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

    fn extension(self) -> &'static str {
        match self {
            Stage::Vertex => ".vert",
            Stage::Fragment => ".frag",
        }
    }
}

/// One `uniform`/`varying`/`attribute` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    pub kind: DeclKind,
    pub ty: Type,
    /// Name; `None` for an anonymous block member (never produced by this renderer, but the
    /// parser must not invent one).
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeclKind {
    Uniform,
    Varying,
    Attribute,
}

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
    /// Any sampler kind.
    Sampler(&'static str),
    /// Unknown, counted as 0 and reported so a typo does not look like a pass.
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
            // A sampler is one unit, not one component.
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
    /// Loops whose bound is not a literal or a constant (`for (int i = 0; i < count; i++)`).
    pub dynamic_loops: Vec<String>,
    /// Texture samples found in a vertex source: fatal on the target (0 vertex texture
    /// units), and legal everywhere else, so it is a budget failure, not a syntax one.
    pub vertex_texture_samples: Vec<String>,
    /// Types the parser did not know. A budget pass over unknown types is a guess, so the
    /// caller treats a non-empty list as a failure of the port, not of the shader.
    pub unknown_types: Vec<String>,
}

impl Measured {
    pub fn uniform(&self, name: &str) -> Option<&Decl> {
        self.uniforms.iter().find(|d| d.name == name)
    }
}

/// Reads the declarations out of one stage's GLSL 1.20 source.
pub fn measure(source: &str, stage: Stage) -> Measured {
    let code = strip_comments(source);
    let mut out = Measured::default();

    for statement in code.split(';') {
        let words: Vec<&str> = statement
            .split_whitespace()
            .filter(|w| !is_qualifier(w))
            .collect();
        // `uniform mat4 view_proj`, `uniform vec4 lights[MAX_LIGHTS]`, `varying vec2 uv`.
        let kind = statement_kind(statement);
        let Some(kind) = kind else { continue };
        let mut iter = words.iter().peekable();
        // Skip a precision qualifier that survived (`highp`).
        while let Some(w) = iter.peek() {
            if matches!(**w, "highp" | "mediump" | "lowp") {
                iter.next();
            } else {
                break;
            }
        }
        let Some(ty_word) = iter.next() else { continue };
        let ty = Type::parse(ty_word);
        if ty == Type::Unknown && !ty_word.starts_with("mat") && !ty_word.starts_with("vec") {
            out.unknown_types.push((*ty_word).to_string());
        }
        // Everything after the type is `name[size], name2`.
        let rest: String = iter.map(|w| *w).collect::<Vec<_>>().join(" ");
        for part in rest.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (name, count) = parse_name_and_size(part);
            let decl = Decl {
                kind,
                ty,
                name: name.clone(),
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

    for name in find_dynamic_loops(&code) {
        out.dynamic_loops.push(name);
    }
    if stage == Stage::Vertex {
        for name in ["texture2D", "texture2DLod", "texture2DProj", "textureCube", "textureGrad"] {
            if code.contains(name) {
                out.vertex_texture_samples.push(name.to_string());
            }
        }
    }
    out
}

fn is_qualifier(word: &str) -> bool {
    matches!(
        word,
        "uniform" | "varying" | "attribute" | "const" | "flat" | "smooth" | "invariant" | "centroid"
    )
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

/// `lights[16]` -> ("lights", 16); `uv` -> ("uv", 1).
fn parse_name_and_size(part: &str) -> (String, u32) {
    if let Some(open) = part.find('[') {
        let name = part[..open].trim().to_string();
        let inner = &part[open + 1..];
        let size = inner
            .split(']')
            .next()
            .unwrap_or("")
            .trim()
            .parse::<u32>()
            .unwrap_or(1);
        (name, size.max(1))
    } else {
        (part.to_string(), 1)
    }
}

/// Removes `//` and `/* */` comments without touching string literals (GLSL has none we use).
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
                // Keep newlines so line numbers in the driver's log still match.
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

/// `for (...; ... < X; ...)` loops whose bound is not a literal and `while` loops, which
/// GLSL 1.20 on SM 3.0 cannot compile when the count is data dependent.
fn find_dynamic_loops(code: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(pos) = rest.find("for") {
        // Only a real loop header follows `for`.
        let after = &rest[pos + 3..];
        let after = after.trim_start();
        let Some(header) = after.strip_prefix('(') else {
            rest = &rest[pos + 3..];
            continue;
        };
        let header = header.split(')').next().unwrap_or("");
        if !header.contains('<') {
            // `for (;;)` or `for (int i = 0; i <= n; ...)`: be conservative, treat as dynamic
            // unless the condition is missing entirely.
            if header.contains("<=") {
                found.push(header.trim().to_string());
            }
            rest = &rest[pos + 3..];
            continue;
        }
        let bound = header.split('<').nth(1).unwrap_or("").trim();
        let constant = bound
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("");
        let is_literal = constant.chars().all(|c| c.is_ascii_digit()) && !constant.is_empty();
        let is_constant_name = constant
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if !is_literal && !is_constant_name {
            found.push(header.trim().to_string());
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
                "{} components, the limit is {uniform_limit}: move arrays into a texture \
                 (docs/gl21-x1650-plan.ru.md §7)",
                measured.uniform_components
            ),
        ));
    }
    if measured.varying_floats > limits.max_varying_floats {
        return Err(Error::budget(
            format!("{} varyings", stage.name()),
            format!(
                "{} floats, the limit is {}: pack colour into one u32 and flags into one float",
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
    if stage == Stage::Vertex && !measured.vertex_texture_samples.is_empty() {
        if limits.max_vertex_texture_units == 0 {
            return Err(Error::budget(
                "vertex texture fetch",
                format!(
                    "{} in a vertex shader, and this GPU has 0 vertex texture units: \
                     compute the value on the CPU (docs/gl21-x1650-plan.ru.md §8.1)",
                    measured.vertex_texture_samples.join(", ")
                ),
            ));
        }
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
    if !measured.unknown_types.is_empty() {
        return Err(Error::budget(
            format!("{} types", stage.name()),
            format!(
                "the budget check does not know {}: teach budget::Type about it",
                measured.unknown_types.join(", ")
            ),
        ));
    }
    Ok(())
}

/// The uniforms two stages share, checked for a block (2.1 has no blocks; a name declared in
/// both stages must have the same type, which old drivers only warn about).
pub fn check_linked(vertex: &Measured, fragment: &Measured) -> Result<()> {
    for v in &vertex.uniforms {
        if let Some(f) = fragment.uniform(&v.name) {
            if f.ty != v.ty {
                return Err(Error::budget(
                    "linked uniforms",
                    format!(
                        "`{}` is {} in the vertex stage and {} in the fragment stage",
                        v.name,
                        type_name(v.ty),
                        type_name(f.ty)
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn type_name(t: Type) -> String {
    format!("{t:?}").to_lowercase()
}

impl Stage {
    /// The suffix a source file carries.
    pub fn suffix(self) -> &'static str {
        self.extension()
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
        assert_eq!(m.uniform("lights").unwrap().ty, Type::Vec4);
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
    fn a_flat_varying_still_costs_its_components() {
        let m = measure("flat varying vec3 tint;", Stage::Vertex);
        assert_eq!(m.varying_floats, 3);
        assert_eq!(m.varyings[0].name, "tint");
    }

    #[test]
    fn a_declaration_list_shares_the_type() {
        let m = measure("uniform vec2 a, b, c[4];", Stage::Fragment);
        assert_eq!(m.uniform_components, 2 + 2 + 8);
        assert_eq!(m.uniforms.len(), 3);
    }

    #[test]
    fn vertex_texture_fetch_is_a_budget_failure_not_a_syntax_one() {
        let src = "uniform sampler2D cloud; varying vec2 uv; void main() { vec4 c = texture2D(cloud, uv); }";
        let m = measure(src, Stage::Vertex);
        assert_eq!(m.vertex_texture_samples, vec!["texture2D"]);
        let err = check(&m, Stage::Vertex, &Limits::X1650).unwrap_err();
        assert!(err.to_string().contains("vertex texture"), "{err}");
        // The same source in a fragment shader is fine.
        let m = measure(src, Stage::Fragment);
        assert!(check(&m, Stage::Fragment, &Limits::X1650).is_ok());
    }

    #[test]
    fn const_bounded_loops_pass_and_data_bounded_loops_do_not() {
        let ok = "void main() { for (int i = 0; i < MAX_LIGHTS; i++) { } }";
        assert!(measure(ok, Stage::Fragment).dynamic_loops.is_empty());
        let ok = "void main() { for (int i = 0; i < 4; i++) { } }";
        assert!(measure(ok, Stage::Fragment).dynamic_loops.is_empty());

        let bad = "void main() { for (int i = 0; i < light_count; i++) { } }";
        let m = measure(bad, Stage::Fragment);
        assert_eq!(m.dynamic_loops.len(), 1);
        assert!(check(&m, Stage::Fragment, &Limits::X1650).is_err());
    }

    #[test]
    fn the_targets_budgets_refuse_what_llvmpipe_would_accept() {
        // Exactly the measured shape of terrain.wgsl's Globals (docs §7): 936 components.
        let mut src = String::from("uniform mat4 view_proj; uniform mat4 inverse_view;\n");
        src.push_str("uniform vec4 fog; uniform vec4 sky; uniform vec4 misc; uniform vec4 sunrise;\n");
        src.push_str("uniform vec4 geometry[MAX_LIGHTS];\n");
        let m = measure(&src, Stage::Vertex);
        assert!(m.uniform_components >= 512, "{}", m.uniform_components);
        assert!(check(&m, Stage::Vertex, &Limits::X1650).is_err());
        // The same source is fine on a 3.3 driver, which is why the check must run here.
        assert!(check(&m, Stage::Vertex, &Limits::GL33).is_ok());
    }

    #[test]
    fn linked_programs_must_agree_on_shared_uniforms() {
        let v = measure("uniform vec4 tint;", Stage::Vertex);
        let f = measure("uniform vec3 tint;", Stage::Fragment);
        assert!(check_linked(&v, &f).is_err());
        let f = measure("uniform vec4 tint;", Stage::Fragment);
        assert!(check_linked(&v, &f).is_ok());
    }

    #[test]
    fn an_unknown_type_is_a_failure_not_a_silent_zero() {
        let m = measure("uniform sampler2DArray sprites;", Stage::Fragment);
        assert!(m.unknown_types.contains(&"sampler2DArray".to_string()));
        assert!(check(&m, Stage::Fragment, &Limits::X1650).is_err());
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
        assert!(check(&m, Stage::Vertex, &Limits::GL33).is_err());
        // The legacy format uses five, which fits with room to spare.
        let ok = measure(
            "attribute vec3 a_position;\nattribute vec2 a_texcoord;\nattribute vec4 a_color;\n\
             attribute vec2 a_light;\nattribute vec4 a_extra;\n",
            Stage::Vertex,
        );
        assert!(check(&ok, Stage::Vertex, &Limits::X1650).is_ok());
    }

    #[test]
    fn stage_names_and_suffixes() {
        assert_eq!(Stage::Vertex.suffix(), ".vert");
        assert_eq!(Stage::Fragment.suffix(), ".frag");
    }
}
