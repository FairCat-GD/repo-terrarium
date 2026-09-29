//! Where the strip goes on a screen. Pure arithmetic, so it is tested on every OS.

/// A rectangle in `AppKit` screen coordinates: points, origin at the bottom-left of the primary
/// screen, y growing upward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }
}

/// The screen edge the strip sits on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    Bottom,
    Top,
}

/// The strip's frame on a screen, centred horizontally.
///
/// At the bottom it sits on the screen's own edge, where the Dock may cover the soil band; the
/// Dock is not fought. At the top it sits under the menu bar, which would otherwise cover the
/// sky: `visible` is the screen minus the menu bar and the Dock, and the Dock is never on top.
pub fn strip_frame(screen: Rect, visible: Rect, edge: Edge, width: f64, height: f64) -> Rect {
    let y = match edge {
        Edge::Bottom => screen.y,
        Edge::Top => visible.max_y() - height,
    };
    Rect {
        x: screen.x + (screen.width - width) / 2.0,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1512×982 pt screen with a 33 pt menu bar and a 70 pt Dock at the bottom.
    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1512.0,
        height: 982.0,
    };
    const VISIBLE: Rect = Rect {
        x: 0.0,
        y: 70.0,
        width: 1512.0,
        height: 879.0,
    };

    #[test]
    fn the_bottom_strip_sits_on_the_screen_edge_under_the_dock() {
        assert_eq!(
            strip_frame(SCREEN, VISIBLE, Edge::Bottom, 1512.0, 148.0),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1512.0,
                height: 148.0
            }
        );
    }

    #[test]
    fn the_top_strip_sits_under_the_menu_bar() {
        assert_eq!(
            strip_frame(SCREEN, VISIBLE, Edge::Top, 1512.0, 148.0),
            Rect {
                x: 0.0,
                y: 949.0 - 148.0,
                width: 1512.0,
                height: 148.0
            }
        );
    }

    #[test]
    fn a_screen_left_of_and_below_the_primary_keeps_its_own_origin() {
        let screen = Rect {
            x: -1920.0,
            y: -300.0,
            width: 1920.0,
            height: 1080.0,
        };
        let strip = |y| Rect {
            x: -1920.0,
            y,
            width: 1920.0,
            height: 222.0,
        };
        assert_eq!(
            strip_frame(screen, screen, Edge::Bottom, 1920.0, 222.0),
            strip(-300.0)
        );
        assert_eq!(
            strip_frame(screen, screen, Edge::Top, 1920.0, 222.0),
            strip(780.0 - 222.0)
        );
    }

    #[test]
    fn a_strip_wider_than_its_screen_is_centred_on_it() {
        let screen = Rect {
            x: 100.0,
            y: 0.0,
            width: 600.0,
            height: 400.0,
        };
        assert_eq!(
            strip_frame(screen, screen, Edge::Bottom, 640.0, 74.0),
            Rect {
                x: 80.0,
                y: 0.0,
                width: 640.0,
                height: 74.0
            }
        );
    }
}
