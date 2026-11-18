//! One copper source for 2D and 3D: Layout draws the board plan's copper
//! (`sparksmith_solidcraft::project::plan::copper`, through
//! `apps/sparksmith/src/pcb_editor/copper_paint.rs`), the same bodies the
//! 3D view meshes and the exact build builds, so the two views cannot show
//! different copper.
//!
//! The check parses Layout (`apps/sparksmith/src/pcb_editor.rs` and
//! `pcb_editor/`) and rejects, outside test code, every way to derive
//! document copper geometry on its own:
//!
//! - the core's track and pad outline functions (`track_shape`,
//!   `track_outline`, `shape_contour`) and a pad's outline (`.outline()`
//!   without arguments);
//! - a fill's stored copper (`.computed`) and a via's pad size
//!   (`.diameter`).
//!
//! It also requires that `copper_paint.rs` takes the copper from
//! `plan::copper`. Counted, reasoned exceptions name functions that read
//! these for something other than drawing document copper (a property
//! row, a route preview of copper not yet in the document). Test code
//! (`#[cfg(test)]` items and `*tests.rs` files) is not checked.
use crate::{Result, root};
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

/// Layout, relative to the workspace root.
const LAYOUT: &[&str] = &[
    "apps/sparksmith/src/pcb_editor.rs",
    "apps/sparksmith/src/pcb_editor/",
];

/// Layout's one copper source, and the call it must make.
const SOURCE: (&str, &str, &str) = (
    "apps/sparksmith/src/pcb_editor/copper_paint.rs",
    "plan",
    "copper",
);

/// Functions that derive copper geometry from the document.
const DERIVE: &[&str] = &["track_shape", "track_outline", "shape_contour"];

/// Fields that hold copper geometry in the document.
const FIELDS: &[&str] = &["computed", "diameter"];

/// Functions that read copper geometry for something other than drawing
/// the document's copper: (file, function, why). A stale entry fails.
const EXCEPTIONS: &[(&str, &str, &str)] = &[
    (
        "apps/sparksmith/src/pcb_editor.rs",
        "bare_hole_land",
        "a bare hole has no copper and no plan body; its keep-out land is outlined, never filled",
    ),
    (
        "apps/sparksmith/src/pcb_editor/fill_card.rs",
        "fill_state",
        "the fill card counts the computed regions and removed islands; it draws nothing",
    ),
    (
        "apps/sparksmith/src/pcb_editor/via_card.rs",
        "via_section",
        "the via card shows the via's pad size as a value",
    ),
    (
        "apps/sparksmith/src/pcb_editor/route_tool.rs",
        "paint_plan",
        "a route step's preview draws copper the step would add, not yet in the document or the plan",
    ),
];

#[derive(Default)]
struct Finder {
    /// Uses outside an exception: (function, what, line).
    found: Vec<(String, String, usize)>,
    /// The innermost function being read.
    stack: Vec<String>,
    /// Whether `plan::copper` is called.
    calls_source: bool,
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Meta>()
                .is_ok_and(|m| m.path().is_ident("test"))
    })
}

fn line(span: proc_macro2::Span) -> usize {
    span.start().line
}

impl Finder {
    fn note(&mut self, what: String, at: usize) {
        let function = self.stack.last().cloned().unwrap_or_default();
        self.found.push((function, what, at));
    }
}

impl<'ast> Visit<'ast> for Finder {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !test_only(&item.attrs) {
            visit::visit_item_mod(self, item);
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !test_only(&item.attrs) {
            self.stack.push(item.sig.ident.to_string());
            visit::visit_item_fn(self, item);
            self.stack.pop();
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !test_only(&item.attrs) {
            self.stack.push(item.sig.ident.to_string());
            visit::visit_impl_item_fn(self, item);
            self.stack.pop();
        }
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let name = call.method.to_string();
        if DERIVE.contains(&name.as_str()) || (name == "outline" && call.args.is_empty()) {
            self.note(format!(".{name}()"), line(call.method.span()));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        let segments: Vec<String> = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        if let Some(last) = segments.last()
            && DERIVE.contains(&last.as_str())
        {
            let at = path
                .path
                .segments
                .first()
                .map_or_else(proc_macro2::Span::call_site, |s| s.ident.span());
            self.note(segments.join("::"), line(at));
        }
        if segments.len() >= 2
            && segments.get(segments.len() - 2).map(String::as_str) == Some(SOURCE.1)
            && segments.last().map(String::as_str) == Some(SOURCE.2)
        {
            self.calls_source = true;
        }
        visit::visit_expr_path(self, path);
    }

    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if let syn::Member::Named(name) = &field.member
            && FIELDS.contains(&name.to_string().as_str())
        {
            self.note(format!(".{name}"), line(name.span()));
        }
        visit::visit_expr_field(self, field);
    }
}

fn rust_files(directory: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn scan(source: &str) -> std::result::Result<Finder, String> {
    let syntax = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut finder = Finder::default();
    finder.visit_file(&syntax);
    Ok(finder)
}

/// Layout's source files, relative to the root, without test code.
fn sources(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    rust_files(&root.join("apps/sparksmith/src"), &mut files)?;
    let mut out = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let test = relative.ends_with("tests.rs") || relative.contains("/tests/");
        if !test && LAYOUT.iter().any(|m| relative.starts_with(m)) {
            out.push((relative, file));
        }
    }
    out.sort();
    Ok(out)
}

pub fn check() -> Result<()> {
    let root = root()?;
    let mut violations = Vec::new();
    let mut used = vec![false; EXCEPTIONS.len()];
    let mut source_called = false;
    for (relative, file) in sources(&root)? {
        let text = std::fs::read_to_string(&file)?;
        let finder = scan(&text).map_err(|e| format!("{relative}: cannot parse: {e}"))?;
        if relative == SOURCE.0 {
            source_called = finder.calls_source;
        }
        for (function, what, at) in finder.found {
            match EXCEPTIONS
                .iter()
                .position(|(f, n, _)| *f == relative && *n == function)
            {
                Some(k) => {
                    if let Some(slot) = used.get_mut(k) {
                        *slot = true;
                    }
                }
                None => violations.push(format!(
                    "{relative}:{at}: `{function}` reads {what}: Layout draws only the board \
                     plan's copper (copper_paint::LayoutCopper, from plan::copper), the bodies \
                     the 3D view meshes; take it from there"
                )),
            }
        }
    }
    if !source_called {
        violations.push(format!(
            "{}: Layout's copper must come from {}::{}",
            SOURCE.0, SOURCE.1, SOURCE.2
        ));
    }
    for ((file, name, _), used) in EXCEPTIONS.iter().zip(used) {
        if !used {
            violations.push(format!(
                "{file}: the exception for `{name}` matches nothing; remove it"
            ));
        }
    }
    if violations.is_empty() {
        eprintln!("One copper source for 2D and 3D passed");
        Ok(())
    } else {
        Err(violations.join("\n").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(source: &str) -> Vec<String> {
        scan(source)
            .map(|f| f.found.into_iter().map(|(_, w, _)| w).collect())
            .unwrap_or_default()
    }

    #[test]
    fn own_copper_geometry_is_found() {
        assert_eq!(
            found("fn f(t: &Track) -> Vec<Point> { layout::track_outline(t) }"),
            ["layout::track_outline"]
        );
        assert_eq!(
            found("fn f(p: &PlacedPad) -> Shape { p.outline() }"),
            [".outline()"]
        );
        assert_eq!(
            found("fn f(v: &Via) -> f64 { v.diameter / 2.0 }"),
            [".diameter"]
        );
        assert_eq!(
            found("fn f(f: &Fill) -> bool { f.computed.is_some() }"),
            [".computed"]
        );
        // A shape flattened at a tolerance, a fill's drawn outline field.
        assert!(found("fn f(s: &Shape, f: &Fill) { s.outline(0.1); f.outline.len(); }").is_empty());
        // Test code is skipped.
        assert!(
            found("#[cfg(test)] mod tests { fn t(p: &PlacedPad) { p.outline(); } }").is_empty()
        );
    }

    #[test]
    fn the_plan_call_is_seen() {
        assert!(
            scan("fn u(p: &Project, m: &mut plan::Memo) { let _ = plan::copper(p, m); }")
                .is_ok_and(|f| f.calls_source)
        );
    }

    #[test]
    fn every_exception_says_why() {
        for (file, name, why) in EXCEPTIONS {
            assert!(!name.is_empty() && !why.is_empty(), "{file}");
        }
    }
}
