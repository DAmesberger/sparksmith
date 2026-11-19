//! `library.generate` (slice GP1): make a part with a generator module,
//! publish its revisions, embed the module and record where the part came
//! from, as one undo step on the candidate document. The module runs
//! through the engine's host runtime ([`GeneratorRuntime`]); the core
//! never interprets it.
use super::Outcome;
use crate::diagnostics::{Error, Result};
use crate::ids::ModelRef;
use crate::ids::{
    BindingRef, ContentHash, DefinitionId, DefinitionRef, FootprintRef, PackageRef, Revision,
    SymbolRef,
};
use crate::library::PackageModel;
use crate::library::bundle::{AssetBytes, MAX_ASSETS, PackedAsset};
use crate::library::generated::{
    self, GeneratedFrom, GeneratedPart, GeneratorParams, GeneratorRuntime, GeneratorScope,
    MAX_MODULE_BYTES, MAX_PARAMS, MODULE_MEDIA_TYPE, ParamValue, PartKey, expand,
};
use crate::library::model3d::{self, ImportModel, ModelDefinition, ModelFormat};
use crate::library::package_model::{PartDerived, PartFileModel, PartValue};
use crate::library::{DefinitionMeta, MAX_LABEL, MAX_NOTE, drafts};
use crate::model::Document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sparksmith_generator_abi::v1;
use std::sync::Arc;

/// `library.generate`'s parameters.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Generate {
    /// The module: its SHA-256, and its bytes unless the project already
    /// embeds it.
    pub module: ModuleSource,
    pub scope: GeneratorScope,
    /// Parameters by key (`library.generators` lists each module's). The
    /// rest are filled in: a parameter a project setting decides from the
    /// project, every other from the module's default. Lengths are integer
    /// nanometres, angles integer millidegrees.
    #[serde(default)]
    pub params: GeneratorParams,
    /// The part's name; the generator's suggestion when absent (the
    /// replaced part's name when `replaces` is given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The root revision of a generated part to make again: the new part is
    /// published as the next revision of each of its lineages instead of as
    /// new lineages, so `project.preview_update` can move instances to it
    /// with its impact listed. The usual way to take a changed module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<DefinitionRef>,
    /// The part's 3D model as the host evaluated it: the part file the
    /// module names, at its values (slice GP2). Required: a part never
    /// exists without its model, and the core never evaluates one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<GeneratedModel>,
    /// The SHA-256 of the bundle manifest the module ships in, when the
    /// host read it from a bundle: recorded as the part's provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<ContentHash>,
    /// The name of the module's preset the parameters start from
    /// (`SOT-23-5`), recorded with the part: its package in a person's
    /// words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// A generated part's 3D model, from the host: the part file the module's
/// generation names (by SHA-256) evaluated at its values. The core checks
/// that it is that file and those values; the bounds and surfaces are the
/// host's evaluation, rechecked by every board build.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GeneratedModel {
    /// The Model the part file becomes when the project holds none of
    /// those bytes yet: format `solidcraft`, its asset, bounds as the file
    /// is, licence and provenance.
    pub definition: ModelDefinition,
    /// That Model's name.
    pub name: String,
    /// The part file's bytes, unless the project packs them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<AssetBytes>,
    /// The generation's values in the part file's own units.
    pub values: std::collections::BTreeMap<String, PartValue>,
    /// What the part file evaluated to at those values.
    pub derived: PartDerived,
}

/// `library.package_from_generator`'s parameters (slice D1): a generator's
/// package as the open working set's own.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageFromGenerator {
    /// The package draft of the open working set.
    pub draft: crate::ids::DefinitionId,
    /// The package-scope `library.generate` to make (or find) the package
    /// with, its model evaluated by the host.
    pub generate: Generate,
    /// Replace a non-empty footprint instead of refusing it.
    #[serde(default)]
    pub replace: bool,
    /// How the package's current terminals carry over to the generated
    /// package's (and with them the binding's mappings): by number unless
    /// the request chooses position.
    #[serde(default)]
    pub terminals: super::working_package::TerminalMatch,
    /// With matching by number: current terminals that are a generated
    /// terminal under another number, stated explicitly. Never inferred.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renumber: Vec<super::working_package::Renumber>,
}

/// `library.package_from_generator`: make the package with its generator
/// (`library.generate`, package scope; the part the project already holds
/// is found, not made again), then give the working set's package and
/// footprint drafts its content, terminals matched as asked. One undo
/// step; any refusal changes nothing.
pub(crate) fn package_from_generator(
    document: &mut Document,
    request: PackageFromGenerator,
    runtime: Option<&dyn GeneratorRuntime>,
) -> Result<Outcome> {
    if request.generate.scope != GeneratorScope::Package {
        return Err(Error::Invalid(
            "generated.scope: a working set's package comes from a package-scope generation".into(),
        ));
    }
    super::working_package::check_working_package(document, request.draft, request.replace)?;
    let Outcome::Generated { report } = generate(document, request.generate, runtime)? else {
        return Err(Error::Invalid(
            "generated.record: the generator made no package".into(),
        ));
    };
    let library = &document.project.library;
    let package = report
        .revisions
        .iter()
        .find(|r| r.kind == crate::ids::DefinitionKind::Package)
        .and_then(|r| library.package(PackageRef::new(r.id, r.revision)))
        .map(|p| p.definition.clone());
    let footprint = report
        .revisions
        .iter()
        .find(|r| r.kind == crate::ids::DefinitionKind::Footprint)
        .and_then(|r| library.footprint(FootprintRef::new(r.id, r.revision)))
        .map(|f| f.definition.clone());
    let (Some(package), Some(footprint)) = (package, footprint) else {
        return Err(Error::Invalid(
            "generated.record: the generated package or its footprint is missing".into(),
        ));
    };
    let provenance = format!(
        "Package made by the generator ‘{}’ version {} (parameters recorded with its generated package {}); verify against the component datasheet.",
        report.key.generator,
        library
            .generated
            .iter()
            .find(|g| g.root == report.root)
            .map_or(0, |g| g.from.version),
        report.root.id
    );
    let outcome = super::working_package::adopt_generated_package(
        document,
        request.draft,
        request.replace,
        request.terminals,
        &request.renumber,
        (&package, &footprint),
        provenance,
    )?;
    retarget_working_set(document, request.draft);
    Ok(outcome)
}

/// The working set's sibling drafts pinning each other's pending revisions
/// (as the part editor's own edits pin them): the footprint drafts and
/// bindings of the package draft `package` name its pending revision, a
/// binding names its footprint drafts' pending revisions and a component
/// its binding drafts'. A drafted stored part pins the stored revisions it
/// was drafted from, so without this the footprint and binding would still
/// be checked against the package the generator replaced (E2E run 9).
fn retarget_working_set(document: &mut Document, package: DefinitionId) {
    use crate::library::Definition;
    let pending = |id: DefinitionId| document.drafts.get(id).and_then(|d| d.pending());
    let Some(package_revision) = pending(package) else {
        return;
    };
    let footprints: Vec<(DefinitionId, Revision)> = document
        .drafts
        .items
        .iter()
        .filter_map(|d| match &d.definition {
            Definition::Footprint(f) if f.package.id == package => Some((d.id, d.pending()?)),
            _ => None,
        })
        .collect();
    let bindings: Vec<(DefinitionId, Revision)> = document
        .drafts
        .items
        .iter()
        .filter_map(|d| match &d.definition {
            Definition::Binding(b) if b.package.id == package => Some((d.id, d.pending()?)),
            _ => None,
        })
        .collect();
    let drafts = Arc::make_mut(&mut document.drafts);
    for draft in &mut drafts.items {
        match &mut draft.definition {
            Definition::Footprint(f) if f.package.id == package => {
                f.package = PackageRef::new(package, package_revision);
            }
            Definition::Binding(b) if b.package.id == package => {
                b.package = PackageRef::new(package, package_revision);
                for r in &mut b.footprints {
                    if let Some((id, revision)) = footprints.iter().find(|(id, _)| *id == r.id) {
                        *r = FootprintRef::new(*id, *revision);
                    }
                }
            }
            Definition::Component(c) => {
                for r in &mut c.implementations {
                    if let Some((id, revision)) = bindings.iter().find(|(id, _)| *id == r.id) {
                        *r = BindingRef::new(*id, *revision);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A module by hash, with its bytes when they are not embedded yet.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModuleSource {
    pub hash: ContentHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<AssetBytes>,
}

/// What `library.generate` made or found.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GenerateReport {
    /// The part's root: its component (complete part) or package revision.
    pub root: DefinitionRef,
    /// Every revision of the part, root first.
    pub revisions: Vec<DefinitionRef>,
    pub key: PartKey,
    /// SHA-256 of the canonical generation.
    pub result: ContentHash,
    /// False when the project already held the part with this key: then
    /// nothing changed.
    pub created: bool,
    /// The part this one is the next revision of, when an earlier version
    /// of the same generator made it (or `replaces` named it). Instances
    /// still pin the old revision until `project.apply_update` moves them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced: Option<DefinitionRef>,
    /// The module's remarks for the person reviewing the part.
    pub notes: Vec<String>,
}

pub(super) fn runtime_needed() -> Error {
    Error::Unsupported(
        "generator.runtime: library.generate needs the host's generator runtime; this host has none".into(),
    )
}

/// The module's bytes: the inline copy (checked against its hash) or the
/// project's embedded one.
fn module_bytes(document: &Document, source: &ModuleSource) -> Result<(AssetBytes, bool)> {
    let packed = document
        .project
        .library
        .assets
        .iter()
        .find(|a| a.hash == source.hash);
    if let Some(asset) = packed
        && asset.media_type != MODULE_MEDIA_TYPE
    {
        return Err(Error::Conflict(format!(
            "generated.module: sha256 {} is packed in this project as {}, not as a generator module",
            source.hash, asset.media_type
        )));
    }
    match (&source.data, packed) {
        (Some(data), _) => {
            if data.len() > MAX_MODULE_BYTES {
                return Err(Error::Limit(format!(
                    "generator.too_large: the generator module is {} bytes, more than the {MAX_MODULE_BYTES} allowed. It was not run.",
                    data.len()
                )));
            }
            let actual = data.hash();
            if actual != source.hash {
                return Err(Error::Invalid(format!(
                    "generated.module_changed: the module's bytes hash to sha256 {actual}, not to the sha256 {} named with them. It was not run.",
                    source.hash
                )));
            }
            Ok((data.clone(), packed.is_none()))
        }
        (None, Some(asset)) => Ok((asset.data.clone(), false)),
        (None, None) => Err(Error::NotFound(format!(
            "generated.module_missing: the generator module sha256 {} is not embedded in this project; give its bytes",
            source.hash
        ))),
    }
}

/// What `library.generate` will run: the module's bytes (and whether the
/// project must embed them), its description and the complete request.
pub(super) struct Prepared {
    pub(super) bytes: AssetBytes,
    pub(super) embed: bool,
    pub(super) description: v1::Description,
    pub(super) wire: v1::Request,
}

/// The module bytes and complete request `request` runs on `document`:
/// what the host evaluates the part's model from before it applies
/// `library.generate` (the core never evaluates a part file).
pub fn generator_request(
    document: &Document,
    request: &Generate,
    runtime: &dyn GeneratorRuntime,
) -> Result<(AssetBytes, v1::Request)> {
    let prepared = prepare(document, request, runtime)?;
    Ok((prepared.bytes, prepared.wire))
}

pub(super) fn prepare(
    document: &Document,
    request: &Generate,
    runtime: &dyn GeneratorRuntime,
) -> Result<Prepared> {
    if request.params.len() > MAX_PARAMS {
        return Err(Error::Limit(format!(
            "generated.params: a request gives at most {MAX_PARAMS} parameters"
        )));
    }
    if let Some(name) = &request.name
        && (name.trim().is_empty() || name.len() > MAX_LABEL)
    {
        return Err(Error::Invalid(format!(
            "generated.name: a part's name is 1–{MAX_LABEL} bytes"
        )));
    }
    let (bytes, embed) = module_bytes(document, &request.module)?;
    let description = runtime
        .describe(&bytes)
        .map_err(|f| generated::failure(&f))?;
    let wire = generated::completed(
        &document.project.rules,
        &document.drafts,
        &description,
        request.scope,
        &request.params,
    )
    .map_err(|f| generated::failure(&f))?;
    Ok(Prepared {
        bytes,
        embed,
        description,
        wire,
    })
}

/// `library.generate`.
pub(crate) fn generate(
    document: &mut Document,
    request: Generate,
    runtime: Option<&dyn GeneratorRuntime>,
) -> Result<Outcome> {
    let runtime = runtime.ok_or_else(runtime_needed)?;
    let Prepared {
        bytes,
        embed,
        description,
        wire,
    } = prepare(document, &request, runtime)?;
    let key = generated::key(&description.id, &wire)?;
    let scope = request.scope;
    // The part this request makes again: the one named, or the newest part
    // a version of the same generator made from the same parameters in
    // the same scope (the regenerate rule, `docs/generator-plugins.md`).
    let replaces = match request.replaces {
        Some(root) => Some(root),
        None => match generated::predecessor(
            &document.project.library,
            &description,
            request.scope,
            &request.params,
            &wire,
        ) {
            Some(part) if part.from.version == description.version => {
                // The same version of the generator, whatever bytes it was
                // built to: the part it made stands, and nothing changes.
                let same = found(part, key);
                // Another name asks for another part on the same package
                // (a second MPN in the same can): its own component.
                let root = part.root;
                let other_name = request.name.as_deref().filter(|name| {
                    scope == GeneratorScope::Component
                        && document
                            .project
                            .library
                            .definition(root)
                            .is_some_and(|(meta, _)| meta.name.trim() != name.trim())
                });
                return match other_name {
                    Some(name) => second_part(document, root, name, same),
                    None => Ok(same),
                };
            }
            Some(part) if part.from.version > description.version => {
                refuse_older(document, &description, part)?;
                None
            }
            Some(part) => Some(part.root),
            None => None,
        },
    };
    // The lineages and their base revisions: the replaced part's, or new.
    let replaced = match replaces {
        Some(root) => {
            let library = &document.project.library;
            let part = generated::record_of(library, root).ok_or_else(|| {
                Error::NotFound(format!(
                    "generated.replaces: {} {} is not the root of a generated part",
                    root.id, root.revision
                ))
            })?;
            if part.from.scope != scope {
                return Err(Error::Invalid(format!(
                    "generated.replaces: {} was made in the other scope",
                    generated::describe(library, part)
                )));
            }
            if part.from.generator != description.id {
                return Err(Error::Invalid(format!(
                    "generated.replaces: {} was made by the generator ‘{}’, not by ‘{}’",
                    generated::describe(library, part),
                    part.from.generator,
                    description.id
                )));
            }
            refuse_older(document, &description, part)?;
            let mut lineages = Vec::with_capacity(part.expansion.len());
            for r in &part.expansion {
                let latest = library
                    .latest(r.id)
                    .map(|(_, revision)| revision)
                    .ok_or_else(|| {
                        Error::NotFound(format!("generated.replaces: {} is gone", r.id))
                    })?;
                lineages.push((r.id, Some(latest)));
            }
            let name = library.definition(root).map(|(meta, _)| meta.name);
            Some((lineages, name, root, part.from.ids.clone()))
        }
        None => None,
    };
    let made = runtime
        .generate(&bytes, &wire)
        .map_err(|f| generated::failure(&f))?;
    // A part of a generic family selects its family's standard fields, so
    // each placed part holds its own value ("4.7 kΩ").
    let complete: GeneratorParams = wire
        .params
        .iter()
        .map(|(k, v)| (k.clone(), ParamValue::from_wire(v)))
        .collect();
    let selects_fields = scope == GeneratorScope::Component
        && crate::library::standards::get()?
            .generic
            .family_of(&description.id, &complete)
            .is_some_and(|f| !f.fields.is_empty());
    let host_fields = if selects_fields {
        super::library::import_bundle(document, &crate::library::standard_fields::bundle()?)?;
        generated::family_fields(&document.project.library, &description.id, &complete)?
    } else {
        None
    };
    // The generator's default top marking, its standard-field placeholders
    // turned into the project's fields (host data, as the fields are).
    let host_marking = match &made.generation.marking {
        Some(marking) if scope == GeneratorScope::Component => {
            let names_fields = std::iter::once(marking)
                .chain(&marking.decals)
                .flat_map(|m| &m.lines)
                .any(|l| l.contains("field:"));
            if !selects_fields && names_fields {
                super::library::import_bundle(
                    document,
                    &crate::library::standard_fields::bundle()?,
                )?;
            }
            let standards = crate::library::standards::get()?;
            let library = &document.project.library;
            expand::marking(&made.generation, &description.licence, |name| {
                standards.fields.local(library, name).map(|f| f.id)
            })?
        }
        _ => None,
    };
    let result = ContentHash(made.result_hash);
    let supplied = supplied_model(&request, &made.generation)?;
    let (held, new_model) = model_of(document, supplied)?;
    // The bundle: the one the host named, or the one recorded with the
    // same module before.
    let bundle = request.bundle.or_else(|| {
        document
            .project
            .library
            .generated
            .iter()
            .find(|p| p.from.module == request.module.hash)
            .and_then(|p| p.from.bundle)
    });
    let project = Arc::make_mut(&mut document.project);
    let model_ref = match (held, &new_model) {
        (Some(held), _) => held,
        (None, _) => ModelRef::new(DefinitionId(project.allocate()?), Revision::FIRST),
    };
    let package_model = PackageModel::PartFile(Box::new(PartFileModel {
        model: model_ref,
        values: supplied.values.clone(),
        derived: supplied.derived.clone(),
    }));
    let lineages: Vec<(DefinitionId, Option<Revision>)> = match &replaced {
        Some((lineages, _, _, _)) => lineages.clone(),
        None => {
            let mut fresh = Vec::with_capacity(scope.kinds().len());
            for _ in scope.kinds() {
                fresh.push((DefinitionId(project.allocate()?), None));
            }
            fresh
        }
    };
    let mut expansion = Vec::with_capacity(lineages.len());
    for (kind, (id, base)) in scope.kinds().iter().zip(&lineages) {
        let revision = match base {
            Some(base) => base.next().ok_or_else(|| {
                Error::Limit(format!("generated.replaces: {id} has no next revision"))
            })?,
            None => Revision::FIRST,
        };
        expansion.push(DefinitionRef {
            kind: *kind,
            id: *id,
            revision,
        });
    }
    let at = |i: usize| -> Result<DefinitionRef> {
        expansion
            .get(i)
            .copied()
            .ok_or_else(|| Error::Invalid("generated.expansion: wrong number of lineages".into()))
    };
    let refs = match scope {
        GeneratorScope::Component => {
            let (component, symbol, package, footprint, binding) =
                (at(0)?, at(1)?, at(2)?, at(3)?, at(4)?);
            expand::Refs {
                component: Some((
                    component.id,
                    SymbolRef::new(symbol.id, symbol.revision),
                    BindingRef::new(binding.id, binding.revision),
                )),
                package: PackageRef::new(package.id, package.revision),
                footprint: FootprintRef::new(footprint.id, footprint.revision),
            }
        }
        GeneratorScope::Package => {
            let (package, footprint) = (at(0)?, at(1)?);
            expand::Refs {
                component: None,
                package: PackageRef::new(package.id, package.revision),
                footprint: FootprintRef::new(footprint.id, footprint.revision),
            }
        }
    };
    let mut ids: Vec<DefinitionId> = lineages.iter().map(|(id, _)| *id).collect();
    let bases: Vec<Option<Revision>> = lineages.iter().map(|(_, base)| *base).collect();
    let previous_ids = replaced
        .as_ref()
        .map(|(_, _, _, ids)| ids.clone())
        .unwrap_or_default();
    let (mut definitions, key_ids) = generated::expand_at(
        &made.generation,
        refs,
        &expansion,
        package_model,
        &previous_ids,
    )?;
    if let Some((fields, template)) = &host_fields {
        generated::select_fields(&mut definitions, fields, template.as_ref());
    }
    let (host_marking, artwork) = match host_marking {
        Some((spec, artwork)) => (Some(spec), artwork),
        None => (None, Vec::new()),
    };
    generated::set_marking(&mut definitions, host_marking.as_ref());
    generated::set_leads(
        &mut definitions,
        &generated::generation_leads(&made.generation)?,
    );
    // A part of a generic family is named by the family's label
    // ("Chip resistor 0603", "Pin header 2×5 2.54 mm"), as the Place
    // palette and the chat name it, so every search reads it alike.
    let family_label = (scope == GeneratorScope::Component)
        .then(|| crate::library::standards::get().ok())
        .flatten()
        .and_then(|standards| {
            let family = standards.generic.family_for(&description.id, &complete)?;
            let size = request.preset.as_deref().and_then(|p| {
                description
                    .presets()
                    .iter()
                    .find(|x| x.key == p || x.name == p)
                    .and_then(|x| x.aliases.first().cloned())
            });
            standards
                .generic
                .label_for(family, &complete, size.as_deref())
        });
    let base_name = request
        .name
        .clone()
        .or_else(|| replaced.as_ref().and_then(|(_, name, _, _)| name.clone()))
        .or(family_label)
        .unwrap_or_else(|| {
            made.generation
                .component
                .as_ref()
                .map(|c| c.name.clone())
                .unwrap_or_else(|| format!("{} package", description.title))
        });
    // The preset the parameters started from says what the part is and how
    // far its values are backed; the generator's notes are remarks for the
    // reviewer, kept in the provenance, never the part's description.
    let preset = request.preset.as_deref().and_then(|p| {
        description
            .presets()
            .iter()
            .find(|x| x.key == p || x.name == p)
    });
    let basis_note = preset.and_then(preset_basis_note);
    let notes: Vec<String> = basis_note
        .iter()
        .cloned()
        .chain(made.generation.notes.iter().cloned())
        .collect();
    let provenance = bounded(format!(
        "Made by the generator ‘{}’ ({} version {}, parameters sha256 {}, result sha256 {result}{}){}.{}",
        description.title,
        description.id,
        description.version,
        key.params,
        bundle
            .map(|b| format!(", bundle sha256 {b}"))
            .unwrap_or_default(),
        preset
            .map(|p| format!(" from the preset ‘{}’ ({})", p.name, basis_words(p)))
            .unwrap_or_default(),
        if notes.is_empty() {
            String::new()
        } else {
            format!(" Generator notes: {}", notes.join(" "))
        }
    ));
    let part_description = bounded(preset.map(|p| p.description.clone()).unwrap_or_default());
    // The marking's artwork, packed as SVG once per hash.
    for asset in artwork {
        if project.library.assets.iter().any(|a| a.hash == asset.hash) {
            continue;
        }
        if project.library.assets.len() >= MAX_ASSETS {
            return Err(Error::Limit(format!(
                "a project packs at most {MAX_ASSETS} assets"
            )));
        }
        project.library.assets.push(asset);
    }
    if embed {
        if project.library.assets.len() >= MAX_ASSETS {
            return Err(Error::Limit(format!(
                "a project packs at most {MAX_ASSETS} assets"
            )));
        }
        project.library.assets.push(PackedAsset {
            hash: request.module.hash,
            media_type: MODULE_MEDIA_TYPE.into(),
            data: bytes,
        });
    }
    let suffix = |kind: crate::ids::DefinitionKind| -> &'static str {
        use crate::ids::DefinitionKind as K;
        match (scope, kind) {
            (GeneratorScope::Component, K::Component) => "",
            (GeneratorScope::Package, K::Package) => "",
            (_, K::Symbol) => " symbol",
            (_, K::Package) => " package",
            (_, K::Footprint) => " footprint",
            (_, K::Binding) => " binding",
            _ => "",
        }
    };
    if let Some((meta, definition, fresh)) = new_model {
        let project = Arc::make_mut(&mut document.project);
        if project.library.assets.len() + fresh.len() > MAX_ASSETS {
            return Err(Error::Limit(format!(
                "a project packs at most {MAX_ASSETS} assets"
            )));
        }
        project.library.assets.extend(fresh);
        let drafts_mut = Arc::make_mut(&mut document.drafts);
        drafts::create(
            drafts_mut,
            &project.library,
            model_ref.id,
            None,
            meta,
            definition,
        )?;
        ids.insert(0, model_ref.id);
    }
    let library = &document.project.library;
    let drafts_mut = Arc::make_mut(&mut document.drafts);
    for ((r, definition), base) in definitions.into_iter().zip(bases) {
        let meta = DefinitionMeta {
            name: label(&base_name, suffix(r.kind)),
            description: part_description.clone(),
            provenance: provenance.clone(),
        };
        drafts::create(drafts_mut, library, r.id, base, meta, definition)?;
    }
    super::apply_library(
        document,
        super::LibraryCommand::Publish {
            drafts: ids,
            close: true,
        },
        None,
    )?;
    let root = expansion
        .first()
        .copied()
        .ok_or_else(|| Error::Invalid("generated.expansion: nothing was made".into()))?;
    let part = GeneratedPart {
        root,
        expansion: expansion.clone(),
        from: GeneratedFrom {
            generator: description.id.clone(),
            version: description.version,
            module: request.module.hash,
            bundle,
            abi: wire.abi,
            scope,
            params: wire
                .params
                .iter()
                .map(|(k, v)| (k.clone(), ParamValue::from_wire(v)))
                .collect(),
            result,
            ids: key_ids,
            preset: request.preset.clone(),
        },
    };
    Arc::make_mut(&mut document.project)
        .library
        .generated
        .push(part);
    Ok(Outcome::Generated {
        report: GenerateReport {
            root,
            revisions: expansion,
            key,
            result,
            created: true,
            replaced: replaced.map(|(_, _, root, _)| root),
            notes,
        },
    })
}

/// How far a preset's values are backed, in words: its basis and the
/// document it cites.
fn basis_words(preset: &v1::Preset) -> String {
    let cited = preset
        .citation
        .as_ref()
        .map(|c| c.document.clone())
        .unwrap_or_else(|| "no document named".into());
    match preset.basis {
        v1::Basis::Cited => format!("cited: {cited}"),
        v1::Basis::CitedUnverified => format!("cited, not checked against the source: {cited}"),
        v1::Basis::Estimated => "estimated".into(),
    }
}

/// The reviewer's note a preset that is not plainly cited adds before the
/// generator's own: that its values are estimated or unverified, with the
/// preset's note.
fn preset_basis_note(preset: &v1::Preset) -> Option<String> {
    let what = match preset.basis {
        v1::Basis::Cited => return None,
        v1::Basis::CitedUnverified => "cited but not checked against the source",
        v1::Basis::Estimated => "estimated",
    };
    let note = preset
        .note
        .as_deref()
        .map(|n| format!(" {n}"))
        .unwrap_or_else(|| " Confirm them against the device's drawing.".into());
    Some(format!(
        "The preset ‘{}’ is {what}: its values, lands included, are not the maker's recommended layout until confirmed.{note}",
        preset.name
    ))
}

/// The host's evaluation of the part file `generation` names, checked to
/// be that file at those values.
pub(super) fn supplied_model<'a>(
    request: &'a Generate,
    generation: &v1::Generation,
) -> Result<&'a GeneratedModel> {
    let (asset, part_values) = expand::part_file(generation)?;
    let supplied = request.model.as_ref().ok_or_else(|| {
        Error::Invalid(format!(
            "generated.model_unevaluated: the part's 3D model, the part file sha256 {asset}, was not evaluated by the host. Nothing was changed."
        ))
    })?;
    expand::part_values_match(part_values, &supplied.values)?;
    if supplied.definition.asset.hash != asset
        || supplied.definition.format != ModelFormat::Solidcraft
    {
        return Err(Error::Invalid(format!(
            "generated.model: the host evaluated the part file sha256 {}, but the generator names sha256 {asset}. Nothing was changed.",
            supplied.definition.asset.hash
        )));
    }
    Ok(supplied)
}

/// A new Model lineage's draft content and the assets it packs.
pub(super) type NewModel = (DefinitionMeta, crate::library::Definition, Vec<PackedAsset>);

/// The part file as a Model of the project: the newest one packing those
/// bytes, or else what a new lineage is made of.
pub(super) fn model_of(
    document: &Document,
    supplied: &GeneratedModel,
) -> Result<(Option<ModelRef>, Option<NewModel>)> {
    let asset = supplied.definition.asset.hash;
    let held = document
        .project
        .library
        .models
        .iter()
        .filter(|m| {
            m.definition.format == ModelFormat::Solidcraft && m.definition.asset.hash == asset
        })
        .max_by_key(|m| (m.id, m.revision))
        .map(|m| ModelRef::new(m.id, m.revision));
    if held.is_some() {
        return Ok((held, None));
    }
    let import = ImportModel {
        name: supplied.name.clone(),
        assets: supplied
            .data
            .iter()
            .map(|data| PackedAsset {
                hash: asset,
                media_type: ModelFormat::Solidcraft.media_type().into(),
                data: data.clone(),
            })
            .collect(),
        definition: Box::new(supplied.definition.clone()),
        assign: None,
    };
    if let Some(data) = &supplied.data
        && data.hash() != asset
    {
        return Err(Error::Invalid(format!(
            "generated.model: the part file's bytes hash to sha256 {}, not to sha256 {asset}. Nothing was changed.",
            data.hash()
        )));
    }
    let (meta, definition) = model3d::import_model(&import)?;
    let fresh = model3d::assets_to_pack(&document.project.library, &import)?;
    Ok((None, Some((meta, definition, fresh))))
}

/// The report for a part the project already holds: nothing changed.
/// A second part from the parameters of a part the generator already made
/// (`root`), under another `name`: its component, symbol and binding copied
/// as new lineages named after it and published, on the same package and
/// footprint, so each MPN is its own component. The copy is the person's
/// own (its provenance names the original); the generated record stays the
/// original's. `same` is what the request found.
fn second_part(
    document: &mut Document,
    root: DefinitionRef,
    name: &str,
    same: Outcome,
) -> Result<Outcome> {
    let Outcome::Generated { report } = same else {
        return Ok(same);
    };
    let component = crate::ids::ComponentRef::new(root.id, root.revision);
    let lineages = match super::apply_library(
        document,
        super::LibraryCommand::Duplicate {
            component,
            name: name.to_owned(),
        },
        None,
    )? {
        Outcome::Duplicated { lineages } => lineages,
        _ => return Err(Error::Invalid("generated.second_part: nothing was copied".into())),
    };
    let published = match super::apply_library(
        document,
        super::LibraryCommand::Publish {
            drafts: lineages,
            close: true,
        },
        None,
    )? {
        Outcome::Published { revisions } => revisions,
        _ => Vec::new(),
    };
    let new_root = published
        .iter()
        .find(|r| r.kind == crate::ids::DefinitionKind::Component)
        .copied()
        .ok_or_else(|| Error::Invalid("generated.second_part: no component was published".into()))?;
    let original = document
        .project
        .library
        .definition(root)
        .map(|(meta, _)| meta.name)
        .unwrap_or_default();
    let mut revisions = published;
    revisions.extend(
        report
            .revisions
            .iter()
            .filter(|r| {
                !matches!(
                    r.kind,
                    crate::ids::DefinitionKind::Component
                        | crate::ids::DefinitionKind::Symbol
                        | crate::ids::DefinitionKind::Binding
                )
            })
            .copied(),
    );
    Ok(Outcome::Generated {
        report: GenerateReport {
            root: new_root,
            revisions,
            created: true,
            notes: vec![format!(
                "‘{original}’ is this generator's part for these parameters, so ‘{name}’ is a new component on its package and footprint, with its own symbol and binding."
            )],
            ..report
        },
    })
}

fn found(existing: &GeneratedPart, key: PartKey) -> Outcome {
    let key = existing.from.key().unwrap_or(key);
    Outcome::Generated {
        report: GenerateReport {
            root: existing.root,
            revisions: existing.expansion.clone(),
            key,
            result: existing.from.result,
            created: false,
            replaced: None,
            notes: Vec::new(),
        },
    }
}

/// Refuse making `part` again with an older version of its generator.
fn refuse_older(
    document: &Document,
    description: &v1::Description,
    part: &GeneratedPart,
) -> Result<()> {
    if part.from.generator == description.id && part.from.version > description.version {
        return Err(Error::Conflict(format!(
            "generated.older_version: {} was made by version {} of the generator ‘{}’; this module is version {}, which is older. A generated part is never downgraded. Nothing was changed.",
            generated::describe(&document.project.library, part),
            part.from.version,
            description.title,
            description.version
        )));
    }
    Ok(())
}

/// `name` with `suffix`, within the label bound.
fn label(name: &str, suffix: &str) -> String {
    let mut end = name.len().min(MAX_LABEL.saturating_sub(suffix.len()));
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &name[..end])
}

/// `text` within the note bound.
pub(super) fn bounded(text: String) -> String {
    if text.len() <= MAX_NOTE {
        return text;
    }
    let mut end = MAX_NOTE;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}
