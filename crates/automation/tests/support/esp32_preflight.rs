use sparksmith_core::ObjectRef;
use sparksmith_core::fabrication::PreflightReport;
use sparksmith_core::issues::Severity;

/// The committed J1.B6→U1.43 and J1.B7→U1.42 paths each measure 29.885562 mm.
/// Their parallel overlap at the 0.127 mm gap is 21.951411 mm per side.
pub(super) const USB_UNCOUPLED_MM: f64 = 7.934_151;

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
        (0, 0, 1, 0),
        "{report:?}"
    );
    assert_eq!(report.items.len(), 1, "{report:?}");
    let item = &report.items[0];
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
            "USB: 90 Ω check excludes {uncoupled_mm:.3} mm of USB_DP and {uncoupled_mm:.3} mm of USB_DN"
        )
    );
}
