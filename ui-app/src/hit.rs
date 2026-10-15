//! Which part of the hosted shell a point is on: the platform hosts turn this into
//! moving, resizing or plain client input.

use poly_ui::draw::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The transparent shadow: inert.
    Nowhere,
    Client,
    /// Title bar without its buttons: moves the window.
    Caption,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Hit {
    /// Resize direction `[x, y]` (the macOS host resizes itself; Windows does it from
    /// `WM_NCHITTEST`): -1 moves the left/top edge, 1 the right/bottom edge.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn edges(self) -> Option<[i8; 2]> {
        Some(match self {
            Hit::Left => [-1, 0],
            Hit::Right => [1, 0],
            Hit::Top => [0, -1],
            Hit::Bottom => [0, 1],
            Hit::TopLeft => [-1, -1],
            Hit::TopRight => [1, -1],
            Hit::BottomLeft => [-1, 1],
            Hit::BottomRight => [1, 1],
            Hit::Nowhere | Hit::Client | Hit::Caption => return None,
        })
    }
}

/// Window part under `p` (client pixels) for a window of `size` whose shell sits `margin`
/// pixels inside its edge; `areas` = shell window, caption and grip of the last frame.
/// A band from 8 pt outside to 4 pt inside the shell edge resizes (16 pt at the corners).
pub fn hit_test(p: [f32; 2], size: [f32; 2], margin: f32, scale: f32, areas: [Rect; 3]) -> Hit {
    let [window, caption, grip] = areas;
    let (m, outside, inside, corner) = (margin, 8.0 * scale, 4.0 * scale, 16.0 * scale);
    let [w, h] = size;
    let band = |v: f32, edge: f32, sign: f32| (-outside..inside).contains(&((v - edge) * sign));
    let [x, y] = p;
    let span_y = (m - outside..h - m + outside).contains(&y);
    let span_x = (m - outside..w - m + outside).contains(&x);
    let left = span_y && band(x, m, 1.0);
    let right = span_y && band(x, w - m, -1.0);
    let top = span_x && band(y, m, 1.0);
    let bottom = span_x && band(y, h - m, -1.0);
    let near_left = x < m + corner;
    let near_right = x > w - m - corner;
    let near_top = y < m + corner;
    let near_bottom = y > h - m - corner;
    match () {
        _ if (top && near_left) || (left && near_top) => Hit::TopLeft,
        _ if (top && near_right) || (right && near_top) => Hit::TopRight,
        _ if (bottom && near_left) || (left && near_bottom) => Hit::BottomLeft,
        _ if (bottom && near_right) || (right && near_bottom) => Hit::BottomRight,
        _ if left => Hit::Left,
        _ if right => Hit::Right,
        _ if top => Hit::Top,
        _ if bottom => Hit::Bottom,
        _ if grip.contains(p) => Hit::BottomRight,
        _ if caption.contains(p) => Hit::Caption,
        _ if window.contains(p) => Hit::Client,
        // The shadow: inert.
        _ => Hit::Nowhere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_edges_corners_caption_and_shadow() {
        // 1000×700 window, shell 40 px inside, scale 1.
        let window = Rect::new(40.0, 40.0, 920.0, 620.0);
        let caption = Rect::new(40.0, 40.0, 860.0, 48.0);
        let grip = Rect::new(936.0, 636.0, 16.0, 16.0);
        let hit = |x, y| hit_test([x, y], [1000.0, 700.0], 40.0, 1.0, [window, caption, grip]);
        assert_eq!(hit(5.0, 5.0), Hit::Nowhere, "deep in the shadow");
        assert_eq!(hit(34.0, 300.0), Hit::Left);
        assert_eq!(hit(42.0, 300.0), Hit::Left, "a few px inside the edge");
        assert_eq!(hit(963.0, 300.0), Hit::Right);
        assert_eq!(hit(500.0, 35.0), Hit::Top);
        assert_eq!(hit(500.0, 663.0), Hit::Bottom);
        assert_eq!(hit(38.0, 38.0), Hit::TopLeft);
        assert_eq!(
            hit(50.0, 41.0),
            Hit::TopLeft,
            "corner zone along the top edge"
        );
        assert_eq!(hit(962.0, 662.0), Hit::BottomRight);
        assert_eq!(hit(500.0, 70.0), Hit::Caption);
        assert_eq!(hit(500.0, 300.0), Hit::Client);
        assert_eq!(hit(940.0, 640.0), Hit::BottomRight, "grip");
        assert_eq!(
            hit(500.0, 20.0),
            Hit::Nowhere,
            "shadow above the resize band"
        );
    }
}
