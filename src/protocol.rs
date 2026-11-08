//! Wire protocol of the DX-Light / Robobloq USB light strip (VID 0x1A86, PID 0xFE07).
//!
//! Reconstructed from the official DX-Light 3.11.0 app (see `docs/protocol.md`).
//!
//! Every command is a "RB" packet:
//!
//! ```text
//! 'R' 'B' <len> <seq> <action> <payload ...> <checksum>
//! ```
//!
//! * `len`      total packet length, header and checksum included
//! * `seq`      rolling sequence number 1..=254, echoed back in replies
//! * `checksum` sum of all preceding bytes, modulo 256
//!
//! On the wire each packet is sent as one 64-byte HID output report
//! (zero padded), preceded by report id 0.

pub const VENDOR_ID: u16 = 0x1A86;
pub const PRODUCT_ID: u16 = 0xFE07;

/// Size of one HID report (without the report id byte).
pub const REPORT_LEN: usize = 64;

/// Action codes understood by the HID light (from the official app).
#[allow(dead_code)]
pub mod action {
    pub const SET_SYNC_SCREEN: u8 = 0x80;
    pub const WRITE_DEVICE_INFO: u8 = 0x81;
    pub const READ_DEVICE_INFO: u8 = 0x82;
    pub const READ_DEVICE_UUID: u8 = 0x83;
    pub const SET_LED_EFFECT: u8 = 0x85;
    pub const SET_SECTION_LED: u8 = 0x86;
    pub const SET_BRIGHTNESS: u8 = 0x87;
    pub const SET_AUTO_OFF: u8 = 0x89;
    pub const SET_DYNAMIC_SPEED: u8 = 0x8A;
    pub const SET_SOUND_SENSITIVITY: u8 = 0x8B;
    pub const SET_EXTERNAL_AUDIO: u8 = 0x91;
    pub const SET_OPEN_URL: u8 = 0x93;
    pub const SET_LAMPS_AMOUNT: u8 = 0x95;
    pub const SET_WHITE_BRIGHT: u8 = 0x96;
    pub const TURN_OFF_LIGHT: u8 = 0x97;
    pub const SET_COMPUTER_RHYTHM: u8 = 0x98;
}

/// Highest LED index the protocol accepts in a section.
const MAX_LED: u8 = 254;

/// Rolling sequence number, 1..=254 like the official app.
#[derive(Debug, Default, Clone)]
pub struct Sequence(u8);

impl Sequence {
    pub fn next(&mut self) -> u8 {
        self.0 = if self.0 >= 254 { 1 } else { self.0 + 1 };
        self.0
    }
}

pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

/// Build a complete "RB" packet.
pub fn rb_packet(seq: u8, action: u8, payload: &[u8]) -> Vec<u8> {
    let len = 6 + payload.len();
    assert!(len <= REPORT_LEN, "RB packet of {len} bytes does not fit one report");
    let mut p = Vec::with_capacity(len);
    p.extend_from_slice(b"RB");
    p.push(len as u8);
    p.push(seq);
    p.push(action);
    p.extend_from_slice(payload);
    p.push(checksum(&p));
    p
}

/// Set the whole strip to one colour.
///
/// Mirrors what the official app sends for a static colour: LEDs
/// `1..=led_count` get the colour, everything after that up to 254 is black.
/// Payload groups are `[first_led, r, g, b, last_led]`.
pub fn static_color(seq: u8, rgb: [u8; 3], led_count: u8) -> Vec<u8> {
    let [r, g, b] = rgb;
    let payload: Vec<u8> = if led_count == 0 || led_count >= MAX_LED {
        vec![1, r, g, b, MAX_LED]
    } else {
        vec![1, r, g, b, led_count, led_count + 1, 0, 0, 0, MAX_LED]
    };
    rb_packet(seq, action::SET_SECTION_LED, &payload)
}

/// A range of LEDs (1-based, inclusive) set to one colour: `(first, rgb, last)`.
pub type Group = (u8, [u8; 3], u8);

fn flatten(groups: &[Group]) -> Vec<u8> {
    groups
        .iter()
        .flat_map(|&(first, [r, g, b], last)| [first, r, g, b, last])
        .collect()
}

/// `setSectionLED` with arbitrary groups (must fit in one 64-byte report, i.e. ≤ 11 groups).
pub fn section_led(seq: u8, groups: &[Group]) -> Vec<u8> {
    rb_packet(seq, action::SET_SECTION_LED, &flatten(groups))
}

/// Most groups the official app ever puts in one screen-sync frame.
pub const MAX_SYNC_GROUPS: usize = 50;

/// Screen-sync frame:
///
/// ```text
/// 'S' 'C' <u16 BE len> <seq> 0x80 <groups ...> <checksum>
/// ```
///
/// Usually longer than one report; send it with [`to_reports`].
pub fn sync_frame(seq: u8, groups: &[Group]) -> Vec<u8> {
    let len = 7 + 5 * groups.len();
    let mut p = Vec::with_capacity(len);
    p.extend_from_slice(b"SC");
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p.push(seq);
    p.push(action::SET_SYNC_SCREEN);
    p.extend_from_slice(&flatten(groups));
    p.push(checksum(&p));
    p
}

/// Turn one colour per LED into at most `max` contiguous groups,
/// averaging the LEDs that share a group.
pub fn merge_groups(colors: &[[u8; 3]], max: usize) -> Vec<Group> {
    let n = colors.len().min(MAX_LED as usize);
    if n == 0 || max == 0 {
        return Vec::new();
    }
    let count = n.min(max);
    (0..count)
        .map(|j| {
            let start = j * n / count;
            let end = (j + 1) * n / count;
            let slice = &colors[start..end];
            let mut sum = [0u32; 3];
            for c in slice {
                for k in 0..3 {
                    sum[k] += u32::from(c[k]);
                }
            }
            let len = slice.len() as u32;
            let avg = sum.map(|s| ((s + len / 2) / len) as u8);
            ((start + 1) as u8, avg, end as u8)
        })
        .collect()
}

/// Wrap a packet into a HID output report: report id 0 followed by 64 bytes.
pub fn to_report(packet: &[u8]) -> [u8; REPORT_LEN + 1] {
    assert!(packet.len() <= REPORT_LEN);
    let mut report = [0u8; REPORT_LEN + 1];
    report[1..=packet.len()].copy_from_slice(packet);
    report
}

/// Split a packet of any length into consecutive reports, like the official app does.
pub fn to_reports(packet: &[u8]) -> Vec<[u8; REPORT_LEN + 1]> {
    packet.chunks(REPORT_LEN).map(to_report).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn matches_captured_off_packet() {
        // Captured from the official app with Wireshark (80-LED strip, "off").
        let captured = hex("5242104186010000005051000000fe0b");
        assert_eq!(static_color(0x41, [0, 0, 0], 80), captured);
    }

    #[test]
    fn checksums_of_other_captures() {
        for c in [
            "5242104586010000005051000000fe0f",
            "5242106186010000005051000000fe2b",
            "5242101986010000005051000000fee3",
        ] {
            let p = hex(c);
            let (body, sum) = p.split_at(p.len() - 1);
            assert_eq!(checksum(body), sum[0], "{c}");
        }
    }

    #[test]
    fn colour_lands_in_first_group() {
        let p = static_color(1, [0x12, 0x34, 0x56], 80);
        assert_eq!(&p[..5], &[b'R', b'B', 16, 1, action::SET_SECTION_LED]);
        assert_eq!(&p[5..10], &[1, 0x12, 0x34, 0x56, 80]);
        assert_eq!(&p[10..15], &[81, 0, 0, 0, 254]);
        assert_eq!(p[15], checksum(&p[..15]));
    }

    #[test]
    fn single_group_when_led_count_unknown() {
        let p = static_color(1, [1, 2, 3], 0);
        assert_eq!(p.len(), 11);
        assert_eq!(&p[5..10], &[1, 1, 2, 3, 254]);
    }

    #[test]
    fn sequence_wraps_like_official_app() {
        let mut s = Sequence::default();
        assert_eq!(s.next(), 1);
        for _ in 0..252 {
            s.next();
        }
        assert_eq!(s.next(), 254);
        assert_eq!(s.next(), 1);
    }

    /// Screen-sync frame captured from the official app (fullscreen red image).
    const SYNC_CAPTURE: &str = "534301013b8001ff00000102ff00000304ff00000405ff00000708ff00000809ff00000a0bff00000b0cff00000d0eff00000e0fff00001011ff00001112ff00001314ff00001415ff00001617ff00001819ff00001a1bff00001c1dff00001e1fff00002021ff00002122ff00002324ff00002425ff00002627ff00002728ff00002829ff00002a2bff00002b2cff00002d2eff00002e2fff00003132ff00003233ff00003435ff00003536ff00003637ff00003738ff00003839ff00003a3bff00003b3cff00003d3eff00003e3fff00004041ff00004243ff00004445ff00004647ff00004849ff00004a4bff00004b4cff00004d4eff00004e4fff000050e9";

    #[test]
    fn sync_frame_matches_capture() {
        let captured = hex(SYNC_CAPTURE);
        let groups: Vec<Group> = captured[6..captured.len() - 1]
            .chunks(5)
            .map(|g| (g[0], [g[1], g[2], g[3]], g[4]))
            .collect();
        assert_eq!(groups.len(), 50);
        assert_eq!(sync_frame(0x3b, &groups), captured);
    }

    #[test]
    fn sync_frame_splits_into_five_reports() {
        let reports = to_reports(&hex(SYNC_CAPTURE));
        assert_eq!(reports.len(), 5);
        assert!(reports.iter().all(|r| r[0] == 0));
        assert_eq!(&reports[0][1..3], b"SC");
        assert_eq!(reports[4][1], 0xe9);
    }

    #[test]
    fn merge_groups_covers_every_led_once() {
        let colors: Vec<[u8; 3]> = (0..80).map(|i| [i as u8, 0, 0]).collect();
        let groups = merge_groups(&colors, MAX_SYNC_GROUPS);
        assert_eq!(groups.len(), 50);
        assert_eq!(groups[0].0, 1);
        assert_eq!(groups.last().unwrap().2, 80);
        for w in groups.windows(2) {
            assert_eq!(w[0].2 + 1, w[1].0);
        }
        // LEDs 2 and 3 (values 1 and 2) share a group and average to 2 (rounded).
        assert_eq!(groups[1], (2, [2, 0, 0], 3));
        // Fewer LEDs than the limit: one group per LED.
        assert_eq!(merge_groups(&colors[..10], 50).len(), 10);
    }

    #[test]
    fn section_led_layout_test_fits_one_report() {
        let groups = [(1, [255, 255, 0], 3), (4, [255, 0, 0], 14), (15, [0, 255, 0], 40)];
        let p = section_led(1, &groups);
        assert_eq!(p.len(), 6 + 15);
        assert_eq!(&p[5..10], &[1, 255, 255, 0, 3]);
    }

    #[test]
    fn report_has_leading_report_id() {
        let r = to_report(&static_color(7, [9, 9, 9], 80));
        assert_eq!(r.len(), 65);
        assert_eq!(r[0], 0);
        assert_eq!(&r[1..3], b"RB");
        assert!(r[17..].iter().all(|b| *b == 0));
    }
}
