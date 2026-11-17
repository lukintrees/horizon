//! Screen-space SDR effects sample an independent sprite, not a repeating atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Alpha,
    /// ZERO, ONE_MINUS_SRC_COLOR; alpha is ignored and destination alpha kept.
    Vignette,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Material {
    pub blend: Blend,
    pub linear: bool,
    pub clamp: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pass;
    #[test]
    fn sampler_variants_keep_depthless_alpha_or_vignette_pipeline() {
        for linear in [false, true] {
            for clamp in [false, true] {
                for blend in [Blend::Alpha, Blend::Vignette] {
                    let pass = Pass::Screen(Material {
                        blend,
                        linear,
                        clamp,
                    });
                    assert!(pass.is_overlay());
                    assert_eq!(
                        pass.pipeline(),
                        match blend {
                            Blend::Alpha => Pass::Overlay,
                            Blend::Vignette => Pass::OverlayVignette,
                        }
                    );
                }
            }
        }
        assert!(!Pass::Opaque.is_overlay());
        assert_eq!(Pass::OverlayVignette.pipeline(), Pass::OverlayVignette);
    }
}
