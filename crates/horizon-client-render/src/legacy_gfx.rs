//! Opt-in terrain/HUD coordinator using actual game types and the native GL2
//! renderer. This is not yet a replacement for every gfx::Renderer feature:
//! complete material/sky/platform coverage is required before selecting it
//! from the normal Game startup. Unsupported GUI materials are explicit errors.
use crate::{
    entity_bridge, gui_bridge,
    hud::{HudAtlas, HudBatch},
    mesher::SectionMesh,
    paged_hud::Backing,
    render_bridge,
};
use horizon_legacy_render::{Budget, Context, Renderer};
use horizon_render_api::{
    Image, RenderBackend, TextureId,
    atlas::AtlasLayout,
    entity::{self, Kind},
    paged_gui::{self, Cache, Layout},
    scene::{Phase, SceneFrame, SectionKey, WorldScene},
};

pub mod frame;
mod screen;
mod sky;
mod weather;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub gpu: Budget,
    pub sections: usize,
    pub section_vertices: usize,
    pub gui_vertices: usize,
    pub entity_vertices: usize,
    pub glyph_patch_bytes: usize,
    pub hud_page_size: u32,
    pub hud_resident_pages: usize,
    pub hud_backing_bytes: usize,
    pub hud_page_count: usize,
    pub screen_quads: usize,
    pub lightning_bolts: usize,
    pub sky_vertices: usize,
    pub weather_quads: usize,
    pub weather_bytes: usize,
    /// Extracted sprite bytes plus one decoded virtual-atlas page.
    pub screen_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            gpu: Budget::default(),
            sections: 1024,
            section_vertices: 262144,
            gui_vertices: 262144,
            entity_vertices: 262144,
            glyph_patch_bytes: 8 << 20,
            hud_page_size: 512,
            hud_resident_pages: 8,
            hud_backing_bytes: 128 << 20,
            hud_page_count: 4096,
            screen_quads: 256,
            lightning_bolts: 64,
            sky_vertices: 32768,
            weather_quads: 4096,
            weather_bytes: 8 << 20,
            screen_bytes: 8 << 20,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GlintParameters {
    pub strength: f32,
    pub ticks: f32,
}
impl Default for GlintParameters {
    fn default() -> Self {
        Self {
            strength: 1.,
            ticks: 0.,
        }
    }
}
struct GlintTextures {
    item: TextureId,
    armor: TextureId,
    bytes: usize,
}

/// Classic CPU geometry already posed relative to the camera. Projection uses
/// conventional desktop GL depth, not the modern renderer's reversed Z.
pub struct Geometry<'a> {
    pub entities: &'a [crate::mesher::Vertex],
    pub hand: &'a [crate::mesher::Vertex],
    /// World-space name tags/text: after particles, before the first-person hand.
    pub see_through: &'a [crate::mesher::Vertex],
    pub hand_view_projection: [f32; 16],
    pub nether_lighting: bool,
    pub glint: GlintParameters,
}

/// Movable, thread-confined owner: scene -> backend -> native context. No leaked
/// contexts, lifetime transmutation or references to another field of this struct.
pub struct TerrainHudRenderer {
    world: WorldScene<Renderer<'static>>,
    hud: Backing,
    cache: Cache,
    hud_capacity: usize,
    white: TextureId,
    lightmap: Vec<u8>,
    retired_hud: Vec<Cache>,
    glint_textures: Option<GlintTextures>,
    retired_glint: Vec<TextureId>,
    limits: Limits,
    game_layout: Option<crate::chunk::ColumnLayout>,
    screen: screen::Resources,
}
fn diagnostic(error: impl std::fmt::Debug) -> String {
    format!("legacy terrain/HUD: {error:?}")
}

impl TerrainHudRenderer {
    pub fn new(
        context: Context,
        layout: AtlasLayout,
        terrain_pages: &[&[u8]],
        lightmap: &[u8],
        hud: &HudAtlas,
        limits: Limits,
    ) -> Result<Self, String> {
        let backend = Renderer::owned(context, limits.gpu)?;
        let mut world = WorldScene::new(backend, layout, terrain_pages, lightmap, limits.sections)
            .map_err(diagnostic)?;
        let layout = Layout::new(
            [hud.width, hud.height],
            limits.hud_page_size,
            limits.hud_page_count,
        )
        .map_err(diagnostic)?;
        let white = world.backend_mut().create_texture(Image {
            width: 1,
            height: 1,
            rgba: &[255; 4],
        })?;
        let remaining = world.backend_mut().remaining();
        let hud_capacity = limits
            .hud_resident_pages
            .min(remaining.textures)
            .min(remaining.texture_bytes / layout.page_bytes());
        let cache = Cache::new(layout, hud_capacity).map_err(diagnostic)?;
        let backing = Backing::new(layout, &hud.pixels(), limits.hud_backing_bytes)?;
        Ok(Self {
            world,
            hud: backing,
            cache,
            hud_capacity,
            white,
            lightmap: lightmap.to_vec(),
            retired_hud: Vec::new(),
            glint_textures: None,
            retired_glint: Vec::new(),
            limits,
            game_layout: None,
            screen: screen::Resources::default(),
        })
    }

    /// Stage dedicated repeating glint images without changing atlas pixels.
    /// Existing glint remains selected on validation/allocation failure. The
    /// bounded page cache is emptied (framebuffer colour is not cleared).
    pub fn set_glint_textures(&mut self, item: Image<'_>, armor: Image<'_>) -> Result<(), String> {
        self.collect_glint()?;
        self.collect_hud()?;
        self.cache.release(self.world.backend_mut())?;
        let staged = (|| -> Result<_, String> {
            let bytes = item
                .rgba
                .len()
                .checked_add(armor.rgba.len())
                .ok_or("glint size overflow")?;
            let item = self.world.backend_mut().create_texture(item)?;
            self.retired_glint.push(item);
            let armor = self.world.backend_mut().create_texture(armor)?;
            self.retired_glint.push(armor);
            let remaining = self.world.backend_mut().remaining();
            let old_bytes = self.glint_textures.as_ref().map_or(0, |t| t.bytes);
            let old_count = if self.glint_textures.is_some() { 2 } else { 0 };
            let capacity = self
                .limits
                .hud_resident_pages
                .min(remaining.textures + old_count)
                .min((remaining.texture_bytes + old_bytes) / self.hud.layout().page_bytes());
            let cache = Cache::new(self.hud.layout(), capacity).map_err(diagnostic)?;
            Ok((GlintTextures { item, armor, bytes }, cache, capacity))
        })();
        let (textures, cache, capacity) = match staged {
            Ok(value) => value,
            Err(error) => {
                let _ = self.collect_glint();
                return Err(error);
            }
        };
        // Both staged handles now belong to the new active owner.
        self.retired_glint.clear();
        if let Some(old) = self.glint_textures.replace(textures) {
            self.retired_glint.extend([old.item, old.armor]);
        }
        self.cache = cache;
        self.hud_capacity = capacity;
        Ok(())
    }
    fn collect_glint(&mut self) -> Result<(), String> {
        while let Some(&id) = self.retired_glint.last() {
            self.world.backend_mut().remove_texture(id)?;
            self.retired_glint.pop();
        }
        Ok(())
    }

    pub fn upload_section(&mut self, key: SectionKey, mesh: &SectionMesh) -> Result<(), String> {
        render_bridge::upload_section(&mut self.world, key, mesh, self.limits.section_vertices)
            .map_err(diagnostic)
    }
    /// Game keys use section Y relative to the dimension minimum, unlike the
    /// zero-origin low-level probes. Require the layout before accepting them.
    pub fn upload_game_section(
        &mut self,
        key: SectionKey,
        mesh: &SectionMesh,
    ) -> Result<(), String> {
        let layout = self
            .game_layout
            .ok_or("legacy terrain layout must be set before Game uploads")?;
        if usize::try_from(key[1])
            .ok()
            .is_none_or(|y| y >= layout.sections())
        {
            return Err("section Y outside the configured dimension".into());
        }
        self.upload_section(key, mesh)
    }
    pub fn has_section(&self, key: SectionKey) -> bool {
        self.world.has_section(key)
    }
    pub fn clear_sections(&mut self, layout: crate::chunk::ColumnLayout) -> Result<(), String> {
        self.world
            .reset_sections(layout.min_y())
            .map_err(diagnostic)?;
        self.game_layout = Some(layout);
        Ok(())
    }
    pub fn remove_columns(
        &mut self,
        columns: impl IntoIterator<Item = (i32, i32)>,
    ) -> Result<(), String> {
        for (x, z) in columns {
            self.world.unload_column([x, z]).map_err(diagnostic)?;
        }
        Ok(())
    }
    pub fn unload_section(&mut self, key: SectionKey) -> Result<bool, String> {
        self.world.unload_section(key).map_err(diagnostic)
    }
    pub fn update_lightmap(&mut self, rgba: &[u8]) -> Result<(), String> {
        self.world.update_lightmap(rgba).map_err(diagnostic)?;
        self.lightmap.copy_from_slice(rgba);
        Ok(())
    }
    pub fn resident_sections(&self) -> usize {
        self.world.section_count()
    }
    pub fn size(&mut self) -> [u32; 2] {
        self.world.backend_mut().context().size()
    }

    /// Convert all CPU GUI data first, so missing glint assets/unsupported portal materials do
    /// not clear the previous world image. This does not silently hide entities:
    /// callers have not yet switched the full game's gfx::Frame to this subset.
    pub fn render(&mut self, world: &SceneFrame<'_>, hud: &HudBatch) -> Result<(), String> {
        self.render_with_entities(world, hud, &[], false)
    }
    /// Classic camera-relative entity stream; missing glint assets and unsupported
    /// special materials remain explicit errors. Particles follow translucent terrain.
    pub fn render_with_entities(
        &mut self,
        world: &SceneFrame<'_>,
        hud: &HudBatch,
        entities: &[crate::mesher::Vertex],
        nether: bool,
    ) -> Result<(), String> {
        self.render_geometry(
            world,
            hud,
            &Geometry {
                entities,
                hand: &[],
                see_through: &[],
                hand_view_projection: world.view_projection,
                nether_lighting: nether,
                glint: GlintParameters::default(),
            },
        )
    }
    /// Draw the supported classic subset of the actual game frame. Unimplemented
    /// requested passes are errors, never silently hidden. Startup/resource and
    /// visibility scheduling still belong to the caller, not this coordinator.
    pub fn render_game_frame(
        &mut self,
        source: &crate::gfx::Frame<'_>,
        visible: frame::VisibleSections<'_>,
        options: frame::Options,
    ) -> Result<(), String> {
        let size = self.size();
        let prepared = frame::Prepared::new(source, size, visible, self.limits, options)
            .map_err(|error| error.to_string())?;
        if prepared.lightmap.as_slice() != self.lightmap {
            self.update_lightmap(&prepared.lightmap)?;
        }
        self.render_geometry_with_effects(
            &prepared.world,
            prepared.hud,
            &prepared.geometry(),
            prepared.screen,
            &prepared.additive,
            (prepared.weather, prepared.weather_time),
            Some(source),
        )
    }

    pub fn render_geometry(
        &mut self,
        world: &SceneFrame<'_>,
        hud: &HudBatch,
        geometry: &Geometry<'_>,
    ) -> Result<(), String> {
        self.render_geometry_with_effects(
            world,
            hud,
            geometry,
            &[],
            &[],
            (&[], crate::weather::Time::new(0, 0.)),
            None,
        )
    }
    fn render_geometry_with_effects(
        &mut self,
        world: &SceneFrame<'_>,
        hud: &HudBatch,
        geometry: &Geometry<'_>,
        screen_quads: &[crate::screen_effects::ScreenQuad],
        additive_vertices: &[crate::mesher::Vertex],
        precipitation: (&[crate::weather::Vertex], crate::weather::Time),
        sky_source: Option<&crate::gfx::Frame<'_>>,
    ) -> Result<(), String> {
        let sky = sky_source
            .map(|source| sky::Prepared::new(source, self.size(), self.limits))
            .transpose()?;
        let celestial = if let Some(sky) = &sky {
            let mut runs = entity_bridge::prepare_with(
                &sky.celestial,
                &self.lightmap,
                entity::Options {
                    nether: geometry.nether_lighting,
                    glint: None,
                },
                self.hud.layout(),
                self.limits.sky_vertices - sky.basic_vertices,
                entity_bridge::View::Additive,
            )
            .map_err(diagnostic)?;
            for run in &mut runs {
                if let horizon_render_api::Pass::Entity(ref mut m) = run.geometry.pass {
                    m.kind = Kind::SkyCelestial;
                    m.cull = false;
                }
            }
            runs
        } else {
            Vec::new()
        };
        let strength = entity::Strength::new(geometry.glint.strength).map_err(diagnostic)?;
        if !geometry.glint.ticks.is_finite() {
            return Err("invalid glint clock".into());
        }
        let options = entity::Options {
            nether: geometry.nether_lighting,
            glint: self.glint_textures.as_ref().map(|t| entity::GlintState {
                item: t.item,
                armor: t.armor,
                strength,
                ticks: geometry.glint.ticks,
            }),
        };
        if geometry
            .entities
            .len()
            .checked_add(geometry.hand.len())
            .and_then(|n| n.checked_add(geometry.see_through.len()))
            .and_then(|n| n.checked_add(additive_vertices.len()))
            .and_then(|n| n.checked_add(precipitation.0.len()))
            .is_none_or(|n| n > self.limits.entity_vertices)
        {
            return Err("entity/hand input budget exceeded".into());
        }
        if !geometry.hand.is_empty() && geometry.hand_view_projection.iter().any(|v| !v.is_finite())
        {
            return Err("invalid hand projection".into());
        }
        let geometry_limit = self.limits.entity_vertices - precipitation.0.len();
        let mut hand = entity_bridge::prepare_with(
            geometry.hand,
            &self.lightmap,
            options,
            self.hud.layout(),
            geometry_limit,
            entity_bridge::View::World,
        )
        .map_err(diagnostic)?;
        if hand
            .iter()
            .any(|r| matches!(r.geometry.pass, horizon_render_api::Pass::Entity(m) if matches!(m.kind, Kind::Particle | Kind::Shadow | Kind::WaterMask | Kind::Text)))
        {
            return Err("world-only material in hand view".into());
        }
        // The modern hand uses its ordinary entity pipeline even for item-sheet
        // quads; its near-plane bias is not the world item pipeline's default.
        for run in &mut hand {
            if let horizon_render_api::Pass::Entity(ref mut material) = run.geometry.pass {
                material.kind = Kind::Body;
            }
        }
        let remaining = geometry_limit
            - hand
                .iter()
                .map(|r| r.geometry.vertices.len())
                .sum::<usize>();
        let see_through = entity_bridge::prepare_with(
            geometry.see_through,
            &self.lightmap,
            options,
            self.hud.layout(),
            remaining,
            entity_bridge::View::SeeThrough,
        )
        .map_err(diagnostic)?;
        let remaining = remaining
            - see_through
                .iter()
                .map(|r| r.geometry.vertices.len())
                .sum::<usize>();
        let additive = entity_bridge::prepare_with(
            additive_vertices,
            &self.lightmap,
            options,
            self.hud.layout(),
            remaining,
            entity_bridge::View::Additive,
        )
        .map_err(diagnostic)?;
        let remaining = remaining
            - additive
                .iter()
                .map(|r| r.geometry.vertices.len())
                .sum::<usize>();
        let entity = entity_bridge::prepare_with(
            geometry.entities,
            &self.lightmap,
            options,
            self.hud.layout(),
            remaining,
            entity_bridge::View::World,
        )
        .map_err(diagnostic)?;
        let (mut body, particles): (Vec<_>, Vec<_>) = entity.into_iter().partition(
            |r| matches!(r.geometry.pass, horizon_render_api::Pass::Entity(m) if m.kind != Kind::Particle),
        );

        // Match the classic game's material ordering, retaining input order
        // within each kind (not sorting translucent quads by texture/page).
        body.sort_by_key(|r| match r.geometry.pass {
            horizon_render_api::Pass::Entity(m) => m.kind,
            _ => unreachable!(),
        });

        let runs = {
            let source = gui_bridge::glint_runs(
                hud,
                self.size(),
                self.limits.gui_vertices,
                self.glint_textures.as_ref().map(|t| t.item),
            )
            .map_err(diagnostic)?;
            // Drop unpaged scratch before patch staging and GPU submission.
            paged_gui::split_gui(self.hud.layout(), &source, self.limits.gui_vertices)
                .map_err(diagnostic)?
        };
        let changed = self
            .hud
            .patch(&hud.patches, self.limits.glyph_patch_bytes)?;
        self.cache.invalidate(&changed);
        let screens = screen::Prepared::new(
            screen_quads,
            &self.hud,
            self.limits.screen_quads,
            self.limits.screen_bytes,
            self.world.backend_mut().max_texture_size(),
        )?;
        let weather = weather::Prepared::new(
            precipitation.0,
            precipitation.1,
            &self.hud,
            &self.lightmap,
            self.limits.weather_quads,
            self.limits.weather_bytes,
            self.world.backend_mut().max_texture_size(),
        )?;
        self.screen.collect(self.world.backend_mut())?;
        self.collect_hud()?;
        self.world.collect_garbage().map_err(diagnostic)?;
        self.cache.collect_garbage(self.world.backend_mut())?;
        self.collect_glint()?;
        // Screen sprites and virtual GUI pages never coexist on the GPU. Empty
        // the cache before sizing, and again between the hand and overlays.
        if !screens.is_empty() || !weather.is_empty() {
            self.cache.release(self.world.backend_mut())?;
        }
        let remaining = self.world.backend_mut().remaining();
        let paged = !runs.is_empty()
            || !body.is_empty()
            || !particles.is_empty()
            || !hand.is_empty()
            || !see_through.is_empty()
            || !additive.is_empty()
            || !celestial.is_empty();
        if !screens.is_empty()
            && (remaining.textures == 0
                || remaining.meshes == 0
                || remaining.mesh_bytes < 188
                || remaining.texture_bytes
                    < screens.largest_texture().max(if paged {
                        self.hud.layout().page_bytes()
                    } else {
                        0
                    }))
        {
            return Err("no transient screen texture/mesh capacity".into());
        }
        if !weather.is_empty()
            && (remaining.textures == 0
                || remaining.meshes == 0
                || remaining.mesh_bytes < 348
                || remaining.texture_bytes
                    < weather.largest_texture().max(if paged {
                        self.hud.layout().page_bytes()
                    } else {
                        0
                    }))
        {
            return Err("no transient weather texture/mesh capacity".into());
        }
        if sky.as_ref().is_some_and(|s| {
            s.largest_mesh() > remaining.mesh_bytes
                || (s.basic_vertices > 0 && remaining.meshes == 0)
        }) {
            return Err("no transient sky mesh capacity".into());
        }
        if runs.iter().any(|run| {
            std::mem::size_of_val(run.geometry.vertices.as_slice())
                + std::mem::size_of_val(run.geometry.indices.as_slice())
                > remaining.mesh_bytes
        }) || body
            .iter()
            .chain(&particles)
            .chain(&hand)
            .chain(&see_through)
            .chain(&additive)
            .chain(&celestial)
            .any(|run| {
                std::mem::size_of_val(run.geometry.vertices.as_slice())
                    + std::mem::size_of_val(run.geometry.indices.as_slice())
                    > remaining.mesh_bytes
            })
            || ((!runs.is_empty()
                || !body.is_empty()
                || !particles.is_empty()
                || !hand.is_empty()
                || !see_through.is_empty()
                || !additive.is_empty()
                || !celestial.is_empty())
                && remaining.meshes == 0)
        {
            return Err("no transient mesh capacity for legacy frame".into());
        }
        let view = paged_gui::WorldView {
            view_projection: world.view_projection,
            camera: world.camera,
            fog: world.fog,
            lightmap: self.world.lightmap(),
        };
        self.world.validate_visible(world).map_err(diagnostic)?;
        if let Some(sky) = sky.as_ref().filter(|s| !s.is_empty()) {
            use horizon_render_api::{Clear, Frame};
            self.world.backend_mut().render(&Frame {
                view_projection: world.view_projection,
                camera: world.camera,
                clear: Clear::ColorDepth(world.clear),
                fog: world.fog,
                draws: &[],
            })?;
            self.screen.render_sky(
                self.world.backend_mut(),
                sky,
                true,
                self.white,
                world.camera,
            )?;
            let celestial_view = paged_gui::WorldView {
                view_projection: sky.matrix,
                camera: world.camera,
                fog: None,
                lightmap: view.lightmap,
            };
            let backing = &self.hud;
            self.cache
                .render_world(
                    self.world.backend_mut(),
                    &celestial,
                    &celestial_view,
                    |page| backing.load(page),
                )
                .map_err(diagnostic)?;
            self.screen.render_sky(
                self.world.backend_mut(),
                sky,
                false,
                self.white,
                world.camera,
            )?;
            self.world
                .render_phase(world, Phase::SolidLoad)
                .map_err(diagnostic)?;
        } else {
            self.world
                .render_phase(world, Phase::Solid)
                .map_err(diagnostic)?;
        }
        let backing = &self.hud;
        self.cache
            .render_world(self.world.backend_mut(), &body, &view, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)?;
        // The classic game draws lightning, then source-ordered additive runs
        // after body/water-mask, BEFORE translucent terrain and particles.
        self.cache
            .render_world(self.world.backend_mut(), &additive, &view, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)?;
        self.world
            .render_phase(world, Phase::Translucent)
            .map_err(diagnostic)?;
        self.cache
            .render_world(self.world.backend_mut(), &particles, &view, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)?;
        if !weather.is_empty() {
            self.cache.release(self.world.backend_mut())?;
            self.screen
                .render_weather(self.world.backend_mut(), &weather, &view)?;
        }
        self.cache
            .render_world(self.world.backend_mut(), &see_through, &view, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)?;
        if !hand.is_empty() {
            use horizon_render_api::{Clear, Frame};
            self.world.backend_mut().render(&Frame {
                view_projection: geometry.hand_view_projection,
                camera: world.camera,
                clear: Clear::Depth,
                fog: None,
                draws: &[],
            })?;
            let hand_view = paged_gui::WorldView {
                view_projection: geometry.hand_view_projection,
                camera: world.camera,
                fog: None,
                lightmap: view.lightmap,
            };
            self.cache
                .render_world(self.world.backend_mut(), &hand, &hand_view, |page| {
                    backing.load(page)
                })
                .map_err(diagnostic)?;
        }
        if !screens.is_empty() {
            self.cache.release(self.world.backend_mut())?;
            self.screen
                .render(self.world.backend_mut(), &screens, self.white)?;
        }
        self.cache
            .render_gui(self.world.backend_mut(), &runs, self.white, |page| {
                backing.load(page)
            })
            .map_err(diagnostic)
    }
    /// Prepare a compressed replacement before swapping. GPU pages are uploaded
    /// lazily; old resident pages are retired before the next streamed frame.
    pub fn reload_hud(&mut self, hud: &HudAtlas) -> Result<(), String> {
        self.collect_hud()?;
        let layout = Layout::new(
            [hud.width, hud.height],
            self.limits.hud_page_size,
            self.limits.hud_page_count,
        )
        .map_err(diagnostic)?;
        let backing = Backing::new(layout, &hud.pixels(), self.limits.hud_backing_bytes)?;
        let cache = Cache::new(layout, self.hud_capacity).map_err(diagnostic)?;
        self.retired_hud
            .push(std::mem::replace(&mut self.cache, cache));
        self.hud = backing;
        Ok(())
    }
    fn collect_hud(&mut self) -> Result<(), String> {
        while let Some(cache) = self.retired_hud.last_mut() {
            cache.release(self.world.backend_mut())?;
            self.retired_hud.pop();
        }
        Ok(())
    }
    pub fn set_title(&mut self, title: &str) -> Result<(), String> {
        self.world.backend_mut().context().set_title(title)
    }
    /// Pump native window events before computing view/HUD input for the frame.
    /// Zero drawable dimensions mean minimized: suspend rendering until restored.
    pub fn poll_events(&mut self) -> Result<horizon_legacy_render::WindowEvents, String> {
        self.world.backend_mut().context().poll_events()
    }
    /// Resize in-place: resident terrain, atlas pages and foil images survive.
    /// The caller must rebuild projection and HudBatch for the new dimensions.
    pub fn resize(&mut self, size: [u32; 2]) -> Result<(), String> {
        self.world.backend_mut().context().resize(size)
    }
    pub fn is_windowed(&mut self) -> bool {
        self.world.backend_mut().context().is_windowed()
    }
    pub fn present(&mut self) -> Result<bool, String> {
        self.world.backend_mut().context().present()
    }
    pub fn read_rgba(&mut self, rect: [u32; 4]) -> Result<Vec<u8>, String> {
        self.world.read_rgba(rect).map_err(diagnostic)
    }
}
impl Drop for TerrainHudRenderer {
    fn drop(&mut self) {
        let _ = self.screen.collect(self.world.backend_mut());
        if let Some(textures) = self.glint_textures.take() {
            self.retired_glint.extend([textures.item, textures.armor]);
        }
        let _ = self.collect_glint();
        let _ = self.cache.release(self.world.backend_mut());
        let _ = self.world.backend_mut().remove_texture(self.white);
        for mut cache in self.retired_hud.drain(..) {
            let _ = cache.release(self.world.backend_mut());
        }
        // WorldScene then deletes sections/pages/lightmap before its owned
        // backend/context are dropped, including any failed cleanup handles.
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;
    use horizon_render_api::{
        IDENTITY,
        atlas::{SpriteSize, build_page, pack_atlas},
    };
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_actual_sky_matches_disc_fog_then_sunrise_celestial_stars_end_and_world_depth() {
        use crate::mesher::Vertex;
        let atlas = HudAtlas::boot();
        let mut hud = HudBatch::new([64; 2]);
        hud.fill(&atlas, 0., 0., 1., 1., u32::MAX);
        let uv = hud.vertices[0].uv;
        hud.vertices.clear();
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255; 4]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout.clone(),
            &[&page],
            &[0; 1024],
            &atlas,
            Limits::default(),
        )
        .unwrap();
        let empty = frame::VisibleSections::default();
        let options = frame::Options {
            gamma: 0.,
            fullbright: true,
        };
        let fog = |color, sky_end| crate::fog::FogData {
            color,
            sky_end,
            environmental_start: 0.,
            environmental_end: 0.,
            render_distance_start: 0.,
            render_distance_end: 0.,
            cloud_end: 0.,
        };
        let mut f = frame::tests::source(&hud);
        f.skybox = true;
        f.yaw = 0.;
        f.pitch = -45.;
        f.fov_degrees = 90.;
        f.camera = glam::DVec3::new(30_000_000.25, 100.125, -30_000_000.5);
        f.sky.sky_color = [0.2, 0.4, 0.8];
        f.fog = Some(fog([0.1, 0.2, 0.3, 0.5], 128.));
        renderer.render_game_frame(&f, empty, options).unwrap();
        let base = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        // Independent terrain.wgsl analytic octagon/ray reference, not the fan builder.
        for y in [3, 17, 32, 48, 60] {
            for x in [3, 17, 32, 48, 60] {
                let (nx, ny) = (
                    (f64::from(x) + 0.5) / 32. - 1.,
                    (f64::from(y) + 0.5) / 32. - 1.,
                );
                let c = std::f64::consts::FRAC_1_SQRT_2;
                let (dx, dy, dz) = (-nx, (ny + 1.) * c, (1. - ny) * c);
                let (px, pz) = (dx * 16. / dy, dz * 16. / dy);
                let h = px.hypot(pz);
                let phi = pz.atan2(px);
                let sector = std::f64::consts::FRAC_PI_4;
                let edge = 512. * (sector * 0.5).cos()
                    / (phi - ((phi / sector).floor() * sector + sector * 0.5)).cos();
                let dist = 16. + h / edge * ((512_f64 * 512. + 256.).sqrt() - 16.);
                let cylindrical = 16. + h / edge * (512. - 16.);
                let amount =
                    (dist / 128.)
                        .clamp(0., 1.)
                        .max(if cylindrical >= 128. { 1. } else { 0. })
                        * 0.5;
                let expected: [u8; 3] = std::array::from_fn(|i| {
                    if h > edge {
                        ([0.1, 0.2, 0.3][i] * 255_f64).round() as u8
                    } else {
                        (([0.2, 0.4, 0.8][i] * (1. - amount) + [0.1, 0.2, 0.3][i] * amount) * 255.)
                            .round() as u8
                    }
                });
                let actual = &base[((y * 64 + x) * 4) as usize..][..3];
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(&a, b)| a.abs_diff(b) <= 2),
                    "sky {x},{y}: {actual:?} != {expected:?}"
                );
            }
        }
        f.bob = glam::Mat4::from_rotation_z(0.3);
        f.spin = glam::Mat4::from_rotation_x(0.5);
        renderer.render_game_frame(&f, empty, options).unwrap();
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), base);
        f.bob = glam::Mat4::IDENTITY;
        f.spin = glam::Mat4::IDENTITY;
        f.pitch = 0.;
        f.yaw = 90.;
        f.sky.sunrise_color = [1., 0., 0., 1.];
        f.fog = Some(fog([16. / 255., 32. / 255., 48. / 255., 1.], 0.));
        renderer.render_game_frame(&f, empty, options).unwrap();
        let dawn = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        assert!(
            dawn[0] > 230 && dawn[1] < 10 && dawn[2] < 10,
            "sunrise apex {dawn:?}"
        );
        let stable = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        renderer.limits.sky_vertices = 71;
        assert!(renderer.render_game_frame(&f, empty, options).is_err());
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        renderer.limits.sky_vertices = 32768;
        let missing = [[7, 0, 0]];
        assert!(
            renderer
                .render_game_frame(
                    &f,
                    frame::VisibleSections {
                        solid: &missing,
                        translucent: &[]
                    },
                    options
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        f.sky.sunrise_color = [0.; 4];
        f.yaw = 0.;
        f.camera = glam::DVec3::new(30_000_000.25, 0., 0.);
        let celestial =
            [[-4., -4., 4.], [-4., 4., 4.], [4., 4., 4.], [4., -4., 4.]].map(|pos| Vertex {
                pos,
                uv,
                layer: 2,
                color: 0x8000ff00,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            });
        f.celestial = &celestial;
        renderer.render_game_frame(&f, empty, options).unwrap();
        let pixel = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        assert!(
            pixel[..3]
                .iter()
                .zip([16, 160, 48])
                .all(|(&a, b)| a.abs_diff(b) <= 2),
            "celestial {pixel:?}"
        );
        // The prior full world-fog case intentionally left celestial un-fogged;
        // disable its alpha before asserting the later terrain's red color.
        f.fog = Some(fog([16. / 255., 32. / 255., 48. / 255., 0.], 0.));
        let mut solid = SectionMesh::default();
        solid.solid.extend(
            [[-20., -20.], [20., -20.], [20., 20.], [-20., 20.]].map(|p| {
                TerrainVertex::new([p[0] + 0.25, p[1], 8.], [0.5; 2], 0x8000, 0xff0000ff, 0)
            }),
        );
        let key = [1_875_000, 0, 0];
        let keys = [key];
        renderer.upload_section(key, &solid).unwrap();
        renderer
            .render_game_frame(
                &f,
                frame::VisibleSections {
                    solid: &keys,
                    translucent: &[],
                },
                options,
            )
            .unwrap();
        assert_eq!(
            &renderer.read_rgba([32, 32, 1, 1]).unwrap()[..3],
            &[255, 0, 0]
        ); // sky cannot hide later world depth
        renderer.unload_section(key).unwrap();
        renderer.world.collect_garbage().unwrap();
        f.celestial = &[];
        f.skybox = false;
        f.end_sky = true;
        f.end_sky_sprite = Some([
            uv[0] - 0.5 / atlas.width as f32,
            uv[1] - 0.5 / atlas.height as f32,
            uv[0] + 0.5 / atlas.width as f32,
            uv[1] + 0.5 / atlas.height as f32,
        ]);
        f.fog = Some(fog([0., 0., 0., 1.], 512.));
        renderer.render_game_frame(&f, empty, options).unwrap();
        let end = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        assert!(end[..3].iter().all(|&c| c.abs_diff(40) <= 2), "End {end:?}");
        f.dark_disc = true;
        f.pitch = 45.;
        renderer.render_game_frame(&f, empty, options).unwrap();
        assert_eq!(&renderer.read_rgba([32, 32, 1, 1]).unwrap()[..3], &[0; 3]);
        let mut small = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[0; 1024],
            &atlas,
            Limits {
                gpu: Budget {
                    mesh_bytes: 2207,
                    ..Budget::default()
                },
                ..Limits::default()
            },
        )
        .unwrap();
        f.end_sky = false;
        f.skybox = true;
        f.dark_disc = false;
        f.pitch = 0.;
        f.yaw = 90.;
        f.sky.sunrise_color = [0.; 4];
        small.render_game_frame(&f, empty, options).unwrap();
        let saved = small.read_rgba([0, 0, 64, 64]).unwrap();
        f.sky.sunrise_color = [1.; 4];
        assert!(small.render_game_frame(&f, empty, options).is_err());
        assert_eq!(small.read_rgba([0, 0, 64, 64]).unwrap(), saved); // sunrise needs 2208 bytes
        renderer.resize([512; 2]).unwrap();
        let star_hud = HudBatch::new([512; 2]);
        let mut stars = frame::tests::source(&star_hud);
        stars.skybox = true;
        stars.pitch = -45.;
        stars.yaw = 0.;
        stars.fov_degrees = 90.;
        stars.sky.sky_color = [0.; 3];
        stars.fog = Some(fog([0., 0., 0., 1.], 512.));
        stars.stars = Some((glam::Mat4::IDENTITY, 1.));
        renderer.render_game_frame(&stars, empty, options).unwrap();
        let bright = renderer.read_rgba([0, 0, 512, 512]).unwrap();
        assert!(
            bright.iter().step_by(4).any(|&c| c > 200),
            "seeded stars did not draw"
        );
        stars.stars = Some((glam::Mat4::IDENTITY, 0.5));
        renderer.render_game_frame(&stars, empty, options).unwrap();
        let dim = renderer.read_rgba([0, 0, 512, 512]).unwrap();
        assert!(
            bright
                .iter()
                .step_by(4)
                .zip(dim.iter().step_by(4))
                .any(|(&a, &b)| a == 255 && b.abs_diff(64) <= 2),
            "star b*b SDR blend missing"
        );
        stars.stars = Some((glam::Mat4::from_rotation_y(0.8), 0.5));
        renderer.render_game_frame(&stars, empty, options).unwrap();
        assert_ne!(renderer.read_rgba([0, 0, 512, 512]).unwrap(), dim);
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 0);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_actual_weather_animates_sprites_after_translucency_before_text_hand_screen_and_hud() {
        use crate::{
            mesher::Vertex,
            screen_effects::{Blend, ScreenQuad, ScreenVertex},
            weather::{Time, Vertex as WeatherVertex},
        };
        let mut atlas = HudAtlas::boot();
        atlas.width = 8;
        atlas.height = 4;
        atlas.pixels = vec![0; 128];
        for (i, rgb) in [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [0, 255, 255],
            [255, 0, 255],
            [255, 255, 0],
        ]
        .into_iter()
        .enumerate()
        {
            let at = ((1 + i / 3) * 8 + 1 + i % 3) * 4;
            atlas.pixels[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 128]);
        }
        atlas.pixels[24..28].copy_from_slice(&[255; 4]);
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255, 255, 255, 128]]).unwrap();
        let limits = Limits {
            hud_page_size: 4,
            weather_bytes: 88,
            gpu: Budget {
                texture_bytes: 1156,
                mesh_bytes: 704,
            },
            ..Limits::default()
        };
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout.clone(),
            &[&page],
            &[0; 1024],
            &atlas,
            limits,
        )
        .unwrap();
        let mut hud = HudBatch::new([64; 2]);
        hud.quad(
            [[24., 24.], [40., 24.], [40., 40.], [24., 40.]],
            [[6.5 / 8., 0.5 / 4.]; 4],
            0xff306010,
        );
        let mut water = SectionMesh::default();
        water
            .translucent
            .extend([[-2., -2.], [2., -2.], [2., 2.], [-2., 2.]].map(|p| {
                TerrainVertex::new([p[0] + 0.25, p[1] + 4.5, 1.], [0.5; 2], 0, 0xffff0000, 0)
            }));
        let key = [1_875_000, 6, 0];
        let keys = [key];
        renderer.upload_section(key, &water).unwrap();
        let corners = [[-4., -4., 2.], [-4., 4., 2.], [4., 4., 2.], [4., -4., 2.]];
        let rain = corners.map(|pos| WeatherVertex {
            pos,
            uv: [-0.75, -0.75],
            rect: [1. / 8., 1. / 4., 3. / 8., 2. / 4.],
            animation: [1., 0., 0., 0.],
            bases: [0; 4],
            color: u32::MAX,
            light: 0,
            snow: 0,
        });
        let mut snow = rain;
        for v in &mut snow {
            v.snow = 1;
            v.bases = [0, 0x3fe00000, 0, 0x3fe00000];
        }
        let options = frame::Options {
            gamma: 0.,
            fullbright: true,
        };
        let visible = frame::VisibleSections {
            solid: &[],
            translucent: &keys,
        };
        let draw = |renderer: &mut TerrainHudRenderer,
                    hud: &HudBatch,
                    weather: &[WeatherVertex],
                    time: Time,
                    hand: &[Vertex],
                    see: &[Vertex],
                    screens: &[ScreenQuad]| {
            let mut source = frame::tests::source(hud);
            source.camera = glam::DVec3::new(30_000_000.25, 100.5, 0.);
            source.yaw = 0.;
            source.pitch = 0.;
            source.fov_degrees = 90.;
            source.distance = 512.;
            source.sky.fog_color = [0.; 3];
            source.weather = weather;
            source.weather_time = time;
            source.hand = hand;
            source.see_through = see;
            source.screen = screens;
            renderer.render_game_frame(&source, visible, options)
        };
        let near = |renderer: &mut TerrainHudRenderer, at: [u32; 2], expected: [u8; 3]| {
            let actual = renderer.read_rgba([at[0], at[1], 1, 1]).unwrap();
            assert!(
                actual[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                "{at:?}: {actual:?} != {expected:?}"
            );
        };
        draw(&mut renderer, &hud, &rain, Time::new(0, 0.), &[], &[], &[]).unwrap();
        near(&mut renderer, [0, 16], [128, 0, 64]); // blue translucent terrain THEN half-alpha red rain
        near(&mut renderer, [32, 32], [16, 96, 48]); // HUD follows weather
        let repeated: Vec<_> = rain.into_iter().chain(rain).collect();
        draw(
            &mut renderer,
            &hud,
            &repeated,
            Time::new(0, 0.),
            &[],
            &[],
            &[],
        )
        .unwrap();
        near(&mut renderer, [0, 16], [192, 0, 32]); // retained image, both quad instances
        let mut alternate = rain;
        for v in &mut alternate {
            v.rect = [6. / 8., 0., 1. / 8., 1. / 4.];
            v.color = 0x8000ff00;
        }
        let switched: Vec<_> = rain.into_iter().chain(alternate).collect();
        renderer.limits.weather_bytes = 92;
        draw(
            &mut renderer,
            &hud,
            &switched,
            Time::new(0, 0.),
            &[],
            &[],
            &[],
        )
        .unwrap();
        near(&mut renderer, [0, 16], [64, 128, 32]); // sprite switch retains source blend order
        renderer.limits.weather_bytes = 88;
        draw(&mut renderer, &hud, &rain, Time::new(16, 0.), &[], &[], &[]).unwrap();
        near(&mut renderer, [0, 16], [0, 128, 192]); // rain moves to cyan row, not atlas neighbour
        draw(&mut renderer, &hud, &snow, Time::new(0, 0.), &[], &[], &[]).unwrap();
        near(&mut renderer, [0, 16], [128, 128, 64]); // snow bases select yellow independently of rain clock
        let text = corners.map(|pos| Vertex {
            pos,
            uv: [6.5 / 8., 0.5 / 4.],
            layer: 0,
            color: 0xffff0000,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.; 2],
        });
        draw(
            &mut renderer,
            &hud,
            &rain,
            Time::new(0, 0.),
            &[],
            &text,
            &[],
        )
        .unwrap();
        near(&mut renderer, [0, 16], [0, 0, 255]); // see-through follows weather
        let hand = text.map(|mut v| {
            v.pos[0] *= 25.;
            v.pos[1] *= 25.;
            v.pos[2] = 8.;
            v
        });
        draw(
            &mut renderer,
            &hud,
            &rain,
            Time::new(0, 0.),
            &hand,
            &[],
            &[],
        )
        .unwrap();
        near(&mut renderer, [0, 16], [0, 0, 255]); // hand follows weather too
        let overlay = ScreenQuad {
            vertices: [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]].map(|pos| ScreenVertex {
                pos,
                uv: [0.5; 2],
                rect: [6. / 8., 0., 7. / 8., 1. / 4.],
                color: [0., 0., 0., 0.5],
            }),
            blend: Blend::Alpha,
            sampling: 0,
            world_tint: false,
        };
        draw(
            &mut renderer,
            &hud,
            &rain,
            Time::new(0, 0.),
            &hand,
            &[],
            &[overlay],
        )
        .unwrap();
        near(&mut renderer, [0, 16], [0, 0, 127]);
        near(&mut renderer, [32, 32], [16, 96, 48]);
        hud.patches.push(crate::font::Patch {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
            rgba: vec![0, 255, 0, 128],
        });
        draw(&mut renderer, &hud, &rain, Time::new(0, 0.), &[], &[], &[]).unwrap();
        near(&mut renderer, [0, 16], [0, 128, 64]);
        hud.patches.clear();
        renderer.reload_hud(&atlas).unwrap();
        draw(&mut renderer, &hud, &rain, Time::new(0, 0.), &[], &[], &[]).unwrap();
        near(&mut renderer, [0, 16], [128, 0, 64]);
        let stats = renderer.world.backend_mut().stats();
        assert_eq!(stats.texture_bytes, 1156);
        assert_eq!(stats.mesh_bytes, 188);
        let stable = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        renderer.limits.weather_quads = 0;
        assert!(draw(&mut renderer, &hud, &rain, Time::new(0, 0.), &[], &[], &[]).is_err());
        renderer.limits.weather_quads = 4096;
        renderer.limits.entity_vertices = 3;
        assert!(draw(&mut renderer, &hud, &rain, Time::new(0, 0.), &[], &[], &[]).is_err());
        renderer.limits.entity_vertices = 262144;
        renderer.limits.weather_bytes = 87;
        assert!(draw(&mut renderer, &hud, &rain, Time::new(0, 0.), &[], &[], &[]).is_err());
        renderer.limits.weather_bytes = 88;
        assert!(
            draw(
                &mut renderer,
                &hud,
                &rain,
                Time {
                    values: [0., 0., f32::NAN, 0.]
                },
                &[],
                &[],
                &[]
            )
            .is_err()
        );
        let mut invalid = rain;
        invalid[0].snow = 2;
        assert!(
            draw(
                &mut renderer,
                &hud,
                &invalid,
                Time::new(0, 0.),
                &[],
                &[],
                &[]
            )
            .is_err()
        );
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        let mut small = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[0; 1024],
            &atlas,
            Limits {
                gpu: Budget {
                    mesh_bytes: 535,
                    ..limits.gpu
                },
                ..limits
            },
        )
        .unwrap();
        small.upload_section(key, &water).unwrap();
        draw(&mut small, &hud, &[], Time::new(0, 0.), &[], &[], &[]).unwrap();
        let before = small.read_rgba([0, 0, 64, 64]).unwrap();
        assert!(draw(&mut small, &hud, &rain, Time::new(0, 0.), &[], &[], &[]).is_err());
        assert_eq!(small.read_rgba([0, 0, 64, 64]).unwrap(), before); // needs exactly 188 resident + 348 transient mesh bytes
        let mut source = frame::tests::source(&hud);
        source.weather = &rain;
        source.pitch = 0.;
        source.yaw = 0.;
        source.fog = Some(crate::fog::FogData {
            color: [0., 1., 0., 1.],
            environmental_start: 0.,
            environmental_end: 0.,
            render_distance_start: 0.,
            render_distance_end: 0.,
            sky_end: 0.,
            cloud_end: 0.,
        });
        renderer
            .render_game_frame(&source, frame::VisibleSections::default(), options)
            .unwrap();
        near(&mut renderer, [0, 16], [0, 255, 0]);
        near(&mut renderer, [32, 32], [16, 96, 48]);
        renderer.unload_section(key).unwrap();
        renderer.world.collect_garbage().unwrap();
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 0);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_actual_lightning_and_additive_precede_translucency_and_preserve_ray_depth() {
        use crate::{gfx::lightning, mesher::Vertex};
        let atlas = HudAtlas::boot();
        let mut hud = HudBatch::new([64; 2]);
        hud.fill(&atlas, 0., 0., 1., 1., u32::MAX);
        let white_uv = hud.vertices[0].uv;
        hud.vertices.clear();
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255, 255, 255, 128]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[0; 1024],
            &atlas,
            Limits::default(),
        )
        .unwrap();
        let template = lightning::geometry(9, white_uv);
        let mid: [f64; 3] = std::array::from_fn(|axis| {
            template[..4]
                .iter()
                .map(|v| f64::from(v.pos[axis]))
                .sum::<f64>()
                / 4.
        });
        let bolts = [lightning::Bolt {
            at: [30_000_000., 0., 0.],
            seed: 9,
        }; 2];
        let mut source = frame::tests::source(&hud);
        source.camera = glam::DVec3::new(30_000_000. + mid[0], mid[1], mid[2] - 16.);
        source.yaw = 0.;
        source.pitch = 0.;
        source.fov_degrees = 90.;
        source.sky.fog_color = [16. / 255., 32. / 255., 48. / 255.];
        source.distance = 512.;
        let empty = frame::VisibleSections::default();
        let options = frame::Options::default();
        renderer.render_game_frame(&source, empty, options).unwrap();
        let background = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        source.lightning = lightning::Input {
            bolts: &bolts[..1],
            uv: white_uv,
        };
        renderer.render_game_frame(&source, empty, options).unwrap();
        let single = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        assert!(
            single
                .as_chunks::<4>()
                .0
                .iter()
                .zip(background.as_chunks::<4>().0.iter())
                .any(|(a, b)| a[..3] != b[..3]),
            "actual seeded lightning did not draw"
        );
        source.lightning.bolts = &bolts;
        renderer.render_game_frame(&source, empty, options).unwrap();
        let doubled = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        assert!(
            doubled
                .as_chunks::<4>()
                .0
                .iter()
                .zip(single.as_chunks::<4>().0.iter())
                .any(|(a, b)| (0..3).any(|i| a[i] > b[i])),
            "shared seeds lost a bolt instance"
        );
        source.lightning.bolts = &bolts[..1];
        source.fog = Some(crate::fog::FogData {
            color: [16. / 255., 32. / 255., 48. / 255., 1.],
            environmental_start: 0.,
            environmental_end: 0.,
            render_distance_start: 0.,
            render_distance_end: 0.,
            sky_end: 0.,
            cloud_end: 0.,
        });
        renderer.render_game_frame(&source, empty, options).unwrap();
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), single); // unlit/unfogged lightning
        source.fog = None;
        let mut water = SectionMesh::default();
        let camera = source.camera.to_array();
        water
            .translucent
            .extend([[-2., -2.], [2., -2.], [2., 2.], [-2., 2.]].map(|p| {
                TerrainVertex::new(
                    [
                        ((camera[0] - 30_000_000.) + p[0]) as f32,
                        (camera[1] + p[1]) as f32,
                        (camera[2] + 1.) as f32,
                    ],
                    [0.5; 2],
                    0,
                    0xffff0000,
                    0,
                )
            }));
        let key = [1_875_000, 0, 0];
        let keys = [key];
        renderer.upload_section(key, &water).unwrap();
        source.sky.ambient_light_color = [1.; 3];
        let visible = frame::VisibleSections {
            solid: &[],
            translucent: &keys,
        };
        renderer
            .render_game_frame(&source, visible, options)
            .unwrap();
        let covered = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        for (actual, bolt) in covered
            .as_chunks::<4>()
            .0
            .iter()
            .zip(single.as_chunks::<4>().0.iter())
        {
            for c in 0..3 {
                let expected = (f32::from(bolt[c]) * (127. / 255.) + if c == 2 { 128. } else { 0. })
                    .round() as u8;
                assert!(
                    actual[c].abs_diff(expected) <= 2,
                    "translucent terrain did not cover lightning: {actual:?}/{bolt:?}"
                );
            }
        }
        source.lightning.bolts = &[];
        let quad = |z: f32, layer: u32, color: u32| {
            [[-1., -1., z], [-1., 1., z], [1., 1., z], [1., -1., z]].map(|pos| Vertex {
                pos,
                uv: white_uv,
                layer,
                color,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            })
        };
        let near = |renderer: &mut TerrainHudRenderer, expected: [u8; 3]| {
            let actual = renderer.read_rgba([32, 32, 1, 1]).unwrap();
            assert!(
                actual[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                "{actual:?} != {expected:?}"
            );
        };
        let mut extra: Vec<_> = quad(2., 2, 0x8000ff00)
            .into_iter()
            .chain(quad(3., 2, 0x800000ff))
            .collect();
        {
            let mut view = frame::tests::source(&hud);
            view.camera = source.camera;
            view.yaw = 0.;
            view.pitch = 0.;
            view.fov_degrees = 90.;
            view.sky.fog_color = source.sky.fog_color;
            view.additive = &extra;
            renderer.render_game_frame(&view, empty, options).unwrap();
        }
        near(&mut renderer, [144, 160, 48]); // ordinary additive does NOT write depth
        extra[..4].copy_from_slice(&quad(2., 37, 0x80ff0000));
        {
            let mut view = frame::tests::source(&hud);
            view.camera = source.camera;
            view.yaw = 0.;
            view.pitch = 0.;
            view.fov_degrees = 90.;
            view.sky.fog_color = source.sky.fog_color;
            view.additive = &extra;
            renderer.render_game_frame(&view, empty, options).unwrap();
        }
        near(&mut renderer, [16, 32, 176]); // near rays DO hide farther additive quads
        {
            let mut view = frame::tests::source(&hud);
            view.camera = source.camera;
            view.yaw = 0.;
            view.pitch = 0.;
            view.fov_degrees = 90.;
            view.sky.fog_color = source.sky.fog_color;
            view.fog = Some(crate::fog::FogData {
                color: [16. / 255., 32. / 255., 48. / 255., 0.],
                environmental_start: 0.,
                environmental_end: 0.,
                render_distance_start: 0.,
                render_distance_end: 0.,
                sky_end: 0.,
                cloud_end: 0.,
            });
            view.additive = &extra;
            renderer.render_game_frame(&view, empty, options).unwrap();
        }
        near(&mut renderer, [16, 32, 48]); // ray RGBA fog fade ignores fog color alpha
        let stable = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        renderer.limits.lightning_bolts = 0;
        source.lightning.bolts = &bolts[..1];
        assert!(renderer.render_game_frame(&source, empty, options).is_err());
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        renderer.limits.lightning_bolts = 64;
        extra[0].layer = 1;
        source.lightning.bolts = &[];
        source.additive = &extra;
        assert!(renderer.render_game_frame(&source, empty, options).is_err());
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        renderer.unload_section(key).unwrap();
        renderer.world.collect_garbage().unwrap();
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 0);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_actual_frame_screen_repeat_clamp_blur_vignette_order_and_patches() {
        use crate::{
            mesher::Vertex,
            screen_effects::{Blend, ScreenQuad, ScreenVertex},
        };
        let mut atlas = HudAtlas::boot();
        atlas.width = 6;
        atlas.height = 2;
        atlas.pixels = [
            0, 0, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            255, 255, 255, 255,
        ]
        .repeat(2);
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255; 4]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[0; 1024],
            &atlas,
            Limits {
                hud_page_size: 4,
                screen_bytes: 96,
                gpu: Budget {
                    texture_bytes: 1156,
                    mesh_bytes: 516,
                },
                ..Limits::default()
            },
        )
        .unwrap();
        let mut hud = HudBatch::new([64; 2]);
        hud.quad(
            [[24., 24.], [40., 24.], [40., 40.], [24., 40.]],
            [[4.5 / 6., 0.5]; 4],
            0xff306010,
        );
        let full = [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]];
        let uv = [[-0.25, 0.], [3.75, 0.], [3.75, 1.], [-0.25, 1.]];
        let overlay = ScreenQuad {
            vertices: std::array::from_fn(|i| ScreenVertex {
                pos: full[i],
                uv: uv[i],
                rect: [1. / 6., 0., 3. / 6., 1.],
                color: [1.; 4],
            }),
            blend: Blend::Alpha,
            sampling: 0,
            world_tint: true,
        };
        let vignette = ScreenQuad {
            vertices: std::array::from_fn(|i| ScreenVertex {
                pos: full[i],
                uv: uv[i],
                rect: [4. / 6., 0., 1., 1.],
                color: [0.5, 0.25, 0.75, 0.],
            }),
            blend: Blend::Vignette,
            sampling: 0,
            world_tint: false,
        };
        // A blue hand covers the entire view; overlays must draw AFTER it.
        let hand = full.map(|p| Vertex {
            pos: [p[0] * 100., p[1] * 100., 8.],
            uv: [4.5 / 6., 0.5],
            layer: 0,
            color: 0xffff0000,
            light: 0,
            normal: [0; 4],
            glint_uv: [0.; 2],
        });
        let near = |renderer: &mut TerrainHudRenderer, at: [u32; 2], expected: [u8; 3]| {
            let actual = renderer.read_rgba([at[0], at[1], 1, 1]).unwrap();
            assert!(
                actual[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                "{at:?}: {actual:?} != {expected:?}"
            );
        };
        let options = frame::Options {
            gamma: 0.,
            fullbright: true,
        };
        let mut overlays = [overlay, vignette];
        for sampling in 0..4 {
            overlays[0].sampling = sampling;
            let mut source = frame::tests::source(&hud);
            source.camera = glam::DVec3::ZERO;
            source.yaw = 0.;
            source.pitch = 0.;
            source.hand = &hand;
            source.screen = &overlays;
            renderer
                .render_game_frame(&source, frame::VisibleSections::default(), options)
                .unwrap();
            let expected = if sampling & 2 != 0 {
                [128, 0, 0]
            } else if sampling & 1 != 0 {
                [8, 179, 0]
            } else {
                [0, 191, 0]
            };
            near(&mut renderer, [0, 16], expected);
            near(&mut renderer, [32, 32], [16, 96, 48]); // HUD is AFTER vignette, never darkened by it
            let stats = renderer.world.backend_mut().stats();
            assert_eq!(stats.texture_bytes, 1156); // only fixed textures + one HUD page
            assert_eq!(stats.mesh_bytes, 0);
        }
        overlays[0].sampling = 1;
        hud.patches.push(crate::font::Patch {
            x: 1,
            y: 0,
            width: 1,
            height: 2,
            rgba: [0, 0, 255, 255].repeat(2),
        });
        {
            let mut source = frame::tests::source(&hud);
            source.screen = &overlays;
            renderer
                .render_game_frame(&source, frame::VisibleSections::default(), options)
                .unwrap();
        }
        near(&mut renderer, [0, 16], [0, 179, 4]); // patched sprite, no stale extracted-image cache
        hud.patches.clear();
        let stable = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        renderer.limits.screen_bytes = 95;
        {
            let mut source = frame::tests::source(&hud);
            source.screen = &overlays;
            assert!(
                renderer
                    .render_game_frame(&source, frame::VisibleSections::default(), options)
                    .is_err()
            );
        }
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        renderer.limits.screen_bytes = 96;
        overlays[0].vertices[2].rect[0] = 0.;
        {
            let mut source = frame::tests::source(&hud);
            source.screen = &overlays;
            assert!(
                renderer
                    .render_game_frame(&source, frame::VisibleSections::default(), options)
                    .is_err()
            );
        }
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        overlays[0].vertices[2].rect = overlays[0].vertices[0].rect;
        renderer.reload_hud(&atlas).unwrap();
        {
            let mut source = frame::tests::source(&hud);
            source.screen = &overlays;
            renderer
                .render_game_frame(&source, frame::VisibleSections::default(), options)
                .unwrap();
        }
        near(&mut renderer, [0, 16], [8, 179, 0]); // atlas reload reconstructs sprite pixels
        near(&mut renderer, [32, 32], [16, 96, 48]);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_actual_frame_drives_projection_static_depth_fog_lightmap_hand_and_hud() {
        use crate::{block_entity_render::StaticSectionSnapshot, mesher::Vertex};
        use std::sync::Arc;
        let atlas = HudAtlas::boot();
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[64, 128, 192, 255]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[0; 1024],
            &atlas,
            Limits::default(),
        )
        .unwrap();
        let positions = |extent: f32, z: f32| {
            [
                [-extent, -extent, z],
                [extent, -extent, z],
                [extent, extent, z],
                [-extent, extent, z],
            ]
        };
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            positions(4., 4.).map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0)),
        );
        let key = [1_875_000, 0, 0];
        renderer.upload_section(key, &mesh).unwrap();
        let mut hud = HudBatch::new([64; 2]);
        hud.fill(&atlas, 6., 6., 8., 8., 0xff306010);
        let white_uv = hud.vertices[0].uv;
        let quad = |extent, z, color| {
            positions(extent, z).map(|pos| Vertex {
                pos,
                uv: white_uv,
                layer: 0,
                color,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            })
        };
        let snapshots = [StaticSectionSnapshot {
            section: key,
            origin: [30_000_000, 0, 0],
            revision: 1,
            entities: 1,
            vertices: Arc::from(quad(0.5, 2., 0xff00ff00)),
        }];
        let dynamic = quad(1., 3., 0xff0000ff); // farther red must NOT cover nearer static green
        let hand = quad(0.5, 8., 0xffff0000); // farther blue still draws AFTER depth clear
        let mut source = frame::tests::source(&hud);
        source.camera = glam::DVec3::new(30_000_000.25, 0., 0.);
        source.yaw = 0.;
        source.pitch = 0.;
        source.fov_degrees = 90.;
        source.sky.ambient_light_color = [0.5; 3];
        source.entities = &dynamic;
        source.static_block_entities = &snapshots;
        let visible_keys = [key];
        source.visible_static_block_entity_sections = &visible_keys;
        let visible = frame::VisibleSections {
            solid: &visible_keys,
            translucent: &[],
        };
        let near = |renderer: &mut TerrainHudRenderer, at: [u32; 2], expected: [u8; 3]| {
            let actual = renderer.read_rgba([at[0], at[1], 1, 1]).unwrap();
            assert!(
                actual[..3]
                    .iter()
                    .zip(expected)
                    .all(|(&a, b)| a.abs_diff(b) <= 2),
                "{at:?}: {actual:?} != {expected:?}"
            );
        };
        renderer
            .render_game_frame(&source, visible, frame::Options::default())
            .unwrap();
        near(&mut renderer, [32, 32], [0, 127, 0]);
        near(&mut renderer, [8, 32], [32, 64, 96]);
        near(&mut renderer, [8, 56], [16, 96, 48]);
        source.fog = Some(crate::fog::FogData {
            color: [0.25, 0.5, 0.75, 0.5],
            environmental_start: 0.,
            environmental_end: 0.,
            render_distance_start: f32::MAX,
            render_distance_end: f32::MAX,
            sky_end: 100.,
            cloud_end: 100.,
        });
        renderer
            .render_game_frame(&source, visible, frame::Options::default())
            .unwrap();
        near(&mut renderer, [32, 32], [32, 127, 96]);
        source.hand = &hand;
        renderer
            .render_game_frame(&source, visible, frame::Options::default())
            .unwrap();
        near(&mut renderer, [32, 32], [0, 0, 127]);
        near(&mut renderer, [8, 56], [16, 96, 48]);
        let stable = renderer.read_rgba([0, 0, 64, 64]).unwrap();
        source.title = true;
        assert!(
            renderer
                .render_game_frame(&source, visible, frame::Options::default())
                .unwrap_err()
                .contains("panorama")
        );
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        source.title = false;
        source.entities = &dynamic[..3];
        assert!(
            renderer
                .render_game_frame(&source, visible, frame::Options::default())
                .is_err()
        );
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
        source.entities = &dynamic;
        let missing = frame::VisibleSections {
            solid: &[[0; 3]],
            translucent: &[],
        };
        assert!(
            renderer
                .render_game_frame(&source, missing, frame::Options::default())
                .is_err()
        );
        assert_eq!(renderer.read_rgba([0, 0, 64, 64]).unwrap(), stable);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_coordinator_renders_and_unloads_actual_game_geometry() {
        let atlas = HudAtlas::boot();
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[64, 128, 192, 255]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 16 * 16 * 4],
            &atlas,
            Limits::default(),
        )
        .unwrap();
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &mesh).unwrap();
        let mut hud = HudBatch::new([64; 2]);
        hud.fill(&atlas, 16., 16., 32., 32., u32::MAX);
        hud.invert_since(0);
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        renderer.render(&frame, &hud).unwrap();
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [191, 127, 63, 255]
        );
        assert_eq!(
            renderer.read_rgba([4, 4, 1, 1]).unwrap(),
            [64, 128, 192, 255]
        );
        renderer.reload_hud(&atlas).unwrap();
        renderer.render(&frame, &hud).unwrap();
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [191, 127, 63, 255]
        );
        assert_eq!(renderer.resident_sections(), 1);
        assert!(renderer.unload_section([0; 3]).unwrap());
        assert_eq!(renderer.resident_sections(), 0);
        assert!(!renderer.present().unwrap()); // offscreen context, no fake window swap
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_large_hud_streams_one_gpu_page_and_patches_across_a_seam() {
        // Wider than a conventional GL2 driver's texture limit, but only 2 rows:
        // the fixture proves virtual addressing without a giant test allocation.
        let mut atlas = HudAtlas::boot();
        atlas.width = 16385;
        atlas.height = 2;
        atlas.pixels.clear();
        for _ in 0..2 {
            for x in 0..atlas.width {
                atlas.pixels.extend(if x < 510 {
                    [255, 0, 0, 128]
                } else {
                    [0, 0, 255, 128]
                });
            }
        }
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255; 4]]).unwrap();
        let limits = Limits {
            gpu: Budget {
                texture_bytes: (1 << 20) + 4096,
                ..Budget::default()
            },
            ..Limits::default()
        };
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 16 * 16 * 4],
            &atlas,
            limits,
        )
        .unwrap();
        let mut batch = HudBatch::new([64; 2]);
        let (u0, u1) = (509. / atlas.width as f32, 511. / atlas.width as f32);
        batch.quad(
            [[0., 0.], [64., 0.], [64., 64.], [0., 64.]],
            [[u0, 0.25], [u1, 0.25], [u1, 0.25], [u0, 0.25]],
            u32::MAX,
        );
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[],
            translucent: &[],
        };
        renderer.render(&frame, &batch).unwrap();
        for x in 0..64 {
            let expected = if x < 32 {
                [128, 0, 0, 255]
            } else {
                [0, 0, 128, 255]
            };
            assert_eq!(renderer.read_rgba([x, 32, 1, 1]).unwrap(), expected);
        }
        assert_eq!(
            renderer.world.backend_mut().stats().texture_bytes,
            (1 << 20) + 1092
        );
        batch.patches.push(crate::font::Patch {
            x: 509,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![0, 255, 0, 255, 255, 255, 255, 255],
        });
        renderer.render(&frame, &batch).unwrap();
        assert_eq!(
            renderer.read_rgba([16, 32, 1, 1]).unwrap(),
            [0, 255, 0, 255]
        );
        assert_eq!(renderer.read_rgba([48, 32, 1, 1]).unwrap(), [255; 4]);
        batch.patches.push(crate::font::Patch {
            x: atlas.width,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        });
        assert!(renderer.render(&frame, &batch).is_err());
        assert_eq!(
            renderer.read_rgba([16, 32, 1, 1]).unwrap(),
            [0, 255, 0, 255]
        );
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_transient_mesh_preflight_does_not_erase_the_previous_frame() {
        let atlas = HudAtlas::boot();
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[64, 128, 192, 255]]).unwrap();
        let limits = Limits {
            gpu: Budget {
                mesh_bytes: 4 * 44 + 6 * 2,
                ..Budget::default()
            },
            ..Limits::default()
        };
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 16 * 16 * 4],
            &atlas,
            limits,
        )
        .unwrap();
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &mesh).unwrap();
        let mut frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        let mut hud = HudBatch::new([64; 2]);
        renderer.render(&frame, &hud).unwrap();
        frame.solid = &[];
        frame.clear = [1., 0., 0., 1.];
        hud.fill(&atlas, 0., 0., 64., 64., u32::MAX);
        assert!(renderer.render(&frame, &hud).is_err());
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [64, 128, 192, 255]
        );
        let entity = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|pos| {
            crate::mesher::Vertex {
                pos,
                uv: [0.; 2],
                layer: 0,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            }
        });
        assert!(
            renderer
                .render_with_entities(&frame, &HudBatch::new([64; 2]), &entity, false)
                .is_err()
        );
        assert_eq!(
            renderer.read_rgba([32, 32, 1, 1]).unwrap(),
            [64, 128, 192, 255]
        );
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_entities_terrain_particles_hand_and_hud_share_one_page_cache() {
        use crate::mesher::Vertex;
        let mut atlas = HudAtlas::boot();
        atlas.width = 6;
        atlas.height = 2;
        atlas.pixels = [
            255, 0, 0, 128, 255, 0, 0, 128, 0, 255, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 0, 0,
            255, 128,
        ]
        .repeat(2);
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255, 255, 255, 128]]).unwrap();
        let limits = Limits {
            hud_page_size: 4,
            gpu: Budget {
                texture_bytes: 1156,
                ..Budget::default()
            },
            ..Limits::default()
        };
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 1024],
            &atlas,
            limits,
        )
        .unwrap();
        let positions = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]];
        let mut terrain = SectionMesh::default();
        terrain
            .translucent
            .extend(positions.map(|p| TerrainVertex::new(p, [0.5; 2], 0, 0xffff0000, 0xf0f0)));
        renderer.upload_section([0; 3], &terrain).unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[],
            translucent: &[[0; 3]],
        };
        let quad = |u, depth, layer| {
            positions.map(|p| Vertex {
                pos: [p[0], p[1], depth],
                uv: [u, 0.5],
                layer,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            })
        };
        let entities = [quad(0.1, 0.2, 32), quad(0.5, 0.1, 15)].concat();
        let mut hud = HudBatch::new([64; 2]);
        hud.quad(
            [[0., 0.], [8., 0.], [8., 8.], [0., 8.]],
            [[0.9, 0.5]; 4],
            u32::MAX,
        );
        renderer
            .render_with_entities(&frame, &hud, &entities, false)
            .unwrap();
        let near = |actual: &[u8], expected: [u8; 3]| {
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        near(&renderer.read_rgba([32, 32, 1, 1]).unwrap(), [32, 128, 64]);
        near(&renderer.read_rgba([4, 60, 1, 1]).unwrap(), [16, 64, 160]);
        assert_eq!(renderer.world.backend_mut().stats().texture_bytes, 1156);
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 188);
        // A farther hand must draw on top using its independent projection/depth
        // view, but the HUD still follows it. The hand's red texel is half alpha.
        let hand = quad(0.1, 0.9, 0);
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &entities,
                    hand: &hand,
                    see_through: &[],
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([32, 32, 1, 1]).unwrap(), [144, 64, 32]);
        near(&renderer.read_rgba([4, 60, 1, 1]).unwrap(), [72, 32, 144]);
        let before = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        assert!(
            renderer
                .render_with_entities(&frame, &hud, &quad(0.1, 0., 7), false)
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), before);
        let mut invalid = IDENTITY;
        invalid[0] = f32::NAN;
        assert!(
            renderer
                .render_geometry(
                    &frame,
                    &hud,
                    &Geometry {
                        entities: &[],
                        hand: &hand,
                        see_through: &[],
                        hand_view_projection: invalid,
                        nether_lighting: false,
                        glint: GlintParameters::default(),
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), before);
        renderer.update_lightmap(&[0; 1024]).unwrap();
        renderer
            .render_with_entities(&frame, &HudBatch::new([64; 2]), &quad(0.1, -0.5, 0), false)
            .unwrap();
        near(&renderer.read_rgba([32, 32, 1, 1]).unwrap(), [0; 3]);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_shadow_item_mask_text_and_water_obey_classic_submission_order() {
        use crate::mesher::Vertex;
        let mut atlas = HudAtlas::boot();
        atlas.width = 3;
        atlas.height = 1;
        atlas.pixels = vec![255, 0, 0, 128, 0, 0, 0, 16, 0, 0, 0, 0];
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }; 2],
            4,
            1,
            2,
        )
        .unwrap();
        let sprites: [&[u8]; 2] = [&[255; 4], &[255, 255, 255, 128]];
        let pages: Vec<_> = (0..layout.page_count())
            .map(|p| build_page(&layout, p, &sprites).unwrap())
            .collect();
        let refs: Vec<_> = pages.iter().map(Vec::as_slice).collect();
        let limits = Limits {
            hud_page_size: 4,
            gpu: Budget {
                texture_bytes: 1220,
                ..Budget::default()
            },
            ..Limits::default()
        };
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &refs,
            &[255; 1024],
            &atlas,
            limits,
        )
        .unwrap();
        let positions = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]];
        let mut terrain = SectionMesh::default();
        terrain
            .solid
            .extend(positions.map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, 0xff808080, 0xf0f0)));
        terrain.translucent.extend(
            positions
                .map(|p| TerrainVertex::new([p[0], p[1], -0.3], [0.5; 2], 1, 0xffff0000, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &terrain).unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[[0; 3]],
        };
        let entity = |layer, z, u| {
            positions.map(|p| Vertex {
                pos: [p[0], p[1], z],
                uv: [u, 0.5],
                layer,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [0.; 2],
            })
        };
        let mut mask = entity(13, -0.5, f32::NAN);
        for v in &mut mask {
            v.pos[0] = v.pos[0] * 0.5 + 0.5;
            v.color = 0;
        }
        let item = entity(32, -0.2, 0.5 / 3.);
        let shadow = entity(14, 0., 1.5 / 3.);
        // Deliberately opposite to submission order: mask first, item before
        // shadow. Sorting by texture or preserving these phase tags verbatim
        // would hide the item or miss its faint shadow beneath the water.
        let vertices = [mask, item, shadow].concat();
        let hud = HudBatch::new([64; 2]);
        renderer
            .render_with_entities(&frame, &hud, &vertices, false)
            .unwrap();
        let near = |actual: &[u8], expected: [u8; 3]| {
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [94, 30, 158]);
        near(&renderer.read_rgba([48, 32, 1, 1]).unwrap(), [188, 60, 60]);
        assert_eq!(renderer.world.backend_mut().stats().texture_bytes, 1220);
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 376);
        let text = entity(35, 0.0000005, 0.5 / 3.);
        renderer
            .render_with_entities(&frame, &hud, &text, false)
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [96, 32, 160]);
        let mut back = text;
        back.reverse();
        renderer
            .render_with_entities(&frame, &hud, &back, false)
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [64, 64, 192]);
        let names = entity(0, 0.8, 0.5 / 3.);
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &[],
                    hand: &[],
                    see_through: &names,
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [160, 32, 96]);
        let held = entity(32, 0.9, 0.5 / 3.);
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &[],
                    hand: &held,
                    see_through: &names,
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [208, 16, 48]);
        let stable = renderer.read_rgba([16, 32, 1, 1]).unwrap();
        renderer.limits.entity_vertices = 10; // eight input vertices expand to twelve
        assert!(
            renderer
                .render_geometry(
                    &frame,
                    &hud,
                    &Geometry {
                        entities: &[],
                        hand: &held,
                        see_through: &names,
                        hand_view_projection: IDENTITY,
                        nether_lighting: false,
                        glint: GlintParameters::default(),
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([16, 32, 1, 1]).unwrap(), stable);
        renderer.limits.entity_vertices = 12;
        renderer
            .render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: &[],
                    hand: &held,
                    see_through: &names,
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters::default(),
                },
            )
            .unwrap();
        near(&renderer.read_rgba([16, 32, 1, 1]).unwrap(), [208, 16, 48]);
        renderer.limits.entity_vertices = limits.entity_vertices;
        let before = renderer.read_rgba([48, 32, 1, 1]).unwrap();
        assert!(
            renderer
                .render_geometry(
                    &frame,
                    &hud,
                    &Geometry {
                        entities: &[],
                        hand: &mask,
                        see_through: &[],
                        hand_view_projection: IDENTITY,
                        nether_lighting: false,
                        glint: GlintParameters::default(),
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([48, 32, 1, 1]).unwrap(), before);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_glint_hand_reload_rollback_and_exact_page_budget() {
        use crate::mesher::Vertex;
        let mut atlas = HudAtlas::boot();
        atlas.width = 3;
        atlas.height = 1;
        atlas.pixels = [64, 128, 192, 255].repeat(3);
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255; 4]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[128, 255, 64, 255].repeat(256),
            &atlas,
            Limits {
                gpu: Budget {
                    texture_bytes: 1164,
                    mesh_bytes: 516,
                },
                hud_page_size: 4,
                ..Limits::default()
            },
        )
        .unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[],
            translucent: &[],
        };
        let hud = HudBatch::new([64; 2]);
        let quad = |layer| {
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|pos| Vertex {
                pos,
                uv: [0.5 / 3., 0.5],
                layer,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0; 4],
                glint_uv: [-0.25, 1.25],
            })
        };
        let sample = |r: &mut TerrainHudRenderer, expected: [u8; 3]| {
            let actual = r.read_rgba([32, 32, 1, 1]).unwrap();
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        renderer
            .render_with_entities(&frame, &hud, &quad(0), false)
            .unwrap();
        sample(&mut renderer, [32, 128, 48]);
        let before = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        assert!(
            renderer
                .render_with_entities(&frame, &hud, &quad(7), false)
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), before);
        let image = |rgba| Image {
            width: 1,
            height: 1,
            rgba,
        };
        renderer
            .set_glint_textures(image(&[128, 64, 0, 0]), image(&[0, 128, 128, 0]))
            .unwrap();
        let draw = |renderer: &mut TerrainHudRenderer, layer, hand, strength| {
            let vertices = quad(layer);
            renderer.render_geometry(
                &frame,
                &hud,
                &Geometry {
                    entities: if hand { &[] } else { &vertices },
                    hand: if hand { &vertices } else { &[] },
                    see_through: &[],
                    hand_view_projection: IDENTITY,
                    nether_lighting: false,
                    glint: GlintParameters {
                        strength,
                        ticks: 27500.,
                    },
                },
            )
        };
        for hand in [false, true] {
            for (layer, color) in [
                (7, [48, 132, 48]),
                (8, [77, 93, 34]),
                (33, [48, 132, 48]),
                (36, [32, 144, 64]),
            ] {
                draw(&mut renderer, layer, hand, 0.5).unwrap();
                sample(&mut renderer, color);
            }
        }
        draw(&mut renderer, 7, false, 0.).unwrap();
        sample(&mut renderer, [32, 128, 48]);
        let stable = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        for strength in [f32::NAN, -0.1, 1.1] {
            assert!(draw(&mut renderer, 7, false, strength).is_err());
            assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), stable);
        }
        // A half-transparent base becomes opaque with full glint strength.
        let mut patched = HudBatch::new([64; 2]);
        patched.patches.push(crate::font::Patch {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: vec![64, 128, 192, 128],
        });
        renderer
            .render_with_entities(&frame, &patched, &quad(33), false)
            .unwrap();
        sample(&mut renderer, [96, 144, 48]);
        let stable = renderer.read_rgba([32, 32, 1, 1]).unwrap();
        // Stage first image, reject malformed second; active images/frame survive.
        assert!(
            renderer
                .set_glint_textures(
                    image(&[255; 4]),
                    Image {
                        width: 2,
                        height: 2,
                        rgba: &[255; 4]
                    }
                )
                .is_err()
        );
        assert_eq!(renderer.read_rgba([32, 32, 1, 1]).unwrap(), stable);
        draw(&mut renderer, 33, true, 1.).unwrap();
        sample(&mut renderer, [96, 144, 48]);
        // Both images fit, but leave no room for even one atlas page. Roll back
        // BOTH staged handles, retaining the old assets and original cache quota.
        assert!(
            renderer
                .set_glint_textures(
                    Image {
                        width: 2,
                        height: 2,
                        rgba: &[255; 16]
                    },
                    image(&[255; 4])
                )
                .is_err()
        );
        draw(&mut renderer, 7, false, 1.).unwrap();
        sample(&mut renderer, [96, 144, 48]);
        for _ in 0..4 {
            renderer
                .set_glint_textures(image(&[0, 128, 0, 0]), image(&[0, 0, 128, 0]))
                .unwrap();
        }
        // Retired assets must be collected BEFORE the next page preflight/upload:
        // only 64 bytes remain in this deliberately exact texture budget.
        draw(&mut renderer, 33, true, 1.).unwrap();
        sample(&mut renderer, [32, 192, 48]);
        draw(&mut renderer, 36, false, 1.).unwrap();
        sample(&mut renderer, [32, 128, 112]);
        let stats = renderer.world.backend_mut().stats();
        assert_eq!(stats.texture_bytes, 1164);
        assert_eq!(stats.textures, 6);
        assert_eq!(stats.mesh_bytes, 0);
        assert_eq!(stats.meshes, 0);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_gui_foil_masks_pages_patches_and_reloads_with_compact_mesh_budget() {
        let mut atlas = HudAtlas::boot();
        atlas.width = 6;
        atlas.height = 2;
        atlas.pixels = [0, 16, 128, 255, 255, 255]
            .into_iter()
            .flat_map(|a| [255, 255, 255, a])
            .collect::<Vec<_>>()
            .repeat(2);
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[255; 4]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[0; 1024],
            &atlas,
            Limits {
                gpu: Budget {
                    texture_bytes: 1164,
                    mesh_bytes: 324,
                },
                hud_page_size: 4,
                gui_vertices: 256,
                ..Limits::default()
            },
        )
        .unwrap();
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: Some(horizon_render_api::Fog {
                game: None,
                color: [0., 0., 1.],
                start: 0.,
                end: 0.0001,
            }),
            solid: &[],
            translucent: &[],
        };
        let p = [[2., 2.], [62., 2.], [62., 62.], [2., 62.]];
        let mut hud = HudBatch::new([64; 2]);
        hud.glint_alpha = 0.5;
        hud.quad(p, [[4.5 / 6., 0.5]; 4], 0xff302010);
        renderer.render(&frame, &hud).unwrap();
        let stable = renderer.read_rgba([27, 32, 1, 1]).unwrap();
        hud.quad_glint(
            p,
            [[2., 0.], [3., 0.], [3., 1.], [2., 1.]],
            [[0.25, 0.5]; 4],
            0,
        );
        hud.blend_since(6, crate::hud::Blend::Additive);
        assert!(renderer.render(&frame, &hud).is_err());
        assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        let image = |rgba| Image {
            width: 1,
            height: 1,
            rgba,
        };
        renderer
            .set_glint_textures(image(&[128, 64, 0, 0]), image(&[0; 4]))
            .unwrap();
        let near = |r: &mut TerrainHudRenderer, x, expected: [u8; 3]| {
            let actual = r.read_rgba([x, 32, 1, 1]).unwrap();
            for i in 0..3 {
                assert!(
                    (i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2,
                    "{actual:?} != {expected:?}"
                );
            }
        };
        renderer.render(&frame, &hud).unwrap();
        near(&mut renderer, 7, [16, 32, 48]);
        for x in [27, 37, 57] {
            near(&mut renderer, x, [32, 36, 48]);
        }
        hud.patches.push(crate::font::Patch {
            x: 2,
            y: 0,
            width: 1,
            height: 2,
            rgba: [255, 255, 255, 0].repeat(2),
        });
        renderer.render(&frame, &hud).unwrap();
        near(&mut renderer, 27, [16, 32, 48]);
        near(&mut renderer, 37, [32, 36, 48]);
        hud.patches.clear();
        let stable = renderer.read_rgba([37, 32, 1, 1]).unwrap();
        renderer.reload_hud(&atlas).unwrap();
        renderer
            .set_glint_textures(image(&[0, 128, 0, 0]), image(&[0; 4]))
            .unwrap();
        assert_eq!(renderer.read_rgba([37, 32, 1, 1]).unwrap(), stable);
        renderer.render(&frame, &hud).unwrap();
        near(&mut renderer, 27, [16, 48, 48]);
        let stable = renderer.read_rgba([27, 32, 1, 1]).unwrap();
        for bad in [f32::NAN, -0.1, 1.1] {
            hud.glint_alpha = bad;
            assert!(renderer.render(&frame, &hud).is_err());
            assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        }
        hud.glint_alpha = 0.5;
        renderer.limits.gui_vertices = 11;
        assert!(renderer.render(&frame, &hud).is_err());
        renderer.limits.gui_vertices = 12;
        assert!(renderer.render(&frame, &hud).is_err());
        assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        renderer.limits.gui_vertices = 256;
        hud.vertices[7].glint_uv = [f32::NAN; 2];
        assert!(renderer.render(&frame, &hud).is_err());
        hud.vertices[7].glint_uv = [0.25, 0.5];
        hud.vertices[7].uv[0] = 0.5;
        assert!(renderer.render(&frame, &hud).is_err());
        hud.vertices[7].uv[0] = 3.;
        for blend in [crate::hud::Blend::EndPortal, crate::hud::Blend::CitGlint(0)] {
            hud.blends = vec![(6..12, blend)];
            assert!(renderer.render(&frame, &hud).is_err());
            assert_eq!(renderer.read_rgba([27, 32, 1, 1]).unwrap(), stable);
        }
        hud.blends = vec![(6..12, crate::hud::Blend::Additive)];
        renderer.render(&frame, &hud).unwrap();
        let stats = renderer.world.backend_mut().stats();
        assert_eq!(stats.texture_bytes, 1164);
        assert_eq!(stats.textures, 6);
        assert_eq!(stats.mesh_bytes, 0);
        assert_eq!(stats.meshes, 0);
    }
    #[test]
    #[ignore = "requires OSMesa and an explicit GL2 software-driver environment"]
    fn bridge_resize_retains_world_pages_glint_and_rebuilds_hud_at_new_dimensions() {
        let mut atlas = HudAtlas::boot();
        atlas.width = 3;
        atlas.height = 1;
        atlas.pixels = [255; 12].to_vec();
        let layout = pack_atlas(
            &[SpriteSize {
                width: 1,
                height: 1,
            }],
            4,
            1,
            1,
        )
        .unwrap();
        let page = build_page(&layout, 0, &[&[64, 128, 192, 255]]).unwrap();
        let mut renderer = TerrainHudRenderer::new(
            Context::new("osmesa", [64; 2]).unwrap(),
            layout,
            &[&page],
            &[255; 1024],
            &atlas,
            Limits {
                gpu: Budget {
                    texture_bytes: 1164,
                    mesh_bytes: 512,
                },
                hud_page_size: 4,
                ..Limits::default()
            },
        )
        .unwrap();
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(
            [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]]
                .map(|p| TerrainVertex::new(p, [0.5; 2], 0x8000, u32::MAX, 0xf0f0)),
        );
        renderer.upload_section([0; 3], &mesh).unwrap();
        renderer
            .set_glint_textures(
                Image {
                    width: 1,
                    height: 1,
                    rgba: &[0, 128, 0, 0],
                },
                Image {
                    width: 1,
                    height: 1,
                    rgba: &[0; 4],
                },
            )
            .unwrap();
        let item_id = renderer.glint_textures.as_ref().unwrap().item;
        let frame = SceneFrame {
            view_projection: IDENTITY,
            camera: [0.; 3],
            clear: [0., 0., 0., 1.],
            fog: None,
            solid: &[[0; 3]],
            translucent: &[],
        };
        for size in [[64; 2], [96, 48], [40, 96], [128, 32], [64; 2]] {
            renderer.resize(size).unwrap();
            assert_eq!(renderer.size(), size);
            let events = renderer.poll_events().unwrap();
            assert!(events.events.is_empty());
            assert!(!events.close_requested);
            let mut hud = HudBatch::new(size);
            hud.glint_alpha = 1.;
            let p = [
                [2., 2.],
                [size[0] as f32 / 2., 2.],
                [size[0] as f32 / 2., size[1] as f32 / 2.],
                [2., size[1] as f32 / 2.],
            ];
            hud.quad(p, [[0.5 / 3., 0.5]; 4], 0xff302010);
            hud.quad_glint(p, [[2. + 0.5 / 3., 0.5]; 4], [[0.; 2]; 4], 0);
            hud.blend_since(6, crate::hud::Blend::Additive);
            renderer.render(&frame, &hud).unwrap();
            let inside = renderer.read_rgba([8, size[1] - 8, 1, 1]).unwrap();
            for (a, b) in inside[..3].iter().zip([16u8, 96, 48]) {
                assert!(a.abs_diff(b) <= 2, "{inside:?}");
            }
            assert_eq!(
                renderer.read_rgba([size[0] - 2, 2, 1, 1]).unwrap(),
                [64, 128, 192, 255]
            );
            assert!(!renderer.present().unwrap());
            assert_eq!(renderer.resident_sections(), 1);
            assert_eq!(renderer.glint_textures.as_ref().unwrap().item, item_id);
            let stats = renderer.world.backend_mut().stats();
            assert_eq!(stats.texture_bytes, 1164);
            assert_eq!(stats.mesh_bytes, 188);
            assert!(renderer.resize([0, 1]).is_err());
            assert_eq!(renderer.size(), size);
            assert_eq!(
                renderer.read_rgba([size[0] - 2, 2, 1, 1]).unwrap(),
                [64, 128, 192, 255]
            );
        }
        assert!(renderer.unload_section([0; 3]).unwrap());
        assert_eq!(renderer.resident_sections(), 0);
        // Scene unload retires resources; collect at the documented boundary.
        renderer.world.collect_garbage().unwrap();
        assert_eq!(renderer.world.backend_mut().stats().mesh_bytes, 0);
    }
}
