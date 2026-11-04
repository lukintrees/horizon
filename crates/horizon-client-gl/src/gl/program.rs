//! A linked GLSL 1.20 program, with its uniforms measured before the driver sees it.
//!
//! Three things this module does that a plain "compile and link" does not:
//!
//! 1. **Budget first.** The sources are measured by [`crate::budget`] before
//!    `glCreateShader`, so a program that exceeds the target's 512 uniform components, 44
//!    varying floats, 16 samplers or 0 vertex texture units fails here with a sentence, on
//!    every driver, instead of working on llvmpipe and dropping uniforms on the target.
//! 2. **Locations resolved once.** On a 2.1 context a uniform is a loose `glUniform*` call
//!    per member, so every `glGetUniformLocation` is a string lookup in the driver: they are
//!    resolved at link time into a table, and a draw sets values by name.
//! 3. **`#version` is ours.** The source a shader file carries starts without a `#version`
//!    line; the backend prepends `#version 120` (and the strict-mode defines), so the
//!    version is never a property of a file that could drift from the context.


use glow::HasContext;

use crate::budget::{self, Measured, Stage, Type};
use crate::error::{Error, Result};
use crate::gl::Context;

/// What a program is for, in the log and in a budget failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Terrain,
    Entities,
    Hud,
    Screen,
    Sky,
    Other(&'static str),
}

impl Purpose {
    pub fn label(self) -> &'static str {
        match self {
            Purpose::Terrain => "terrain",
            Purpose::Entities => "entities",
            Purpose::Hud => "hud",
            Purpose::Screen => "screen",
            Purpose::Sky => "sky",
            Purpose::Other(name) => name,
        }
    }
}

/// The sources of one program: GLSL 1.20 without a `#version` line.
#[derive(Clone, Copy, Debug)]
pub struct Sources<'a> {
    pub vertex: &'a str,
    pub fragment: &'a str,
    pub purpose: Purpose,
}

/// Where a program's uniform went.
#[derive(Clone, Debug)]
pub struct UniformSlot {
    /// The GLSL name, as the shader declares it: what the budget pass and the driver agree on.
    pub name: String,
    /// The location, or `None` when the linker dropped the member because nothing reads it.
    /// Setting a dead uniform is a no-op, which is what makes optional features usable.
    pub location: Option<glow::UniformLocation>,
    pub ty: Type,
    /// Elements in the array, 1 for a plain uniform. Set a whole array by handing over
    /// `count` times the type's values.
    pub count: u32,
}

impl UniformSlot {
    pub fn is_live(&self) -> bool {
        self.location.is_some()
    }
}

/// A linked program and its uniform locations.
pub struct Program {
    raw: Option<glow::Program>,
    purpose: Purpose,
    /// `ordered[i]` is `UniformId(i)`: the declaration order, which is the only order two
    /// stages and every driver agree on.
    ordered: Vec<UniformSlot>,
    vertex: Measured,
    fragment: Measured,
}

impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Program")
            .field("purpose", &self.purpose)
            .field("uniforms", &self.ordered.len())
            .field("live", &self.raw.is_some())
            .finish()
    }
}

/// The attribute slots every program of this path agrees on, so one vertex format fits all.
pub mod attribs {
    /// `vec3` position in section-local blocks.
    pub const POSITION: u32 = 0;
    /// `vec2` texture coordinates inside the sprite.
    pub const TEXCOORD: u32 = 1;
    /// `vec4` tint (RGBA8, normalized).
    pub const COLOR: u32 = 2;
    /// `vec2` light (sky, block), normalized 0..1.
    pub const LIGHT: u32 = 3;
    /// `vec4` whatever the pass needs: layer and flags, or the origin.
    pub const EXTRA: u32 = 4;
}

/// The source text handed to the driver: the version line, the strict-mode defines, then the
/// file. A separate function so the version is checkable without a context.
pub fn compose_source(body: &str, stage: Stage, strict: bool) -> String {
    let mut out = String::with_capacity(body.len() + 96);
    out.push_str("#version 120\n");
    if strict {
        // Strict mode compiles as the target's compiler would. The define is also what a
        // shader tests when it would rather drop a feature than fail to compile.
        out.push_str("#define HORIZON_GL21_STRICT 1\n");
    }
    out.push_str("#define HORIZON_STAGE_");
    out.push_str(&stage.name().to_uppercase());
    out.push_str(" 1\n");
    out.push_str(body);
    out
}

impl Program {
    /// Measures both stages, then compiles and links.
    ///
    /// The measurement runs first and on purpose: a driver's link error for an over-budget
    /// program names no limit, and on the target it may not even be an error until the world
    /// turns black.
    pub fn create(ctx: &Context, sources: Sources<'_>) -> Result<Self> {
        let limits = ctx.caps().limits;
        let vertex = budget::measure(sources.vertex, Stage::Vertex);
        let fragment = budget::measure(sources.fragment, Stage::Fragment);

        let label = sources.purpose.label();
        budget::check(&vertex, Stage::Vertex, &limits).map_err(|e| named(e, label))?;
        budget::check(&fragment, Stage::Fragment, &limits).map_err(|e| named(e, label))?;
        budget::check_linked(&vertex, &fragment).map_err(|e| named(e, label))?;

        let strict = ctx.caps().strict;
        let gl = ctx.gl();
        let fs_source = compose_source(sources.fragment, Stage::Fragment, strict);
        let vs_source = compose_source(sources.vertex, Stage::Vertex, strict);

        let vs = compile(gl, Stage::Vertex, &vs_source, sources.purpose)?;
        let fs = match compile(gl, Stage::Fragment, &fs_source, sources.purpose) {
            Ok(fs) => fs,
            Err(e) => {
                unsafe { gl.delete_shader(vs) };
                return Err(e);
            }
        };

        let raw = unsafe {
            let program = gl
                .create_program()
                .map_err(|e| Error::gl("glCreateProgram", e))?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            // Attrib locations are bound explicitly: `glBindAttribLocation` before linking
            // is the only way to know them on 2.1, and knowing them once is what lets one
            // 24-byte vertex format serve every program.
            gl.bind_attrib_location(program, attribs::POSITION, "a_position");
            gl.bind_attrib_location(program, attribs::TEXCOORD, "a_texcoord");
            gl.bind_attrib_location(program, attribs::COLOR, "a_color");
            gl.bind_attrib_location(program, attribs::LIGHT, "a_light");
            gl.bind_attrib_location(program, attribs::EXTRA, "a_extra");

            gl.link_program(program);
            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);

            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(Error::Shader {
                    stage: "link",
                    log: format!("{label} program: {log}"),
                });
            }
            program
        };

        let ordered = resolve_uniforms(gl, raw, &vertex, &fragment);
        let program = Self {
            raw: Some(raw),
            purpose: sources.purpose,
            ordered,
            vertex,
            fragment,
        };
        ctx.note_program(&program);
        log::debug!(
            "linked {} program: vertex {} uniform components, fragment {}",
            program.purpose.label(),
            program.vertex.uniform_components,
            program.fragment.uniform_components
        );
        Ok(program)
    }

    /// The uniform table, in declaration order: `uniforms()[id.0]` is the slot `id` names.
    pub fn uniforms(&self) -> &[UniformSlot] {
        &self.ordered
    }

    /// One uniform by name, `None` when the source never declared it (a typo in a caller, and
    /// a cheap one to find in a test).
    pub fn uniform(&self, name: &str) -> Option<&UniformSlot> {
        self.find(name)
    }

    fn find(&self, name: &str) -> Option<&UniformSlot> {
        self.ordered.iter().find(|slot| slot.name == name)
    }

    /// The id a caller records in a draw list. Resolved once, at startup, and then an index.
    pub fn uniform_id(&self, name: &str) -> Option<crate::draw::UniformId> {
        self.ordered
            .iter()
            .position(|slot| slot.name == name)
            .map(|i| crate::draw::UniformId(i as u16))
    }

    /// How many components a stage of this program declared, for the log.
    pub fn declared_uniform_components(&self) -> (u32, u32) {
        (
            self.vertex.uniform_components,
            self.fragment.uniform_components,
        )
    }

    /// The slot a [`UniformId`](crate::draw::UniformId) refers to, if the program declares it.
    pub fn slot(&self, id: crate::draw::UniformId) -> Option<&UniformSlot> {
        self.ordered.get(usize::from(id.0))
    }

    /// A slot's location, for a setter that does not need the rest of the slot.
    pub fn location(&self, id: crate::draw::UniformId) -> Option<&glow::UniformLocation> {
        self.slot(id).and_then(|slot| slot.location.as_ref())
    }

    /// The GL name of a live program, for `glUseProgram`.
    pub fn raw_program(&self) -> Option<glow::Program> {
        self.raw
    }

    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    /// What the budget measured, for the log.
    pub fn measured(&self) -> (&Measured, &Measured) {
        (&self.vertex, &self.fragment)
    }

    /// Deletes the GL object now. Idempotent; the renderer calls it for every program it
    /// made, because a drop after the context is gone cannot call the driver.
    pub fn destroy(&mut self, ctx: &Context) {
        if let Some(raw) = self.raw.take() {
            unsafe { ctx.gl().delete_program(raw) };
            ctx.note_program_destroyed();
        }
    }
}

fn named(error: Error, purpose: &str) -> Error {
    match error {
        Error::Budget { what, detail } => {
            Error::budget(format!("{purpose} program's {what}"), detail)
        }
        other => other,
    }
}

/// The uniform table in declaration order: `ordered[i]` is the slot whose id is `i`.
///
/// Order matters twice. A `UniformId` is computed once from the declaration list, so a caller
/// that sets a uniform by position costs an array index rather than a string lookup; and the
/// declaration order is the only order two stages, two drivers and the budget pass agree on.
fn resolve_uniforms(
    gl: &glow::Context,
    program: glow::Program,
    vertex: &Measured,
    fragment: &Measured,
) -> Vec<UniformSlot> {
    let mut out: Vec<UniformSlot> = Vec::new();
    for decl in vertex.uniforms.iter().chain(fragment.uniforms.iter()) {
        if out.iter().any(|slot| slot.name == decl.name) {
            continue;
        }
        let location = unsafe { gl.get_uniform_location(program, &decl.name) };
        out.push(UniformSlot {
            name: decl.name.clone(),
            location,
            ty: decl.ty,
            count: decl.count,
        });
    }
    out
}

fn compile(
    gl: &glow::Context,
    stage: Stage,
    source: &str,
    purpose: Purpose,
) -> Result<glow::Shader> {
    let kind = match stage {
        Stage::Vertex => glow::VERTEX_SHADER,
        Stage::Fragment => glow::FRAGMENT_SHADER,
    };
    unsafe {
        let shader = gl
            .create_shader(kind)
            .map_err(|e| Error::gl("glCreateShader", e))?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(Error::Shader {
                stage: stage.name(),
                log: format!("{} program: {log}", purpose.label()),
            });
        }
        // The driver's warnings: on the target these are the only sign that a shader is
        // close to a limit its compiler does not enforce.
        let log = gl.get_shader_info_log(shader);
        if !log.trim().is_empty() {
            log::debug!("{} {} shader notes: {log}", purpose.label(), stage.name());
        }
        Ok(shader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the tests need the target's limits: the library measures and compares against a
    // `Caps` it is handed, and importing this at the top was an unused import (`-D warnings`).
    use crate::caps::Limits;

    #[test]
    fn the_source_always_carries_the_version_the_context_has() {
        let out = compose_source("void main() {}\n", Stage::Vertex, false);
        assert!(out.starts_with("#version 120\n"), "{out}");
        assert!(!out.contains("HORIZON_GL21_STRICT"));
        assert!(out.contains("#define HORIZON_STAGE_VERTEX 1"));

        let strict = compose_source("void main() {}\n", Stage::Fragment, true);
        assert!(strict.contains("#define HORIZON_GL21_STRICT 1"));
        assert!(strict.contains("HORIZON_STAGE_FRAGMENT 1"));
    }

    #[test]
    fn the_composed_source_still_measures_within_budget() {
        // The defines must not look like declarations to the budget pass: the composition adds
        // a `#version` and two `#define`s and no uniform, so the body's one is the only one.
        // This assertion was right and `budget::measure` was wrong -- it lost the first
        // declaration after a directive (see `budget::tests::a_directive_before_a_declaration...`).
        let body = "uniform vec4 tint;\nvoid main() { }\n";
        let composed = compose_source(body, Stage::Fragment, true);
        let measured = budget::measure(&composed, Stage::Fragment);
        assert_eq!(measured.uniforms.len(), 1);
        assert!(budget::check(&measured, Stage::Fragment, &Limits::X1650).is_ok());
    }

    #[test]
    fn attribute_slots_are_the_five_of_the_legacy_vertex_format() {
        let slots = [
            attribs::POSITION,
            attribs::TEXCOORD,
            attribs::COLOR,
            attribs::LIGHT,
            attribs::EXTRA,
        ];
        assert_eq!(slots, [0, 1, 2, 3, 4]);
        // `slots` holds `u32` by value, so `copied()` rather than a dereference of `max()`'s
        // `&u32`. The slot-limit check proper is in gl/buffer.rs, against the real format.
        assert!(slots.iter().copied().max().unwrap() < Limits::X1650.max_vertex_attribs);
    }

    #[test]
    fn purpose_labels_reach_the_log() {
        assert_eq!(Purpose::Terrain.label(), "terrain");
        assert_eq!(Purpose::Other("menu blur").label(), "menu blur");
    }
}
