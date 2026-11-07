//! The shader-pack payload: the per-draw entity spans and the values only the
//! game knows that a pack's uniforms read. `gfx::pack_draw` and
//! `gfx::pack_uniforms` re-export these types; the bind layouts, uniform
//! buffers and encoders that consume them stay there.
use glam::Vec3;

/// Where a run of `Frame::entities` begins, and what it is (slice 7): the run lasts until the
/// next span. Recorded only while a shader pack draws entities, so Horizon's own frames carry
/// none; Horizon itself still draws the whole list at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntitySpan {
    /// The first vertex.
    pub start: u32,
    /// The render stage (5.4: `ENTITIES`, `BLOCK_ENTITIES`, `PARTICLES`, `DESTROY`), which picks
    /// the program.
    pub stage: u32,
    /// `entityId` for entities, `blockEntityId` for block entities (5.3), else 0.
    pub id: i32,
    /// An entity part that blends (a name tag, a shadow): `gbuffers_entities_translucent`.
    pub translucent: bool,
    /// Hidden from the camera (Horizon's entity culling, the player in first person): only the
    /// shadow pass draws it.
    pub hidden: bool,
    /// The local player (`shadowPlayer`).
    pub player: bool,
    /// A block entity whose block emits light (`shadowLightBlockEntities`).
    pub emissive: bool,
}

impl EntitySpan {
    /// Marks vertices `start..end` of `spans`' list (one entity) as hidden: the span covering
    /// `start` is split there, the spans inside are hidden, and the geometry after `end` goes
    /// on in the last span's kind. False when no span covers `start`.
    pub fn hide(spans: &mut Vec<EntitySpan>, start: usize, end: usize) -> bool {
        let Some(i) = spans.iter().rposition(|s| s.start as usize <= start) else {
            return false;
        };
        let after = spans[spans.len() - 1];
        if spans[i].start as usize != start {
            let split = EntitySpan {
                start: start as u32,
                ..spans[i]
            };
            spans.insert(i + 1, split);
        }
        for s in spans.iter_mut().skip(i) {
            if s.start as usize >= start {
                s.hidden = true;
            }
        }
        if end > start {
            spans.push(EntitySpan {
                start: end as u32,
                ..after
            });
        }
        true
    }
}

/// What the game passes for the pack's uniforms, beyond what `Frame` carries.
#[derive(Clone, Copy, Debug, Default)]
pub struct PackInputs {
    /// `Level.getDayTime`.
    pub day_time: i64,
    /// The interpolated rain and thunder levels, 0..1.
    pub rain: f32,
    pub thunder: f32,
    /// The fluid at the camera: 0 none, 1 water, 2 lava, 3 powder snow.
    pub eye_in: i32,
    pub blindness: f32,
    /// Block and sky light at the camera, 0..15.
    pub eye_light: [i32; 2],
    pub hide_gui: bool,
    /// The brightness option, 0..1.
    pub brightness: f32,
    /// The biome at the camera (3.4): registry id, `CAT_*`, `PPT_*`, temperature, downfall.
    pub biome: i32,
    pub biome_category: i32,
    pub biome_precipitation: i32,
    pub temperature: f32,
    pub rainfall: f32,
    /// Whether the biome at the camera is known (its chunk is loaded).
    pub biome_known: bool,
    /// `heldItemId`, `heldItemId2` (item.properties, 5.3) and `heldBlockLightValue(2)`.
    pub held_items: [i32; 2],
    pub held_light: [i32; 2],
    /// `isRightHanded` (the Main Hand option) and `is_on_ground`.
    pub right_handed: bool,
    pub on_ground: bool,
    /// The dimension type's lowest block (`bedrockLevel`), build height (`heightLimit`),
    /// logical height, sky light and ceiling.
    pub min_y: i32,
    pub height: i32,
    pub logical_height: i32,
    pub has_skylight: bool,
    pub has_ceiling: bool,
    /// `isElytraFlying`: the player is fall flying.
    pub elytra_flying: bool,
    /// `cloudHeight`, see [`cloud_height`].
    pub cloud_height: f32,
    /// `cloudTime`: how far the game's clouds have moved along X, in blocks (`clouds::drift`
    /// times vanilla's 0.03 blocks a tick). Iris 1.11.7 gives it, though ShaderDoc doesn't list
    /// it: observed with a probe pack, three game times 0.03 apart per tick.
    pub cloud_time: f32,
    /// The values native mods hand their packs (`frame.mods`, `native_shader_params`).
    pub mod_params: [[f32; 4]; 8],
    /// The player's state, for Iris's status uniforms.
    pub status: PlayerStatus,
    /// The End flash: its intensity, 0..1 (0 outside the End), and its direction from the
    /// camera in world space, of length 100 (`endFlashIntensity`, `endFlashPosition`).
    pub end_flash: f32,
    pub end_flash_direction: Vec3,
    /// The dimension type's `ambient_light` (`ambientLight`).
    pub ambient_light: f32,
    /// The player's cave mood, 0..1 (`BiomeAmbientSoundsHandler`'s moodiness):
    /// `playerMood` and `constantMood`.
    pub mood: f32,
}

/// The local player's state behind Iris's status uniforms (`currentPlayerHealth`,
/// `isSpectator`, `is_burning`...).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerStatus {
    /// Survival or adventure: the vitals are -1 in creative and spectator.
    pub survival: bool,
    pub health: f32,
    pub max_health: f32,
    pub food: f32,
    /// Air supply ticks, of 300.
    pub air: f32,
    /// Armor points (the `armor` attribute).
    pub armor: f32,
    pub spectator: bool,
    pub riding: bool,
    pub feet_in_water: bool,
    pub swimming: bool,
    pub burning: bool,
    pub hurt: bool,
    pub invisible: bool,
    pub sneaking: bool,
    pub sprinting: bool,
}

/// The `cloudHeight` uniform. Vanilla reads `visual/cloud_height` only while the cloud color
/// has alpha (`LevelExtractor`); a dimension without clouds has no height, which the uniform
/// reports as NaN. NaN is not vanilla's own value: packs map it to "no clouds".
pub fn cloud_height(cloud_color_alpha: f32, height: f32) -> f32 {
    if crate::clouds::has_color(cloud_color_alpha) {
        height
    } else {
        f32::NAN
    }
}
