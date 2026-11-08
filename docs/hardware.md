# Controller hardware

Findings about the controller board of the 80-LED monitor kit (USB `1a86:fe07`).
Gathered with photos, a multimeter and `wchisp info`; nothing here required opening the chip.

## Board

- Silkscreen: `K18_paul_004_DX_20240930`
- Inputs: USB `5V`, `D-`, `D+`, `GND` (an unpopulated `12V` pad suggests a 12 V variant exists)
- Outputs to the strip: `+12V`, `RH/C1`, `RH/C2`, `GND1`, `GND2`. A boost converter
  (L1, D3, U5) generates the strip voltage from USB 5 V; U1 sits near the data outputs.
- Buttons: SW1 开关 (power), SW2 模式 (mode), SW3 拾音 (sound/mic mode), SW4 (bootloader, see below)
- On-board microphone with a transistor stage (Q1, Q2)
- Header HDR1: `GND RX TX 5V`

## Microcontroller

- **WCH CH32X035F7P6** (RISC-V, 48 MHz, 48 KiB flash, 20 KiB SRAM, TSSOP-20, markings sanded off)
- Bootloader version 02.60
- **SW4 enters the USB bootloader**: hold it while plugging in. The chip then enumerates as
  `4348:55e0` for about 5 seconds unless a flashing tool connects
  (e.g. [`wchisp`](https://github.com/ch32-rs/wchisp)).
- The flash is **read-protected** (`RDPR` set). WCH's bootloader has no read command, and
  removing the protection erases the flash, so the factory firmware cannot be backed up.
  The ROM bootloader itself can't be overwritten, so a custom firmware can always be replaced.

Pinout (TSSOP-20): 1 PB12 · 2 PC14 · 3 PC15 · 4 PC3/RST · 5 PC1 · 6 PA0 · 7 PA1 ·
8 PA2/TX2 · 9 PA3/RX2 · 10 PA4 · 11 PA5 · 12 PA6 · 13 PA7 · 14 PB1 · 15 GND · 16 VDD ·
17 PC16/D− · 18 PC17/D+ · 19 PC18/DIO · 20 PC19/DCK

Which pins drive the LEDs, buttons and microphone has not been mapped yet.

## LED layout

Seen from the front: LED 1 at the bottom of the left edge, then clockwise:
left 1–14 (bottom→top), top 15–40 (left→right), right 41–54 (top→bottom),
bottom 55–80 (right→left). `light-control --test-layout` shows it on your own strip.
