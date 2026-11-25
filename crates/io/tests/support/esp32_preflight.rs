use sparksmith_core::ObjectRef;
use sparksmith_core::fabrication::PreflightReport;
use sparksmith_core::issues::Severity;

/// The committed J1.B6→U1.43 and J1.B7→U1.42 paths each measure 29.885562 mm.
/// Their parallel overlap at the 0.127 mm gap is 21.951411 mm per side.
pub(super) const USB_UNCOUPLED_MM: f64 = 7.934_151;

/// USB_DN's path also runs coupled for 1.245 mm beside USB_DP copper off
/// USB_DP's path: J1's receptacle stubs at x 19.25 and 19.75 mm, 0.3 mm
/// wide, side by side from y 7.355 to 8.6 mm, 0.2 mm apart edge to edge,
/// within three pair gaps (3 × 0.127 mm).
const DN_BESIDE_DP_STUB_MM: f64 = 8.6 - 7.355;

/// The module's exposed pad (U1) has nine ground vias, all open under its
/// solder paste.
const OPEN_IN_PAD: usize = 9;

/// The actual project uses net 6 for USB_DP and net 5 for USB_DN. Assert the
/// entire preflight so a missing coverage warning or any extra issue fails.
pub(super) fn assert_usb_preflight(report: &PreflightReport, uncoupled_mm: f64) {
    assert_eq!(
        (
            report.blocking,
            report.errors,
            report.warnings,
            report.waived
        ),
        (0, 0, 1 + OPEN_IN_PAD, 0),
        "{report:?}"
    );
    assert_eq!(report.items.len(), 1 + OPEN_IN_PAD, "{report:?}");
    let (open, rest): (Vec<_>, Vec<_>) = report
        .items
        .iter()
        .partition(|i| i.code == "board.via_in_pad_open");
    let mut vias = std::collections::BTreeSet::new();
    for item in &open {
        assert_eq!(item.severity, Severity::Warning);
        assert!(item.waiver.is_none());
        assert_eq!(item.title, "Via on GND in U1 95 is open under solder paste");
        let via = match item.objects[..] {
            [ObjectRef::Via { id }, ObjectRef::Pad { instance: 124, .. }] => Some(id),
            _ => None,
        };
        assert!(via.is_some(), "{item:?}");
        vias.extend(via);
    }
    assert_eq!(vias.len(), OPEN_IN_PAD, "each via once");
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
