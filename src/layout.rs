//! Where the LEDs sit around the screen, and how to turn a screen frame into LED colours.
//!
//! The strip is described as segments in LED order (LED 1 first). Each segment runs
//! along one screen edge. Directions are as seen from the front of the screen:
//! "clockwise" means top: left→right, right: top→bottom, bottom: right→left, left: bottom→top.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub edge: Edge,
    pub count: u8,
    /// `false` = clockwise (see module docs), `true` = counter-clockwise.
    #[serde(default)]
    pub reverse: bool,
}

/// Best guess for the 80-LED monitor kit until the layout test says otherwise:
/// LED 1 at the bottom-left corner, going clockwise.
pub fn default_layout() -> Vec<Segment> {
    vec![
        Segment { edge: Edge::Left, count: 14, reverse: false },
        Segment { edge: Edge::Top, count: 26, reverse: false },
        Segment { edge: Edge::Right, count: 14, reverse: false },
        Segment { edge: Edge::Bottom, count: 26, reverse: false },
    ]
}

pub fn led_count(layout: &[Segment]) -> usize {
    layout.iter().map(|s| usize::from(s.count)).sum()
}

/// How far into the screen each LED "looks", as a fraction of the shorter screen side.
const DEPTH: f32 = 0.12;
/// Samples per zone along each axis (so up to SAMPLES² pixels are averaged per LED).
const SAMPLES: u32 = 12;

/// Pixel rectangle `[x0, x1) × [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// Screen zone for every LED, in LED order.
pub fn zones(layout: &[Segment], width: u32, height: u32) -> Vec<Rect> {
    let depth = ((width.min(height) as f32 * DEPTH) as u32).max(1);
    let mut out = Vec::with_capacity(led_count(layout));
    for seg in layout {
        let n = u32::from(seg.count);
        for i in 0..n {
            // Position along the edge in clockwise order.
            let k = if seg.reverse { n - 1 - i } else { i };
            let along = |len: u32| (k * len / n, ((k + 1) * len / n).max(k * len / n + 1));
            let rect = match seg.edge {
                Edge::Top => {
                    let (a, b) = along(width);
                    Rect { x0: a, x1: b, y0: 0, y1: depth }
                }
                Edge::Right => {
                    let (a, b) = along(height);
                    Rect { x0: width - depth, x1: width, y0: a, y1: b }
                }
                Edge::Bottom => {
                    let (a, b) = along(width);
                    Rect { x0: width - b, x1: width - a, y0: height - depth, y1: height }
                }
                Edge::Left => {
                    let (a, b) = along(height);
                    Rect { x0: 0, x1: depth, y0: height - b, y1: height - a }
                }
            };
            out.push(rect);
        }
    }
    out
}

/// Average colour of each zone in a tightly packed RGBA frame.
pub fn sample(rgba: &[u8], width: u32, height: u32, zones: &[Rect]) -> Vec<[u8; 3]> {
    zones
        .iter()
        .map(|z| {
            let x1 = z.x1.min(width);
            let y1 = z.y1.min(height);
            if z.x0 >= x1 || z.y0 >= y1 {
                return [0, 0, 0];
            }
            let sx = ((x1 - z.x0) / SAMPLES).max(1);
            let sy = ((y1 - z.y0) / SAMPLES).max(1);
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            let mut y = z.y0 + sy / 2;
            while y < y1 {
                let row = (y * width) as usize * 4;
                let mut x = z.x0 + sx / 2;
                while x < x1 {
                    let i = row + x as usize * 4;
                    if let Some(px) = rgba.get(i..i + 3) {
                        sum[0] += u32::from(px[0]);
                        sum[1] += u32::from(px[1]);
                        sum[2] += u32::from(px[2]);
                        count += 1;
                    }
                    x += sx;
                }
                y += sy;
            }
            if count == 0 {
                [0, 0, 0]
            } else {
                sum.map(|s| (s / count) as u8)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 100×60 frame: top half red, bottom half blue, with a green column on the far left.
    fn frame() -> (Vec<u8>, u32, u32) {
        let (w, h) = (100u32, 60u32);
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let px = if x < 5 {
                    [0, 255, 0, 255]
                } else if y < h / 2 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                v.extend_from_slice(&px);
            }
        }
        (v, w, h)
    }

    #[test]
    fn default_layout_has_80_leds() {
        assert_eq!(led_count(&default_layout()), 80);
        assert_eq!(zones(&default_layout(), 1920, 1080).len(), 80);
    }

    #[test]
    fn zones_follow_clockwise_order() {
        let z = zones(&default_layout(), 1000, 500);
        // LED 1: bottom of the left edge.
        assert_eq!((z[0].x0, z[0].y1), (0, 500));
        // LED 15: left end of the top edge.
        assert_eq!((z[14].x0, z[14].y0), (0, 0));
        // LED 41: top of the right edge.
        assert_eq!((z[40].x1, z[40].y0), (1000, 0));
        // LED 55: right end of the bottom edge.
        assert_eq!((z[54].x1, z[54].y1), (1000, 500));
        // LED 80: left end of the bottom edge.
        assert_eq!((z[79].x0, z[79].y1), (0, 500));
    }

    #[test]
    fn reverse_flips_a_segment() {
        let fwd = zones(&[Segment { edge: Edge::Top, count: 4, reverse: false }], 400, 100);
        let rev = zones(&[Segment { edge: Edge::Top, count: 4, reverse: true }], 400, 100);
        assert_eq!(fwd[0], rev[3]);
        assert_eq!(fwd[3], rev[0]);
    }

    #[test]
    fn zones_tile_each_edge_without_gaps() {
        let z = zones(&[Segment { edge: Edge::Bottom, count: 26, reverse: false }], 1920, 1080);
        assert_eq!(z[0].x1, 1920);
        assert_eq!(z[25].x0, 0);
        for w in z.windows(2) {
            assert_eq!(w[0].x0, w[1].x1);
        }
    }

    #[test]
    fn sampling_picks_up_edge_colours() {
        let (rgba, w, h) = frame();
        let layout = [
            Segment { edge: Edge::Top, count: 4, reverse: false },
            Segment { edge: Edge::Bottom, count: 4, reverse: false },
            Segment { edge: Edge::Left, count: 2, reverse: false },
        ];
        let colors = sample(&rgba, w, h, &zones(&layout, w, h));
        // Top edge (away from the green column) is red, bottom edge is blue.
        assert_eq!(colors[1], [255, 0, 0]);
        assert_eq!(colors[5], [0, 0, 255]);
        // The left edge zone is 7 px deep, 5 of them green: mostly green.
        assert!(colors[8][1] > 150);
    }
}
