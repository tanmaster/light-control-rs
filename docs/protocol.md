# DX-Light USB protocol

Reconstructed from the official DX-Light 3.11.0 app. It is an Electron app
(`quiklight-desktop` by Robobloq) whose device logic is plain JavaScript inside
`app.asar`, confirmed against USB captures.

## Device

- VID `0x1A86` (WCH / QinHeng), PID `0xFE07`, USB HID
- Controller board `K18_paul_004_DX_20240930`, WCH microcontroller (TSSOP-20),
  buttons for power / mode / mic, on-board microphone
- The strip does **not** remember anything across power cycles: it always starts
  in its rainbow effect, and the official app re-sends all settings on every connect.
  There is no "save to flash" command for colours or effects.

## Packet format

Every command is sent as one 64-byte HID output report (report id 0, zero padded):

```
'R' 'B' <len> <seq> <action> <payload ...> <checksum>
```

- `len`: total packet length including header and checksum
- `seq`: rolling id 1..=254; replies carry the same id
- `checksum`: sum of all preceding bytes modulo 256

| action | code | payload |
|---|---|---|
| setSyncScreen | `0x80` | uses the `SC` header, see below |
| writeDeviceInfo | `0x81` | u16 id, u8, u8 displaySize, `1`, u16 lampsAmount (persistent device config) |
| readDeviceInfo | `0x82` | none; the reply has id, displaySize, LED count, uuid, firmware version |
| readDeviceUUID | `0x83` | none |
| setLedEffect | `0x85` | u8 effectType, u8 effectIndex (type 2 = dynamic 0..6, type 3 = mic rhythm 0..6) |
| setSectionLED | `0x86` | repeated groups `[first_led, R, G, B, last_led]` |
| setBrightness | `0x87` | u8 (min 5) |
| setAutoOff | `0x89` | u8, u8 |
| setDynamicSpeed | `0x8A` | u8 5..100 |
| setSoundSensitivity | `0x8B` | u8 5..100 |
| setExternalAudio | `0x91` | u16 |
| setOpenUrl | `0x93` | u8 |
| setLampsAmount | `0x95` | u8 |
| setWhiteBright | `0x96` | u8, u8 |
| turnOffLight | `0x97` | none |
| setComputerRhythm | `0x98` | u8, u8 |

### Static colour

The official app sends, for an 80-LED strip:

```
setSectionLED [1, R, G, B, 80,  81, 0, 0, 0, 254]
```

Example capture `52 42 10 41 86 01 00 00 00 50 51 00 00 00 fe 0b`:
length 16, seq `0x41`, setSectionLED, LEDs 1–80 = `#000000`, LEDs 81–254 = `#000000`, checksum `0x0b`.

## Screen sync (`SC` packets)

```
'S' 'C' <u16 BE len> <seq> 0x80 <groups of [first_led, R, G, B, last_led]> <checksum>
```

One frame is larger than 64 bytes and is split across several consecutive reports
(about 5 for 80 LEDs). The official app sends a frame roughly every 40 ms.
Groups are LED ranges, which is why some LEDs appear to be "addressed twice" in captures.
