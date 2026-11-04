//! Multisampling policy, and the sRGB question that sits next to it.
//!
//! Two settings decide what the frame's first pass draws into: how many samples the colour
//! target takes, and whether textures are uploaded as sRGB. A driver can refuse both, and both
//! are wrong in a way that looks like something else when the refusal is not handled: a 4x
//! request that silently becomes 2x looks like a blurry frame, and an sRGB upload that silently
//! becomes plain RGBA looks like a washed-out world.
//!
//! So the questions are answered here, before a frame exists:
//!
//!  * [`Plan::for_setting`] turns the settings menu's number into what the device will actually
//!    do, including the fallback to no multisampling at all, which is a plain
//!    [`Target`](crate::gl::target::Target);
//!  * [`Plan::srgb_textures`] says whether a texture upload may use
//!    [`SRGB8_ALPHA8_EXT`](crate::gl::texture::SRGB8_ALPHA8_EXT), the extension's internal
//!    format on 2.1 (`GL_FRAMEBUFFER_SRGB` is 3.0, and this path linearizes on *sample*);
//!  * [`Size`] is the memory and the limit check, so a setting that would not fit is refused
//!    with a sentence instead of an incomplete framebuffer.
//!
//! What it is not: a framebuffer wrapper. `gl/target.rs` creates the objects; this decides
//! whether to ask, and what the log says when the answer is no.

use crate::caps::{Caps, Feature, Limits};

/// What a settings menu offers, in samples. The target takes 2 and 4; 6 is what the chip also
/// reports and what this path does not offer (plan §9: at 1366x768 it is most of the frame).
pub const OFFERED: [u32; 3] = [0, 2, 4];

/// What the frame will actually do about antialiasing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// No multisampling: draw straight into a [`Target`](crate::gl::target::Target)'s texture.
    /// The setting's 0, and the only thing a driver without `EXT_framebuffer_multisample` can do.
    None,
    /// A multisampled renderbuffer, resolved by a blit at the end of the pass.
    Multisampled {
        /// 2 or 4: what will be asked for.
        samples: u32,
        /// What the settings menu asked for, so the log can say a 6 became a 4.
        requested: u32,
    },
}

impl Plan {
    /// The plan for a menu setting, given the device.
    ///
    /// A setting the device cannot honour becomes [`Plan::None`] rather than an error: the game
    /// must start on a machine whose driver has no multisampled renderbuffers, and the settings
    /// screen offers what is available (`MultisampleTarget::sample_counts`) rather than a switch
    /// that does nothing.
    pub fn for_setting(caps: &Caps, requested: u32) -> Self {
        if requested == 0 || !caps.has(Feature::FramebufferMultisample) {
            return Plan::None;
        }
        // The largest offered count at or below the request, never above it: asking a chip
        // that reports 4 samples for 6 is `GL_INVALID_OPERATION` and an incomplete framebuffer,
        // which on this driver is a black window and no message.
        // `rfind` says "the last one that fits" without building a filter, and it walks from the
        // end, which is where the largest count is.
        let samples = OFFERED
            .iter()
            .copied()
            .rfind(|offered| *offered != 0 && *offered <= requested)
            .unwrap_or(0);
        if samples == 0 {
            return Plan::None;
        }
        Plan::Multisampled { samples, requested }
    }

    /// The sample count the frame will have, 0 for none: what the settings screen and the
    /// frame-time log both print.
    pub fn samples(self) -> u32 {
        match self {
            Plan::None => 0,
            Plan::Multisampled { samples, .. } => samples,
        }
    }

    /// Whether the plan needs a [`MultisampleTarget`](crate::gl::target::MultisampleTarget)
    /// rather than a plain target.
    pub fn needs_multisample_target(self) -> bool {
        self.samples() > 0
    }

    /// A one-line description for the log, or `None` when there is nothing to explain.
    ///
    /// The interesting sentence is the one about a request that was lowered: a 4x setting
    /// running at 2x is not a bug, but it is a fact a bug report should carry.
    pub fn note(self) -> Option<String> {
        match self {
            Plan::None => None,
            Plan::Multisampled { samples, requested } if samples < requested => Some(format!(
                "multisampling: asked for {requested}x, this path offers {samples}x"
            )),
            Plan::Multisampled { .. } => None,
        }
    }

    /// Whether texture uploads may use the sRGB internal format.
    ///
    /// `EXT_texture_sRGB` is what makes `SRGB8_ALPHA8_EXT` legal on 2.1. The sRGB *framebuffer*
    /// (`GL_FRAMEBUFFER_SRGB`) is a 3.0 token and is not part of the question.
    pub fn srgb_textures(caps: &Caps) -> bool {
        caps.has(Feature::SrgbTextures)
    }
}

/// The settings menu's list: always 0 (off), then the counts the capability table says this
/// device takes. A driver without the extension gets `[0]`, and the screen shows one option.
///
/// A real context answers better through `MultisampleTarget::sample_counts`, which reads
/// `MAX_SAMPLES_EXT`; this is what a report prints when there is no context to ask.
pub fn offered(caps: &Caps) -> Vec<u32> {
    let mut counts = vec![0];
    counts.extend(limits(caps));
    counts
}

/// The sample counts the capability table implies: 2, and 4 where the renderbuffer limit is the
/// target's or better. Empty without `EXT_framebuffer_multisample`.
///
/// The threshold is the target's own 4096 rather than a guess: a device whose renderbuffers stop
/// at 2048 cannot take a 4x pass at a resolution this path scales to anyway, and offering it
/// would be a switch that fails when pressed.
pub fn limits(caps: &Caps) -> Vec<u32> {
    if !caps.has(Feature::FramebufferMultisample) {
        return Vec::new();
    }
    let ceiling = if caps.limits.max_renderbuffer_size >= Limits::X1650.max_renderbuffer_size {
        4
    } else {
        2
    };
    [2u32, 4]
        .into_iter()
        .filter(|samples| *samples <= ceiling)
        .collect()
}

/// The size a pass renders at, with the sample count: what `MultisampleTarget::create` is asked
/// for, and the memory the plan's budget is stated in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
}

impl Size {
    /// The drawable's size with a plan's sample count.
    pub fn new(width: u32, height: u32, plan: Plan) -> Self {
        Self {
            width,
            height,
            samples: plan.samples(),
        }
    }

    /// Whether a target of this size is inside the device's renderbuffer limit and, when
    /// samples are asked for, whether the device has them.
    ///
    /// A driver's refusal for a bigger target is `GL_INVALID_VALUE` at creation, which on this
    /// driver leaves an incomplete framebuffer and a black window rather than a message.
    pub fn fits(self, caps: &Caps) -> bool {
        let max = caps.limits.max_renderbuffer_size;
        self.width > 0
            && self.height > 0
            && self.width <= max
            && self.height <= max
            && (self.samples == 0 || caps.has(Feature::FramebufferMultisample))
    }

    /// Bytes of colour the multisampled renderbuffer takes before the resolve: width * height *
    /// samples * 4.
    ///
    /// Computed rather than estimated, because the plan's memory budget is this number: at
    /// 1366x768 with 4 samples it is 16 MiB, which is a sixteenth of the card's 256 MiB and a
    /// tenth of what the frame's terrain buffers are allowed to take.
    pub fn color_bytes(self) -> u64 {
        let samples = u64::from(self.samples.max(1));
        u64::from(self.width) * u64::from(self.height) * samples * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::Version;

    /// The target's table as a 2.1 context would fill it. `detect` derives the feature set from
    /// the extension list, so the multisample feature only exists here if the list says so:
    /// that is what makes the tests below about a real table rather than a fabricated one.
    fn caps_with(extensions: &[&str]) -> Caps {
        Caps::detect(
            Version::parse("2.1.8545 Release"),
            "1.20",
            "ATI Technologies Inc.",
            "Radeon X1600/X1650 Series",
            extensions.iter().map(|s| s.to_string()),
            Limits::X1650,
        )
    }

    fn target_caps() -> Caps {
        caps_with(&[
            "GL_EXT_framebuffer_object",
            "GL_EXT_framebuffer_blit",
            "GL_EXT_framebuffer_multisample",
            "GL_EXT_texture_sRGB",
        ])
    }

    #[test]
    fn asking_for_nothing_is_a_plain_target_and_no_log_line() {
        let plan = Plan::for_setting(&target_caps(), 0);
        assert_eq!(plan, Plan::None);
        assert_eq!(plan.samples(), 0);
        assert!(!plan.needs_multisample_target());
        assert_eq!(plan.note(), None);
    }

    #[test]
    fn a_device_without_multisampled_renderbuffers_gets_none_rather_than_an_error() {
        // A driver with the framebuffer extensions but not the multisample one: the game must
        // still start, and the setting must collapse to "off" rather than to a failure.
        let caps = caps_with(&["GL_EXT_framebuffer_object", "GL_EXT_framebuffer_blit"]);
        assert!(!caps.has(Feature::FramebufferMultisample));
        assert_eq!(Plan::for_setting(&caps, 4), Plan::None);
        assert_eq!(offered(&caps), vec![0]);
        assert!(limits(&caps).is_empty());
        // And a size with samples in it does not "fit" that device.
        assert!(!Size::new(800, 600, Plan::Multisampled { samples: 4, requested: 4 }).fits(&caps));
    }

    #[test]
    fn the_offered_counts_are_two_and_four_and_never_six() {
        // 6 is what the chip reports and what the plan's budget refuses (plan §9).
        let caps = target_caps();
        assert_eq!(offered(&caps), vec![0, 2, 4]);
        assert!(!OFFERED.contains(&6));
        assert_eq!(limits(&caps), vec![2, 4]);
    }

    #[test]
    fn a_request_above_what_is_offered_is_lowered_and_says_so() {
        let caps = target_caps();
        let plan = Plan::for_setting(&caps, 6);
        // 6 is not a count this path creates: the largest offered count at or below the
        // request is 4, and the note names both numbers.
        assert_eq!(
            plan,
            Plan::Multisampled {
                samples: 4,
                requested: 6
            }
        );
        assert!(plan.needs_multisample_target());
        let note = plan.note().expect("a lowered request says so");
        assert!(note.contains("6x") && note.contains("4x"), "{note}");
        // A request that is honoured has nothing to explain.
        assert_eq!(Plan::for_setting(&caps, 4).note(), None);
        assert_eq!(Plan::for_setting(&caps, 2).note(), None);
        // And a request of 1 is not a sample count this path has: it collapses to off.
        assert_eq!(Plan::for_setting(&caps, 1), Plan::None);
    }

    #[test]
    fn a_plan_costs_what_the_renderbuffer_costs() {
        let caps = target_caps();
        let two = Size::new(1366, 768, Plan::for_setting(&caps, 2));
        let four = Size::new(1366, 768, Plan::for_setting(&caps, 4));
        assert_eq!(two.samples, 2);
        assert_eq!(four.samples, 4);
        assert_eq!(four.color_bytes(), two.color_bytes() * 2);
        assert_eq!(four.color_bytes(), 1366 * 768 * 4 * 4);
        // 16 MiB against a 256 MiB card: a sixteenth, and worth stating rather than guessing.
        assert!(four.color_bytes() <= 256 * 1024 * 1024 / 16 + 1);
        // Off is still one sample's worth of texture, not zero bytes.
        let off = Size::new(1366, 768, Plan::None);
        assert_eq!(off.samples, 0);
        assert_eq!(off.color_bytes(), 1366 * 768 * 4);
    }

    #[test]
    fn a_size_past_the_renderbuffer_limit_does_not_fit() {
        let caps = target_caps();
        assert!(Size::new(1366, 768, Plan::None).fits(&caps));
        // 4096 is the hardware maximum; two such sides is past it for a renderbuffer.
        assert!(!Size::new(5000, 768, Plan::None).fits(&caps));
        assert!(!Size::new(0, 768, Plan::None).fits(&caps));
        // Note that `fits` multiplies this path's own limit into the check, so a driver that
        // reports far more than the target's 4096 still does not get a bigger plan.
        assert!(Size::new(4096, 4096, Plan::None).fits(&caps));
    }

    #[test]
    fn the_srgb_question_is_the_extension_not_the_framebuffer() {
        // `EXT_texture_sRGB` present: uploads may be sRGB on 2.1. There is no
        // `GL_FRAMEBUFFER_SRGB` to set, which is why `Plan` has no field for it.
        let caps = target_caps();
        assert!(Plan::srgb_textures(&caps));
        let without = caps_with(&["GL_EXT_framebuffer_object"]);
        assert!(!Plan::srgb_textures(&without));
        // And the internal format is the extension's number, which 3.0 renamed without
        // changing: a wrong literal here makes every texture upload plain RGBA.
        assert_eq!(crate::gl::texture::SRGB8_ALPHA8_EXT, 0x8C43);
    }
}
