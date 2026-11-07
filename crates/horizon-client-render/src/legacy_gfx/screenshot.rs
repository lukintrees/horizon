//! Native screenshot command state and PNG encoding. No persistent readback
//! buffers: a request completes once, after draw and before swap/present.
use horizon_render_api::{render_contract::ScreenshotRequest, screenshot::Plan};
use std::io::Write;

pub(crate) type Completion = Result<[u32; 2], String>;
pub(crate) struct State {
    pub pending: Option<ScreenshotRequest>,
    pub result: Option<Completion>,
    budget: usize,
}
impl State {
    pub fn new(budget: usize) -> Self {
        Self {
            pending: None,
            result: None,
            budget,
        }
    }
    pub fn plan(&self, request: &ScreenshotRequest, source: [u32; 2]) -> Result<Plan, String> {
        // A fixed-size icon ignores `scaled`, matching the modern adapter.
        // Supersampled native captures need a separate render target, not a
        // silent lower-resolution substitute for the requested screenshot.
        if request.scaled && request.size.is_none() {
            return Err(
                "legacy GL2 scaled screenshots are not implemented; request native resolution"
                    .into(),
            );
        }
        Plan::new(source, request.size, self.budget)
            .map_err(|e| format!("legacy screenshot: {e:?}"))
    }
    pub fn request(&mut self, request: ScreenshotRequest, source: [u32; 2]) -> Result<(), String> {
        if self.pending.is_some() || self.result.is_some() {
            return Err("legacy screenshot request or completion is still outstanding".into());
        }
        self.plan(&request, source)?;
        self.pending = Some(request);
        Ok(())
    }
}

pub(super) fn write(request: &ScreenshotRequest, plan: &Plan, pixels: Vec<u8>) -> Completion {
    let pixels = plan
        .prepare(pixels)
        .map_err(|e| format!("legacy screenshot: {e:?}"))?;
    let [width, height] = plan.size();
    let mut file =
        std::io::BufWriter::new(std::fs::File::create(&request.path).map_err(|e| e.to_string())?);
    let mut encoder = png::Encoder::new(&mut file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&pixels)
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    Ok([width, height])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(size: Option<[u32; 2]>, scaled: bool) -> ScreenshotRequest {
        ScreenshotRequest {
            path: "not-written-by-state-test.png".into(),
            size,
            scaled,
        }
    }
    #[test]
    fn queue_is_bounded_and_rejected_requests_never_replace_pending_or_results() {
        let mut state = State::new(32);
        assert!(state.request(request(None, true), [2, 2]).is_err());
        assert!(state.request(request(Some([0, 1]), false), [2, 2]).is_err());
        assert!(state.request(request(None, false), [4, 4]).is_err());
        assert!(state.pending.is_none());
        state.request(request(None, false), [2, 2]).unwrap();
        assert!(state.request(request(Some([1, 1]), false), [2, 2]).is_err());
        assert!(state.pending.take().unwrap().size.is_none());
        state.result = Some(Err("readback failure".into()));
        assert!(state.request(request(None, false), [2, 2]).is_err());
        assert_eq!(state.result.take(), Some(Err("readback failure".into())));
        state.request(request(Some([1, 1]), true), [2, 2]).unwrap();
        assert_eq!(state.pending.take().unwrap().size, Some([1, 1]));
    }
}
