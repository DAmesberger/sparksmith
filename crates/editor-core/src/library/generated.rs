//! Generated parts (slice GP1, `docs/generator-plugins.md`): parts a
//! generator module made, the provenance that names the module and the
//! complete parameters, and the checks that keep a project's copy equal to
//! what the module makes.
//!
//! **Identity (slice B4).** A part's key is its generator's id with the
//! SHA-256 of the canonical request (ABI, scope and complete parameters as
//! recorded): nothing in it depends on the module's bytes, so the same
//! source built on any host makes the same part. The generator's version
//! tells revisions of one lineage apart (the regenerate rule, G15). The
//! result hash (SHA-256 of the canonical generation) is the integrity
//! check; the module hash names the embedded bytes that check runs; the
//! bundle hash is provenance, what a person approves. All are recorded in
//! [`GeneratedFrom`].
//!
//! **Embedding.** The module's bytes are a packed asset of media type
//! [`MODULE_MEDIA_TYPE`] (`Library::assets`, one container entry per hash),
//! so a project reopens offline with no generator folder, and every part
//! one module made shares its one copy.
//!
//! **Records.** [`GeneratedPart`] sits beside the revisions it made
//! (`Library::generated`), outside their bodies, as origins do: a revision
//! drafted from a generated one and published again is the person's own,
//! and carries no claim to be generated.
//!
//! The core never runs a module: it states what it needs as a
//! [`GeneratorRuntime`] (the engine's host setting) or as the plan
//! [`checks`] and the comparison [`compare`], which the host drives.
pub mod expand;

use super::bundle::PackedAsset;
use super::model3d::ModelFormat;
use super::{Definition, Library, PackageModel};
use crate::diagnostics::{Error, Result};
use crate::fields::{FieldSelection, ValueTemplate};
use crate::ids::{
    BindingRef, ComponentRef, ContentHash, DefinitionId, DefinitionKind, DefinitionRef,
    FootprintRef, PackageRef, PartRef, SymbolRef,
};
use crate::units::{Nm, PointNm};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparksmith_generator_abi::v1;
use sparksmith_generator_abi::{Failure, Generated};
use std::collections::{BTreeMap, BTreeSet};

pub use sparksmith_generator_abi::GeneratorRuntime;

/// The media type of an embedded generator module.
pub const MODULE_MEDIA_TYPE: &str = "application/wasm";
/// Largest embedded module: the host's limit (`sparksmith_generators::Limits`).
pub const MAX_MODULE_BYTES: usize = 1024 * 1024;
/// Generated parts one project records.
pub const MAX_GENERATED: usize = 4096;
/// Parameters one request gives.
pub const MAX_PARAMS: usize = 256;

/// What a generator made: a complete part or a package on its own.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorScope {
    /// Component, symbol, package, footprint and binding.
    Component,
    /// Package and footprint.
    Package,
}

impl GeneratorScope {
    pub fn wire(self) -> v1::Scope {
        match self {
            Self::Component => v1::Scope::Component,
            Self::Package => v1::Scope::Package,
        }
    }

    /// The kind of the part's root revision.
    pub fn root_kind(self) -> DefinitionKind {
        match self {
            Self::Component => DefinitionKind::Component,
            Self::Package => DefinitionKind::Package,
        }
    }

    /// The kinds of the revisions the expansion makes, in its order.
    pub fn kinds(self) -> &'static [DefinitionKind] {
        match self {
            Self::Component => &[
                DefinitionKind::Component,
                DefinitionKind::Symbol,
                DefinitionKind::Package,
                DefinitionKind::Footprint,
                DefinitionKind::Binding,
            ],
            Self::Package => &[DefinitionKind::Package, DefinitionKind::Footprint],
        }
    }
}

/// A parameter value as the ABI carries it: an integer (lengths in nm,
/// angles in millidegrees, counts), a boolean, text (choice keys, free
/// text, decimal quantities) or a list.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(untagged)]
pub enum ParamValue {
    Int(i64),
    Bool(bool),
    Text(String),
    List(Vec<ParamValue>),
}

impl ParamValue {
    pub fn wire(&self) -> v1::ParamValue {
        match self {
            Self::Int(v) => v1::ParamValue::Int(*v),
            Self::Bool(v) => v1::ParamValue::Bool(*v),
            Self::Text(v) => v1::ParamValue::Text(v.clone()),
            Self::List(v) => v1::ParamValue::List(v.iter().map(Self::wire).collect()),
        }
    }

    pub fn from_wire(value: &v1::ParamValue) -> Self {
        match value {
            v1::ParamValue::Int(v) => Self::Int(*v),
            v1::ParamValue::Bool(v) => Self::Bool(*v),
            v1::ParamValue::Text(v) => Self::Text(v.clone()),
            v1::ParamValue::List(v) => Self::List(v.iter().map(Self::from_wire).collect()),
        }
    }
}

/// The request for `given` parameters in `scope`, every default filled
/// from the description and checked: the ABI's one completion, the same the
/// host runs (`sparksmith_generators::params::complete`). Defaults are
/// filled before hashing, so a default that changes in a later module can
/// never change a part's key.
pub use sparksmith_generator_abi::params::complete;

/// The value of a project setting a parameter declares as its context, if
/// the core knows the setting.
fn context_value(
    rules: &crate::rules::ProjectRules,
    drafts: &super::Drafts,
    param: &v1::Param,
) -> Option<v1::ParamValue> {
    // Without a fab profile the board has no land margins.
    let land = |pick: fn(&crate::rules::LandRules) -> f64| {
        let mm = pick(&rules.land_estimators().ok()?.land);
        crate::units::Nm::from_mm_f64_rounded(mm)
            .ok()
            .map(|nm| v1::ParamValue::Int(nm.get()))
    };
    match param.context.as_deref()? {
        "project.land.chip_gap_min" => land(|l| l.chip_gap_min),
        "project.land.gull_wing_width_extra" => land(|l| l.gull_wing_width_extra),
        "project.land.exposed_pad_clearance" => land(|l| l.exposed_pad_clearance),
        "project.mask.web_min" => {
            let rules = rules.land_estimators().ok()?;
            crate::units::Nm::from_mm_f64_rounded(rules.mask.web_min)
                .ok()
                .map(|nm| v1::ParamValue::Int(nm.get()))
        }
        "project.symbols.resistor_style" => {
            let key = match drafts.symbols.resistor {
                super::symbol_style::ResistorStyle::Iec => "iec",
                super::symbol_style::ResistorStyle::Us => "us",
            };
            Some(v1::ParamValue::Text(key.into()))
        }
        _ => None,
    }
}

/// The complete request `params` make in a project with these `rules` and
/// `drafts` (its symbol conventions): a parameter the request leaves out
/// whose description names a project setting takes the project's value,
/// every other the module's default (G7).
pub fn completed(
    rules: &crate::rules::ProjectRules,
    drafts: &super::Drafts,
    description: &v1::Description,
    scope: GeneratorScope,
    params: &GeneratorParams,
) -> std::result::Result<v1::Request, Failure> {
    let mut given = wire_params(params);
    for param in description.params() {
        if given.contains_key(&param.key) {
            continue;
        }
        if let Some(value) = context_value(rules, drafts, param)
            && sparksmith_generator_abi::params::fits(&param.ty, &value)
        {
            given.insert(param.key.clone(), value);
        }
    }
    complete(description, scope.wire(), &given)
}

/// Whether a part recorded with `old` parameters was made from the same
/// parameters as `wire`: every recorded value is the same, and a parameter
/// the newer module adds was not given (it takes the module's default or
/// the project's setting).
fn same_params(old: &GeneratorParams, wire: &v1::Request, given: &GeneratorParams) -> bool {
    old.iter()
        .all(|(k, v)| wire.params.get(k) == Some(&v.wire()))
        && wire
            .params
            .keys()
            .all(|k| old.contains_key(k) || !given.contains_key(k))
}

/// The newest part (highest version, then revision) the generator
/// `description` names made from the same parameters in `scope`: `given`
/// as asked, `wire` as completed.
pub fn predecessor<'a>(
    library: &'a Library,
    description: &v1::Description,
    scope: GeneratorScope,
    given: &GeneratorParams,
    wire: &v1::Request,
) -> Option<&'a GeneratedPart> {
    library
        .generated
        .iter()
        .filter(|part| {
            part.from.scope == scope
                && part.from.generator == description.id
                && same_params(&part.from.params, wire, given)
        })
        .max_by_key(|p| (p.from.version, p.root.revision, p.root.id))
}

/// Where a part a generator would make from given parameters stands in a
/// project (the regenerate rule, G15): what `library.generate` with that
/// module would do, decided without running it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Standing {
    /// The project holds no part from these parameters: making it creates
    /// one.
    New,
    /// The same version of the generator made it: making it again changes
    /// nothing.
    Held { root: DefinitionRef, version: u32 },
    /// An older version made it: this one makes its next revisions, and
    /// placed instances keep theirs until the update is applied.
    Update {
        root: DefinitionRef,
        made_by: u32,
        version: u32,
    },
    /// A newer version made it: this module is refused, a generated part
    /// is never downgraded.
    Newer {
        root: DefinitionRef,
        made_by: u32,
        version: u32,
    },
    /// The parameters are not ones the module takes: each problem with the
    /// parameter it names, if any.
    Invalid { problems: Vec<ParamProblem> },
}

/// One parameter problem of [`Standing::Invalid`], in the module's words.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParamProblem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub message: String,
}

impl Standing {
    /// The part the project holds from these parameters, if any.
    pub fn root(&self) -> Option<DefinitionRef> {
        match self {
            Self::Held { root, .. } | Self::Update { root, .. } | Self::Newer { root, .. } => {
                Some(*root)
            }
            Self::New | Self::Invalid { .. } => None,
        }
    }
}

/// Where the part `description`'s module makes from `params` in `scope`
/// stands in a project of `library`, `rules` and `drafts` ([`Standing`]),
/// by the rule `library.generate` applies: the newest part the same
/// generator made from the same parameters in the same scope, and its
/// version against this one.
pub fn standing(
    library: &Library,
    rules: &crate::rules::ProjectRules,
    drafts: &super::Drafts,
    description: &v1::Description,
    scope: GeneratorScope,
    params: &GeneratorParams,
) -> Standing {
    let wire = match completed(rules, drafts, description, scope, params) {
        Ok(wire) => wire,
        Err(failure) => {
            return Standing::Invalid {
                problems: failure
                    .problems
                    .into_iter()
                    .map(|p| ParamProblem {
                        field: p.field,
                        message: p.message,
                    })
                    .collect(),
            };
        }
    };
    match predecessor(library, description, scope, params, &wire) {
        None => Standing::New,
        Some(part) => {
            let (root, made_by, version) = (part.root, part.from.version, description.version);
            match made_by.cmp(&version) {
                std::cmp::Ordering::Equal => Standing::Held { root, version },
                std::cmp::Ordering::Less => Standing::Update {
                    root,
                    made_by,
                    version,
                },
                std::cmp::Ordering::Greater => Standing::Newer {
                    root,
                    made_by,
                    version,
                },
            }
        }
    }
}

/// Parameters by key.
pub type GeneratorParams = BTreeMap<String, ParamValue>;

/// Parameters in wire form.
pub fn wire_params(params: &GeneratorParams) -> v1::Params {
    params
        .iter()
        .map(|(key, value)| (key.clone(), value.wire()))
        .collect()
}

/// Which generator made a part from which parameters, and what it made.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GeneratedFrom {
    /// The generator's id (its description's `id`).
    pub generator: String,
    /// The generator's version that made this revision of the part.
    pub version: u32,
    /// SHA-256 of the module's bytes, packed in `Library::assets`: what the
    /// check on open runs.
    pub module: ContentHash,
    /// SHA-256 of the bundle's manifest the module came from (manifest,
    /// module and part files): provenance, and what a person approves.
    /// Absent for a module given by its bytes alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<ContentHash>,
    /// The ABI the module speaks (1).
    pub abi: u32,
    pub scope: GeneratorScope,
    /// Complete: every parameter the module declares, defaults and project
    /// settings filled in when the part was made.
    pub params: GeneratorParams,
    /// SHA-256 of the canonical generation the module returned.
    pub result: ContentHash,
    /// The generation keys and the local ids they were given (slice B1).
    pub ids: expand::KeyIds,
    /// The name of the preset the parameters started from (`SOT-23-5`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// A part's key: the generator's id and the params hash. Equal keys ask
/// the same generator for the same part; its version tells revisions
/// apart.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(deny_unknown_fields)]
pub struct PartKey {
    pub generator: String,
    pub params: ContentHash,
}

impl GeneratedFrom {
    /// The request the module was called with.
    pub fn request(&self) -> v1::Request {
        v1::Request {
            abi: self.abi,
            scope: self.scope.wire(),
            params: wire_params(&self.params),
        }
    }

    pub fn key(&self) -> Result<PartKey> {
        key(&self.generator, &self.request())
    }
}

/// The key of `request` to generator `id`: the params hash is the SHA-256
/// of the request's canonical bytes (ABI, scope and complete parameters).
pub fn key(id: &str, request: &v1::Request) -> Result<PartKey> {
    let bytes = sparksmith_generator_abi::encode(request).map_err(|e| {
        Error::Invalid(format!(
            "generated.params: the parameters cannot be written canonically ({e})"
        ))
    })?;
    let params = sparksmith_generator_abi::sha256(&bytes);
    Ok(PartKey {
        generator: id.to_string(),
        params: ContentHash(params),
    })
}

/// One generated part: the revisions its expansion made, root first, and
/// where it came from.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GeneratedPart {
    /// The component (complete part) or package (package scope) revision.
    pub root: DefinitionRef,
    /// Every revision the expansion made, in its order
    /// ([`GeneratorScope::kinds`]); the first is `root`.
    pub expansion: Vec<DefinitionRef>,
    pub from: GeneratedFrom,
}

impl GeneratedPart {
    /// The lineage references the expansion fills.
    pub fn refs(&self) -> Result<expand::Refs> {
        let malformed = || {
            Error::Invalid(format!(
                "generated.record: the generated part {} {} lists revisions of the wrong kinds",
                self.root.id, self.root.revision
            ))
        };
        let kinds: Vec<DefinitionKind> = self.expansion.iter().map(|r| r.kind).collect();
        if kinds != self.from.scope.kinds() || self.expansion.first() != Some(&self.root) {
            return Err(malformed());
        }
        let at = |i: usize| self.expansion.get(i).copied().ok_or_else(malformed);
        Ok(match self.from.scope {
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
        })
    }
}

/// The record of a revision `root`, if a generator made it.
pub fn record_of(library: &Library, root: DefinitionRef) -> Option<&GeneratedPart> {
    library.generated.iter().find(|g| g.root == root)
}

/// The SDK's parameter of a generation's orderable part numbers
/// (`sparksmith_generator_sdk::orderable`): `MANUFACTURER=MPN` entries,
/// recorded with the part's complete parameters.
pub const ORDERABLE_PARAM: &str = "part_numbers";

/// The one manufacturer Part a placed instance of `component` is fitted
/// with: when `component` is a generated part whose generation named
/// exactly one orderable part number, the latest Part of that number
/// pinning exactly this revision (the one `library.generate` published, or
/// carried to it). `None` for any other component, for a generation of no
/// or several part numbers (a module's ordering codes: the person
/// chooses), or when that Part is gone.
pub fn sole_orderable(library: &Library, component: ComponentRef) -> Option<PartRef> {
    let record = record_of(library, component.erased())?;
    let [(manufacturer, mpn)] = declared_orderables(record).try_into().ok()?;
    // The Part of this exact revision, whatever later revisions the Part
    // has: a placement pinned to an older component revision takes the
    // Part revision that pins it.
    library
        .parts
        .iter()
        .filter(|p| {
            p.definition.component == component
                && p.definition.manufacturer == manufacturer
                && p.definition.mpn == mpn
        })
        .max_by_key(|p| p.revision)
        .map(|p| PartRef::new(p.id, p.revision))
}

/// The orderable parts (manufacturer, part number) a generated part's
/// recorded `part_numbers` declare, each `MANUFACTURER=MPN` entry trimmed.
pub fn declared_orderables(part: &GeneratedPart) -> Vec<(String, String)> {
    let Some(ParamValue::List(items)) = part.from.params.get(ORDERABLE_PARAM) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            ParamValue::Text(entry) => entry
                .rsplit_once('=')
                .map(|(m, n)| (m.trim().to_owned(), n.trim().to_owned())),
            _ => None,
        })
        .collect()
}

/// The orderable parts any generation of `component`'s lineage declared:
/// the manufacturer Parts the generator owns there, as opposed to ones a
/// person added.
pub fn owned_orderables(library: &Library, component: DefinitionId) -> Vec<(String, String)> {
    library
        .generated
        .iter()
        .filter(|g| g.root.id == component)
        .flat_map(declared_orderables)
        .collect()
}

/// The recipe a library record of `root` carries when a generator made
/// it: generator, version, preset, complete parameters and the generic
/// family those read as (from the current standards; none outside an
/// engine call).
pub fn recipe(library: &Library, root: DefinitionRef) -> Option<super::bundle::GeneratedRecipe> {
    let part = record_of(library, root)?;
    let family = crate::library::standards::get().ok().and_then(|s| {
        s.generic
            .family_for(&part.from.generator, &part.from.params)
            .map(|f| f.family.name().to_owned())
    });
    Some(super::bundle::GeneratedRecipe {
        generator: part.from.generator.clone(),
        version: part.from.version,
        preset: part.from.preset.clone(),
        params: part.from.params.clone(),
        family,
    })
}

/// The SHA-256 of a generated part's recipe in component scope: its
/// generator, version and complete parameters, the identity under which
/// `library.import` takes a bundle's copy of the part as the project's own
/// one. The catalog tells by it which folder Parts fit a generic part the
/// project made.
pub fn recipe_hash(generator: &str, version: u32, params: &GeneratorParams) -> Result<ContentHash> {
    let bytes = serde_json::to_vec(&(generator, version, params))?;
    Ok(ContentHash(Sha256::digest(&bytes).into()))
}

/// The newest record with `key` (highest version, then revision), if the
/// project holds that part.
pub fn find<'a>(library: &'a Library, key: &PartKey) -> Option<&'a GeneratedPart> {
    library
        .generated
        .iter()
        .filter(|g| g.from.key().is_ok_and(|k| k == *key))
        .max_by_key(|g| (g.from.version, g.root.revision, g.root.id))
}

/// The embedded module `hash`, if the project packs it as a module.
pub fn module(library: &Library, hash: ContentHash) -> Option<&PackedAsset> {
    library
        .assets
        .iter()
        .find(|a| a.hash == hash && a.media_type == MODULE_MEDIA_TYPE)
}

/// The module hashes the generated parts name.
pub fn named_modules(library: &Library) -> BTreeSet<ContentHash> {
    library.generated.iter().map(|g| g.from.module).collect()
}

/// A short form of a hash for messages: the first 12 hex digits.
pub fn short(hash: ContentHash) -> String {
    hash.to_string().chars().take(12).collect()
}

/// A failure of a module in the document's error type, its problems' own
/// codes and sentences kept.
pub fn failure(failure: &Failure) -> Error {
    let code = failure.code().unwrap_or("generator.failed");
    Error::Invalid(format!("{code}: {failure}"))
}

/// Structural rules of the records (`Library::validate`): bounded, each
/// root and expansion revision held with the kinds of its scope, every
/// revision made by one part only, each key once, ABI 1, and every module
/// packed as a module, so the project reopens offline.
pub fn validate(library: &Library) -> Result<()> {
    if library.generated.len() > MAX_GENERATED {
        return Err(Error::Limit(format!(
            "a project records at most {MAX_GENERATED} generated parts"
        )));
    }
    let mut made: BTreeSet<DefinitionRef> = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for part in library.generated.iter() {
        let name = describe(library, part);
        if part.from.abi != v1::ABI {
            return Err(Error::Unsupported(format!(
                "generated.abi: {name} was made by a generator of ABI {}; this version runs ABI {}",
                part.from.abi,
                v1::ABI
            )));
        }
        if part.from.params.len() > MAX_PARAMS {
            return Err(Error::Limit(format!(
                "generated.params: {name} records more than {MAX_PARAMS} parameters"
            )));
        }
        part.refs()?;
        for r in &part.expansion {
            if library.definition(*r).is_none() {
                return Err(Error::Invalid(format!(
                    "generated.record: {name} names {} {}, which the project does not hold",
                    r.id, r.revision
                )));
            }
            if !made.insert(*r) {
                return Err(Error::Invalid(format!(
                    "generated.record: {} {} is recorded as made by two generated parts",
                    r.id, r.revision
                )));
            }
        }
        let request = part.from.request();
        let bytes = sparksmith_generator_abi::encode(&request).map_err(|e| {
            Error::Invalid(format!(
                "generated.params: the parameters of {name} cannot be written canonically ({e})"
            ))
        })?;
        if bytes.len() > v1::limits::REQUEST_BYTES {
            return Err(Error::Limit(format!(
                "generated.params: the parameters of {name} exceed {} bytes",
                v1::limits::REQUEST_BYTES
            )));
        }
        if part.from.version == 0
            || part.from.generator.is_empty()
            || part.from.generator.len() > 1024
        {
            return Err(Error::Invalid(format!(
                "generated.record: {name} names no generator or version"
            )));
        }
        part.from.ids.validate()?;
        if !keys.insert((part.from.key()?, part.from.version)) {
            return Err(Error::Invalid(format!(
                "generated.record: {name} is recorded twice by one version of a generator with the same parameters"
            )));
        }
        let Some(asset) = library.assets.iter().find(|a| a.hash == part.from.module) else {
            return Err(Error::Invalid(format!(
                "generated.module_missing: the generator module sha256 {} that made {name} is not embedded in this project",
                part.from.module
            )));
        };
        if asset.media_type != MODULE_MEDIA_TYPE {
            return Err(Error::Invalid(format!(
                "generated.module_missing: the asset sha256 {} that made {name} is packed as {}, not as a generator module",
                part.from.module, asset.media_type
            )));
        }
    }
    Ok(())
}

/// The field selections and value template the host gives a component
/// made from `params` by `generator`: its generic family's (the shipped
/// catalog's `fields` and `value_template`), as the project's copies of
/// the standard fields, each shown on the symbol. `None` when the part
/// belongs to no family that selects fields. They are host data outside
/// the generation and its result hash, so [`compare`] keeps the stored
/// ones; `NotFound` when the standard fields are not imported.
pub fn family_fields(
    library: &Library,
    generator: &str,
    params: &GeneratorParams,
) -> Result<Option<(Vec<FieldSelection>, Option<ValueTemplate>)>> {
    let standards = super::standards::get()?;
    let Some(family) = standards
        .generic
        .family_of(generator, params)
        .filter(|f| !f.fields.is_empty())
    else {
        return Ok(None);
    };
    let fields = family
        .fields
        .iter()
        .map(|name| {
            standards
                .fields
                .local(library, name)
                .map(|field| FieldSelection {
                    field,
                    default: None,
                    show_on_symbol: true,
                })
                .ok_or_else(|| {
                    Error::NotFound(format!("the standard field {name} is not imported"))
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let template = if family.value_template.is_empty() {
        None
    } else {
        Some(ValueTemplate::parse(&family.value_template, |name| {
            family
                .fields
                .iter()
                .position(|f| f == name)
                .and_then(|i| fields.get(i))
                .map(|s| s.field.id)
        })?)
    };
    Ok(Some((fields, template)))
}

/// Give the component of `definitions` the host's `fields` and value
/// `template`.
pub fn select_fields(
    definitions: &mut [(DefinitionRef, Definition)],
    fields: &[FieldSelection],
    template: Option<&ValueTemplate>,
) {
    for (_, definition) in definitions {
        if let Definition::Component(component) = definition {
            component.fields = fields.to_vec();
            component.value_template = template.cloned();
        }
    }
}

/// The lead contacts of `generation`, by the package terminal each names
/// (its key), in the package frame on the seating plane.
pub fn generation_leads(generation: &v1::Generation) -> Result<Vec<(String, PointNm)>> {
    generation
        .package
        .geometry
        .leads
        .iter()
        .map(|lead| {
            let [x, y, _] = lead.contact;
            Ok((lead.key.clone(), PointNm::new(Nm::new(x)?, Nm::new(y)?)))
        })
        .collect()
}

/// Give the package of `definitions` its lead contacts, each on the
/// package terminal its designation names, so the board check finds a
/// lead off its land. Host data beside the expansion, as the fields are:
/// [`compare`] keeps the stored ones, so a part made before packages
/// carried them still opens.
pub fn set_leads(definitions: &mut [(DefinitionRef, Definition)], leads: &[(String, PointNm)]) {
    for (_, definition) in definitions {
        if let Definition::Package(package) = definition {
            package.leads = leads
                .iter()
                .filter_map(|(key, contact)| {
                    let terminal = package.terminals.iter().find(|t| t.designation == *key)?;
                    Some(super::PackageLead {
                        terminal: terminal.id,
                        contact: *contact,
                    })
                })
                .collect();
        }
    }
}

/// Give the component of `definitions` the host's default `marking`
/// (the generation's, its fields made the project's: [`expand::marking`]).
pub fn set_marking(
    definitions: &mut [(DefinitionRef, Definition)],
    marking: Option<&super::marking::MarkingSpec>,
) {
    for (_, definition) in definitions {
        if let Definition::Component(component) = definition {
            component.marking = marking.cloned().map(Box::new);
        }
    }
}

/// The part's name in messages: its root's name and revision.
pub fn describe(library: &Library, part: &GeneratedPart) -> String {
    let name = library
        .definition(part.root)
        .map(|(meta, _)| meta.name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{}", part.root.id));
    format!("‘{name}’ ({})", part.root.revision)
}

/// SHA-256 of the canonical JSON of `definitions` in order: what
/// [`compare`] names when a stored part and its expansion differ.
pub fn expansion_hash(definitions: &[(DefinitionRef, Definition)]) -> Result<ContentHash> {
    let mut hasher = Sha256::new();
    for (r, definition) in definitions {
        let bytes = serde_json::to_vec(&(r, definition))
            .map_err(|e| Error::Invalid(format!("generated.expansion: {e}")))?;
        hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(ContentHash(hasher.finalize().into()))
}

/// The definitions `generation` expands to at the revisions `expansion`,
/// its package holding `model`, with local ids from `ids`; and the map
/// with every new key added.
pub fn expand_at(
    generation: &v1::Generation,
    refs: expand::Refs,
    expansion: &[DefinitionRef],
    model: PackageModel,
    ids: &expand::KeyIds,
) -> Result<(Vec<(DefinitionRef, Definition)>, expand::KeyIds)> {
    let (made, ids) = expand::definitions(generation, refs, model, ids)?;
    if made.len() != expansion.len() {
        return Err(Error::Invalid(
            "generated.expansion: the generation makes a different set of definitions".into(),
        ));
    }
    let definitions = expansion
        .iter()
        .copied()
        .zip(made.into_iter().map(|(_, d)| d))
        .collect();
    Ok((definitions, ids))
}

/// One check the host runs on load: the part, its module's bytes and the
/// request to run.
pub struct Check<'a> {
    pub part: &'a GeneratedPart,
    pub module: &'a PackedAsset,
    pub request: v1::Request,
}

/// What [`verify`] runs, in record order. Fails when a record names a
/// module the project does not embed ([`validate`] has refused that file
/// already).
pub fn checks(library: &Library) -> Result<Vec<Check<'_>>> {
    library
        .generated
        .iter()
        .map(|part| {
            let module = library
                .assets
                .iter()
                .find(|a| a.hash == part.from.module)
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "generated.module_missing: the generator module sha256 {} that made {} is not embedded in this project",
                        part.from.module,
                        describe(library, part)
                    ))
                })?;
            Ok(Check {
                part,
                module,
                request: part.from.request(),
            })
        })
        .collect()
}

/// The refusal for an embedded module whose bytes no longer hash to its
/// recorded hash: both hashes in plain words.
pub fn module_changed(name: &str, recorded: ContentHash, actual: ContentHash) -> Error {
    Error::Invalid(format!(
        "generated.module_changed: the generator module embedded in this project for {name} has been changed: the project records sha256 {recorded}, but the embedded bytes hash to sha256 {actual}. The project was not opened."
    ))
}

/// Compare what the module made now (`generated`, from the check's
/// module and request) with the record and the stored revisions: the
/// module's bytes must still hash to the recorded hash, the result hash
/// must equal the recorded one, and the stored revisions must equal the
/// expansion exactly. Each refusal names both hashes.
pub fn compare(library: &Library, check: &Check<'_>, generated: &Generated) -> Result<()> {
    let part = check.part;
    let name = describe(library, part);
    let actual = check.module.data.hash();
    if actual != part.from.module {
        return Err(module_changed(&name, part.from.module, actual));
    }
    let result = ContentHash(generated.result_hash);
    if result != part.from.result {
        return Err(Error::Invalid(format!(
            "generated.result_changed: {name} was recorded as generator result sha256 {}, but its embedded module now makes sha256 {result} from the same parameters. The project was not opened.",
            part.from.result
        )));
    }
    // The model the stored package holds, checked against the generation:
    // the part file it names (a Model revision of this project packing
    // those bytes) at its values. What the host derived from evaluating
    // the file is the stored package's own, rechecked by every board build.
    let refs = part.refs()?;
    let model = library
        .package(refs.package)
        .map(|p| p.definition.model.clone())
        .ok_or_else(|| {
            Error::Invalid(format!(
                "generated.record: the package of {name} is not in the library"
            ))
        })?;
    check_model(library, &generated.generation, &model).map_err(|e| {
        Error::Invalid(format!(
            "generated.expansion_changed: {name} stored in this project differs from what its generator makes from the recorded parameters: {e}. The project was not opened."
        ))
    })?;
    let (expected, ids) = expand_at(
        &generated.generation,
        refs,
        &part.expansion,
        model,
        &part.from.ids,
    )?;
    if ids != part.from.ids {
        return Err(Error::Invalid(format!(
            "generated.expansion_changed: {name} stored in this project differs from what its generator makes from the recorded parameters: the generator makes objects the recorded key map does not name. The project was not opened."
        )));
    }
    let stored: Vec<(DefinitionRef, Definition)> = part
        .expansion
        .iter()
        .filter_map(|r| library.definition(*r).map(|(_, d)| (*r, d)))
        .collect();
    // The fields the host selected are outside the generation
    // ([`family_fields`]): the stored ones stand; so do the package's lead
    // contacts ([`set_leads`]).
    let mut expected = expected;
    if let Some(package) = stored.iter().find_map(|(_, d)| match d {
        Definition::Package(p) => Some(p),
        _ => None,
    }) {
        let leads: Vec<(String, PointNm)> = package
            .leads
            .iter()
            .filter_map(|lead| {
                let terminal = package.terminals.iter().find(|t| t.id == lead.terminal)?;
                Some((terminal.designation.clone(), lead.contact))
            })
            .collect();
        set_leads(&mut expected, &leads);
    }
    if let Some(component) = stored.iter().find_map(|(_, d)| match d {
        Definition::Component(c) => Some(c),
        _ => None,
    }) {
        select_fields(
            &mut expected,
            &component.fields,
            component.value_template.as_ref(),
        );
        set_marking(&mut expected, component.marking.as_deref());
    }
    // The pin-1 polarity is the host's reading of the generation (its
    // pin-1 terminal and `pin1…` graphics), not the module's output: the
    // stored one stands, so a part made before the host read it opens as
    // it was made.
    for ((_, wanted), (_, kept)) in expected.iter_mut().zip(stored.iter()) {
        if let (Definition::Footprint(wanted), Definition::Footprint(kept)) = (wanted, kept) {
            wanted.polarity.clone_from(&kept.polarity);
        }
    }
    if stored != expected {
        return Err(Error::Invalid(format!(
            "generated.expansion_changed: {name} stored in this project differs from what its generator makes from the recorded parameters: the stored part hashes to sha256 {}, the generator's expansion to sha256 {}. The project was not opened.",
            expansion_hash(&stored)?,
            expansion_hash(&expected)?
        )));
    }
    Ok(())
}

/// Whether `model` is the part file `generation` names, as a published
/// Model revision of `library` of format `solidcraft` packing those bytes,
/// at the generation's values.
pub fn check_model(
    library: &Library,
    generation: &v1::Generation,
    model: &PackageModel,
) -> Result<()> {
    if expand::no_body(generation) {
        return match model {
            PackageModel::NoBody => Ok(()),
            _ => Err(Error::Invalid(
                "generated.model: its generator makes the package without a body, but it has a 3D model".into(),
            )),
        };
    }
    let (asset, values) = expand::part_file(generation)?;
    let PackageModel::PartFile(part) = model else {
        return Err(Error::Invalid(
            "generated.model: its package's 3D model is not its generator's part file".into(),
        ));
    };
    let held = library.model(part.model).ok_or_else(|| {
        Error::Invalid(format!(
            "generated.model: its part file {} is not in the library",
            part.model
        ))
    })?;
    if held.definition.format != ModelFormat::Solidcraft || held.definition.asset.hash != asset {
        return Err(Error::Invalid(format!(
            "generated.model: its package uses the model {}, not the generator's part file sha256 {asset}",
            part.model
        )));
    }
    expand::part_values_match(values, &part.values)
}

/// Every check run and compared, through `run` (the host's runtime, which
/// may cache a compiled module per hash).
pub fn verify(
    library: &Library,
    run: &mut dyn FnMut(&PackedAsset, &v1::Request) -> std::result::Result<Generated, Failure>,
) -> Result<()> {
    for check in checks(library)? {
        let actual = check.module.data.hash();
        if actual != check.part.from.module {
            return Err(module_changed(
                &describe(library, check.part),
                check.part.from.module,
                actual,
            ));
        }
        let generated = run(check.module, &check.request).map_err(|f| {
            Error::Invalid(format!(
                "generated.unverified: {} could not be checked against its generator: {f} The project was not opened.",
                describe(library, check.part)
            ))
        })?;
        compare(library, &check, &generated)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
