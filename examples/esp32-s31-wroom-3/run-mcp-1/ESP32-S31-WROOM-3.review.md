# Datasheet import review: ESP32-S31-WROOM-3

Status: **draft, not published**. Nothing enters a library until a person reviews the rows below and publishes.

## Summary

Drafts of the ESP32-S31-WROOM-3 (PCB antenna) module: component with 99 terminals (GPIO identity, IO MUX / LP IO MUX / analog functions with mux codes, pull options, 18 peripheral resources and 7 pin rules), block symbol, 99-terminal LGA package, footprint (94 row pads 0.8 x 0.4 mm at 0.85 mm, 3 x 3 ground array turned 45 deg as pad 95, corner pads 96-99, antenna copper keep-out) and binding, made by the shipped 'module' generator (preset module-esp32-s31-wroom-3, every value restated from Fig. 10-1 and Fig. 11-1). Pin data was written with library.edit because library.pinout_apply could not be called from this client (its u64 plan base exceeds 2^53 and is rounded in transit, so every apply was refused as stale); all 99 terminals are marked unreviewed. Check first: corner pad numbers 96-99, pad 95 (EPAD), the ground-array centre, and the SPI2 default functions of IO9-IO19.

## Sources

- **DS**: ESP32-S31-WROOM-3 & ESP32-S31-WROOM-3U Datasheet Pre-release v0.7
- **CHIP**: ESP32-S31 Series Datasheet Pre-release v0.5

## Open questions

- [ ] Pin data was entered with library.edit, not library.pinout_apply (the plan base could not be passed exactly); review every pin row in the Pins tab.
- [ ] Is the side-view '1' dimension the module board thickness (1.0 mm)?
- [ ] Confirm the corner pad numbers (96 bottom-left, 97 bottom-right, 98 top-right, 99 top-left) and pad 95 as the centre EPAD against Espressif's footprint.
- [ ] Confirm the ground-array centre (0, -3.0) mm and pitch 2.35 mm; the drawing shows a chamfered pad nearest the antenna, which the footprint draws square.
- [ ] The host antenna keep-out is not dimensioned in this datasheet; set its extension from the ESP32-S31 Hardware Design Guidelines.
- [ ] Confirm that the bold design-default of IO9-IO19 is the SPI2 pad function (F0) rather than GPIO.
- [ ] Drive kinds are not stated in the datasheets: push_pull assumed, open_drain for LP I2C. Confirm.
- [ ] USB_DP/USB_DM and USB1P1_P0/N0 are passive analog; confirm this suits the design's ERC.
- [ ] LCD roles exceed the 16-role resource limit, so they are split into LCD (DATA0-11, PCLK, H_ENABLE, H_SYNC, V_SYNC) and LCD_DATA_HI (DATA12-23).
- [ ] LP_PROBE_TOP_OUT0-7 carry no peripheral; their peripheral is not named in Table 2-4.
- [ ] Peripherals reachable only through the GPIO matrix (UART1-3, I2C0/1, I2S0/1, PCNT, CAN FD, LEDC, MCPWM, RMT, PARLIO, GP-SPI3) are not declared.
- [ ] The footprint has no pin-1 polarity pad (footprint.polarity_missing); add one if wanted.
- [ ] Both datasheets are preliminary (v0.7 / v0.5); recheck against the released revisions.

## Extracted values

| Item | Value | Source | Confidence | Note |
|---|---|---|---|---|
| antenna keep-out | copper keep-out over the module's 6 mm antenna region only; no extension beyond the outline | DS p.13 Section 3.1 note | Low | Host keep-out size is not stated; the note refers to the ESP32-S31 Hardware Design Guidelines. |
| function drive kinds | push_pull for digital outputs and bidirectionals; open_drain for LP_I2C_SCL/SDA | CHIP p.23 Table 2-3 type column (I/O/T, O) | Low | The datasheet gives no drive kind; chosen so the functions validate. |
| board thickness | 1.0 mm | DS p.64 Figure 10-1, side view | Medium | Read from the unlabelled '1' dimension in the side view. |
| shield can centre | (0, -3.0) mm | DS p.64 Figure 10-1, top view (0.5 mm edge offsets) | Medium | Derived: can 0.5 mm in from the bottom and side edges. |
| pad row centre lines | 0.7 mm in from the module edge: x = +/-10.3 mm, bottom y = -14.3 mm | DS p.64 Figure 10-1, bottom view (0.7) | Medium | Derived from the 0.7 dimensions and the outline. |
| first pad centres of the rows | left (-10.3, 7.2), bottom (-8.925, -14.3), right (10.3, -13.2), top (8.925, 8.3) mm | DS p.67 Figure 11-1 | Medium | Derived: rows centred between the corner pads; top row y = -14.3 + 22.6. |
| ground pad array | 3 x 3 pads of 1.8 mm, 0.55 mm gaps (pitch 2.35 mm), 6.5 mm overall, turned 45 deg | DS p.67 Figure 11-1 (1.8, 0.55, 6.5) | Medium | Pitch derived as 1.8 + 0.55. |
| ground array centre | (0, -3.0) mm | DS p.67 Figure 11-1 centre lines | Medium | Derived from the centre lines through the 13th left-row pad and the bottom-row midpoint. |
| ground array and corner pad numbers | 95 = EPAD (centre array, one terminal); 96 bottom-left, 97 bottom-right, 98 top-right, 99 top-left | DS p.62 Figure 9-1 | Medium | Read from the peripheral schematic symbol; Table 3-1 lists 92~99 as GND. |
| default functions | bold entries: GPIO on IO0-IO8, IO35-IO53, IO60-IO61, SD pins (F1 GPIO20-25); SPI2 pads (F0) on IO9-IO19; MTDO/MTCK/MTDI/MTMS on IO54-IO57; UART0 on TX0/RX0; USB1P1 on IO33/IO34 | CHIP p.23 Table 2-3 note 1, Table 2-7 | Medium | Bold read from the rendered table. |
| USB pins | USB_DP/USB_DM and USB1P1_P0/N0 modelled as passive analog | CHIP p.32 Table 2-9; Table 2-7 | Medium | Pin type Analog in the chip tables. |
| module outline | 22.0 x 30.0 mm (+/-0.2) | DS p.64 Figure 10-1 | High |  |
| seated height | 3.5 mm (+/-0.15) | DS p.64 Figure 10-1, side view | High |  |
| antenna region depth | 6 mm from the +y edge | DS p.64 Figure 10-1, top view | High |  |
| shield can size | 21 x 23 mm | DS p.64 Figure 10-1, top view | High |  |
| pad pitch | 0.85 mm | DS p.67 Figure 11-1 | High |  |
| row pad size | 0.8 mm across the edge x 0.4 mm along it (94 pads) | DS p.67 Figure 11-1 (94 x 0.8, 94 x 0.4) | High |  |
| corner pad size | 0.8 x 0.8 mm (4 pads) | DS p.67 Figure 11-1 (4 x 0.8) | High |  |
| host land | equal to the pad (LGA, castellation 0, land growth 0, land width 0.4 mm) | DS p.67 Figure 11-1 | High |  |
| row pad counts | left 25, bottom 22, right 25, top 22 | DS p.12 Figure 3-1 | High |  |
| row spans | 20.4 mm (25 pads), 17.85 mm (22 pads) | DS p.67 Figure 11-1 | High |  |
| corner pad centre spans | 20.6 mm (x) x 22.6 mm (y) | DS p.67 Figure 11-1 | High |  |
| pin 1 position | top of the left row, below the top-left corner pad | DS p.67 Figure 11-1 (Pin 1 callout) | High | Built footprint pad 1 at (-10.3, 7.2) mm. |
| terminal count | 99 | DS p.14 Section 3.2, Table 3-1 | High |  |
| pin names and module functions | Table 3-1 rows for pins 1-99 | DS p.14 Table 3-1 (continued p. 15) | High |  |
| IO MUX functions F0-F4 | per GPIO as printed | CHIP p.23 Table 2-3 (pp. 23-25) | High |  |
| LP IO MUX functions F0-F3 | GPIO0-GPIO7 | CHIP p.26 Table 2-5 | High |  |
| analog functions F0 | XTAL_32K, TOUCH_CH0-13, USB1P1, PAD_COMP0-3, ADC1/ADC2 channels | CHIP p.27 Table 2-7 (pp. 27-28) | High |  |
| IO36 strapping default | floating on the chip (no bit value); pulled up inside the module; selects VDD_SPI 3.3 V (1) / 1.8 V (0) | DS p.18 Table 4-1; Table 4-4 p. 21; Table 3-1 note 2 | High |  |
| IO37 strapping default | floating, no internal pulls; JTAG signal source | DS p.18 Table 4-1; Section 4.4 p. 22 | High |  |
| IO60 strapping default | weak pull-up, 1 | DS p.18 Table 4-1 | High |  |
| IO61 strapping default | weak pull-up, 1 (SPI Boot) | DS p.18 Table 4-1; Table 4-3 p. 20 | High |  |
| LP UART flow control vs LP I2C | cannot be used together (forbid rule) | CHIP p.26 Table 2-5 note 2 | High |  |

## Drafts

- component 2 ‘ESP32-S31-WROOM-3’: pinout.unreviewed
- symbol 3 ‘ESP32-S31-WROOM-3 symbol’: 
- binding 6 ‘ESP32-S31-WROOM-3 binding’: 
- package 4 ‘ESP32-S31-WROOM-3 package’: 
- footprint 5 ‘ESP32-S31-WROOM-3 footprint’: footprint.polarity_missing

Imported pins: 99 unreviewed, 0 uncertain. A person checks each against the datasheet in the Pins tab before publishing.
