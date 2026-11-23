# USB-C multi-tool: first board to order and build

**Status:** design, 23 November 2026, updated with the founder's decisions of the same day. Nothing is built or ordered. Every number below either cites a source with the date it was read, or is marked **unverified**. Prices are search-index or distributor snapshots read on 2026-11-23 and must be re-read before ordering. Related: [autonomous build protocol](autonomous-pcb-build-protocol.md), [high-speed routing](../high-speed-routing.md), [fabrication output](../fabrication-output.md), [LLM integration](../llm-integration.md), [agent harness](agent-harness.md), [ESP32-S31 dev board example](../../examples/esp32-s31-devboard/README.md).

## 1. Purpose and users

A handheld tool that sits inline between a USB-C charger or host and a USB-C device. It measures power, decodes USB Power Delivery, tests chargers up to 240 W, and lets a USB4 link keep running through it. Think of it as a pocket multi-tool for USB-C.

Who it is for:

- Hardware developers bringing up a USB-C product: what did the charger offer, what did my device ask for, how much current does it really draw?
- Repair technicians and IT staff: is it the charger, the cable or the laptop?
- Reviewers and enthusiasts testing chargers and cables, including 240 W EPR chargers and e-marked cables.

It is also Sparksmith's first board that will be ordered and built, and the subject of the [autonomous build protocol](autonomous-pcb-build-protocol.md).

## 2. Features and operating modes

### Ports

| Port | Part | Rating | Use |
|---|---|---|---|
| J1 | USB4 receptacle | 40 Gbit/s, 20 Vdc | Pass-through input: the cable from the charger or host |
| J2 | USB4 plug | 40 Gbit/s, 20 Vdc | Pass-through output: plugs into the device |
| J5 | USB-C receptacle, USB 2.0 | **48 Vdc, 5 A, 240 W** | Charger test input |
| J4 | Load port | ≥ 60 V, ≥ 5 A | An external electronic load for charger tests |
| J3 | USB-C receptacle, USB 2.0 | | Service: charging the tool's cell, logs and firmware at USB 2.0 high speed |

### Modes

| Mode | Path | Voltage limit | What happens |
|---|---|---|---|
| **Charger test (PD sink)** | Charger → J5 → R2 shunt → J4 load port | **Full EPR: 48 V, 5 A, 240 W** | The tool negotiates any offered profile (fixed, PPS, EPR fixed, EPR AVS) as a sink and switches VBUS to an external electronic load on J4 |
| **Pass-through, transparent** | Charger or host → J1 → R1 shunt → J2 → device | **SPR only: ≤ 20 V, ≤ 5 A, ≤ 100 W** | CC is wired straight through; the tool only listens. Allowed only when the upstream source has said it is not EPR-capable (section 8) |
| **Pass-through, guarded** | Same | **SPR only** | Used when the upstream source is EPR-capable. The tool ends the PD conversation on each side and offers the device only SPR profiles; EPR cannot be entered (section 8) |

The pass-through connectors J1 and J2 are both rated 20 Vdc, so the pass-through path never carries more than SPR. Charger tests at up to 48 V use their own EPR-rated receptacle J5. Founder decisions, 2026-11-23; the separate J5 follows from the connector choice in section 7.

### v1 (this board)

| Feature | What it means in v1 |
|---|---|
| Power measurement | VBUS voltage, current in both directions, power, energy and charge, from a 20-bit monitor (INA228) on a 4-terminal shunt, one per path |
| PD decode and log | Every SOP, SOP′ and SOP″ message, SPR and EPR, timestamped, shown and logged. In guarded pass-through the log shows two conversations (charger ↔ tool, tool ↔ device) |
| Cable e-marker | Decoded whenever the source asks the cable (Discover Identity on SOP′). An EPR source must check the cable before it enters EPR, so charger tests show it. Active reading without a source present is v2 (risk R7) |
| CC and SBU monitor | CC1, CC2, SBU1, SBU2 voltages through the port protectors |
| Charger test | On J5 into J4, as above |
| Display, buttons, battery | Small display, 5 buttons, a Li-ion cell |
| Logging | Wi-Fi 6 and Bluetooth LE to a phone or PC; USB 2.0 high-speed on J3 |
| USB4 pass-through | The four high-speed pairs, USB 2.0 and SBU pass straight through. No active parts on the lanes. USB4 uses the same four pairs as USB 3.2, so the board carries USB 3.2 too, and links fall back to lower speeds on their own |

### v2 (later boards)

- An **EPR-rated board-mount USB4 plug and receptacle pair for one board thickness**, so pass-through can carry EPR. Ask Same Sky, JAE, Amphenol and Molex (founder decision). None publishes one that I could find (section 7).
- USB 2.0 traffic capture with an FPGA. The ESP32-S31 already has USB 2.0 high-speed OTG, so the capture can stream to a PC through the same MCU (section 6); the v2 board adds the FPGA and its capture front end, not a new MCU.
- Active source emulation (the tool acts as a charger).
- Active e-marker reading (the tool supplies VCONN).
- USB4 traffic recording is out of scope for both.

## 3. The connector rule, checked

The founder asked to place a USB-C **plug** and a **receptacle**, because the specification forbids receptacle-to-receptacle couplers. I read the specification to check that.

Source: *USB Type-C Cable and Connector Specification, Release 2.5, March 2026* (usb.org, archive "USB Type-C 2.5 Release 202603.zip", read 2026-11-23), Section 2.2, pages 38–39:

> "Cables and connectors, including USB Type-C to USB legacy cables and adapters, are explicitly defined within this specification. These are the only connectors and cables that are authorized by the licensing terms of this specification."

> "USB Type-C receptacle to USB legacy adapters are explicitly not defined or allowed. Such adapters would allow many invalid, and potentially unsafe, cable connections to be constructed by users."

What this means, read plainly:

- The specification does not contain a sentence that names a receptacle-to-receptacle coupler. It forbids it the other way round: only the cables and adapters it lists are authorised, and no USB-C receptacle-to-receptacle adapter is listed.
- **A plug-to-receptacle device is not listed either.** The only defined parts with a USB-C plug and a receptacle are the two legacy adapters (USB-C plug to Standard-A receptacle, and USB 2.0 USB-C plug to Micro-B receptacle). The inline tool is therefore outside the specification's defined set, as every inline USB-C power tester on the market is. It is better than a coupler, because it cannot join two cables at both ends, but it can still act as a short extension.
- Design consequence: plug plus receptacle, as the founder asked. No USB-IF compliance claim, the tool is never called a cable or adapter, and its added loss is stated honestly (section 7).

Further rules from the same document that the design follows:

- Any cable assembly with electronics that VBUS may power "shall ensure that it can functionally tolerate, or be protected from, a VBUS voltage of up to 50.9 V (48 V + 5% + 500 mV)" (Section 4.6.2, p. 227). Everything on the J5 side is rated for this. For SPR the same section asks for 21 V; by the same rule (nominal + 5 % + 500 mV) the pass-through path sees at most 21.5 V.
- A USB PD source "shall not provide more than 48 V nominal on VBUS" (Section 4.6, p. 225).
- EPR cables "are required to be electronically marked for 5 A and 50 V and include the EPR Mode Capable bit set" (Section 2.4, p. 42).
- The full-featured cable's VBUS bypass capacitor: 10 nF, "minimum recommended voltage rating of 30 V, 63 V if EPR-capable" (Section 3.4.1, table notes, p. 84). We use the same rule for our VBUS capacitors.
- S-parameters for the high-speed pairs are "normalized with an 85 Ω differential impedance" (Section 3.7, p. 96).

## 4. Block diagram

```
 charger / host                                                          device
     │ cable                                                               ▲
     ▼                                                                     │
 J1 USB4 receptacle (20 V) ═══ 4 high-speed pairs, D+/D−: copper only ═══ J2 USB4 plug (20 V)
     │ VBUS ≤ 21.5 V                                                       ▲ VBUS
     ├─ D1 TVS                                                             │
     └─ R1 5 mΩ ── Q1/Q2 back-to-back ── D2 ───────────────────────────────┘
          │ Kelvin   U3 LM74502H, OV trip 22 V (hardware)
          ▼
        U2 INA228 (bus-overvoltage alert)

 J1 CC/SBU ─ U4 TPD4S480 ─ node A ─ S1 CC switch (default open) ─ node B ─ U8 TPD4S480 ─ J2
                            │        (SBU: A to B straight)          │
                      UCPD1 (listen or sink)                 S2 mux ─ UCPD2 (source in guarded
                                                              │       mode, or sink on J5)
 charger (EPR) ─► J5 USB-C 48 V ─ U11 TPD4S480 (CC) ──────────┘
                  │ VBUS ≤ 50.9 V
                  ├─ D3 TVS
                  └─ R2 5 mΩ ── Q3/Q4 back-to-back ── J4 load port ─► electronic load
                       │ Kelvin    U9 LM74502H
                       ▼
                     U10 INA228

          U1 STM32G0B1 (UCPD1, UCPD2, both INA228 on I²C) ── UART ── U5 ESP32-S31 module
                                                                     ├─ display, 5 buttons
                                                                     ├─ Wi-Fi 6, BLE
                                                                     └─ USB 2.0 HS OTG ─ J3
 J3 USB-C (USB 2.0) ─► U6 BQ25188 ─► 1S Li-ion ─► U7 buck-boost ─► 3V3
```

## 5. Key parts

All parts below have public datasheets. "Read" means the value was read from that source on 2026-11-23; anything else is marked.

| Ref | Part | MPN | Why | Source |
|---|---|---|---|---|
| U1 | PD front end MCU | STM32G0B1CEU6 | Two UCPD (USB-C PD) blocks: UCPD1 on the J1 side listens or sinks; UCPD2 is the source toward J2 in guarded pass-through, or, through mux S2, the sink on J5 for charger tests. PD protocol is firmware, so EPR, AVS and the guard are firmware. ST states its UCPD MCUs support PD r3.1 EPR messages (family claim, **unverified for G0B1 and for PD 3.2 AVS**) | DS13560 rev 6 (Feb 2026); ST AN5418 (two UCPD instances); [ST USB-C page](https://www.st.com/content/st_com/en/stm32-usb-c.html) |
| U2, U10 | Power monitors | INA228AIDGSR | −0.3 to 85 V common mode, 20-bit, ±163.84 mV or ±40.96 mV shunt range, offset ±1 µV max, gain error ±0.05 % max, energy and charge accumulators, alert output. One per path, so no switch ever joins J1's and J5's VBUS | TI SLYS021A (May 2022), [ti.com/product/INA228](https://www.ti.com/product/INA228) |
| R1, R2 | Shunts | Bourns CSS2H-2512K-5L00F (5 mΩ, 2512, 4-terminal) | 25 mV at 5 A, inside the ±40.96 mV range; 125 mW at 5 A. Rated 2.5 W at 70 °C terminal temperature per a hosted copy of the datasheet; the 5 mΩ value is listed "available upon request" (**availability unverified**) | [Bourns CSS2H-2512 datasheet](https://www.ttiasia.com/content/dam/ttiinc/manufacturers/bourns/Resistors/Power/PDF/css2h-2512.pdf) |
| U4, U8, U11 | CC/SBU protectors | TPD4S480RUKR | 48 V short-to-VBUS protection on CC1, CC2, SBU1, SBU2, 63 V DC tolerant, IEC 61000-4-2 ESD; series FETs that pass VCONN (600 mA at TJ ≤ 105 °C, 1.25 A at TJ ≤ 85 °C). J1's and J2's CC and SBU pass through U4 and U8. U11 protects J5's CC. On-resistance and state when unpowered: **not confirmed from the PDF** | TI SLVSH00 (Nov 2024), [ti.com/product/TPD4S480](https://www.ti.com/product/TPD4S480) |
| S1 | CC isolation switch | **not chosen (O3)** | Two channels, ≥ 6 V, carries BMC and VCONN (≥ 600 mA), **open when unpowered**. Sits between U4 and U8, so never sees more than their clamped voltage | |
| S2 | CC mux for UCPD2 | **not chosen (O3)** | 2 × 2:1, ≥ 6 V, BMC only; selects J2's node B or J5's CC | |
| D1, D2, D3 | VBUS surge clamps | Semtech TDS5311P | 53 V working voltage, so it does not conduct at 50.9 V; clamping 63 V at 24 A (8/20 µs) per Semtech's launch figures (**clamp value not read from the datasheet**) | [Semtech TDS5311P](https://www.semtech.com/products/circuit-protection/tvs-diodes/tds5311p) |
| U3, Q1, Q2 | Pass-through VBUS switch | LM74502HDDFR + two ≥ 80 V N-MOSFETs | 3.2–65 V back-to-back FET controller, 11 mA gate drive (H version), overvoltage cut-off set by a resistor divider on its OV pin. Off unless pass-through is active; hardware trip at 22 V (section 8). FETs not chosen (**O4**) | TI SNOSDE5A (May 2022), [ti.com/product/LM74502](https://www.ti.com/product/LM74502) |
| U9, Q3, Q4 | Load-port switch | LM74502HDDFR + two ≥ 80 V N-MOSFETs | Same circuit, no 22 V trip; on only in charger-test mode after the contract | As above |
| J1 | USB4 receptacle | Same Sky **UJ40-C-H-G-MSMT-22B-P24-TR-68** | USB4 Gen 3, 40 Gbit/s, 5 A, 20 Vdc, IP68, mid-mount, **for a 0.8 mm PCB**; both contact rows are SMT pads on the top face | [Datasheet, 08/21/2026](https://www.sameskydevices.com/product/resource/uj40-c-h-g-msmt-22b-p24-tr-68.pdf) |
| J2 | USB4 plug | Same Sky **UP40-C-V-G-BMT-10B-P24-TR** | USB4 Gen 3, 40 Gbit/s, 5 A, 20 Vdc. Board-edge straddle: A row on one face, B row on the other; the gap between the rows is 0.80 ± 0.10 mm, **for a 0.8 mm PCB** | [Datasheet rev 1.0, 08/16/2026](https://www.sameskydevices.com/product/resource/up40-c-v-g-bmt-10b-p24-tr.pdf) |
| J5 | Charger-test receptacle | GCT **USB4110** | USB 2.0 USB-C receptacle, SMT top mount, **48 V DC, 5 A collectively on VBUS, 240 W**. Only VBUS, CC and GND are used | [GCT drawing](https://gct.co/files/drawings/usb4110.pdf) |
| J3 | Service USB-C | GCT USB4105-GF-A | USB 2.0 receptacle already used on the ESP32-S31 dev board example ([board rules](../board-rules-and-shape.md)) | GCT drawing |
| J4 | Load port | **not chosen (O5)** | Two terminals for an external electronic load, rated ≥ 60 V and ≥ 5 A | |
| U5 | Main MCU | Espressif **ESP32-S31-WROOM-3-N16R8V** | ESP32-S31: dual-core 32-bit RISC-V up to 320 MHz; 2.4 GHz Wi-Fi 6, Bluetooth 5.4 LE and Classic, 802.15.4; **USB 2.0 high-speed OTG** with integrated transceiver; USB Serial/JTAG; LCD/camera controller. Module: 16 MB flash, 8 MB PSRAM, 22 × 30 × 3.5 mm, PCB antenna, 99 pins, 54 GPIOs. Already a library part in Sparksmith (section 6) | ESP32-S31 Series Datasheet v0.6 (pre-release, 2026-11-20) p. 5, p. 71; ESP32-S31-WROOM-3 & -3U Datasheet v0.7 (pre-release, 2026-09-03) p. 3, p. 15, p. 44–45, [documentation.espressif.com](https://documentation.espressif.com/esp32-s31-wroom-3_wroom-3u_datasheet_en.pdf) |
| U6 | Charger | TI BQ25188 | 1 A linear Li-ion charger with power path, I²C, 18 V input rating | [ti.com/product/BQ25180](https://www.ti.com/product/BQ25180) (names BQ25188 as the newer part) |
| U7 | 3.3 V rail | TI TPS63802 | Buck-boost, so 3.3 V holds over the whole cell range. Must deliver the module's 0.6 A (section 6); **TPS63802 ratings not read in this pass** | TI datasheet |
| DS1 | Display | Sharp LS013B7DH05 | 1.26 in, 144 × 168 memory LCD: readable in daylight, microwatts when static, SPI. Outline 20.88 × 24.36 mm per Future Electronics' listing (**not read from Sharp's datasheet**) | [DigiKey listing](https://www.digikey.com/product-detail/en/sharp-microelectronics/LS013B7DH05/LS013B7DH05-ND/5799456) |
| SW1 | Buttons | 4-way + centre switch, e.g. ALPS SKRHABE010 | **Ratings and footprint unverified** | ALPS datasheet |
| BT1 | Cell | 1S Li-ion pouch with JST S2B-PH-SM4-TB connector | Capacity to be sized against the Wi-Fi load in section 6 | JST PH datasheet |

Parts not used, and why:

- **TI TPS26750 / TPS26750A** (stand-alone PD controller, 240 W EPR sink, PD 3.2 certified per TI's EVM page). A good EPR sink, but a port controller cannot listen to two other parties or run the guard. Fallback if the STM32 EPR firmware is immature (risk R4).
- **TI TPS1663** (60 V, 6 A eFuse, 31 mΩ). 31 mΩ is 0.78 W at 5 A, and its absolute maximum is 62 V in revision G (67 V in older sources). Too little margin at 50.9 V plus a surge.

## 6. MCU: ESP32-S31, with an STM32G0B1 for PD

Founder decision: **ESP32-S31**, the chip of Sparksmith's ESP32-S31 dev board example.

### Module

**ESP32-S31-WROOM-3-N16R8V** (16 MB Quad SPI flash, 8 MB Octal SPI PSRAM, PCB antenna; WROOM-3 datasheet v0.7, Table 1-1, p. 3). Why this variant:

- It is already in Sparksmith: [examples/esp32-s31-wroom-3](../../examples/esp32-s31-wroom-3/README.md) has the component, symbol, footprint (cross-checked against Espressif's published footprint and STEP model) and 3D part file, and the [dev board](../../examples/esp32-s31-devboard/README.md) uses it. Reusing it means the autonomous build starts from a reviewed-shape part rather than a fresh import. The part is still an unreviewed draft, and all three Espressif documents are pre-release.
- 8 MB PSRAM is plenty for logs and a display buffer; 16 MB flash holds two firmware images for safe updates.
- **Availability:** no LCSC or JLCPCB listing for any ESP32-S31 module was found on 2026-11-23. An EEVblog thread (September 2026) reports the modules only began shipping in summer 2026 and that Espressif pointed small buyers to Taobao ([EEVblog](https://www.eevblog.com/forum/manufacture/esp32-s31-wroom-and-jlcpcb/)). Plan for JLCPCB consigned parts or global sourcing, and check stock at the BOM step (risk R14).
- The smaller **ESP32-S31-WROOM-1** (18.0 × 25.5 × 3.1 mm, 61 pins, same 54 GPIOs; WROOM-1 datasheet v0.5, Table 1-1, p. 3) would save board area. It is not in the library yet. If the 38 mm board width gets tight, switch to it (it is one more datasheet-to-component run).

### What the S31 brings

| Need | ESP32-S31 | Source |
|---|---|---|
| CPU | Dual-core 32-bit RISC-V up to 320 MHz, plus a low-power RISC-V core | Chip datasheet v0.6, p. 2–4 |
| Radio | 2.4 GHz Wi-Fi 6 (802.11ax) and 802.11b/g/n, Bluetooth 5.4 LE, Bluetooth Classic, 802.15.4 | Chip datasheet v0.6, cover and p. 2 |
| USB | **USB 2.0 High-Speed OTG** (480 Mbit/s, host or device, integrated transceiver), plus a USB Serial/JTAG controller | Chip datasheet v0.6, §4.2.2.6, p. 71 |
| Display | LCD and camera controller, general-purpose SPI | Chip datasheet v0.6, p. 5 |
| Flashing | Joint download boot over UART0, USB Serial/JTAG or USB 2.0 high-speed OTG | WROOM-3 datasheet v0.7, Table 4-3, p. 20 |
| No USB PD | Not listed among the peripherals, so U1 stays | Chip datasheet v0.6, p. 5 |

### USB wiring

- **J3 → OTG_HS** (module pins 40 USB_DP and 41 USB_DM; WROOM-3 datasheet Table 3-1, p. 15): a 90 Ω pair, so logs stream at USB 2.0 high speed and firmware can be flashed over the same port (OTG download boot).
- **USB Serial/JTAG** (IO33/IO34, module pins 42/43, the path the dev board example uses) goes to test pads for debugging and recovery. Note the dev board connects its USB-C to Serial/JTAG and leaves OTG_HS open; this board does the opposite.

### Pin budget (GPIOs)

| Function | GPIOs |
|---|---|
| Display (Sharp memory LCD: SCK, MOSI, CS, DISP, EXTCOMIN) | 5 |
| Buttons (4-way + centre) | 5 |
| UART to U1, plus U1's NRST and BOOT0 | 4 |
| I²C to U6 (charger) and a spare sensor bus | 2 |
| U6 interrupt, cell-voltage ADC | 2 |
| Boot strap IO61 (button), EN (reset) | 1 (+ EN, not a GPIO) |
| USB Serial/JTAG test pads (IO33/IO34) | 2 |
| **Total** | **21 of 54** (OTG_HS uses dedicated pins 40/41, not GPIOs) |

The INA228s and the UCPDs sit on U1, which owns all real-time PD and measurement work and sends results to U5 over UART.

### Power

- The module needs a supply able to deliver at least **0.6 A** at 3.0–3.6 V (WROOM-3 datasheet v0.7, Table 6-2, p. 44). U7 must meet this from a cell at its end-of-discharge voltage.
- Wi-Fi transmit peaks at 375 mA (802.11b, 1 Mbps, 20 dBm) and receive at about 110–120 mA (Table 6-4, p. 45). Logging over BLE or duty-cycled Wi-Fi keeps the average far lower; the cell is sized once firmware duty cycles are known (**not estimated here**).

### Does the S31 change the v1/v2 split?

Yes, in v2's favour. v2's USB 2.0 capture needs high-speed USB to stream to a PC, which was the only reason to consider an ESP32-P4 + ESP32-C6 pair. The S31 has USB 2.0 high-speed OTG **and** Wi-Fi 6/BLE in one chip, so v2 keeps the S31 and adds an FPGA (and its USB capture front end). The S31's OTG is a host/device controller, not a bus analyser, so it cannot capture traffic between two other devices by itself; the FPGA stays a v2 part. Nothing moves from v2 into v1.

## 7. Connectors, board thickness and the high-speed lanes

### How existing inline testers do it

| Product | Ports | Data pass-through | Voltage | Source |
|---|---|---|---|---|
| ChargerLAB POWER-Z KM003C / KM002C | "Standard USB-C Male (Plug) that supports USB4 Gen3 (Bidirectional)" and "Standard USB-C Female (Receptacle) that supports USB4 Gen3 (Bidirectional)" | USB4 Gen3 | VBUS 0–50 V; D+/D− and CC withstand 50 V | ChargerLAB POWER-Z KM003C/KM002C User Manual ([manuals.plus copy](https://manuals.plus/m/1bf47b09e519855ee177f4ed9169fd5ef6ce8c37fd4277e45968e428f176eefe.pdf)) |
| ChargerLAB POWER-Z C240 | Plug and receptacle | USB 2.0 (retail listing; **not from ChargerLAB**) | 240 W | [Newegg listing](https://www.newegg.com/p/0AF-0359-00060), [ChargerLAB](https://chargerlab.com/unboxing-of-the-brand-new-chargerlab-power-z-c240) |
| Plugable USBC-VAMETER3 | Plug and receptacle, bidirectional | "USB 1.1 through 3.1 Gen 2" (vendor claim) | EPR 4.5–50 V, 50 mA–6.5 A | [Plugable](https://plugable.com/en-ca/products/usbc-vameter3), [CDW](https://www.cdw.com/product/plugable-usb-c-power-meter-tester/7757102) |
| AVHzY CT-3 | Plug and receptacle | "USB 3.1" (seller listing, unverified) | 0–26 V | Seller listing |
| Infineon (Cypress) CY4500 EZ-PD Protocol Analyzer | "a Type-C plug at one end and a Type-C receptacle at the other" (J3 plug, J2 receptacle) on one board | Pass-through for VBUS, VCONN, USB 3.1 and USB 2.0 (Infineon product page) | SPR | CY4500 user guide 002-12170 ([PDF](https://www.infineon.com/dgdl/Infineon-CY4500_EZ-PD_Protocol_Analyzer_GUIDE-UserManual-v01_00-EN.pdf)); [product page](https://infineon.com/cms/en/product/evaluation-boards/cy4500) |
| Infineon CY4500-EPR | Same construction, "board enclosed in a transparent casing" | Not stated in the guide | "Extended power range (EPR) – 48 V at 5 A" | User guide 002-39580 Rev. \*\*, 2024-08-29 ([PDF](https://www.infineon.com/assets/row/public/documents/24/44/infineon-cy4500-epr-ez-pd-protocol-analyzer-utility-usermanual-en.pdf)) |
| Total Phase USB Power Delivery Analyzer (TP350110) | Type-C pass-through | SuperSpeed 5/10 Gbit/s (USB 3.1 Gen 1/2), USB 2.0 | Not checked | User manual ([Total Phase](https://www.totalphase.com/support/articles/217474437-USB-Power-Delivery-Analyzer-User-Manual/)) |
| PassMark USB-C 20G loopback plug | A single plug, not a pass-through | USB4 at 20 Gbit/s | Not checked | [PassMark specifications](https://www.passmark.com/products/usb20g-loopback/specifications.php) |

**No public teardown, board photo with dimensions, or published BOM of any of them was found** (searched for KM003C, KM002C, C240, Plugable, AVHzY, FNIRSI and Treedix teardowns; Infineon's CY4500 guides describe the board but list no connector part numbers or thickness). So which plug and receptacle they use, their board thickness, and whether any of them uses a stiffener, a stepped or milled edge, or rigid-flex are **unknown**; nothing found suggests any of those. I found no TI PD-analyzer reference board. What the list shows: professional analysers (Infineon, Total Phase) pass data up to 10 Gbit/s; the cheap meters limit data to USB 2.0 or 10 Gbit/s; only the KM003C claims USB4 Gen3 and 50 V together. That matches what the parts catalogues show: board-mount USB4 parts at 48 V are rare, and a 48 V USB4 plug was not found at all.

The thickness problem itself is known among makers: a USB-C test-board project on LCSC's open-hardware platform reports buying a 0.8 mm plug and a 1.0 mm receptacle and having to spread the receptacle's pins to fit ([oshwhub](https://oshwhub.com/hshgc/type-c-ce-shi-ban)). The fix used in practice is to buy plug and receptacle **made for the same board thickness**, and connector makers sell them that way: 0.8 mm "clamp" plugs (straddle mount) with 0.8 mm "sinking" (mid-mount) receptacles. The 0.8 mm clamp plugs from Chinese makers that I could read are USB 3.2 Gen 2 (10 Gbit/s), not USB4 (Mingchuang: Gen 2, PD 100 W; Hongkang: USB 3.1, 10 Gbit/s).

### Matched families checked (datasheets read 2026-11-23)

| Family | Plug | Receptacle | Same thickness? | Data rate | Voltage |
|---|---|---|---|---|---|
| **Same Sky USB4** | UP40-C-V-G-BMT-10B-P24-TR, straddle, 0.8 mm | **UJ40-C-H-G-MSMT-22B-P24-TR-68**, mid-mount, 0.8 mm | **Yes, 0.8 mm** | **40 Gbit/s (USB4 Gen 3)** | 20 Vdc both |
| Same Sky USB4, EPR receptacle | UP40 (0.8 mm) | UJ40-C-H-G-MSMT-TR, 1.0 mm, B row through-hole | No (0.8 / 1.0) | 40 Gbit/s | 20 V / 48 V |
| GCT 48 V | USB4155, horizontal SMT plug at the board edge, both rows on the top face, PCB 0.8–1.0 mm | GCT publishes Gen 2 receptacles at 48 V (USB4115, USB4160, USB4225 vertical; USB4200 flag type, dual-row SMT); **no horizontal Gen 2 receptacle confirmed** | Partly | **10 Gbit/s per lane (USB 3.2 Gen 2)** | **48 V DC, 5 A, 240 W** both |
| JAE DX07 | DX07P024MJ1 / DX07P024AJ1 plugs (2016 catalogue MB-0297/MB-0301): 10 Gbit/s (USB 3.1 Gen 2), 20 Vrms | DX07B024JAA mid-mount: USB4 40 Gbit/s and Thunderbolt 4 certified, "Supports 240W (5A/48V)", 48 Vrms (MB-0362-5, Oct 2022); footprint drawings SJ122843/4 not obtained | Unknown | Plug 10 Gbit/s | Plug 20 V, receptacle 48 V |
| Molex, Würth, Amphenol | USB 3.2 Gen 2 plugs listed (Molex 105444, Würth 632712000021), **not read** | Molex 105455 mid-mount USB4 receptacle listed at 20 V | Not checked | | |

Sources: `https://www.sameskydevices.com/product/resource/<part>.pdf`; `https://gct.co/files/drawings/<part>.pdf`; JAE [MB-0362-5](https://mm.digikey.com/Volume0/opasdata/d220001/medias/docus/5784/DX07.pdf) and [MB-0297-3](https://static.chipdip.ru/lib/914/DOC041914032.pdf).

A **matched USB4 Gen 3 pair at 48 V does not exist** in what I could read. A **matched pair at 48 V exists only at USB 3.2 Gen 2 (10 Gbit/s per lane)**, from GCT, and even there a horizontal receptacle is not confirmed. A **matched USB4 Gen 3 pair at 20 V does exist**, from Same Sky, for 0.8 mm boards.

### The fab angle: local thinning ruled out

- JLCPCB lists "Blind slot depth (D): ≥0.2mm" for FR4 boards ≥ 0.8 mm, and countersinks and back-drilling; no cavity with plated pads, no stepped or partial-thickness board ([capabilities](https://jlcpcb.com/capabilities/pcb-capabilities), read 2026-11-23). Special processes are priced automatically at order; no amount is published ([extra charges](https://jlcpcb.com/help/article/in-what-cases-will-there-be-charged-extra)).
- PCBWay's milling page describes depth milling as removing board material, "In general, milling does not require any additional charges", and asks not to mill copper ([PCB milling](https://www.pcbway.com/pcb_prototype/PCB_Milling.html)).
- Thinning a 1.0 mm board to 0.8 mm at the plug edge removes the bottom copper layer there, and the straddle plug needs pads on both faces. **Ruled out**, whatever it costs.

### Decision D2 (founder, 2026-11-23)

**One 0.8 mm, 4-layer JLCPCB board with Same Sky's matched USB4 pair, both used exactly as their datasheets say:**

- **J2 = UP40-C-V-G-BMT-10B-P24-TR** (plug, 0.8 mm, USB4 Gen 3, 20 Vdc).
- **J1 = UJ40-C-H-G-MSMT-22B-P24-TR-68** (receptacle, 0.8 mm, USB4 Gen 3, 20 Vdc).
- **J5 = GCT USB4110** (48 V, USB 2.0) for charger tests at full EPR.

The data rate stays **40 Gbit/s**. Both pass-through connectors are rated 20 Vdc, which matches the founder's SPR-only pass-through, and the EPR guard (section 8) keeps the path there. EPR charger testing moves to J5, which is rated 48 V. This is the matched-thickness approach the market uses, with no part outside its datasheet.

JLCPCB holds thin boards to "± 0.1mm" ("For the 0.8mm board thickness, the finished board thickness ranges from 0.7mm(T-0.1) to 0.9mm(T+0.1)", capabilities page, read 2026-11-23), the same range as the plug's 0.80 ± 0.10 mm gap. At the extremes the gap and the board meet at 0.7 or 0.9 mm with no margin; the plug's contacts are sprung (**fit to be checked on first articles**).

**The cost: four vias.** The 0.8 mm receptacle puts both contact rows on the top face (A row behind, B row in front by the cutout, 0.5 mm pitch), while the plug puts its B row on the bottom face. So the B-row lane conductors (TX2 at B2/B3 and RX1 at B10/B11) must change from L1 to L4 at J1. There is no room to escape the B row on L1 (the A row is behind it, the cutout in front), so each of the four conductors drops through a **via in its pad**, filled and capped. The via analysis below shows why this is acceptable: on a 0.8 mm board an L1-to-L4 via has **no stub**. These four filled vias in pad are the **one approved exception** to the no-via rule (founder, 2026-11-23); no other via may touch a lane. The B-row low-speed contacts (CC2, SBU2, D+/D− position 2) change layer the same way with ordinary vias.

Via-in-pad at JLCPCB: epoxy filled and capped is the default only from 6 layers; on 4 layers it is an option, "Additional charges will apply when the via hole<0.3mm and the via diameter≤0.4mm" (quote page, read 2026-11-23; amount not stated). The B-row pads sit at 0.5 mm pitch; their width must be read from the footprint (the drawing's 0.25 mm may be the row stagger, **unverified**). The via is therefore JLCPCB's smallest, 0.15 mm hole and 0.25 mm diameter ("Min. Via hole size/diameter: 0.15/0.25mm", `library/profiles/jlcpcb-4-layer.json`), placed at the pad's rear end. This is at the fabricator's absolute minimum, so the build's first check is the production check under `jlcpcb-4-layer` with the "Absolute minimum" values (**to be confirmed in Sparksmith on the real footprint**).

**Fallback, only if the via-in-pad check (O6) fails:** (A) UJ40-C-H-G-MSMT-TR as J1 (40 Gbit/s, 48 V, through-hole B row that changes layer without vias), 0.2 mm off its 1.0 mm datasheet and only with Same Sky's written confirmation; (B) GCT's 48 V parts, at 10 Gbit/s per lane, so USB4 would train at Gen 2; (C) JAE DX07B024JAA as J1, if its drawings show bottom-face B-row pads at 0.8 mm.

### How the lanes run

Each USB-C contact connects to the same contact on the other connector (A2 to A2, B11 to B11, and so on). The plug is mounted with its A row on the top face, so every pin lines up across the board and no pair crosses another.

- **A-row pairs** (TX1 at A2/A3, RX2 at A10/A11): J1 A-row pads on L1 → straight on L1 → J2 A-row pads on L1. No via.
- **B-row pairs** (TX2 at B2/B3, RX1 at B10/B11): J1 B-row pads on L1 → via in pad to L4 → straight on L4 → J2 B-row pads on L4.

Each pair runs over a solid ground plane; the two vias of a pair sit side by side, with ground return vias next to them.

### Vias and through-hole pins on a lane, analysed

A plated barrel through the board carries the signal like a via. What matters is whether any part of it hangs off the signal path as a **stub**: a dead end that reflects, worst at the quarter-wave frequency f = c / (4 · L · √εeff).

- **Via in pad, L1 to L4, on a 0.8 mm board:** the signal enters at L1 and leaves at L4, so the whole barrel is part of the path. **No stub.** What remains is the barrel's capacitance and its anti-pads on L2 and L3, which set its impedance (to be sized with the fabricator's field solver).
- **Through-hole pin (fallback A) routed out on L4:** again the whole barrel is in the path; the only stub is the pin tail below L4, in air (εeff ≈ 1, towards 2 with solder). For a 1–2 mm tail: f ≈ 26–75 GHz.
- **Through-hole pin routed out on L1 instead:** barrel (0.8 mm, √εr ≈ 2.1) plus tail become the stub, ≈ 3.7 mm electrical, f ≈ 20 GHz: twice the 10 GHz Nyquist frequency and inside the 15 GHz range the spec's loss limits cover. Never do this.
- A via from L1 to an **inner** layer would leave a stub to the far face; the design never does that.

USB4 Gen 3 runs 20 Gbit/s per lane with its Nyquist frequency at 10 GHz, and the specification's loss limits run to 15 GHz (Section 3.7.2.5.1, p. 108). A stubless transition keeps nothing resonant in that range.

### Stackup

A 4-layer FR4 board from JLCPCB with controlled impedance, **0.8 mm**:

- L1 top: A-row pairs, VBUS pours, parts.
- L2: solid ground, no splits under the lanes.
- L3: solid ground under the lanes; 3V3 and slow signals elsewhere.
- L4 bottom: B-row pairs, VBUS pours.

JLCPCB's impedance page ([jlcpcb.com/impedance](https://jlcpcb.com/impedance), read 2026-11-23) offers 4-layer controlled impedance at 0.8, 1.0, 1.2, 1.6 and 2.0 mm, and 6-layer only at 1.2, 1.6 and 2.0 mm, so the board is **4 layers**. Impedance tolerance ±10 % (capabilities page). Stated dielectric constants: prepreg 7628 4.4, 3313 4.1, 1080 3.91, 2116 4.16, core 4.6, solder mask 3.8. **The 0.8 mm stackup table could not be read by our fetch**; the build reads it and enters it into Sparksmith. On a 0.8 mm board the outer prepreg is thin, so 85 Ω traces will be narrow (about 4–6 mil, **estimate**).

### Impedance targets

| Pairs | Target | Source |
|---|---|---|
| USB4 TX/RX (4 pairs) | **85 Ω** differential, ±10 % (JLCPCB's tolerance). Founder decision D5 | USB Type-C R2.5 §3.7: S-parameters normalised to 85 Ω (p. 96). The raw cable pair target is 90 Ω ± 5 Ω (§3.7.1, p. 96); a board segment inside the channel follows the 85 Ω reference |
| USB 2.0 D+/D− (2 pass-through pairs; J3 to the S31) | 90 Ω differential, ±15 % | TI SLLU149E §5.3 (in Sparksmith's `library/standards/differential-pairs.json`) |

Track width and gap come from Sparksmith's `impedance.table` on the entered stackup and are checked with JLCPCB's impedance calculator before ordering ([high-speed routing](../high-speed-routing.md#impedance-model): the built-in model is quasi-static, about 5 % accuracy, no solder mask).

### Insertion loss estimate

Target lane length: **≤ 25 mm** pad to pad (section 11). Loss per inch at 10 GHz, from two public tables:

| Source | Material | Loss at 10 GHz | How derived |
|---|---|---|---|
| TI SNLA426 (June 2023), Table 9-1, 85 Ω pairs | FR4, Dk 4.4, Df 0.014 | ≈ 0.84 dB/in | Linear between the table's 0.686 dB (8 GHz) and 1.289 dB (16 GHz) |
| TI SNLA426, Table 9-1 | "Tetra Functional FR4", Dk 4.1, Df 0.022 | ≈ 1.18 dB/in | Between 0.961 dB and 1.844 dB |
| AMD UG863 rev 1.12 (June 2026), Tables 5–6, 6 mil trace | Standard FR4_LP, Df 0.017, 5 µm roughness | ≈ 1.6 dB/in | Linear between 1.085 dB (6 GHz) and 1.996 dB (12.9 GHz) |
| AMD UG863, 4 mil trace, 8 µm roughness | Same | ≈ 2.1 dB/in | Between 1.356 dB and 2.694 dB |

The interpolation is mine (loss is not linear in frequency; treat it as ±10 %). TI does not state trace geometry; AMD does not state microstrip or stripline. Our 0.8 mm board forces narrow traces (about 4–6 mil), so the AMD rows are the closer guide and the upper end of the range is the likelier one.

| Item | Loss at 10 GHz | Basis |
|---|---|---|
| 25 mm (0.98 in) of FR4 microstrip | 0.8–2.1 dB | Tables above |
| One added mated connector pair | up to 1.0 dB | Normative limit for a USB4 Gen3 receptacle, Table 3-29 (p. 124). The plug's own loss is not published (**unverified**) |
| B-row via in pad, L1 to L4 (2 of the 4 pairs) | small; no stub | Analysis above; capacitance not estimated |
| Plug straddle pads, receptacle pads | not estimated | Needs a field solver or a measurement |
| **Tool total** | **about 2–3 dB**, unverified | |

For scale: a whole passive USB4 Gen3 cable may lose up to 7.5 dB at 10 GHz (Section 3.7.2.5.1, p. 108). The host and device board budgets are in the USB4 specification, which I did not read.

**Remaining risk, honestly:** the tool takes a quarter to a half of a full cable's loss budget. With a short cable the link should stay at Gen3. With a cable already at its limit, it may train at Gen2 (20 Gbit/s) or below; USB4 falls back on its own, so the failure is slower, not broken. Reflections at the pads and vias may matter more than trace loss. The acceptance test T6 decides. Mitigations in cost order: shorter lanes; the thinnest prepreg JLCPCB offers with the widest trace that gives 85 Ω; low-loss laminate or a linear redriver in v2.

## 8. Keeping EPR off the pass-through: the guard

J1 and J2 are rated 20 Vdc. The firmware must **actively** stop any EPR contract on the pass-through path, and a hardware trip backs it up.

### Why listening alone is not enough

In transparent pass-through, CC is one wire shared by the charger and the device; the tool cannot change their messages. An EPR-capable charger sets the EPR Mode Capable bit in its first fixed PDO, and an EPR-capable device will then ask to enter EPR. The tool could only cut power or inject a Hard Reset, which a device would retry. So the guard ends the conversation on each side and speaks for each party.

### Hardware for it

- **S1**, a CC switch between the two port protectors, open when unpowered (section 5). Closed: the charger and the device talk directly (transparent). Open: the tool talks to each side separately (guarded).
- **UCPD1** (STM32G0B1) on the J1 side, after U4: listener in transparent mode, sink (Rd) in guarded mode.
- **UCPD2**, through mux S2 on the J2 side after U8: off in transparent mode, **source (Rp)** in guarded mode. (In charger-test mode S2 moves UCPD2 to J5.)
- **Q1/Q2 with U3** on the pass-through VBUS: off at power-up, whenever the tool is unpowered, and in charger-test mode.

### Firmware sequence

1. **Attach, probe first.** At power-up and after every detach, S1 is open and Q1/Q2 are off. When a source attaches on J1, UCPD1 presents Rd and receives the source's Source_Capabilities.
2. **Decide.**
   - If no PDO is above 20 V and the EPR Mode Capable bit is clear, the source can never offer EPR. The tool removes its Rd (the source sees a detach and drops VBUS), closes S1 and Q1/Q2, and listens. The source then sees the device and negotiates directly: **transparent mode**. USB4 and every alternate mode work exactly as without the tool.
   - Otherwise (an EPR-capable source): **guarded mode**. S1 stays open. UCPD1 stays a sink upstream. UCPD2 presents Rp downstream and offers the device a filtered copy of the source's capabilities.
3. **Filter (guarded).** Downstream the tool offers only the source's fixed PDOs up to 20 V, PPS and SPR AVS PDOs up to 20 V, with the EPR Mode Capable bit cleared and current capped at 5 A. It never sends EPR capabilities.
4. **Relay a power request (guarded).** The device's Request goes to the source as the tool's own Request for the same SPR PDO. Only after the source's PS_RDY and an INA228 reading inside the requested voltage's tolerance does the tool send PS_RDY to the device. Q1/Q2 close only once the first contract stands.
5. **Refuse EPR (guarded).** An EPR_Mode (Enter) or an EPR request from the device is answered as a source that has no EPR (Not_Supported or Reject, whichever PD R3.2 prescribes; **message choice to be read from the PD specification**). Upstream, the tool as sink never sends EPR_Mode (Enter) in pass-through.
6. **Relay everything else (guarded).** SOP structured VDMs (Discover Identity, SVIDs, Modes, Enter Mode), Enter_USB, DR_Swap and alerts are forwarded between the two sides, each side with its own MessageID and GoodCRC, so USB4 and DisplayPort entry still work. The relay must answer within PD's response timers (tSenderResponse and others, **values to be read from PD R3.2**); the source's voltage transition time adds to the device's wait.
7. **Watch (transparent).** If a new Source_Capabilities with the EPR bit appears (after a Hard Reset, for example), the tool opens Q1/Q2 at once, opens S1, and restarts at step 1.

### Hardware backstop, independent of firmware

- **U3's OV pin**: a resistor divider trips at **22 V** and turns Q1/Q2 off. 22 V is the author's choice: above 21.5 V, the highest a 20 V source may present by the spec's rule (nominal + 5 % + 500 mV), below anything EPR offers (28 V and up). Trip time **not read from the LM74502 datasheet**.
- **INA228's bus-overvoltage alert** (U2) at 21.5 V interrupts U1, which opens S1 and logs the event.
- **Fail-safe defaults**: S1 open and Q1/Q2 off with the tool unpowered or reset, so a flat battery means no pass-through, never an unguarded one.

What this does not cover: a source that violates PD and raises VBUS without a contract. The 22 V trip covers that too; J1's own VBUS pins still see whatever the source applies, as any 20 V receptacle would.

**User-visible rule:** "Pass-through (J1 → J2): up to 20 V. Charger test: plug the charger into J5, up to 48 V into the load port."

## 9. Power paths

| Path | Mode | Max |
|---|---|---|
| J1 → D1 → R1 → Q1/Q2 → D2 → J2 | Pass-through | 20 V (trip 22 V), 5 A, 100 W |
| J5 → D3 → R2 → Q3/Q4 → J4 | Charger test | 48 V nominal (50.9 V tolerated), 5 A, 240 W |

The two paths share no VBUS copper. GND is a plane on L2 and L3 and pours on L1/L4, joined at all connectors.

Losses at 5 A (I²R), per path:

| Element | Resistance | Loss at 5 A | Source |
|---|---|---|---|
| Shunt | 5 mΩ | 125 mW | |
| Two back-to-back FETs | 2 × ≤ 5 mΩ (target) | ≤ 250 mW | **target, FETs not chosen** |
| One mated connector pair, VBUS + GND | ≤ 40 mΩ per mated contact initial; 4 VBUS and 4 GND contacts in parallel, so ≤ 10 mΩ each way | ≤ 500 mW | Type-C R2.5 §3.7.8.1 (p. 134): "40 mΩ (Max) initial for VBUS, GND and all other contacts" |

Up to about 0.9 W in the tool at 5 A, and up to about 175 mV dropped across VBUS and GND together. Worst-case limits, not measurements.

**Copper width.** The reference is **IPC-2152** (*Standard for Determining Current Carrying Capacity in Printed Board Design*). It is a paid standard; buying it is the founder's call (decision D7). Until it is in hand, widths come from the older **IPC-2221** formula, I = k · ΔT^0.44 · A^0.725 (k = 0.048 external, 0.024 internal; A in mil²), as reproduced in public calculators (**formula not read from the standard**). IPC-2221's charts are generally more conservative than IPC-2152's for outer layers (**unverified**). Conservative choices: ΔT = 10 °C, and 1.5 × the result.

| Layer | Copper | ΔT | IPC-2221 width for 5 A | Used |
|---|---|---|---|---|
| Outer | 35 µm (1 oz) | 10 °C | 2.8 mm | **≥ 4 mm** on each of L1 and L4 |
| Inner | 15.2 µm (0.5 oz) | 10 °C | 17 mm | not used for VBUS |

The VBUS contacts are 0.25–0.35 mm pads, so the narrowest copper is at the connectors; it is short, and the makers rate the four VBUS contacts at 5 A together. Thermal test T3 checks it.

**Clearance at 48 V.** The J5-side VBUS net (to J4) needs a clearance to other nets. The reference is IPC-2221's electrical clearance table (paid; decision D7). Until then: a class-to-class isolation clearance in Sparksmith of **0.5 mm** between 48 V copper and everything else on outer layers (**author's conservative choice, unsourced**, well above JLCPCB's minimum spacing), marked unsourced in the record.

**Tool supply.** v1 runs from its battery, charged from J3. It never draws from the measured VBUS, so it does not change what it measures.

## 10. Layout rules

1. J1 and J2 on one axis at opposite board edges, A rows on L1. J5 and J4 together, away from the lanes.
2. USB4 pairs: straight, no test points, no ESD parts, no branches. A-row pairs on L1 with no via. B-row pairs drop to L4 through one filled via in each J1 B-row pad, and run on L4 to J2. No other via on any lane, and never a via to an inner layer. Length ≤ 25 mm. Intra-pair skew as close to zero as the pins allow; `pair_max_skew` 0.1 mm (**author's choice**).
3. Solid ground on L2 under L1 pairs and on L3 under L4 pairs, at least 3 track widths beyond each pair's outer edge (**author's choice**), no splits and no other vias under the pairs.
4. Ground return vias next to each B-row via pair and at both connectors' ground contacts, and stitching to both connector shells. Anti-pads on L2/L3 around the B-row vias sized for 85 Ω through the via (**to be set with the fabricator's field solver**).
5. Neighbouring pairs at least 3 × trace width apart (**author's choice**, common practice).
6. USB 2.0 D+/D− at 90 Ω: both pass-through pairs straight through (A6/A7 on L1, B6/B7 dropping to L4 at J1); J3 to the S31's OTG_HS pins.
7. CC and SBU: J1 → U4 → node A → (S1 for CC) → node B → U8 → J2, all short; UCPD taps at nodes A and B; J5's CC → U11 → S2.
8. VBUS: pours on L1 and L4 only, ≥ 4 mm; R1 and R2 with Kelvin traces to their INA228; each TVS within 5 mm of its connector; 0.5 mm isolation clearance on the 48 V net.
9. Hot parts (shunts, FETs) away from the cell.
10. ESP32-S31-WROOM-3 antenna at a board edge with the module's 22 × 6 mm keep-out, and the clearance Espressif's hardware design guidelines ask for (antenna outside the board or a cut-out with at least 15 mm clearance; HWDG v0.3 p. 28, as recorded in the module's review).
11. Fab profile `jlcpcb-4-layer`; board 0.8 mm; via-in-pad epoxy filled and capped.

## 11. Board size and enclosure

- **Board: about 40 × 100 mm, 4 layers, 0.8 mm (sketch).** The USB axis runs across the width near one end: about 40 mm edge to edge, less the plug pads and the receptacle footprint, leaves about 25 mm of lane. The ESP32-S31-WROOM-3 (22 × 30 mm, antenna at an edge), J5, J4, the display, buttons and the cell sit along the length. **Unverified until placed in Sparksmith**; if it does not fit, the WROOM-1 module (18 × 25.5 mm) is the first thing to change.
- **Enclosure:** two 3D-printed halves, about 46 × 108 × 16 mm, the plug out of one side, J1 opening on the other, J5 and J4 accessible, the display under a window, no metal over the antenna. Designed in SolidCraft against the board's published interface. The mid-mount receptacle hangs below the board; the enclosure leaves room for it.
- **Mechanical risk:** the tool hangs on the device's port by the plug. Keep it light (target under 45 g with the cell, **unverified**) and show the short-cable use in the manual. A 0.8 mm board is more flexible; place a mounting point near J2.

## 12. Cost

### BOM, prices read 2026-11-23

Snapshots from search results and distributor pages; they change daily. "Not read" means no price was obtained; none is invented.

| Part | Qty | Unit price at qty 1 | Source |
|---|---|---|---|
| INA228AIDGSR | 2 | ≈ US$3.68 | LCSC listing via search |
| TPD4S480RUKR | 3 | ≈ US$1.71 (LCSC), US$2.09 (DigiKey Brazil, USD) | Search snapshot |
| STM32G0B1CEU6 | 1 | ≈ US$4.05 | LCSC C2829190 via search |
| TDS5311P | 3 | ≈ US$1.58 (DigiKey via Findchips) | Search snapshot |
| Sharp LS013B7DH05 | 1 | ≈ US$7.72–7.94 (DigiKey, Octopart); 28-week lead time | Search snapshot |
| **Subtotal of read prices** | | **≈ US$29–30** | |
| ESP32-S31-WROOM-3-N16R8V | 1 | not read | Pre-release module |
| UJ40-C-H-G-MSMT-22B-P24-TR-68, UP40-C-V-G-BMT-10B-P24-TR | 1 each | not read | Datasheets dated 08/2026 |
| GCT USB4110, USB4105 | 1 each | not read | |
| 2 × CSS2H-2512K-5L00F, 2 × LM74502H, 4 MOSFETs, S1, S2, J4, BQ25188, TPS63802, buttons, cell, passives | | not read | |

No complete BOM total is claimed. The build's BOM step prices every line from JLCPCB/LCSC stock and records the date.

### Fabrication and assembly

JLCPCB's assembly price page ([help article, updated 22 Oct 2026](https://jlcpcb.com/help/article/pcb-assembly-price), read 2026-11-23), Standard PCBA:

| Fee | Amount |
|---|---|
| Setup | US$25.56 single side, US$51.12 both sides |
| Stencil | US$8.21 single side, US$16.42 both sides |
| SMT | US$0.0016 per joint (1–50,000) |
| Feeder loading | US$1.53 per extended part type |
| Hand soldering labour | US$3.58 per order |
| Manual assembly | US$0.0164 per joint |
| Handling | ≥ US$14.93 |
| Confirm parts placement | US$0.45 |

Fixed fees for one order, both sides (the plug straddles) and about 26 extended part types (**assumption**): 51.12 + 16.42 + 39.78 + 3.58 + 14.93 + 0.45 ≈ **US$126 per order**, plus about US$0.60 of joints per board (~370 joints, **assumption**). About **US$26 per board at 5 units** and **US$3.1 at 50 units**, before parts and the bare board. Via-in-pad fill and capping adds a charge not published (section 7).

**Bare PCB price: not read.** JLCPCB states only "From $2.00 / 5 pcs" for FR-4 and quotes 4-layer controlled impedance only with a file. The build's ordering step gets the real quote.

**Totals per 5 and per 50 units cannot be stated honestly yet**: the bare board and several BOM lines have no read price. The build record will close this.

## 13. Test plan

| # | Test | Method | Pass |
|---|---|---|---|
| T1 | Bring-up | Current-limited supply on the cell input; rails; both MCUs programmed (S31 over J3 OTG download boot); display; buttons | Rails within datasheet limits; both MCUs answer; J3 enumerates at high speed |
| T2 | V/I accuracy | Programmable supply and electronic load; a calibrated reference DMM in series and in parallel. Charger-test path (J5 → J4): 5, 9, 15, 20, 28, 36, 48 V. Pass-through path (J1 → J2): 5, 9, 15, 20 V. At 0.1, 1, 3, 5 A, both directions | Error within INA228's specified error plus the shunt tolerance (1 %) after one-point calibration; raw and calibrated recorded |
| T3 | Thermal | 48 V / 5 A J5 → J4 for 30 min; 20 V / 5 A J1 → J2 for 30 min; thermal camera | Hottest point and its rise recorded; no part above its rating |
| T4 | PD decode | Chargers: one SPR 100 W, one EPR 140 W (28 V), one EPR 240 W (48 V) with AVS; compare with a reference PD analyser where available | Every message decoded; EPR entry and AVS requests seen; e-marker identity decoded |
| T5 | Charger test | Request each offered PDO, including EPR 48 V and AVS steps, from the 240 W charger on J5, with an electronic load on J4 | VBUS within the PD tolerance of the request; J1/J2 untouched |
| T6 | **USB4 acceptance** | A USB4 40 Gbit/s NVMe enclosure and SSD on a USB4 host. Sequential read and write, same tool and settings, three runs, direct and then through the tool with the same cable. Record the link speed the host reports | **Link at USB4 Gen3 and throughput within 5 % of direct** (founder decision D6) |
| T7 | USB 2.0 pass-through | A USB 2.0 high-speed device through the tool | Enumerates at high speed |
| T8 | Fault | 48 V on CC and SBU at J1 and at J5 through a current limit | Tool survives; nothing above 21.5 V at J2; the protectors report the fault |
| T9 | **EPR guard** | 240 W EPR charger on J1, an EPR-capable laptop on J2, pass-through on. Scope on J2 VBUS for the whole session, across 20 plug-ins, a Hard Reset and a charger power cycle | J2 VBUS never above 21.5 V; laptop charges at its SPR contract; guarded mode logged |
| T10 | Hardware trip | Firmware guard disabled (test build), lab supply ramped 20 → 30 V on J1 | Q1/Q2 open at 22 V ± divider tolerance; J2 peak recorded |
| T11 | Guarded relay | A USB4 dock that is an EPR source, laptop on J2 | USB4 link comes up through guarded mode (Enter_USB relayed); otherwise recorded as a known limit |
| T12 | Plug fit | Measure finished board thickness at J2 on every first article; insert J2 into a reference receptacle 50 times | Board 0.7–0.9 mm; plug contacts hold; no lifted pads |

## 14. Risks

| # | Risk | Effect | Mitigation |
|---|---|---|---|
| R1 | No EPR-rated USB4 board-mount plug or matched pair | Pass-through limited to SPR; charger tests need J5 | Accepted for v1; vendor enquiry for v2 |
| R2 | 2–3 dB extra loss plus reflections | USB4 falls to Gen2 with long cables | Short lanes; measure (T6); v2 redriver |
| R3 | Via-in-pad on 0.5 mm-pitch pads at JLCPCB's absolute minimum | Fab refusal or yield loss on the four B-row vias | Production check under `jlcpcb-4-layer` minimum values; DFM before order; fallback in section 7 |
| R4 | STM32 PD firmware for PD 3.2 EPR and AVS immature | Charger test or decode gaps | TPS26750A as sink fallback; test early on ST's DISCO board |
| R5 | Tool outside the Type-C specification's defined parts | No compliance claim; users may chain it | Manual; no compliance logos |
| R6 | 48 V clearance and conductor sizing from IPC-2221 stand-ins, not IPC-2152 | Over- or under-design | Conservative values marked; T3; decision D7 |
| R7 | Active e-marker read needs VCONN and a source role | v1 sees the e-marker only when a source asks | v2 |
| R8 | Guarded relay of VDMs, Enter_USB and DR_Swap is complex firmware; PD timers are tight | Some docks or alternate modes fail in guarded mode | Transparent mode for every non-EPR source; T11; known-limits list |
| R9 | Lead times (display 28 weeks); new connector part numbers (datasheets 08/2026) may have little stock | Delay | Check stock before ordering; buy for 50 |
| R10 | Plug strain on the device's port | Damage to a device | Weight target, mounting point near J2, manual |
| R11 | S1, S2 and the TPD4S480 unpowered state not confirmed | Fail-safe default not met | O3 before layout |
| R12 | ESP32-S31 documents are pre-release (chip v0.6, WROOM-3 v0.7) | Pin or rating changes | Re-check at the final revisions; the module part stays a draft until reviewed |
| R13 | Thin-board tolerance (0.7–0.9 mm) at the edge of the plug's 0.70–0.90 mm gap | Loose or tight plug | T12 on first articles |
| R14 | ESP32-S31 modules not stocked by JLCPCB/LCSC (not found 2026-11-23) | Assembly needs consigned or sourced modules; delay | Consign modules to JLCPCB or use its global sourcing; WROOM-1 as a second option if stocked first |

## 15. What this board asks of Sparksmith

### Features it exercises

- Datasheet-to-component through MCP for every IC and every connector ([method](../../library/assist/workflows/datasheet-to-component/method.md)), with the review report per part; the ESP32-S31-WROOM-3 part reused from the library example.
- Schematic capture across 4–5 sheets; ERC.
- Stackup from the fabricator's table; `impedance.table` for 85 Ω and 90 Ω classes; `diff_pair.add`; `layout.route_pair` with a layer change at the B-row pads; skew and uncoupled-length checks.
- Copper fills for VBUS and ground; the mid-mount cutout; plated slots for shell legs; via-in-pad; class-to-class isolation clearance for the 48 V net.
- Fab profile `jlcpcb-4-layer` at 0.8 mm; `fabrication.preflight`; `project.export_fabrication` with BOM and pick-and-place; byte-identical replay (`--date none`).
- Board plan to native 3D and a SolidCraft enclosure through the published interface.

### Gaps it would expose (each to be confirmed against the code before work starts)

| # | Gap | Where it bites |
|---|---|---|
| G1 | No USB4 85 Ω entry in the shipped differential-pair table (`library/standards/differential-pairs.json`, still on branch `integ-1`, not `main`) | Pair suggestion for the eight USB4 nets |
| G2 | No insertion-loss or via model: impedance is quasi-static only | Section 7's estimate and via analysis are by hand |
| G3 | No fabricator stackup in the library beyond the generic 1.6 mm `fr4-1oz` preset | Entering JLCPCB's 0.8 mm stackup |
| G4 | Board-edge straddle footprints (pads on both faces of one part), mid-mount cutouts and filled via-in-pad: support unconfirmed | J1, J2 and the B-row vias |
| G5 | No net-class rule for "no vias", "these layers only" or "at most one via, through all layers" | Lane rules; today only by review |
| G6 | No check that the reference plane under a pair is continuous | Lane quality |
| G7 | No current-capacity check (IPC-2152 data, net-ties decision 8) | 5 A paths |
| G8 | Kelvin sense connections designed, not built ([net ties](net-ties.md)) | Shunts R1, R2 |
| G9 | No sourced high-voltage clearance rule | 48 V net |
| G10 | No netlist export (IPC-D-356); Gerber X2 net attributes exist | Independent verification ([protocol](autonomous-pcb-build-protocol.md)) |
| G11 | Net-intent check (harness H7) and MCP trace tap (H4) not built | Build record and checks |
| G12 | Placement by constraints (harness H10) not built | Placing the connectors on one axis |

## 16. Decisions

### Made by the founder, 2026-11-23

| # | Decision |
|---|---|
| D1 | Keep USB4. Charger tests as a PD sink cover full EPR 48 V / 240 W through a 48 V-rated receptacle (now J5). Pass-through is SPR only (≤ 20 V, 100 W), and the firmware actively blocks EPR in pass-through (section 8). v2: ask Same Sky, JAE, Amphenol and Molex for an EPR-rated board-mount USB4 plug |
| D2 | One 0.8 mm 4-layer board with Same Sky's matched USB4 pair (UP40-C-V-G-BMT-10B-P24-TR plug, UJ40-C-H-G-MSMT-22B-P24-TR-68 receptacle), 40 Gbit/s, no mechanical split. EPR charger tests on the separate GCT USB4110 port J5 with its own INA228 and CC mux S2. The four filled vias in pad on the B-row lane conductors at J1 are the one approved exception to the no-via rule (section 7) |
| D3 | MCU: ESP32-S31 (module variant recommended in section 6) |
| D5 | 85 Ω for the USB4 lanes |
| D6 | Acceptance: Gen3 link and within 5 % of direct SSD throughput |
| D7 | IPC-2152 and IPC-2221 are the references. Buying them is the founder's call; until then estimates stay conservative and marked |

### Recommended here, for confirmation

| # | Decision | Recommendation |
|---|---|---|
| D3a | Module variant | ESP32-S31-WROOM-3-N16R8V (in the library); WROOM-1 if the board is too tight |
| D4 | Display | Sharp LS013B7DH05 |
| D8 | Load port J4 for 240 W charger tests | A 2-pin connector rated ≥ 60 V, ≥ 5 A (part not chosen, O5) |

### Open items

| # | Item |
|---|---|
| O3 | Choose S1 and S2; confirm U4/U8/U11 on-resistance and unpowered state from the TPD4S480 PDF |
| O4 | Choose the four MOSFETs (≥ 80 V, ≤ 5 mΩ) from JLCPCB stock |
| O5 | Choose J4 |
| O6 | Read JLCPCB's 0.8 mm 4-layer impedance stackup; confirm via-in-pad on 4 layers and its charge |
| O7 | Vendor enquiry: EPR-rated USB4 plug and matched receptacle (v2); JAE drawings SJ122843/SJ122844; stock of the two Same Sky parts |
| O8 | Read PD R3.2 for the guard's reply messages and timers (section 8) |
| O9 | Size the cell once firmware duty cycles are known |

## Sources

- USB Type-C Cable and Connector Specification, Release 2.5, March 2026, usb.org (archive "USB Type-C 2.5 Release 202603.zip"), read 2026-11-23: §2.2 (p. 38–39), §2.4 (p. 42), §3.4.1 notes (p. 84), §3.7 and §3.7.1 (p. 96), §3.7.2.5.1 (p. 108), Table 3-29 (p. 124), §3.7.8.1 (p. 134), §4.6 (p. 225), §4.6.2 (p. 227).
- Espressif, read 2026-11-23 from documentation.espressif.com: ESP32-S31 Series Datasheet v0.6 (pre-release, 2026-11-20); ESP32-S31-WROOM-3 & -3U Datasheet v0.7 (pre-release, 2026-09-03); ESP32-S31-WROOM-1 & -1U Datasheet v0.5 (pre-release, 2026-09-18). Hardware design guidelines v0.3 as cited in `examples/esp32-s31-wroom-3`.
- Same Sky datasheets, read 2026-11-23 from `https://www.sameskydevices.com/product/resource/<part>.pdf`: [UJ40-C-H-G-MSMT-22B-P24-TR-68](https://www.sameskydevices.com/product/resource/uj40-c-h-g-msmt-22b-p24-tr-68.pdf) (08/21/2026), [UP40-C-V-G-BMT-10B-P24-TR](https://www.sameskydevices.com/product/resource/up40-c-v-g-bmt-10b-p24-tr.pdf) (rev 1.0), UJ40-C-H-G-MSMT-TR (rev 1.03), UJ40-C-H-G-L1-SMT-TR, UJ40-C-V-G-SMT-TR, UJ40-C-H-G-MSMT-2-TR, UJ40-C-H-G-MSMT-2B-P24-TR, UJ40-C-H-SMT-4B-P24-TR, UJ40-C-H-SMT-6B-P24-TR, UJ40-32-C-H-G3-MSMT-R-P24-TR, UJ40-32-C-H-G3-SMT-P24-TR.
- GCT drawings, read 2026-11-23 from `https://gct.co/files/drawings/<part>.pdf`: USB4110, USB4155, USB4115, USB4125, USB4151, USB4160, USB4200, USB4225.
- JAE: DX07 Series USB4 40Gbps Right Angle Receptacle, MB-0362-5, Oct 2022 ([PDF](https://mm.digikey.com/Volume0/opasdata/d220001/medias/docus/5784/DX07.pdf)); DX07 Series Receptacle and Plug, MB-0297-3 / MB-0301, 2015–2016 ([PDF](https://static.chipdip.ru/lib/914/DOC041914032.pdf)).
- Infineon CY4500 and CY4500-EPR user guides; Total Phase USB Power Delivery Analyzer user manual; PassMark USB-C 20G loopback specifications; EEVblog thread on ESP32-S31 and JLCPCB; all read 2026-11-23.
- ChargerLAB POWER-Z KM003C/KM002C User Manual ([manuals.plus](https://manuals.plus/m/1bf47b09e519855ee177f4ed9169fd5ef6ce8c37fd4277e45968e428f176eefe.pdf)); Plugable USBC-VAMETER3 product pages; LCSC open-hardware USB-C test board ([oshwhub](https://oshwhub.com/hshgc/type-c-ce-shi-ban)), all read 2026-11-23.
- TI: INA228 (SLYS021A), TPD4S480 (SLVSH00), LM74502 (SNOSDE5A), TPS1663 (SLVSET9G), TPS26750 (SLVSH67), BQ25180 (SLUSE99C), SNLA426 Table 9-1, SLLU149E §5.3.
- AMD UG863 rev 1.12, Material Properties and Insertion Losses, [docs.amd.com](https://docs.amd.com/r/en-US/ug863-versal-pcb-design/Material-Properties-and-Insertion-Losses).
- Semtech TDS5311P product page and launch figures.
- ST: DS13560 rev 6, AN5418, STM32 USB-C page.
- JLCPCB: [impedance](https://jlcpcb.com/impedance), [capabilities](https://jlcpcb.com/capabilities/pcb-capabilities), [extra charges](https://jlcpcb.com/help/article/in-what-cases-will-there-be-charged-extra), [assembly price](https://jlcpcb.com/help/article/pcb-assembly-price); PCBWay [PCB milling](https://www.pcbway.com/pcb_prototype/PCB_Milling.html); all read 2026-11-23.
