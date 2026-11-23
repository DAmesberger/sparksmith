//! The host part of the interface type: the bundled IBM Plex faces, user overrides, and
//! language-specific system fallbacks (CJK, Hangul, Thai, Hebrew), installed under the names the
//! kit's type API gives (see [`crate::kit::type_face`]).
//! The same definitions are used on desktop and web; the web shell adds Japanese and Chinese
//! faces on demand.

use std::sync::Arc;

use egui::{FontDefinitions, FontFamily};

use crate::kit::type_face::{
    BOLD_FAMILY, CHINESE_FALLBACK, CJK_FALLBACK, HANGUL_FALLBACK, MEDIUM_FAMILY, SEMIBOLD_FAMILY, SYMBOL_FALLBACK, UI_FONT, fallback_order,
    needs_cjk_font, sans_families, semibold,
};
// The Thai, Hebrew and Arabic faces come only from the desktop's system font search (and the tests).
#[cfg(not(target_arch = "wasm32"))]
use crate::kit::type_face::{ARABIC_FALLBACK, HEBREW_FALLBACK, THAI_FALLBACK};

const PLEX_SANS: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf"));
const PLEX_MONO: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf"));
const WEIGHTS: [(&str, f32); 4] = [("plex-sans", 400.0), (MEDIUM_FAMILY, 500.0), (SEMIBOLD_FAMILY, 600.0), (BOLD_FAMILY, 700.0)];

fn all_families() -> impl Iterator<Item = FontFamily> {
    sans_families().into_iter().chain([FontFamily::Monospace])
}

/// IBM Plex Sans at 400/500/600/700 and Plex Mono Regular, followed by egui's symbol/emoji
/// fallbacks. Each variable instance shares the embedded bytes and selects a real weight.
pub fn base_font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for ((name, weight), family) in WEIGHTS.into_iter().zip(sans_families()) {
        let mut data = egui::FontData::from_static(PLEX_SANS);
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
        fonts.families.insert(family, std::iter::once(name.to_owned()).chain(fallbacks.iter().cloned()).collect());
    }
    fonts.font_data.insert("plex-mono".to_owned(), Arc::new(egui::FontData::from_static(PLEX_MONO)));
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "plex-mono".to_owned());
    with_symbol_fallback(fonts)
}

// Keep pending definitions too: set_fonts takes effect on the next frame, while the web
// fetch and preference update can both finish before that frame starts.
fn installed_fonts_id() -> egui::Id {
    egui::Id::unique("solidcraft_interface_fonts")
}

fn install(ctx: &egui::Context, fonts: FontDefinitions) {
    let fonts = with_symbol_fallback(fonts);
    ctx.data_mut(|d| d.insert_temp(installed_fonts_id(), fonts.clone()));
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|style| {
        if let Some(heading) = style.text_styles.get_mut(&egui::TextStyle::Heading) {
            heading.family = semibold(heading.size).family;
        }
    });
}

/// Install only the bundled definitions (also used by the offscreen scenario shell).
pub fn setup_bundled_fonts(ctx: &egui::Context) {
    install(ctx, base_font_definitions());
}

/// The language fallbacks: system fonts, or the web app's download.
const LANGUAGE_FALLBACKS: [&str; 6] = [
    CJK_FALLBACK,
    CHINESE_FALLBACK,
    HANGUL_FALLBACK,
    crate::kit::type_face::THAI_FALLBACK,
    crate::kit::type_face::HEBREW_FALLBACK,
    crate::kit::type_face::ARABIC_FALLBACK,
];

/// `fonts` with egui's built-in Hack after the other bundled faces and before the language
/// fallbacks in every sans family. Ubuntu-Light, NotoEmoji and emoji-icon-font lack common symbols
/// such as ▲ ▼ ↑ ↓ → ⁻, which Hack has, so they no longer show as empty boxes where no system font
/// fills in (#2955). Text the other faces draw keeps them. A symbol Hack has no longer comes from a
/// language face, where it may be far larger than the text around it: Hiragino's ⇧ stands half as
/// tall again as a capital, in a full em. Hack has no letter, mark or punctuation of those
/// languages, so their text keeps its face, and a CJK UI's promoted face stays first.
pub fn with_symbol_fallback(mut fonts: egui::FontDefinitions) -> egui::FontDefinitions {
    if fonts.font_data.contains_key(SYMBOL_FALLBACK) {
        for family in sans_families() {
            let list = fonts.families.entry(family).or_default();
            list.retain(|name| name != SYMBOL_FALLBACK);
            let after_bundled = list.iter().rposition(|name| !LANGUAGE_FALLBACKS.contains(&name.as_str())).map_or(0, |last| last + 1);
            list.insert(after_bundled, SYMBOL_FALLBACK.to_owned());
        }
    }
    fonts
}

/// Use one primary CJK face for mixed-script labels in every sans weight family.
/// An explicit interface font stays first; Plex and the other fallbacks remain available.
fn promote_font_to_front(fonts: &mut FontDefinitions, name: &str) {
    for family in sans_families() {
        if let Some(list) = fonts.families.get_mut(&family)
            && let Some(pos) = list.iter().position(|entry| entry == name)
        {
            let item = list.remove(pos);
            let first = usize::from(list.first().is_some_and(|entry| entry == UI_FONT));
            list.insert(first, item);
        }
    }
}

/// Candidate font paths to look for on different operating systems.
#[cfg(not(target_arch = "wasm32"))]
const FONT_CANDIDATES: &[&str] = &[
    // Windows
    "C:\\Windows\\Fonts\\meiryo.ttc",
    "C:\\Windows\\Fonts\\YuGothM.ttc",
    "C:\\Windows\\Fonts\\YuGothR.ttc",
    "C:\\Windows\\Fonts\\msgothic.ttc",
    "C:\\Windows\\Fonts\\BIZ-UDGothicR.ttc",
    "C:\\Windows\\Fonts\\BIZ-UDMinchoM.ttc",
    "C:\\Windows\\Fonts\\yumin.ttf",
    // macOS
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/Hiragino Sans W3.ttc",
    "/System/Library/Fonts/Hiragino Sans W4.ttc",
    "/System/Library/Fonts/Hiragino Sans W6.ttc",
    "/System/Library/Fonts/Supplemental/Hiragino Sans W3.ttc",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/System/Library/Fonts/Supplemental/Songti.ttc",
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    "/Library/Fonts/Arial Unicode.ttf",
    "/Library/Fonts/Hiragino Sans W3.ttc",
    // Linux
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJKjp-Regular.otf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
    "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
    "/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf",
    "/usr/share/fonts/truetype/takao-gothic/TakaoGothic.ttf",
    "/usr/share/fonts/truetype/vlgothic/VL-PGothic-Regular.ttf",
    "/usr/share/fonts/truetype/ipafont/ipag.ttf",
    "/usr/share/fonts/truetype/ipaexfont/ipaexg.ttf",
    "/usr/share/fonts/opentype/ipafont/ipag.ttf",
    "/usr/share/fonts/opentype/ipaexfont/ipaexg.ttf",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    "/usr/share/fonts/cjkuni-uming/uming.ttc",
];

/// Hangul fonts, appended after the CJK fallback so Korean renders even when the CJK font found
/// is a Japanese one without Hangul (Meiryo, Hiragino).
#[cfg(not(target_arch = "wasm32"))]
const HANGUL_FONT_CANDIDATES: &[&str] = &[
    // Windows: Malgun Gothic
    "C:\\Windows\\Fonts\\malgun.ttf",
    // macOS: Apple SD Gothic Neo
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    "/System/Library/Fonts/Supplemental/AppleGothic.ttf",
    // Linux: Nanum Gothic (Noto Sans CJK already covers Hangul)
    "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
    "/usr/share/fonts/nanum/NanumGothic.ttf",
];

/// Thai fonts, appended last: none of the Latin or CJK faces draw Thai, so without one Thai text
/// shows as empty boxes (#1902).
#[cfg(not(target_arch = "wasm32"))]
const THAI_FONT_CANDIDATES: &[&str] = &[
    // Windows: Leelawadee UI (Windows 8 and later), Leelawadee, Tahoma
    "C:\\Windows\\Fonts\\LeelawUI.ttf",
    "C:\\Windows\\Fonts\\leelawad.ttf",
    "C:\\Windows\\Fonts\\tahoma.ttf",
    // macOS: Thonburi, Ayuthaya
    "/System/Library/Fonts/Thonburi.ttc",
    "/System/Library/Fonts/Supplemental/Thonburi.ttc",
    "/System/Library/Fonts/Supplemental/Ayuthaya.ttf",
    "/Library/Fonts/Thonburi.ttf",
    // Linux: Noto Sans Thai, TLWG Loma and Garuda
    "/usr/share/fonts/noto/NotoSansThai-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSansThai-Regular.ttf",
    "/usr/share/fonts/google-noto-vf/NotoSansThai[wght].ttf",
    "/usr/share/fonts/opentype/noto/NotoSansThai-Regular.ttf",
    "/usr/share/fonts/truetype/tlwg/Loma.ttf",
    "/usr/share/fonts/opentype/tlwg/Loma.otf",
    "/usr/share/fonts/truetype/tlwg/Garuda.ttf",
    "/usr/share/fonts/opentype/tlwg/Garuda.otf",
    "/usr/share/fonts/tlwg/Loma.otf",
];

/// Hebrew fonts, appended last: egui's built-in faces have no Hebrew, so without one the Hebrew
/// interface and Hebrew names show as empty boxes (#3457).
#[cfg(not(target_arch = "wasm32"))]
const HEBREW_FONT_CANDIDATES: &[&str] = &[
    // Windows: Segoe UI, Arial
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
    // macOS: Arial Hebrew, SF Hebrew
    "/System/Library/Fonts/ArialHB.ttc",
    "/System/Library/Fonts/SFHebrew.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    // Linux: Noto Sans Hebrew, DejaVu Sans
    "/usr/share/fonts/noto/NotoSansHebrew-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansHebrew-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSansHebrew-Regular.ttf",
    "/usr/share/fonts/google-noto-vf/NotoSansHebrew[wght].ttf",
    "/usr/share/fonts/opentype/noto/NotoSansHebrew-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
];

/// Arabic fonts with the joining forms the shaper uses. egui's built-in faces have no Arabic.
#[cfg(not(target_arch = "wasm32"))]
const ARABIC_FONT_CANDIDATES: &[&str] = &[
    // Windows: Segoe UI, Tahoma, Arial
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\tahoma.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
    // macOS: SF Arabic (11 and later), Geeza Pro, Damascus, Tahoma
    "/System/Library/Fonts/SFArabic.ttf",
    "/System/Library/Fonts/GeezaPro.ttc",
    "/System/Library/Fonts/Supplemental/GeezaPro.ttc",
    "/System/Library/Fonts/Supplemental/Damascus.ttc",
    "/System/Library/Fonts/Supplemental/Tahoma.ttf",
    // Linux: Noto Sans Arabic, Noto Naskh Arabic, KACST One, DejaVu Sans
    "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSansArabic-Regular.ttf",
    "/usr/share/fonts/google-noto-vf/NotoSansArabic[wght].ttf",
    "/usr/share/fonts/opentype/noto/NotoSansArabic-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoNaskhArabic-Regular.ttf",
    "/usr/share/fonts/noto/NotoNaskhArabic-Regular.ttf",
    "/usr/share/fonts/truetype/kacst-one/KacstOne.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
];

/// Fonts covering simplified Chinese, appended after the CJK fallback so Chinese renders even
/// when the CJK font found is a Japanese one without simplified characters (Hiragino Gothic,
/// Meiryo, Yu Gothic).
#[cfg(not(target_arch = "wasm32"))]
const CHINESE_FONT_CANDIDATES: &[&str] = &[
    // Windows: Microsoft YaHei
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\msyh.ttf",
    // macOS: PingFang SC, Hiragino Sans GB, Songti SC, STHeiti, Arial Unicode MS
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/STHeiti Medium.ttc",
    "/System/Library/Fonts/Supplemental/Songti.ttc",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/Library/Fonts/Arial Unicode.ttf",
    // Linux: Noto Sans CJK SC
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
];

/// The font folders named by `SOLIDCRAFT_FONT_DIRS` (a path list), or else by the single folder
/// in `CRAFT_FONTS_DIR` (the craft-fonts pack).
#[cfg(not(target_arch = "wasm32"))]
fn font_override_dirs(dirs: Option<std::ffi::OsString>, pack: Option<std::ffi::OsString>) -> Vec<std::path::PathBuf> {
    match (dirs, pack) {
        (Some(dirs), _) => std::env::split_paths(&dirs).collect(),
        (None, Some(pack)) => vec![std::path::PathBuf::from(pack)],
        (None, None) => Vec::new(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy)]
struct Script {
    name: &'static str,
    probes: &'static str,
    candidates: &'static [&'static str],
}

#[cfg(not(target_arch = "wasm32"))]
const SCRIPTS: [Script; 6] = [
    Script { name: CJK_FALLBACK, probes: "あ漢", candidates: FONT_CANDIDATES },
    Script { name: CHINESE_FALLBACK, probes: "设置", candidates: CHINESE_FONT_CANDIDATES },
    Script { name: HANGUL_FALLBACK, probes: "한글", candidates: HANGUL_FONT_CANDIDATES },
    Script { name: THAI_FALLBACK, probes: "กา่", candidates: THAI_FONT_CANDIDATES },
    Script { name: HEBREW_FALLBACK, probes: "את", candidates: HEBREW_FONT_CANDIDATES },
    Script { name: ARABIC_FALLBACK, probes: "بم", candidates: ARABIC_FONT_CANDIDATES },
];

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct SystemFonts(std::collections::BTreeMap<&'static str, Arc<egui::FontData>>);

#[cfg(not(target_arch = "wasm32"))]
impl SystemFonts {
    /// Collections may have different coverage in each face. Keep the index
    /// of the face that actually contains all the script's probe characters.
    fn consider(&mut self, path: &std::path::Path, scripts: &[Script]) {
        let Ok(bytes) = std::fs::read(path) else { return };
        let count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);
        // A TTC offset array physically needs four bytes per face.
        if count as usize > bytes.len() / 4 {
            return;
        }
        for index in 0..count {
            let Ok(face) = ttf_parser::Face::parse(&bytes, index) else { continue };
            let mut data = None;
            for script in scripts {
                if !self.0.contains_key(script.name) && script.probes.chars().all(|c| face.glyph_index(c).is_some()) {
                    let shared = data.get_or_insert_with(|| {
                        let mut font = egui::FontData::from_owned(bytes.clone());
                        font.index = index;
                        Arc::new(font)
                    });
                    self.0.insert(script.name, Arc::clone(shared));
                }
            }
            if scripts.iter().all(|s| self.0.contains_key(s.name)) {
                break;
            }
        }
    }

    fn search(&mut self, roots: &[std::path::PathBuf], scripts: &[Script]) {
        let mut dirs: Vec<_> = roots.iter().rev().cloned().collect();
        while let Some(dir) = dirs.pop() {
            if scripts.iter().all(|s| self.0.contains_key(s.name)) {
                break;
            }
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            let mut children = Vec::new();
            for entry in entries {
                let path = entry.path();
                // Descend only real directories, so symlink cycles cannot
                // recurse indefinitely. A symlink to a font file is fine.
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    children.push(path);
                } else if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|ext| ["ttf", "otf", "ttc", "otc"].iter().any(|e| ext.eq_ignore_ascii_case(e)))
                    && path.is_file()
                {
                    self.consider(&path, scripts);
                    if scripts.iter().all(|s| self.0.contains_key(s.name)) {
                        return;
                    }
                }
            }
            dirs.extend(children.into_iter().rev());
        }
    }
}

/// Explicit font directories take precedence. Prefer familiar system fonts
/// when available, then examine the remaining system and user directories.
/// File names never establish script coverage.
#[cfg(not(target_arch = "wasm32"))]
fn discover_fonts(overrides: &[std::path::PathBuf], scripts: &[Script], roots: &[std::path::PathBuf]) -> SystemFonts {
    let mut found = SystemFonts::default();
    found.search(overrides, scripts);
    for script in scripts {
        for path in script.candidates {
            if found.0.contains_key(script.name) {
                break;
            }
            found.consider(std::path::Path::new(path), std::slice::from_ref(script));
        }
    }
    found.search(roots, scripts);
    found
}

#[cfg(not(target_arch = "wasm32"))]
static SYSTEM_FONTS: std::sync::OnceLock<SystemFonts> = std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
fn system_fallbacks() -> &'static SystemFonts {
    SYSTEM_FONTS.get_or_init(|| {
        let overrides = font_override_dirs(std::env::var_os("SOLIDCRAFT_FONT_DIRS"), std::env::var_os("CRAFT_FONTS_DIR"));
        discover_fonts(&overrides, &SCRIPTS, &system_font_dirs())
    })
}

/// Whether the system fallback faces have been searched for.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_fonts_ready() -> bool {
    SYSTEM_FONTS.get().is_some()
}

/// Search for the system fallback faces on a thread and repaint when they are found.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_system_fonts_in_background(ctx: &egui::Context) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    if SYSTEM_FONTS.get().is_some() {
        return;
    }
    let mut first = false;
    STARTED.call_once(|| first = true);
    if !first {
        return;
    }
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("system-fonts".into()).spawn(move || {
        system_fallbacks();
        ctx.request_repaint();
    });
    // Without a thread the first frame waits for them, as it always did.
    if spawned.is_err() {
        system_fallbacks();
    }
}

/// [`setup_fonts_for`] with the system fallbacks that are there already, never waiting for the search.
#[cfg(not(target_arch = "wasm32"))]
pub fn setup_fonts_without_waiting(ctx: &egui::Context, language: &str) {
    install_fonts(ctx, language, SYSTEM_FONTS.get());
}

#[cfg(not(target_arch = "wasm32"))]
fn install_fonts(ctx: &egui::Context, language: &str, system: Option<&SystemFonts>) {
    let mut fonts = base_font_definitions();
    let order = fallback_order(language);
    for name in order {
        if let Some(data) = system.and_then(|f| f.0.get(name)) {
            fonts.font_data.insert(name.to_owned(), Arc::clone(data));
            for family in all_families() {
                let list = fonts.families.entry(family).or_default();
                list.retain(|entry| entry != name);
                list.push(name.to_owned());
            }
        }
    }
    if needs_cjk_font(language) {
        // Select the first installed face in language order. Keep explicit interface choices ahead of it.
        if let Some(primary) =
            order.into_iter().find(|name| ![THAI_FALLBACK, HEBREW_FALLBACK, ARABIC_FALLBACK].contains(name) && fonts.font_data.contains_key(*name))
        {
            promote_font_to_front(&mut fonts, primary);
        }
    }
    install(ctx, fonts);
}

/// Platform system and per-user font folders for all script fallbacks.
#[cfg(not(target_arch = "wasm32"))]
fn system_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Ok(windir) = std::env::var("WINDIR").or_else(|_| std::env::var("SystemRoot")) {
            dirs.push(std::path::PathBuf::from(windir).join("Fonts"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(std::path::PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        for d in ["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts"] {
            dirs.push(std::path::PathBuf::from(d));
        }
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(std::path::PathBuf::from(home).join("Library/Fonts"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(std::path::PathBuf::from("/usr/share/fonts"));
        dirs.push(std::path::PathBuf::from("/usr/local/share/fonts"));
        dirs.push(std::path::PathBuf::from("/run/host/fonts"));
        dirs.push(std::path::PathBuf::from("/run/host/user-fonts"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(std::path::PathBuf::from(&home).join(".local/share/fonts"));
            dirs.push(std::path::PathBuf::from(home).join(".fonts"));
        }
    }
    dirs
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod discovery_tests {
    use super::*;

    // Give the bundled test face a synthetic Unicode cmap. The font parser
    // still checks real TrueType tables; this test reads no installed fonts.
    fn font_with(chars: &[char]) -> Result<Vec<u8>, &'static str> {
        let mut bytes = PLEX_MONO.to_vec();
        let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let record = bytes[12..].chunks_exact(16).take(tables).find(|r| &r[..4] == b"cmap").ok_or("cmap")?;
        let offset = u32::from_be_bytes(record[8..12].try_into().map_err(|_| "offset")?) as usize;
        let len = u32::from_be_bytes(record[12..].try_into().map_err(|_| "length")?) as usize;
        let mut chars = chars.to_vec();
        chars.sort_unstable();
        chars.dedup();
        // Platform 3, encoding 10, format 12, one single-glyph group per character.
        let mut map = vec![0, 0, 0, 1, 0, 3, 0, 10, 0, 0, 0, 12, 0, 12, 0, 0];
        map.extend((16 + 12 * chars.len() as u32).to_be_bytes());
        map.extend(0u32.to_be_bytes());
        map.extend((chars.len() as u32).to_be_bytes());
        for c in chars {
            map.extend(u32::from(c).to_be_bytes());
            map.extend(u32::from(c).to_be_bytes());
            map.extend(1u32.to_be_bytes());
        }
        if map.len() > len {
            return Err("cmap space");
        }
        bytes[offset..offset + map.len()].copy_from_slice(&map);
        Ok(bytes)
    }

    #[test]
    fn installed_fallbacks_use_script_coverage_across_override_and_user_folders() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let override_dir = root.path().join("override");
        let user = root.path().join("user");
        let deep = user.join("family/weight/desktop/regular");
        std::fs::create_dir_all(&override_dir)?;
        std::fs::create_dir_all(&deep)?;
        std::fs::write(override_dir.join("NotoSansThai-Regular.ttf"), font_with(&['A'])?)?;
        let thai = font_with(&['ก', 'า', '่', 'ข'])?;
        std::fs::write(override_dir.join("locally-named.otf"), &thai)?;
        let mut scripts = SCRIPTS;
        for script in &mut scripts {
            script.candidates = &[];
        }
        let all: Vec<_> = scripts.iter().flat_map(|s| s.probes.chars()).collect();
        std::fs::write(deep.join("our-family.ttf"), font_with(&all)?)?;
        let fonts = discover_fonts(&[override_dir], &scripts, &[user]);
        for script in &scripts {
            let data = fonts.0.get(script.name).ok_or(script.name)?;
            let face = ttf_parser::Face::parse(&data.font, data.index).map_err(|_| "face")?;
            assert!(script.probes.chars().all(|c| face.glyph_index(c).is_some()), "{}", script.name);
        }
        let chosen = fonts.0.get(THAI_FALLBACK).ok_or("Thai")?;
        assert_eq!(chosen.font.as_ref(), thai.as_slice(), "explicit font folder takes precedence");
        Ok(())
    }
}
