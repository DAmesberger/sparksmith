use sparksmith_core::ObjectRef;
use sparksmith_core::fabrication::PreflightReport;
use sparksmith_core::issues::Severity;

/// What the 90 Ω check excludes of each of the committed J1.B6→U1.43 and
/// J1.B7→U1.42 paths, at the class's 0.127 mm gap on JLCPCB's
/// JLC04161H-7628 stackup.
pub(super) const USB_UNCOUPLED_MM: f64 = 7.932_388;

/// J1's (instance 106) two locating holes against the GND land beside each,
/// as GCT draws them: hole pad 17 by A1B12 (pad 1), hole pad 18 by B1A12
/// (pad 12).
const J1_HOLES: [(u32, u32, &str); 2] = [(17, 1, "A1B12"), (18, 12, "B1A12")];

/// USB_DN's path also runs coupled for 1.245 mm beside USB_DP copper off
/// USB_DP's path: J1's receptacle stubs at x 19.25 and 19.75 mm, 0.3 mm
/// wide, side by side from y 7.355 to 8.6 mm, 0.2 mm apart edge to edge,
/// within three pair gaps (3 × 0.127 mm).
const DN_BESIDE_DP_STUB_MM: f64 = 8.6 - 7.355;

/// The actual project uses net 6 for USB_DP and net 5 for USB_DN. Assert the
/// entire preflight so a missing coverage warning or any extra issue fails:
/// under JLCPCB 4-layer J1's two locating holes are not stated (JLCPCB
/// states "NPTH to Track" and no value for pads) and waived with a reason,
/// and the module's nine thermal-pad vias are filled and capped, so nothing
/// is said of them.
pub(super) fn assert_usb_preflight(report: &PreflightReport, uncoupled_mm: f64) {
    assert_eq!(
        (
            report.blocking,
            report.errors,
            report.warnings,
            report.waived
        ),
        (0, 2, 1, 2),
        "{report:?}"
    );
    assert_eq!(report.items.len(), 3, "{report:?}");
    let (holes, rest): (Vec<_>, Vec<_>) = report
        .items
        .iter()
        .partition(|i| i.code == "board.pad_hole_clearance");
    for (hole, land, name) in J1_HOLES {
        let item = holes.iter().find(|i| {
            i.objects
                == [
                    ObjectRef::Pad {
                        instance: 106,
                        pad: sparksmith_core::ids::PadId(hole),
                    },
                    ObjectRef::Pad {
                        instance: 106,
                        pad: sparksmith_core::ids::PadId(land),
                    },
                ]
        });
        assert!(item.is_some(), "J1 pad {hole}: {report:?}");
        let Some(item) = item else { continue };
        assert_eq!(item.severity, Severity::Error);
        assert_eq!(
            item.title,
            format!("J1 pad {hole}: hole is 0.175 mm from J1 pad {name} on GND")
        );
        assert!(
            item.not_stated.is_some() && item.needs.is_none(),
            "{item:?}"
        );
        assert!(
            item.waiver
                .as_ref()
                .is_some_and(|w| w.reason.contains("GCT")),
            "{item:?}"
        );
        assert!(!item.blocks());
    }
    assert_eq!(rest.len(), 1, "{report:?}");
    let item = rest[0];
    assert_eq!(item.code, "impedance.uncoupled");
    assert_eq!(item.severity, Severity::Warning);
    assert!(item.waiver.is_none());
    assert_eq!(
        item.objects,
        [ObjectRef::Net { id: 6 }, ObjectRef::Net { id: 5 }]
    );
    assert_eq!(
        item.title,
        format!(
            "USB: 90 Ω check excludes {uncoupled_mm:.3} mm of USB_DP and {:.3} mm of USB_DN",
            uncoupled_mm - DN_BESIDE_DP_STUB_MM
        )
    );
}
