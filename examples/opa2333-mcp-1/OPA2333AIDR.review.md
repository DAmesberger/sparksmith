# Datasheet import review: OPA2333AIDR

Status: **draft, not published**. Nothing enters a library until a person reviews the rows below and publishes.

## Summary

Dual zero-drift CMOS op amp (TI OPA2333) in the 8-pin SOIC (D, TI outline D0008A). Built with the gull-wing generator (component, symbol, package with its 3D part-file model, footprint, binding). The generator's estimated lands were replaced by the datasheet's example land pattern (0.60 x 1.55 mm, 1.27 mm pitch, 5.4 mm row span). Pins 1-8 named and typed from the OPA2333 Pin Functions table. Check first: pin names/directions against p.4, the push-pull drive declared on OUT A/OUT B (inferred), and the land pattern against p.47. Note: library.pinout_apply could not be used because the plan's base value exceeds the client's integer precision; the same names, functions, sources and unreviewed state were written with library.edit, and a re-preview of the pinout document shows no differences.

## Sources

- **DS**: OPAx333 1.8-V, microPower, CMOS Operational Amplifiers, Zero-Drift Series (OPA333, OPA2333) SBOS351E, March 2006, revised December 2015 (package addendum dated 15-Jul-2026; D0008A drawing 4214825/C 02/2019)

## Open questions

- [ ] OUT A / OUT B are declared push-pull analog outputs; the pin table only says 'O'. Confirm the drive kind is acceptable for ERC.
- [ ] The body thickness is not stated (only 1.75 mm max overall); the 3D model uses 1.57 mm body over 0.18 mm standoff. Acceptable for the model?
- [ ] The standoff dimension reads .004-.010 [0.11-0.25] on p.46; .004 in is 0.10 mm. Confirm the reading (it affects only the 3D model).
- [ ] Courtyard (5.5 x 7.5 mm) and silkscreen come from the tool, not the datasheet; confirm against house rules.
- [ ] Pin names use ASCII hyphen-minus ('-IN A', 'V-') for the datasheet's minus sign; keep or change?
- [ ] The pinout was written with library.edit instead of library.pinout_apply (client integer-precision limit on the plan base); confirm each pin in the Pins tab.

## Extracted values

| Item | Value | Source | Confidence | Note |
|---|---|---|---|---|
| output drive | push_pull on OUT A and OUT B | DS p.1 Section 3 Description: rail-to-rail output swinging within 50 mV of the rails | Low | Inferred; the pin table gives only 'O'. |
| body height (3D model) | 1.57 mm (1.75 max minus 0.18 standoff) | DS p.46 D0008A Package Outline, .069 MAX [1.75] | Low | No nominal body thickness is stated; derived from the maximum height. |
| courtyard | 5.5 x 7.5 mm rectangle | DS p.47 D0008A Example Board Layout (lands the courtyard encloses) | Low | Not stated in the datasheet; drawn by library.footprint_from_pads around the lands and body. |
| body length (along the rows) | 4.81-5.00 mm; 4.905 mm used | DS p.46 D0008A Package Outline, .189-.197 [4.81-5.00] NOTE 3 | Medium | Midpoint of the stated range (derived). |
| body width | 3.81-3.98 mm; 3.895 mm used | DS p.46 D0008A Package Outline, .150-.157 [3.81-3.98] NOTE 4 | Medium | Midpoint (derived). |
| lead span | 5.80-6.19 mm; 5.995 mm used | DS p.46 D0008A Package Outline, .228-.244 TYP [5.80-6.19] | Medium | Midpoint (derived). |
| lead width | 0.31-0.51 mm; 0.41 mm used | DS p.46 D0008A Package Outline, 8X .012-.020 [0.31-0.51] | Medium | Midpoint (derived); 3D model only. |
| lead thickness | 0.13-0.25 mm; 0.19 mm used | DS p.46 D0008A Package Outline, .005-.010 TYP [0.13-0.25] | Medium | Midpoint (derived); 3D model only. |
| foot length | 0.41-1.27 mm; 0.84 mm used | DS p.46 D0008A Package Outline, Detail A, .016-.050 [0.41-1.27] | Medium | Midpoint (derived); 3D model only. |
| standoff | printed .004-.010 [0.11-0.25] mm; 0.18 mm used | DS p.46 D0008A Package Outline, Detail A | Medium | Midpoint (derived); 3D model only. |
| land pitch and positions | 1.27 mm; x = -1.905, -0.635, +0.635, +1.905 mm | DS p.47 D0008A Example Board Layout, 6X (.050) [1.27] | Medium | Positions derived from the pitch, symmetric about the package centre. |
| solder mask | NSMD, opening 0.07 mm max all around; footprint mask expansion 0.05 mm | DS p.47 D0008A Example Board Layout, Solder Mask Details | Medium | 0.05 mm is the footprint default and lies within the stated 0.07 mm max. |
| pin count / variant | 8 terminals, SOIC-8 (D), no exposed pad | DS p.4 OPA2333 D or DGK Package, 8-Pin SOIC or VSSOP, Top View; Pin Functions: OPA2333 | High |  |
| pin functions | 1 OUT A (O), 2 -IN A (I), 3 +IN A (I), 4 V- (power), 5 +IN B (I), 6 -IN B (I), 7 OUT B (O), 8 V+ (power) | DS p.4 Pin Functions: OPA2333, SOIC/VSSOP column | High | Names use ASCII '-' for the datasheet's minus sign. |
| pin 1 location | Pin 1 top-left in top view, pins numbered counter-clockwise; footprint pad 1 at (-1.905, -2.70) mm, pad 8 opposite at (-1.905, +2.70) mm | DS p.46 D0008A Package Outline, PIN 1 ID AREA | High |  |
| pitch | 1.27 mm (.050 in) | DS p.46 D0008A Package Outline, 6X .050 [1.27] | High |  |
| maximum height | 1.75 mm max (.069 in) | DS p.46 D0008A Package Outline, .069 MAX [1.75] | High | Package envelope height 1.75 mm. |
| land size | 0.60 x 1.55 mm (8X) | DS p.47 D0008A Example Board Layout, 8X (.024) [0.6], 8X (.061) [1.55] | High | Reference dimensions in parentheses on the drawing. |
| land row span | 5.4 mm centre to centre; rows at y = +/-2.70 mm | DS p.47 D0008A Example Board Layout, (.213) [5.4] | High | Row positions derived as half the span. |
| device marking | O2333A | DS p.26 Package Option Addendum, Packaging Information, OPA2333AIDR row, Part marking | High | Set as display-only 3D marking with a pin-1 dot. |

## Drafts

- component 2 ‘OPA2333AIDR’: pinout.unreviewed
- symbol 3 ‘OPA2333AIDR symbol’: 
- binding 6 ‘OPA2333AIDR binding’: 
- package 4 ‘OPA2333AIDR package’: 
- footprint 5 ‘OPA2333AIDR footprint’: 

Imported pins: 8 unreviewed, 0 uncertain. A person checks each against the datasheet in the Pins tab before publishing.
