//! The macOS presenter: a Core Animation layer handed the native image, which the window server
//! scales to the strip nearest-pixel (`terrarium_platform::layer`). It is how the platform
//! shows a bitmap, without the full-size picture softbuffer hands it, which macOS
//! colour-matches on the CPU every frame, or a GPU device, whose first use costs far more
//! memory than the strip.

use std::sync::Arc;

use terrarium_compose::frame::{NATIVE_HEIGHT, NativeImage};
use terrarium_compose::scale::{LETTERBOX, StripScale};
use terrarium_platform::layer::{ImageArea, ImageLayer};
use winit::window::Window;

use super::{Backend, PresentError, Presenter, Shown};

pub struct LayerPresenter {
    layer: ImageLayer,
    /// The window the layer is on, kept while it is.
    _window: Arc<Window>,
}

impl LayerPresenter {
    pub fn new(window: &Arc<Window>) -> Result<Self, PresentError> {
        let layer = ImageLayer::attach(&**window, LETTERBOX).map_err(|_| PresentError::NoLayer)?;
        Ok(Self {
            layer,
            _window: Arc::clone(window),
        })
    }
}

impl Presenter for LayerPresenter {
    /// The whole image goes every time: it is a native image, no larger than 840 by 74.
    fn present(
        &mut self,
        frame: &NativeImage,
        scale: &StripScale,
        _whole: bool,
    ) -> Result<Shown, PresentError> {
        let shown = self.layer.show(
            frame.pixels().as_flattened(),
            frame.width() as usize,
            NATIVE_HEIGHT as usize,
            area(frame, scale),
        );
        if shown {
            Ok(Shown::Presented)
        } else {
            Err(PresentError::NoImage)
        }
    }

    fn backend(&self) -> Backend {
        Backend::Layer
    }
}

/// Where `frame` goes at `scale`: from `image_x`, its columns at the strip's scale, which the
/// window's edge cuts as `upscale_into` cuts at the strip's.
fn area(frame: &NativeImage, scale: &StripScale) -> ImageArea {
    ImageArea {
        x: scale.image_x,
        width: frame.width().saturating_mul(scale.scale),
        height: scale.height,
    }
}

#[cfg(test)]
mod tests {
    use terrarium_compose::scale::SizeStep;

    use super::*;

    fn area_at(width: u32) -> ImageArea {
        let scale = StripScale::for_physical_width(width, SizeStep::Full);
        let frame = NativeImage::new(scale.viewport(), [0, 0, 0, u8::MAX]);
        area(&frame, &scale)
    }

    #[test]
    fn the_image_is_scaled_by_the_strip_s_integer_from_where_it_starts() {
        // 2940 at 4×: 735 columns, exactly the window's width.
        assert_eq!(
            area_at(2940),
            ImageArea {
                x: 0,
                width: 2940,
                height: 296
            }
        );
        // 1301 at 2×: 651 columns run one pixel past the window, which cuts the last.
        assert_eq!(area_at(1301).width, 1302);
        // 1683 at 2×: the widest viewport, letterboxed either side.
        assert_eq!(
            area_at(1683),
            ImageArea {
                x: 1,
                width: 1680,
                height: 148
            }
        );
    }
}
