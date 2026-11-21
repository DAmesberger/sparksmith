//! Generated 2D codes of a templated payload, laid out as their dark
//! modules: a QR Code (ISO/IEC 18004), encoded by Nayuki's `qrcodegen`
//! (MIT), or a square Data Matrix ECC 200 (ISO/IEC 16022), encoded by
//! `datamatrix` (MIT OR Apache-2.0).
//!
//! The symbol is drawn without its quiet zone (four light modules round a
//! QR Code, one round a Data Matrix, which the surface around it
//! provides); the layout keeps the zone free inside the marking area.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Payload bytes one code encodes (bounds the symbol to version 10 at
/// level M, 57 modules).
pub const MAX_PAYLOAD: usize = 128;
/// Light modules round a QR Code the area must leave free.
pub const QUIET_ZONE: u32 = 4;
/// Light modules round a Data Matrix (ISO/IEC 16022: one module).
pub const DATA_MATRIX_QUIET_ZONE: u32 = 1;
/// The smallest module side a decal draws, mm (est.: laser-marked module
/// codes are read down to about 0.1 mm modules).
pub const MIN_MODULE_MM: f64 = 0.08;

/// The symbology of a generated code.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Symbology {
    #[default]
    Qr,
    /// Data Matrix ECC 200, square symbols only; its error correction is
    /// fixed by the symbol size, so [`Ecc`] does not apply.
    DataMatrix,
}

impl Symbology {
    /// Light modules round the symbol the area must leave free.
    pub fn quiet_zone(self) -> u32 {
        match self {
            Self::Qr => QUIET_ZONE,
            Self::DataMatrix => DATA_MATRIX_QUIET_ZONE,
        }
    }
}

/// A QR Code error-correction level.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ecc {
    Low,
    #[default]
    Medium,
    Quartile,
    High,
}

/// An encoded symbol: `size` × `size` modules, row by row from the top
/// (`rows[0]` is the top row), `'1'` dark and `'0'` light.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Symbol {
    pub size: u32,
    pub rows: Vec<String>,
}

impl Symbol {
    /// Whether the module at column `x`, row `y` (from the top) is dark.
    pub fn dark(&self, x: u32, y: u32) -> bool {
        self.rows
            .get(y as usize)
            .and_then(|r| r.as_bytes().get(x as usize))
            .is_some_and(|b| *b == b'1')
    }
}

/// Encode `payload` as a QR Code at `ecc` (raised when that costs no
/// size), or as the smallest square Data Matrix that holds it; `Err`
/// names why it cannot be.
pub fn encode(symbology: Symbology, payload: &str, ecc: Ecc) -> Result<Symbol, String> {
    if payload.is_empty() {
        return Err("a 2D code has no payload".into());
    }
    if payload.len() > MAX_PAYLOAD {
        return Err(format!(
            "a 2D code's payload is at most {MAX_PAYLOAD} bytes"
        ));
    }
    match symbology {
        Symbology::Qr => {
            use qrcodegen::{QrCode, QrCodeEcc};
            let level = match ecc {
                Ecc::Low => QrCodeEcc::Low,
                Ecc::Medium => QrCodeEcc::Medium,
                Ecc::Quartile => QrCodeEcc::Quartile,
                Ecc::High => QrCodeEcc::High,
            };
            let code = QrCode::encode_text(payload, level)
                .map_err(|_| "the 2D code's payload is too long for a QR Code".to_string())?;
            let size = u32::try_from(code.size()).map_err(|_| "a QR Code has no size")?;
            let rows = (0..code.size())
                .map(|y| {
                    (0..code.size())
                        .map(|x| if code.get_module(x, y) { '1' } else { '0' })
                        .collect()
                })
                .collect();
            Ok(Symbol { size, rows })
        }
        Symbology::DataMatrix => {
            use datamatrix::{DataMatrix, SymbolList};
            let code =
                DataMatrix::encode(payload.as_bytes(), SymbolList::default().enforce_square())
                    .map_err(|_| {
                        "the 2D code's payload is too long for a Data Matrix".to_string()
                    })?;
            let bitmap = code.bitmap();
            let (width, height) = (bitmap.width(), bitmap.height());
            if width != height || width == 0 {
                return Err("a Data Matrix symbol is not square".into());
            }
            let size = u32::try_from(width).map_err(|_| "a Data Matrix has no size")?;
            let rows = bitmap
                .bits()
                .chunks(width)
                .map(|row| row.iter().map(|d| if *d { '1' } else { '0' }).collect())
                .collect();
            Ok(Symbol { size, rows })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fallible = std::result::Result<(), Box<dyn std::error::Error>>;

    /// ISO/IEC 18004 structure, checked independently of the encoder: the
    /// size is 4v + 17, the three finder patterns sit in their corners
    /// with their separators, and the timing patterns alternate.
    #[test]
    fn qr_symbols_have_the_standard_structure() -> Fallible {
        let s = encode(Symbology::Qr, "U1 SN-000123", Ecc::Medium)?;
        assert_eq!((s.size - 17) % 4, 0);
        assert_eq!(s.rows.len() as u32, s.size);
        assert!(s.rows.iter().all(|r| r.len() as u32 == s.size));
        let n = s.size;
        let finder = |x0: u32, y0: u32| {
            for y in 0..7 {
                for x in 0..7 {
                    let ring = x == 0 || y == 0 || x == 6 || y == 6;
                    let core = (2..=4).contains(&x) && (2..=4).contains(&y);
                    assert_eq!(s.dark(x0 + x, y0 + y), ring || core, "finder at {x0},{y0}");
                }
            }
        };
        finder(0, 0);
        finder(n - 7, 0);
        finder(0, n - 7);
        // Separators: the light ring outside the top-left finder.
        for k in 0..8 {
            assert!(!s.dark(7, k) && !s.dark(k, 7));
        }
        // Timing: row 6 and column 6 alternate between the finders.
        for k in 8..n - 8 {
            assert_eq!(s.dark(k, 6), k % 2 == 0);
            assert_eq!(s.dark(6, k), k % 2 == 0);
        }
        // The dark module beside the bottom-left finder (always dark).
        assert!(s.dark(8, n - 8));
        Ok(())
    }

    /// The symbols decode with an independent reader (`rqrr`, a Rust port
    /// of quirc) from a picture drawn the way a decal draws them: dark
    /// modules on a light ground with the quiet zone.
    #[test]
    fn generated_codes_decode_with_a_reference_reader() -> Fallible {
        for (payload, ecc) in [
            ("U1", Ecc::Medium),
            ("SN:2026-11-000123 LOT A7", Ecc::Low),
            ("https://example.com/p/ESP-0042", Ecc::High),
            (&"Q".repeat(MAX_PAYLOAD) as &str, Ecc::Quartile),
        ] {
            let s = encode(Symbology::Qr, payload, ecc)?;
            let px = 4u32;
            let side = (s.size + 2 * QUIET_ZONE) * px;
            let mut image = rqrr::PreparedImage::prepare_from_greyscale(
                side as usize,
                side as usize,
                |x, y| {
                    let m = |v: usize| (v as u32 / px).checked_sub(QUIET_ZONE);
                    match (m(x), m(y)) {
                        (Some(mx), Some(my)) if s.dark(mx, my) => 0,
                        _ => 255,
                    }
                },
            );
            let grids = image.detect_grids();
            assert_eq!(grids.len(), 1, "{payload}");
            let (_, text) = grids[0].decode()?;
            assert_eq!(text, payload);
        }
        Ok(())
    }

    /// ISO/IEC 16022 structure, checked independently of the encoder: a
    /// solid L finder on the left and bottom edges and alternating clock
    /// tracks on the top and right; and the symbol reads back with the
    /// crate's own decoder. The smallest square symbol holds the payload:
    /// Espressif's module Data Matrix of 18 characters (two reserved, the
    /// date code YYWW and the 12 hex digits of the MAC) takes 16 to 20
    /// modules a side by its characters (18 for a MAC with letters).
    #[test]
    fn data_matrix_symbols_have_the_standard_structure() -> Fallible {
        for payload in ["U1", "XXYYWW000000000000", &"D".repeat(MAX_PAYLOAD) as &str] {
            let s = encode(Symbology::DataMatrix, payload, Ecc::Medium)?;
            let n = s.size;
            assert_eq!(s.rows.len() as u32, n);
            for k in 0..n {
                assert!(s.dark(0, k), "{payload}: left finder at {k}");
                assert!(s.dark(k, n - 1), "{payload}: bottom finder at {k}");
                assert_eq!(s.dark(k, 0), k % 2 == 0, "{payload}: top clock at {k}");
                assert_eq!(
                    s.dark(n - 1, k),
                    k % 2 == 1,
                    "{payload}: right clock at {k}"
                );
            }
            let bits: Vec<bool> = (0..n * n).map(|i| s.dark(i % n, i / n)).collect();
            let read =
                datamatrix::DataMatrix::decode(&bits, n as usize).map_err(|e| format!("{e:?}"))?;
            assert_eq!(read, payload.as_bytes());
        }
        for (payload, side) in [
            ("XXYYWW000000000000", 16),
            ("XXYYWWMACMACMACMAC", 18),
            ("XX2246A0B1C2D3E4F5", 18),
        ] {
            assert_eq!(
                encode(Symbology::DataMatrix, payload, Ecc::Medium)?.size,
                side,
                "{payload}"
            );
        }
        assert_eq!(Symbology::DataMatrix.quiet_zone(), 1);
        Ok(())
    }

    #[test]
    fn payloads_are_bounded() {
        assert!(encode(Symbology::Qr, "", Ecc::Medium).is_err());
        assert!(encode(Symbology::Qr, &"x".repeat(MAX_PAYLOAD + 1), Ecc::Medium).is_err());
        assert!(encode(Symbology::Qr, &"x".repeat(MAX_PAYLOAD), Ecc::High).is_ok());
    }
}
