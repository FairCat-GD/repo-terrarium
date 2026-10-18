//! macOS: the strip's picture shown as the platform shows a bitmap, in a Core Animation layer.
//! The layer is handed the native image, a few hundred columns by 74 rows, tagged sRGB as the
//! compositor makes it, in a window whose backing is sRGB too, and the window server scales it
//! to the strip with nearest sampling (I-11) and matches its colours to the display. Nothing is
//! scaled, colour-matched or drawn with the GPU in the app, and the process holds no GPU
//! device, whose first use alone costs some 80 MB on Apple silicon.

use objc2::rc::Retained;
use objc2_app_kit::NSColorSpace;
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapInfo, CGColor, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
    CGImageAlphaInfo, CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_quartz_core::{CALayer, CATransaction, kCAFilterNearest, kCAGravityResize};
use raw_window_handle::HasWindowHandle;

use crate::macos::{PlatformError, appkit_view};

/// Where the image goes in the window, in its physical pixels from the top left: its left
/// edge and width (the image may run past the window's right edge, which cuts it), and its
/// height. Beside and below it is the letterbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageArea {
    pub x: u32,
    pub width: u32,
    pub height: u32,
}

/// A layer over a window's view that shows the images it is given.
pub struct ImageLayer {
    layer: Retained<CALayer>,
    space: CFRetained<CGColorSpace>,
    /// Whether the view's layer counts y from the top, as winit's flipped view does.
    flipped: bool,
}

impl ImageLayer {
    /// A layer on `window`'s view, which is filled with `letterbox` (RGBA) wherever no image
    /// lies.
    pub fn attach(
        window: &impl HasWindowHandle,
        letterbox: [u8; 4],
    ) -> Result<ImageLayer, PlatformError> {
        let view = appkit_view(window)?;
        // The window's backing is sRGB, as the images are, so Core Animation takes them as
        // they are, and the window server matches the window to the display on the GPU, as it
        // does every window's.
        view.window()
            .ok_or(PlatformError::NoWindow)?
            .setColorSpace(Some(&NSColorSpace::sRGBColorSpace()));
        view.setWantsLayer(true);
        let root = view.layer().ok_or(PlatformError::NoLayer)?;
        // SAFETY: `kCGColorSpaceSRGB` is a constant `CFString` the framework defines, read
        // only.
        let name = unsafe { kCGColorSpaceSRGB };
        let space = CGColorSpace::with_name(Some(name)).ok_or(PlatformError::NoLayer)?;
        let [r, g, b, a] = letterbox.map(|channel| f64::from(channel) / 255.0);
        root.setBackgroundColor(Some(&CGColor::new_srgb(r, g, b, a)));
        root.setMasksToBounds(true);
        // A presenter that could not start may have left a layer of its own here (wgpu's
        // Metal layer, made with its surface): nothing is drawn over the picture but it.
        // SAFETY: no sublayers is a valid value; the view's own layer is not among them.
        unsafe { root.setSublayers(None) };
        let layer = CALayer::new();
        // SAFETY: the filter's name is a constant `NSString` the framework defines, read only.
        let nearest = unsafe { kCAFilterNearest };
        // SAFETY: as is the gravity's.
        let resize = unsafe { kCAGravityResize };
        layer.setMagnificationFilter(nearest);
        layer.setMinificationFilter(nearest);
        layer.setContentsGravity(resize);
        layer.setOpaque(true);
        // From the top left, as the strip's pixels are counted.
        layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
        root.addSublayer(&layer);
        Ok(ImageLayer {
            layer,
            space,
            flipped: root.isGeometryFlipped(),
        })
    }

    /// Places the images over `area` of a window `window_height` physical pixels tall, at
    /// `backing` pixels a point: only when the strip is placed or rescaled, not every frame.
    pub fn place(&self, area: ImageArea, window_height: u32, backing: f64) {
        let points = |pixels: u32| f64::from(pixels) / backing;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.layer.setContentsScale(backing);
        self.layer.setFrame(CGRect::new(
            CGPoint::new(
                points(area.x),
                top(self.flipped, points(window_height), points(area.height)),
            ),
            CGSize::new(points(area.width), points(area.height)),
        ));
        CATransaction::commit();
    }

    /// Shows `rgba`, `width` by `height` pixels in rows of red, green, blue and an ignored
    /// fourth byte, where [`ImageLayer::place`] put it. False if the OS could not make an
    /// image of it.
    pub fn show(&self, rgba: &[u8], width: usize, height: usize) -> bool {
        let data = CFData::from_bytes(rgba);
        let Some(provider) = CGDataProvider::with_cf_data(Some(&data)) else {
            return false;
        };
        let info =
            CGBitmapInfo(CGImageAlphaInfo::NoneSkipLast.0 | CGImageByteOrderInfo::OrderDefault.0);
        // SAFETY: the provider holds `width × height` pixels of four bytes, rows `width × 4`
        // bytes apart, which the sizes passed describe; there is no decode array.
        let image = unsafe {
            CGImage::new(
                width,
                height,
                8,
                32,
                width * 4,
                Some(&self.space),
                info,
                Some(&provider),
                std::ptr::null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        };
        let Some(image) = image else {
            return false;
        };
        // Without actions: a new picture replaces the last at once, with no fade.
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // SAFETY: a `CGImage` is a valid `contents` for a layer.
        unsafe { self.layer.setContents(Some(image.as_ref())) };
        CATransaction::commit();
        true
    }
}

impl Drop for ImageLayer {
    fn drop(&mut self) {
        self.layer.removeFromSuperlayer();
    }
}

/// Where the image's top lies, in points, in a layer `height` tall: the top itself when y
/// counts down from it, `height − image` when y counts up from the bottom.
fn top(flipped: bool, height: f64, image: f64) -> f64 {
    if flipped { 0.0 } else { height - image }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_starts_at_the_window_s_top_however_y_is_counted() {
        // A window still 200 points tall around a strip 148 tall.
        assert!(top(true, 200.0, 148.0).abs() < f64::EPSILON);
        assert!((top(false, 200.0, 148.0) - 52.0).abs() < f64::EPSILON);
    }
}
