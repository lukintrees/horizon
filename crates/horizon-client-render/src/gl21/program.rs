//! The shaders the GL 2.1 path draws with, and the tables the driver binds them by.
//!
//! `docs/opengl21-plan.md` §8 names a risk in its own plan: the shaders and the code that feeds them
//! drift apart, and the first thing to notice is a driver on a player's machine. So the pieces the
//! code is built around are written down once, here, and the tests compare them with the GLSL itself:
//!
//! - **The attribute slots**: 0 `a_pos`, 1 `a_uv`, 2 `a_light`, 3 `a_color`, with the type and offset
//!   `glVertexAttribPointer` gets from `gl21::vertex`. A driver binds a `vec3` to a slot a shader
//!   declares `vec2` without a word and the picture is wrong with no error anywhere; this table is
//!   where that is caught instead.
//! - **The uniforms**: every one a shader declares is listed with what sets it, and every one listed
//!   is declared. A uniform nothing sets reads as zero, which turns a fog colour or a fade black.
//!
//! The sources are `include_str!`d rather than read from disk, so the check needs no paths and runs in
//! `cargo test` on a machine with no OpenGL at all.

/// One vertex attribute: the slot a shader's `attribute` is bound to, and how the buffer is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// The slot: `glBindAttribLocation`, and the number `glVertexAttribPointer` takes.
    pub location: u32,
    /// The name in the shader.
    pub name: &'static str,
    /// The component type in the vertex buffer.
    pub kind: AttributeKind,
    /// Components a vertex supplies.
    pub size: u32,
    /// Whether the driver divides an integer attribute by its type's maximum.
    pub normalized: bool,
    /// Where the attribute starts in a vertex.
    pub offset: u32,
}

/// The `gl*Pointer` type argument: the component type in the vertex buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeKind {
    Float,
    UnsignedShort,
    UnsignedByte,
}

impl AttributeKind {
    /// The GL constant, as `tools/gl21_shaders.py` and the plan's table name it.
    pub fn named(self) -> &'static str {
        match self {
            AttributeKind::Float => "GL_FLOAT",
            AttributeKind::UnsignedShort => "GL_UNSIGNED_SHORT",
            AttributeKind::UnsignedByte => "GL_UNSIGNED_BYTE",
        }
    }

    /// A component's size in bytes, so that an offset can be checked against the layout.
    pub fn bytes(self) -> u32 {
        match self {
            AttributeKind::Float => 4,
            AttributeKind::UnsignedShort => 2,
            AttributeKind::UnsignedByte => 1,
        }
    }
}

impl Attribute {
    /// The GLSL type the shader must declare for this slot: `float`, `vec2`, `vec3`, `vec4`.
    pub fn glsl(&self) -> String {
        if self.size == 1 {
            "float".to_string()
        } else {
            format!("vec{}", self.size)
        }
    }

    /// One past the attribute's last byte in a vertex.
    pub fn end(&self) -> u32 {
        self.offset + self.size * self.kind.bytes()
    }
}

/// `terrain.vert`'s attributes, in slot order: exactly `gl21::vertex::GlVertex`.
pub const TERRAIN: [Attribute; 4] = [
    Attribute {
        location: 0,
        name: "a_pos",
        kind: AttributeKind::Float,
        size: 3,
        normalized: false,
        offset: 0,
    },
    Attribute {
        location: 1,
        name: "a_uv",
        kind: AttributeKind::Float,
        size: 2,
        normalized: false,
        offset: 12,
    },
    Attribute {
        location: 2,
        name: "a_light",
        kind: AttributeKind::UnsignedShort,
        size: 2,
        normalized: true,
        offset: 20,
    },
    Attribute {
        location: 3,
        name: "a_color",
        kind: AttributeKind::UnsignedByte,
        size: 4,
        normalized: true,
        offset: 24,
    },
];

/// `present.vert`'s one attribute: the full-screen quad's corners.
pub const PRESENT: [Attribute; 1] = [Attribute {
    location: 0,
    name: "a_pos",
    kind: AttributeKind::Float,
    size: 2,
    normalized: false,
    offset: 0,
}];

/// A uniform a shader declares, and what sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Uniform {
    /// The name in GLSL, as declared.
    pub name: &'static str,
    /// The GLSL type, as declared.
    pub kind: &'static str,
    /// What sets it and how often: for the reader, not for a check.
    pub set_by: &'static str,
}

/// The uniforms of the terrain program (`terrain.vert` and `terrain.frag`).
pub const TERRAIN_UNIFORMS: [Uniform; 10] = [
    Uniform {
        name: "u_view_proj",
        kind: "mat4",
        set_by: "the camera's view-projection, once a frame",
    },
    Uniform {
        name: "u_origin",
        kind: "vec3",
        set_by: "the section's origin relative to the camera, per section",
    },
    Uniform {
        name: "u_sprite",
        kind: "vec4",
        set_by: "the sprite's atlas rectangle from `Atlas::rect`, per draw",
    },
    Uniform {
        name: "u_fade",
        kind: "float",
        set_by: "the section's visibility, 0 to 1",
    },
    Uniform {
        name: "u_atlas",
        kind: "sampler2D",
        set_by: "texture unit 0, the sprite atlas",
    },
    Uniform {
        name: "u_lightmap",
        kind: "sampler2D",
        set_by: "texture unit 1, the 16x16 lightmap",
    },
    Uniform {
        name: "u_fog_color",
        kind: "vec4",
        set_by: "the fog colour, its strength in the alpha",
    },
    Uniform {
        name: "u_fog",
        kind: "vec4",
        set_by: "the fog's two distance pairs: environmental start/end, render start/end",
    },
    Uniform {
        name: "u_bias",
        kind: "float",
        set_by: "the sampler's LOD bias, passed to `texture2D` (GL_EXT_texture_lod_bias)",
    },
    Uniform {
        name: "u_alpha_test",
        kind: "float",
        set_by: "`Pass::alpha_test`: 0.5 solid, 0.01 see-through, 0 for no test",
    },
];

/// The uniforms of the present program (`present.frag`).
pub const PRESENT_UNIFORMS: [Uniform; 1] = [Uniform {
    name: "u_frame",
    kind: "sampler2D",
    set_by: "texture unit 0, the frame Horizon drew",
}];

/// `hud.vert`'s attributes: exactly `gl21::hud::GlHudVertex` (five of the sixteen slots R500 has).
pub const HUD: [Attribute; 5] = [
    Attribute {
        location: 0,
        name: "a_pos",
        kind: AttributeKind::Float,
        size: 2,
        normalized: false,
        offset: 0,
    },
    Attribute {
        location: 1,
        name: "a_uv",
        kind: AttributeKind::Float,
        size: 2,
        normalized: false,
        offset: 8,
    },
    Attribute {
        location: 2,
        name: "a_color",
        kind: AttributeKind::UnsignedByte,
        size: 4,
        normalized: true,
        offset: 16,
    },
    Attribute {
        location: 3,
        name: "a_depth",
        kind: AttributeKind::Float,
        size: 1,
        normalized: false,
        offset: 20,
    },
    Attribute {
        location: 4,
        name: "a_glint_uv",
        kind: AttributeKind::Float,
        size: 2,
        normalized: false,
        offset: 24,
    },
];

/// The uniforms of the HUD program (`hud.vert` and `hud.frag`).
pub const HUD_UNIFORMS: [Uniform; 6] = [
    Uniform {
        name: "u_hud",
        kind: "vec4",
        set_by: "x,y: window pixels; z: `HudDraws::scale`; w: the glint texture's time",
    },
    Uniform {
        name: "u_aspect",
        kind: "vec2",
        set_by: "x: 1 or -1, the y direction of the target's origin",
    },
    Uniform {
        name: "u_atlas",
        kind: "sampler2D",
        set_by: "texture unit 0, the sprite and glyph atlas",
    },
    Uniform {
        name: "u_glint",
        kind: "sampler2D",
        set_by: "texture unit 1, the repeating glint texture",
    },
    Uniform {
        name: "u_linear",
        kind: "sampler2D",
        set_by: "texture unit 2, the atlas again with bilinear filtering (the vignette's blur)",
    },
    Uniform {
        name: "u_use_linear",
        kind: "float",
        set_by: "1 for a run whose `hud::State::linear` is set, 0 for the plain sample",
    },
];

/// The shader sources the renderer compiles: (file name, source).
pub const SOURCES: [(&str, &str); 6] = [
    ("terrain.vert", include_str!("terrain.vert")),
    ("terrain.frag", include_str!("terrain.frag")),
    ("present.vert", include_str!("present.vert")),
    ("present.frag", include_str!("present.frag")),
    ("hud.vert", include_str!("hud.vert")),
    ("hud.frag", include_str!("hud.frag")),
];

/// One stage's declarations: what a shader says it reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Declarations {
    /// (type, name) of each `attribute`, in the source's order.
    pub attributes: Vec<(String, String)>,
    /// (type, name) of each `uniform`.
    pub uniforms: Vec<(String, String)>,
}

/// `source`'s `attribute` and `uniform` declarations, with `//` comments removed.
///
/// GLSL 1.20 declares them one name at a time (`uniform vec4 u_fog;`); a list of names in one
/// declaration is the older GLSL this path does not write, and is treated as a mismatch.
pub fn declarations(source: &str) -> Declarations {
    let mut out = Declarations::default();
    for line in source.lines() {
        let code = match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        };
        let words: Vec<&str> = code.split_whitespace().collect();
        if words.len() != 3 || !words[2].ends_with(';') {
            continue;
        }
        let name = words[2].trim_end_matches(';');
        let pair = (words[1].to_string(), name.to_string());
        match words[0] {
            "attribute" => out.attributes.push(pair),
            "uniform" => out.uniforms.push(pair),
            _ => {}
        }
    }
    out
}

/// How many times `name` appears in `source` with its comments removed. A uniform declared and never
/// read is what a driver reports as an inactive uniform and what silently keeps a default.
fn mentions(source: &str, name: &str) -> usize {
    let code: String = source
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let boundary = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut count = 0;
    let mut rest = code.as_str();
    while let Some(at) = rest.find(name) {
        let after = rest[at + name.len()..].chars().next();
        if boundary(rest[..at].chars().next_back()) && boundary(after) {
            count += 1;
        }
        rest = &rest[at + name.len()..];
    }
    count
}

/// What a program's GLSL disagrees with the tables about. Empty when they agree.
///
/// Not a parser: the shaders are short and this path declares one name a line, so the mistakes worth
/// catching — a renamed attribute, an attribute of the wrong width, a uniform of the wrong type, an
/// attribute or uniform the code does not know about — are all visible in the declarations.
pub fn mismatches(
    stage: &[(&str, &str)],
    attributes: &[Attribute],
    uniforms: &[Uniform],
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut found_attributes: Vec<String> = Vec::new();
    let mut found_uniforms: Vec<String> = Vec::new();
    for (file, source) in stage {
        let found = declarations(source);
        for (kind, name) in &found.attributes {
            found_attributes.push(name.clone());
            match attributes.iter().find(|a| a.name == name) {
                None => problems.push(format!("{file}: attribute `{name}` is in no table")),
                Some(attribute) if attribute.glsl() != *kind => problems.push(format!(
                    "{file}: `{name}` is declared `{kind}`, the layout gives a {} of {} {}",
                    attribute.glsl(),
                    attribute.size,
                    attribute.kind.named()
                )),
                Some(_) => {}
            }
        }
        for (kind, name) in &found.uniforms {
            found_uniforms.push(name.clone());
            if mentions(source, name) < 2 {
                problems.push(format!(
                    "{file}: uniform `{name}` is declared and never read: a driver would report it \
                     inactive, and whatever it should set stays at zero"
                ));
            }
            match uniforms.iter().find(|u| u.name == name) {
                None => problems.push(format!("{file}: uniform `{name}` is in no table")),
                Some(uniform) if uniform.kind != *kind => problems.push(format!(
                    "{file}: `{name}` is declared `{kind}`, the table says `{}`",
                    uniform.kind
                )),
                Some(_) => {}
            }
        }
    }
    for attribute in attributes {
        if !found_attributes.iter().any(|name| name == attribute.name) {
            problems.push(format!(
                "attribute `{}` (slot {}) is declared by no shader: the slot would read zeros",
                attribute.name, attribute.location
            ));
        }
    }
    for uniform in uniforms {
        if !found_uniforms.iter().any(|name| name == uniform.name) {
            let message = format!("uniform `{}` is declared by no shader", uniform.name);
            problems.push(message);
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl21::vertex::{GlVertex, STRIDE};

    fn terrain() -> Vec<(&'static str, &'static str)> {
        vec![
            ("terrain.vert", SOURCES[0].1),
            ("terrain.frag", SOURCES[1].1),
        ]
    }

    fn present() -> Vec<(&'static str, &'static str)> {
        vec![
            ("present.vert", SOURCES[2].1),
            ("present.frag", SOURCES[3].1),
        ]
    }

    fn hud() -> Vec<(&'static str, &'static str)> {
        vec![("hud.vert", SOURCES[4].1), ("hud.frag", SOURCES[5].1)]
    }

    #[test]
    fn the_hud_tables_and_its_shaders_agree() {
        let problems = mismatches(&hud(), &HUD, &HUD_UNIFORMS);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_hud_attribute_table_is_the_hud_vertex_layout() {
        use crate::gl21::hud::{GlHudVertex, HUD_COLOR, HUD_DEPTH, HUD_GLINT_UV, HUD_STRIDE};
        assert_eq!(HUD_STRIDE, std::mem::size_of::<GlHudVertex>() as u32);
        let offsets = HUD.map(|a| a.offset);
        assert_eq!(offsets, [0, 8, HUD_COLOR, HUD_DEPTH, HUD_GLINT_UV]);
        for pair in HUD.windows(2) {
            let (before, after) = (pair[0], pair[1]);
            assert_eq!(before.location + 1, after.location, "{}", after.name);
            assert_eq!(before.end(), after.offset, "{}", after.name);
        }
        assert_eq!(HUD[HUD.len() - 1].end(), HUD_STRIDE);
    }

    #[test]
    fn the_tables_and_the_shaders_agree() {
        let problems = mismatches(&terrain(), &TERRAIN, &TERRAIN_UNIFORMS);
        assert!(problems.is_empty(), "{problems:?}");
        let problems = mismatches(&present(), &PRESENT, &PRESENT_UNIFORMS);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_attribute_table_is_the_vertex_layout() {
        // Every attribute starts where the one before it ends, the first at 0 and the last at the
        // stride: `glVertexAttribPointer` is given this table, so it has to be the struct.
        assert_eq!(STRIDE, std::mem::size_of::<GlVertex>() as u32);
        assert_eq!(TERRAIN[0].offset, 0);
        for pair in TERRAIN.windows(2) {
            let (before, after) = (pair[0], pair[1]);
            assert_eq!(before.location + 1, after.location, "{}", after.name);
            assert_eq!(before.end(), after.offset, "{}", after.name);
        }
        assert_eq!(TERRAIN[TERRAIN.len() - 1].end(), STRIDE);
        // And the fields they claim, by the offsets the vertex's own test checks.
        assert_eq!(
            TERRAIN.map(|a| a.offset),
            [
                std::mem::offset_of!(GlVertex, pos) as u32,
                std::mem::offset_of!(GlVertex, uv) as u32,
                std::mem::offset_of!(GlVertex, light) as u32,
                std::mem::offset_of!(GlVertex, color) as u32,
            ]
        );
    }

    #[test]
    fn the_shaders_declare_the_terrain_layout_and_nothing_else() {
        assert_eq!(
            declarations(SOURCES[0].1).attributes,
            vec![
                ("vec3".to_string(), "a_pos".to_string()),
                ("vec2".to_string(), "a_uv".to_string()),
                ("vec2".to_string(), "a_light".to_string()),
                ("vec4".to_string(), "a_color".to_string()),
            ]
        );
        // A fragment stage has no attributes.
        assert!(declarations(SOURCES[1].1).attributes.is_empty());
    }

    #[test]
    fn the_uniform_tables_list_exactly_what_the_shaders_declare() {
        let mut declared: Vec<String> = declarations(SOURCES[0].1)
            .uniforms
            .into_iter()
            .chain(declarations(SOURCES[1].1).uniforms)
            .map(|(_, name)| name)
            .collect();
        let mut listed: Vec<String> = TERRAIN_UNIFORMS
            .iter()
            .map(|u| u.name.to_string())
            .collect();
        declared.sort();
        listed.sort();
        assert_eq!(declared, listed);
        // The present pass's one.
        assert_eq!(
            declarations(SOURCES[3].1).uniforms,
            vec![("sampler2D".to_string(), "u_frame".to_string())]
        );
    }

    #[test]
    fn every_attribute_and_uniform_has_a_writer() {
        for uniform in TERRAIN_UNIFORMS.iter().chain(&PRESENT_UNIFORMS) {
            assert!(!uniform.set_by.is_empty(), "{}", uniform.name);
        }
        for attribute in TERRAIN.iter().chain(&PRESENT) {
            assert!(!attribute.name.is_empty());
        }
    }

    #[test]
    fn a_renamed_or_narrowed_attribute_is_caught() {
        let renamed = [("terrain.vert", "#version 120\nattribute vec3 a_position;\n")];
        let problems = mismatches(&renamed, &TERRAIN, &TERRAIN_UNIFORMS);
        let names_position = problems.iter().any(|p| p.contains("a_position"));
        let wants_pos = problems.iter().any(|p| p.contains("`a_pos`"));
        assert!(names_position, "{problems:?}");
        assert!(wants_pos, "{problems:?}");
        // Every attribute of the layout it fails to declare is its own problem, and so is every
        // uniform the stage no longer declares: the point is that none of them pass silently.
        let missing_attributes = problems
            .iter()
            .filter(|p| p.contains("declared by no shader"));
        let silent = TERRAIN.len() + TERRAIN_UNIFORMS.len();
        assert_eq!(missing_attributes.count(), silent);

        // The same slot with the wrong width: a driver would bind it and read garbage.
        let narrow = [(
            "terrain.vert",
            "#version 120\nattribute vec3 a_pos;\nattribute vec3 a_uv;\n",
        )];
        let problems = mismatches(&narrow, &TERRAIN, &TERRAIN_UNIFORMS);
        let width = problems
            .iter()
            .any(|p| p.contains("a_uv") && p.contains("vec2"));
        assert!(width, "{problems:?}");
    }

    #[test]
    fn a_uniform_nobody_sets_or_of_the_wrong_type_is_caught() {
        let extra = [("terrain.frag", "#version 120\nuniform vec3 u_fog_colour;\n")];
        let problems = mismatches(&extra, &TERRAIN, &TERRAIN_UNIFORMS);
        let named = problems.iter().any(|p| p.contains("u_fog_colour"));
        assert!(named, "{problems:?}");

        // A uniform the code uploads as a vec3 while the shader reads a vec4.
        let wrong = [("terrain.vert", "#version 120\nuniform vec4 u_origin;\n")];
        let problems = mismatches(&wrong, &TERRAIN, &TERRAIN_UNIFORMS);
        let typed = |p: &String| p.contains("u_origin") && p.contains("vec4");
        assert!(problems.iter().any(typed), "{problems:?}");
    }

    #[test]
    fn comments_about_declarations_are_not_declarations() {
        let found = declarations("// uniform vec3 u_nothing;\nuniform vec3 u_fog;\n");
        let fog = ("vec3".to_string(), "u_fog".to_string());
        assert_eq!(found.uniforms, vec![fog]);
    }
}
