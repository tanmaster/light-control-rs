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

/// Wrap a packet into a HID output report: report id 0 followed by 64 bytes.
pub fn to_report(packet: &[u8]) -> [u8; REPORT_LEN + 1] {
    assert!(packet.len() <= REPORT_LEN);
    let mut report = [0u8; REPORT_LEN + 1];
    report[1..=packet.len()].copy_from_slice(packet);
    report
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

    #[test]
    fn report_has_leading_report_id() {
        let r = to_report(&static_color(7, [9, 9, 9], 80));
        assert_eq!(r.len(), 65);
        assert_eq!(r[0], 0);
        assert_eq!(&r[1..3], b"RB");
        assert!(r[17..].iter().all(|b| *b == 0));
    }
}
