# Datasheet import review: TLV75733PDBVR

Status: **draft, not published**. Nothing enters a library until a person reviews the rows below and publishes.

## Summary

TI TLV757P fixed 3.3 V LDO in SOT-23-5 (DBV0005A). Built with the tabbed generator (SOT-23-5 preset overridden by the datasheet values): component, symbol, package with 3D part-file model, footprint with the datasheet's example land pattern (5 lands 0.6 x 1.1 mm, rows 2.6 mm apart centre to centre, 0.95 mm pitch), binding. Pins IN 1, GND 2, EN 3, NC 4 (device_nc), OUT 5 from Table 4-1. Check first: pin names against Table 4-1, and pin 1 / pin 5 / pin 4 / pin 3 positions in the footprint against the DBV outline. The pinout was entered with library.edit, not library.pinout_apply: the apply's 64-bit plan base lost precision in this client's JSON transport and was refused as stale. No pin rules: the datasheet states no pin-use restrictions (no strapping, mux or exclusive functions).

## Sources

- **DS**: TLV757P 1A, Low IQ, Small-Size, Low-Dropout Regulator (SBVS322C), with DBV0005A package outline, example board layout and package addendum SBVS322C, October 2017, revised March 2024; DBV0005A drawing 4214839/K 08/2024

## Open questions

- [ ] Foot length 0.40 mm (3D model only) was chosen within the drawing's 0.3-0.6 mm range because the generator refuses the 0.45 mm midpoint; confirm this is acceptable for the 3D model.
- [ ] The pinout was set with library.edit instead of library.pinout_apply (the plan base was too large for this client's JSON transport); confirm each pin's name, class and function in the Pins tab.
- [ ] GND is modelled as a power_input function; confirm this suits your ERC conventions.
- [ ] NC (pin 4) is device_nc (no internal connection); the datasheet does not say whether it may be tied to a net. Confirm how you want to treat it.

## Extracted values

| Item | Value | Source | Confidence | Note |
|---|---|---|---|---|
| foot length | 0.40 mm (drawing range 0.3-0.6) | DS p.31 DBV0005A outline, 0.6/0.3 TYP | Low | Not the midpoint (0.45): the generator refuses 0.45 because (span - body width)/2 - foot = 0.15 mm must exceed the lead thickness 0.15 mm. 0.40 mm chosen inside the stated range; affects the 3D model only, not the lands. |
| body length | 2.90 mm (range 2.75-3.05) | DS p.31 DBV0005A outline, 3.05/2.75 | Medium | Derived: midpoint of min/max. |
| body width | 1.60 mm (range 1.45-1.75) | DS p.31 DBV0005A outline, 1.75/1.45 | Medium | Derived: midpoint. |
| lead span tip to tip | 2.80 mm (range 2.6-3.0) | DS p.31 DBV0005A outline, 3.0/2.6 | Medium | Derived: midpoint. |
| lead width | 0.40 mm (range 0.3-0.5) | DS p.31 DBV0005A outline, 5X 0.5/0.3 | Medium | Derived: midpoint. |
| lead thickness | 0.15 mm (range 0.08-0.22) | DS p.31 DBV0005A outline, 0.22/0.08 TYP | Medium | Derived: midpoint. |
| seated height | 1.175 mm (range 0.90-1.45 max) | DS p.31 DBV0005A outline, 1.45/0.90 | Medium | Derived: midpoint; body height 1.10 mm = 1.175 - standoff 0.075. |
| standoff | 0.075 mm (range 0.00-0.15) | DS p.31 DBV0005A outline, 0.15/0.00 TYP | Medium | Derived: midpoint. |
| solder mask | non-solder-mask-defined preferred, 0.07 mm max around; footprint uses project default 0.05 mm expansion | DS p.32 DBV0005A solder mask details | Medium | 0.05 mm is within the stated 0.07 mm max. |
| pin count (DBV) | 5 terminals, no thermal pad | DS p.3 Figure 4-1 and Table 4-1 | High |  |
| pin functions table | IN=1 (I), GND=2 (-), EN=3 (I), NC=4 (no internal connection), OUT=5 (O) | DS p.3 Table 4-1. Pin Functions, DBV column | High | GND entered as power_input, IN power_input, OUT power_output, EN digital input. |
| EN behaviour | active high; connect to IN if shutdown not needed; no internal pull stated | DS p.12 Section 6.3.2 Enable (EN) | High | No pull option declared. |
| pin numbering order | 1-2-3 down one side, 4 opposite 3, 5 opposite 1 (counter-clockwise from top) | DS p.31 DBV0005A package outline, top view | High | Generator footprint: pads 1-3 at y = -1.3 mm, x = -0.95/0/+0.95 mm; pad 4 at (+0.95, +1.3); pad 5 at (-0.95, +1.3). Same arrangement rotated 90 degrees. |
| lead pitch | 0.95 mm (1.9 mm between pins 1 and 3, and between 4 and 5) | DS p.31 DBV0005A outline, 2X 0.95 and 1.9 basic dimensions | High |  |
| land size | 0.6 mm x 1.1 mm (5X) | DS p.32 DBV0005A example board layout, 5X (0.6), 5X (1.1) | High | Reference dimensions in the drawing; corner radius R0.05 not modelled (square corners). |
| land row span centre to centre | 2.6 mm | DS p.32 DBV0005A example board layout, (2.6) | High |  |
| land pitch | 0.95 mm; 1.9 mm between pads 4 and 5 | DS p.32 DBV0005A example board layout, 2X (0.95), (1.9) | High |  |
| package marking code | 1FKF | DS p.25 Package option addendum, TLV75733PDBVR row | High | Not applied as a 3D marking. |

## Drafts

- component 2 ‘TLV75733PDBVR’: pinout.unreviewed
- symbol 3 ‘TLV75733PDBVR symbol’: 
- binding 6 ‘TLV75733PDBVR binding’: 
- package 4 ‘TLV75733PDBVR package’: 
- footprint 5 ‘TLV75733PDBVR footprint’: 

Imported pins: 5 unreviewed, 0 uncertain. A person checks each against the datasheet in the Pins tab before publishing.
