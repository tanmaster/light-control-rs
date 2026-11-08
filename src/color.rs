//! Make sampled screen colours look right on LEDs.
//!
//! Two effects make raw screen-sync colours look washed out:
//! * averaging a whole patch of screen mixes colours towards grey;
//! * screen values are gamma-encoded (mid-tones stored brighter than they are),
//!   while LEDs respond linearly, so mid-tones come out too bright.
//!
//! `enhance` pushes colours away from grey (saturation) and applies a gamma curve.

/// How strongly to enhance colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// 1.0 = unchanged, >1 = more saturated.
    pub saturation: f32,
    /// 1.0 = unchanged, >1 = darker mid-tones / richer colours.
    pub gamma: f32,
}

/// Presets offered in the tray menu: (label, tuning).
pub const PRESETS: [(&str, Tuning); 3] = [
    ("Natural", Tuning { saturation: 1.0, gamma: 1.0 }),
    ("Vivid", Tuning { saturation: 1.5, gamma: 1.6 }),
    ("Intense", Tuning { saturation: 2.0, gamma: 2.0 }),
];

pub const DEFAULT: Tuning = PRESETS[1].1;

/// Enhance one colour (channels in 0.0..=255.0).
pub fn enhance(c: [f32; 3], t: Tuning) -> [f32; 3] {
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    c.map(|v| {
        let saturated = (luma + (v - luma) * t.saturation).clamp(0.0, 255.0);
        255.0 * (saturated / 255.0).powf(t.gamma)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.5)
    }

    #[test]
    fn natural_is_identity() {
        let t = PRESETS[0].1;
        for c in [[0.0, 0.0, 0.0], [255.0, 255.0, 255.0], [12.0, 200.0, 99.0]] {
            assert!(close(enhance(c, t), c));
        }
    }

    #[test]
    fn black_and_white_stay_put() {
        for (_, t) in PRESETS {
            assert!(close(enhance([0.0; 3], t), [0.0; 3]));
            assert!(close(enhance([255.0; 3], t), [255.0; 3]));
        }
    }

    #[test]
    fn grey_stays_neutral_but_darker() {
        let out = enhance([128.0; 3], DEFAULT);
        assert!((out[0] - out[1]).abs() < 0.01 && (out[1] - out[2]).abs() < 0.01);
        assert!(out[0] < 128.0);
    }

    #[test]
    fn muted_colours_get_more_saturated() {
        // A dull orange: the spread between channels should grow relative to brightness.
        let input = [180.0, 130.0, 100.0];
        let out = enhance(input, DEFAULT);
        let spread = |c: [f32; 3]| (c[0] - c[2]) / c[0];
        assert!(spread(out) > spread(input));
        // Hue order is kept: red > green > blue.
        assert!(out[0] > out[1] && out[1] > out[2]);
    }

    #[test]
    fn output_stays_in_range() {
        for (_, t) in PRESETS {
            for c in [[255.0, 0.0, 0.0], [0.0, 0.0, 255.0], [255.0, 255.0, 0.0], [3.0, 250.0, 7.0]] {
                assert!(enhance(c, t).iter().all(|v| (0.0..=255.0).contains(v)));
            }
        }
    }
}
