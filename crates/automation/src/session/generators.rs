//! Generator modules in the session (slice GP1, `docs/generator-plugins.md`).
//!
//! `library.generators` lists the shipped modules (`library/generators/`),
//! the modules this project embeds and its generated parts, each with the
//! state of its module ([`state`]). Nothing changes until the person makes
//! the part again: a newer version of its generator then makes the next
//! revisions of the same lineages, and the reply carries the update
//! preview that moves the placed instances. `library.generate_part` reads
//! a module (shipped by name, or embedded by hash) and applies the one
//! typed command `library.generate`, so UI, CLI and MCP make a part the
//! same way.
use super::Session;
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sparksmith_core::Operation;
use sparksmith_core::commands::{Generate, LibraryCommand, ModuleSource};
use sparksmith_core::ids::{ComponentRef, ContentHash, DefinitionKind, DefinitionRef};
use sparksmith_core::library::bundle::{AssetBytes, asset_hash};
use sparksmith_core::library::generated::{
    self, GeneratorParams, GeneratorRuntime, GeneratorScope, ParamValue,
};
pub use sparksmith_generators::abi::v1;
use sparksmith_io::generator_bundle::GeneratorBundle;
use sparksmith_io::shipped::ShippedLibrary;
use std::collections::BTreeMap;
use std::sync::Arc;
use v1::Description;

/// `library.generate_part`. Required: `scope`, and `generator` or
/// `module` (exactly one); the rest is optional.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratePartParams {
    /// Required unless `module` is given: a shipped generator by name
    /// (`library.generators`), e.g. `chip`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// Required unless `generator` is given: a module this project embeds,
    /// by its SHA-256.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<ContentHash>,
    /// Required. `component` (component, symbol, package, footprint and
    /// binding) or `package` (package and footprint).
    pub scope: GeneratorScope,
    /// A preset of the module (`library.generators` lists them), whose
    /// values `params` override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Parameters by key; lengths in integer nanometres, angles in integer
    /// millidegrees. The rest come from project settings and the module's
    /// defaults.
    #[serde(default)]
    pub params: GeneratorParams,
    /// The part's name; the generator's suggestion when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The root revision of a generated part to make again as the next
    /// revision of its lineages (taking a changed module); then
    /// `project.preview_update` moves instances with the impact listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<DefinitionRef>,
    pub expected_revision: u64,
}

/// The longest name, family or text `library.generators` filters by, in
/// bytes.
const MAX_FILTER: usize = 128;

/// `library.generators`: which shipped generators to list, and how much of
/// each. Without filters every one is listed; without `generator` or
/// `detail`, each by name, title, families, scopes, version and its
/// presets' keys (with their names when `family` or `text` filters).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratorsParams {
    /// One shipped generator by name or id, e.g. `gull-wing`: its
    /// parameters and rules, and each preset by key, name and basis
    /// (`cited`, `estimated`, …). `preset` adds one preset's values,
    /// citation and note; `detail` adds every preset's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// One preset by key, e.g. `no-lead-qfn-16-3x3-0.5`: only the
    /// generator that has it, with that preset in full (values, citation,
    /// basis and note).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Only generators that make this family, e.g. `no_lead`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// Only generators whose name, title, families or a preset's key, name
    /// or alias contain this text, ignoring case, e.g. `SOT-23`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Every listed generator in full: parameters, rules and presets with
    /// their values, citations and notes.
    #[serde(default)]
    pub detail: bool,
}

impl GeneratorsParams {
    fn check(&self) -> Result<()> {
        for (what, given) in [
            ("generator", &self.generator),
            ("preset", &self.preset),
            ("family", &self.family),
            ("text", &self.text),
        ] {
            if given.as_ref().is_some_and(|g| g.len() > MAX_FILTER) {
                return Err(invalid(format!("{what} is longer than {MAX_FILTER} bytes")));
            }
        }
        Ok(())
    }

    /// Whether `shipped` passes every filter given.
    fn keeps(&self, shipped: &ShippedGenerator) -> bool {
        let summary = shipped.description.as_ref();
        let named = self
            .generator
            .as_ref()
            .is_none_or(|n| shipped.name == *n || summary.is_some_and(|d| d.generator == *n));
        let family = self.family.as_ref().is_none_or(|f| {
            summary.is_some_and(|d| d.families.iter().any(|g| g.eq_ignore_ascii_case(f)))
        });
        let text = self.text.as_ref().is_none_or(|t| {
            let wanted = t.to_lowercase();
            let has = |s: &str| s.to_lowercase().contains(&wanted);
            has(&shipped.name)
                || summary.is_some_and(|d| {
                    has(&d.name)
                        || d.families.iter().any(|f| has(f))
                        || d.presets.iter().any(|p| {
                            has(&p.key) || has(&p.name) || p.aliases.iter().any(|a| has(a))
                        })
                })
        });
        let preset = self
            .preset
            .as_ref()
            .is_none_or(|k| summary.is_some_and(|d| d.presets.iter().any(|p| p.key == *k)));
        named && preset && family && text
    }
}

/// One shipped generator's description as `{generator}` or `{preset}`
/// gives it without `detail`: its parameters and rules in full, each
/// preset by key, name and basis, and only the preset `full` names with
/// its values, citation and note. The presets' values and notes would
/// outgrow a model's reply budget.
fn compact_presets(description: &mut Value, full: Option<&str>) {
    let Some(presets) = description.get_mut("presets").and_then(Value::as_array_mut) else {
        return;
    };
    for preset in presets.iter_mut() {
        if full.is_some_and(|k| preset["key"] == k) {
            continue;
        }
        let mut short = serde_json::json!({"key": preset["key"], "name": preset["name"]});
        if let Some(basis) = preset.get("basis") {
            short["basis"] = basis.clone();
        }
        *preset = short;
    }
}

/// A shipped generator as the default listing gives it: what a caller
/// needs to pick one, and its presets by key, with their names when the
/// listing is filtered (`named`): unfiltered, the names of every shipped
/// preset would outgrow a model's reply budget.
fn listed(shipped: &ShippedGenerator, named: bool) -> Value {
    let mut out = serde_json::json!({"name": shipped.name});
    if let Some(d) = &shipped.description {
        out["title"] = Value::from(d.name.clone());
        out["families"] = serde_json::json!(d.families);
        out["scopes"] = serde_json::json!(d.scopes);
        out["version"] = Value::from(d.version);
        out["presets"] = d
            .presets
            .iter()
            .map(|p| {
                if named {
                    serde_json::json!({"key": p.key, "name": p.name})
                } else {
                    serde_json::json!({"key": p.key})
                }
            })
            .collect();
    }
    if let Some(refused) = &shipped.refused {
        out["refused"] = Value::from(refused.clone());
    }
    out
}

fn invalid(message: String) -> crate::Error {
    crate::Error::Document(sparksmith_core::Error::Invalid(message))
}

/// A generator module's bytes with where they came from: a shipped
/// bundle (with its part files) or the project.
struct ModuleBytes {
    hash: ContentHash,
    bytes: Vec<u8>,
    bundle: Option<GeneratorBundle>,
}

impl Session {
    /// The shipped library: the one this session was given, else the one
    /// found from the environment and the executable.
    pub(super) fn shipped(&self) -> Result<ShippedLibrary> {
        match &self.shipped {
            Some(library) => Ok(library.clone()),
            None => Ok(ShippedLibrary::locate()?),
        }
    }

    /// Use `library` as the shipped library (tests, hosts that bundle it
    /// elsewhere).
    pub fn set_shipped(&mut self, library: ShippedLibrary) {
        self.shipped = Some(library);
        self.descriptions = std::sync::OnceLock::new();
    }

    /// Give the engine a generator runtime when it has none:
    /// [`session_runtime`], a worker thread per call with the host's
    /// limits, a wall time and no state, waiting for a busy worker.
    pub fn ensure_generators(&mut self) {
        if !self.engine.has_generators() {
            self.engine.set_generators(session_runtime());
        }
    }

    fn runtime(&self) -> Arc<dyn GeneratorRuntime + Send + Sync> {
        self.engine
            .generator_runtime()
            .cloned()
            .unwrap_or_else(session_runtime)
    }

    fn module_bytes(
        &self,
        generator: Option<&str>,
        module: Option<ContentHash>,
    ) -> Result<ModuleBytes> {
        match (generator, module) {
            (Some(name), None) => {
                let bundle = self.shipped()?.generator(name)?;
                Ok(ModuleBytes {
                    hash: asset_hash(&bundle.module),
                    bytes: bundle.module.clone(),
                    bundle: Some(bundle),
                })
            }
            (None, Some(hash)) => {
                let asset = generated::module(&self.engine.project().library, hash).ok_or_else(
                    || {
                        invalid(format!(
                            "generated.module_missing: this project embeds no generator module sha256 {hash}"
                        ))
                    },
                )?;
                Ok(ModuleBytes {
                    hash,
                    bytes: asset.data.as_slice().to_vec(),
                    bundle: None,
                })
            }
            _ => Err(invalid(
                "library.generate_part: give exactly one of generator and module".into(),
            )),
        }
    }

    pub(super) fn describe(&self, bytes: &[u8]) -> Result<Description> {
        self.runtime()
            .describe(bytes)
            .map_err(|f| crate::Error::Document(generated::failure(&f)))
    }

    /// The `library.generate` that `params` ask for, its 3D model
    /// evaluated (GP2), not yet applied: what `library.generate_part`
    /// applies, and what a host that applies commands on its own engine (the
    /// desktop, off its UI thread) sends.
    pub fn generate_request(&self, params: GeneratePartParams) -> Result<Generate> {
        let (mut generate, bundle, made) = self.run_request(params)?;
        let made = made.map_err(|f| crate::Error::Document(generated::failure(&f)))?;
        generate.model = Some(super::generated_models::generated_model(
            &self.engine,
            &made.generation,
            bundle.as_ref(),
        )?);
        Ok(generate)
    }

    /// The module's own refusal of `params`, each problem with the field
    /// it names (empty when the module makes the part): what a form marks
    /// on its fields while the values are edited. Runs the module but not
    /// its part file; call it off a UI thread.
    pub fn refusals(&self, params: GeneratePartParams) -> Result<Vec<generated::ParamProblem>> {
        let (_, _, made) = self.run_request(params)?;
        Ok(match made {
            Ok(_) => Vec::new(),
            Err(failure) => failure
                .problems
                .into_iter()
                .map(|p| generated::ParamProblem {
                    field: p.field,
                    message: p.message,
                })
                .collect(),
        })
    }

    /// `params` as a `library.generate` without its model, the shipped
    /// bundle its module came from, and the module's answer to it.
    #[allow(clippy::type_complexity)]
    pub(super) fn run_request(
        &self,
        params: GeneratePartParams,
    ) -> Result<(
        Generate,
        Option<GeneratorBundle>,
        std::result::Result<
            sparksmith_generators::abi::Generated,
            sparksmith_generators::abi::Failure,
        >,
    )> {
        let module = self.module_bytes(params.generator.as_deref(), params.module)?;
        let mut values = GeneratorParams::new();
        let mut preset_name = None;
        if let Some(preset) = &params.preset {
            let description = self.describe(&module.bytes)?;
            let found = description
                .presets()
                .iter()
                .find(|p| &p.key == preset)
                .ok_or_else(|| {
                    let keys: Vec<&str> = description
                        .presets()
                        .iter()
                        .map(|p| p.key.as_str())
                        .collect();
                    invalid(format!(
                        "generated.preset: the generator ‘{}’ has no preset {preset}; presets: {}",
                        description.title,
                        keys.join(", ")
                    ))
                })?;
            preset_name = Some(found.name.clone());
            values.extend(
                found
                    .params
                    .iter()
                    .map(|(k, v)| (k.clone(), ParamValue::from_wire(v))),
            );
        }
        values.extend(params.params);
        let embedded = generated::module(&self.engine.project().library, module.hash).is_some();
        let generate = Generate {
            module: ModuleSource {
                hash: module.hash,
                data: (!embedded).then(|| AssetBytes::new(module.bytes)),
            },
            scope: params.scope,
            params: values,
            name: params.name,
            replaces: params.replaces,
            model: None,
            bundle: module.bundle.as_ref().map(|b| b.hash),
            preset: preset_name,
        };
        // The model comes first (GP2): the module's part file, evaluated
        // at the values of the request the engine will run.
        let runtime = self.runtime();
        let (bytes, wire) = sparksmith_core::commands::generator_request(
            self.engine.document(),
            &generate,
            runtime.as_ref(),
        )?;
        let made = runtime.generate(&bytes, &wire);
        Ok((generate, module.bundle, made))
    }

    pub(super) fn generate_part(&mut self, params: GeneratePartParams) -> Result<Value> {
        let expected_revision = params.expected_revision;
        let generate = self.generate_request(params)?;
        let operation = Operation::Library(LibraryCommand::Generate(Box::new(generate)));
        self.ensure_generators();
        let mut reply = self.apply(expected_revision, operation)?;
        // Made again: preview moving the instances that pin an older
        // component revision, or whose Part was carried to this one
        // (never applied here).
        let report = &reply["outcome"]["report"];
        let new: Option<DefinitionRef> = serde_json::from_value(report["root"].clone()).ok();
        if let Some(new) = new
            && new.kind == DefinitionKind::Component
        {
            let project = self.engine.project();
            let instances: Vec<_> = project
                .instances
                .iter()
                .filter(|i| {
                    i.component.id == new.id
                        && (i.component.revision != new.revision
                            || sparksmith_core::instances::republished_part(&project.library, i)
                                .is_some())
                })
                .map(|i| i.id)
                .collect();
            if !instances.is_empty() {
                let plan = self.engine.preview_update_hosted(
                    sparksmith_core::updates::Change::ComponentRevision {
                        instances,
                        target: ComponentRef::new(new.id, new.revision),
                        resolutions: Default::default(),
                    },
                    super::hosted(),
                );
                reply["update"] = serde_json::to_value(plan)?;
            }
        }
        Ok(reply)
    }

    /// `library.generators`: the report, its shipped list filtered, and
    /// in full only for one named generator or with `detail`.
    pub(super) fn generators(&self, params: &GeneratorsParams) -> Result<Value> {
        params.check()?;
        let mut report = self.generators_report()?;
        let names: Vec<String> = report.shipped.iter().map(|g| g.name.clone()).collect();
        report.shipped.retain(|g| params.keeps(g));
        if let Some(name) = &params.generator
            && report.shipped.is_empty()
        {
            return Err(crate::Error::Document(sparksmith_core::Error::NotFound(
                format!(
                    "no shipped generator {name} matches: the generators are {}",
                    names.join(", ")
                ),
            )));
        }
        if let Some(key) = &params.preset
            && report.shipped.is_empty()
        {
            return Err(crate::Error::Document(sparksmith_core::Error::NotFound(
                format!(
                    "no shipped generator has preset {key}: library.generators {{text: <part of the key>}} lists the keys that match"
                ),
            )));
        }
        let mut value = serde_json::to_value(&report)?;
        let one = params.generator.is_some() || params.preset.is_some();
        if !params.detail && one {
            if let Some(shipped) = value["shipped"].as_array_mut() {
                for generator in shipped {
                    if let Some(description) = generator.get_mut("description") {
                        compact_presets(description, params.preset.as_deref());
                    }
                }
            }
            value["detail"] = Value::from(
                "presets by key, name and basis; {preset: <key>} gives one preset's values, citation and note, {detail: true} every preset's",
            );
        }
        if !params.detail && !one {
            let named = params.family.is_some() || params.text.is_some();
            value["shipped"] = report.shipped.iter().map(|g| listed(g, named)).collect();
            value["detail"] = Value::from(
                "library.generators {generator: <name>} gives one generator's parameters, rules and presets by key, name and basis; {preset: <key>} adds one preset's values; {family} or {text} adds the presets' names",
            );
        }
        Ok(value)
    }

    /// `library.generators` as typed data: what the desktop reads to build
    /// its catalog rows and its one generator form.
    pub fn generators_report(&self) -> Result<GeneratorsReport> {
        let library = &self.engine.project().library;
        // The shipped modules: name, hash and what each offers.
        let mut shipped_by_lineage: BTreeMap<String, (ContentHash, u32)> = BTreeMap::new();
        let mut shipped = Vec::new();
        let located = self.shipped();
        let names = match &located {
            Ok(found) => found.generators()?,
            Err(_) => Vec::new(),
        };
        if let Ok(found) = &located {
            for name in names {
                let bundle = found.generator(&name)?;
                let bundle_hash = bundle.hash;
                let bytes = bundle.module;
                let hash = asset_hash(&bytes);
                let (description, refused) = match self.describe(&bytes) {
                    Ok(description) => {
                        shipped_by_lineage
                            .insert(description.id.clone(), (hash, description.version));
                        (Some(GeneratorSummary::of(&description)), None)
                    }
                    Err(error) => (None, Some(permanent_refusal(error)?)),
                };
                shipped.push(ShippedGenerator {
                    name,
                    module: hash,
                    bundle: bundle_hash,
                    bytes: bytes.len(),
                    description,
                    refused,
                });
            }
        }
        // The embedded modules, each described once.
        let mut embedded = Vec::new();
        for asset in library
            .assets
            .iter()
            .filter(|a| a.media_type == generated::MODULE_MEDIA_TYPE)
        {
            let description = self.describe(&asset.data).ok();
            embedded.push(EmbeddedGenerator {
                module: asset.hash,
                bytes: asset.data.len(),
                generator: description.as_ref().map(|d| d.id.clone()),
                version: description.as_ref().map(|d| d.version),
                name: description.as_ref().map(|d| d.title.clone()),
                parts: library
                    .generated
                    .iter()
                    .filter(|g| g.from.module == asset.hash)
                    .count(),
            });
        }
        // Each part names its generator and version itself (slice B4).
        let mut parts = Vec::new();
        for part in library.generated.iter() {
            let shipped_now = shipped_by_lineage.get(&part.from.generator).copied();
            let superseded = library
                .generated
                .iter()
                .any(|p| p.root.id == part.root.id && p.root.revision > part.root.revision);
            let state = state(
                part.from.module,
                Some(part.from.version),
                shipped_now,
                superseded,
            );
            let shipped_module = shipped_now
                .map(|(hash, _)| hash)
                .filter(|hash| *hash != part.from.module);
            parts.push(GeneratedPartState {
                root: part.root,
                name: generated::describe(library, part),
                scope: part.from.scope,
                module: part.from.module,
                bundle: part.from.bundle,
                key: part.from.key()?,
                result: part.from.result,
                generator: part.from.generator.clone(),
                version: part.from.version,
                state: state.to_owned(),
                shipped_module,
            });
        }
        Ok(GeneratorsReport {
            shipped,
            shipped_library: located
                .as_ref()
                .ok()
                .map(|l| l.root().display().to_string()),
            embedded,
            parts,
        })
    }
}

/// Busy is a transient host condition, not evidence against this module.
/// Keep its typed rule so callers can retry instead of caching a missing
/// generator. Other module refusals retain their existing presentation.
pub(super) fn permanent_refusal(error: crate::Error) -> Result<String> {
    if error.diagnostic().rule.as_deref() == Some("generator.busy") {
        return Err(error);
    }
    Ok(error.to_string())
}

/// The generator runtime a host gives an engine that applies
/// `library.generate` itself (`Engine::set_generators`): a worker thread
/// per call with the host's limits and wall time, as the session's own.
pub fn runtime() -> Arc<dyn GeneratorRuntime + Send + Sync> {
    Arc::new(sparksmith_generators::worker::Worker::default())
}

/// The generator runtime for batch work nobody is there to retry (a
/// KiCad import's models): the process's workers, but a call made while
/// all are busy waits for one until `deadline` instead of being refused.
/// Interactive callers keep [`runtime`] and its retryable busy reply.
pub fn batch_runtime(deadline: std::time::Instant) -> Arc<dyn GeneratorRuntime + Send + Sync> {
    Arc::new(sparksmith_generators::worker::Worker::default().waiting_until(deadline))
}

/// How long one call of a headless session waits for a busy worker.
/// Every call holds a worker for at most its 2 s wall time, so a minute
/// covers the whole pool turning over many times; past it the call is
/// refused as busy.
pub const SESSION_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The generator runtime of a session whose engine has none
/// ([`Session::ensure_generators`]): the CLI's scripts and replays
/// (`stdio --operator`), its MCP and JSON-lines callers, the built-in
/// agent's headless runs and the tests' sessions. Their calls come one
/// after another with nobody to retry a refusal, so each call made while
/// all of the process's workers are busy waits up to [`SESSION_WAIT`]. The
/// desktop installs [`runtime`] on its engine, which sessions lent it
/// keep: its UI thread is never held waiting.
pub fn session_runtime() -> Arc<dyn GeneratorRuntime + Send + Sync> {
    Arc::new(sparksmith_generators::worker::Worker::default().waiting_for(SESSION_WAIT))
}

/// [`batch_runtime`] on a test fixture's own four-worker pool.
#[cfg(any(test, feature = "test-fixtures"))]
pub fn fixture_batch_runtime(
    deadline: std::time::Instant,
) -> Arc<dyn GeneratorRuntime + Send + Sync> {
    Arc::new(sparksmith_generators::worker::Worker::fixture().waiting_until(deadline))
}

/// A test fixture's isolated four-worker runtime. Install explicitly on
/// its engine so lending sessions and background forks inherit that pool.
#[cfg(any(test, feature = "test-fixtures"))]
pub fn fixture_runtime() -> Arc<dyn GeneratorRuntime + Send + Sync> {
    Arc::new(sparksmith_generators::worker::Worker::fixture())
}

/// `library.generators`' answer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GeneratorsReport {
    /// The modules the program ships (`library/generators/`).
    pub shipped: Vec<ShippedGenerator>,
    /// Where the shipped library is, for the host. Never on the wire: a
    /// reply reaches models and their transcripts, and the host's paths
    /// are private.
    #[serde(skip)]
    pub shipped_library: Option<String>,
    /// The modules this project embeds.
    pub embedded: Vec<EmbeddedGenerator>,
    /// Each generated part, with the state of its module.
    pub parts: Vec<GeneratedPartState>,
}

/// A shipped generator bundle and what its module offers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShippedGenerator {
    /// The bundle's name: `library.generate_part`'s `generator`.
    pub name: String,
    pub module: ContentHash,
    pub bundle: ContentHash,
    pub bytes: usize,
    /// What it offers, when the host could describe it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<GeneratorSummary>,
    /// Why the host could not describe it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

/// A module this project embeds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedGenerator {
    pub module: ContentHash,
    pub bytes: usize,
    pub generator: Option<String>,
    pub version: Option<u32>,
    pub name: Option<String>,
    /// How many of the project's generated parts it made.
    pub parts: usize,
}

/// A generated part and the state of its module ([`state`]).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GeneratedPartState {
    pub root: DefinitionRef,
    pub name: String,
    pub scope: GeneratorScope,
    pub module: ContentHash,
    pub bundle: Option<ContentHash>,
    pub key: generated::PartKey,
    pub result: ContentHash,
    pub generator: String,
    pub version: u32,
    /// `current`, `update_available`, `shipped_older`, `superseded` or
    /// `not_shipped`.
    pub state: String,
    pub shipped_module: Option<ContentHash>,
}

/// What a module offers, for people and models choosing parameters: its
/// description's part capability, flattened.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GeneratorSummary {
    /// The generator's id.
    pub generator: String,
    pub name: String,
    pub licence: String,
    pub families: Vec<String>,
    pub scopes: Vec<v1::Scope>,
    /// Each parameter: key, label, help, type and range, default, group and
    /// the project setting it is filled from.
    pub params: Vec<v1::Param>,
    /// The cross-field rules, in plain words (the module enforces them).
    pub rules: Vec<String>,
    /// Each preset with its values, citation and basis.
    pub presets: Vec<v1::Preset>,
    pub version: u32,
    pub abi: u32,
}

impl GeneratorSummary {
    pub fn of(description: &Description) -> Self {
        Self {
            generator: description.id.clone(),
            name: description.title.clone(),
            licence: description.licence.clone(),
            families: description
                .offer()
                .map(|o| o.families.clone())
                .unwrap_or_default(),
            scopes: description.scopes().to_vec(),
            params: description.params().to_vec(),
            rules: description.rules().iter().map(|r| r.text.clone()).collect(),
            presets: description.presets().to_vec(),
            version: description.version,
            abi: v1::ABI,
        }
    }

    /// The description this summary was made from, as far as a host needs
    /// it to complete and check parameters and to know where a part stands
    /// (`sparksmith_core::library::generated::standing`): the part
    /// capability with its parameters, rules, presets, families and scopes.
    pub fn description(&self) -> Description {
        Description {
            abi: self.abi,
            id: self.generator.clone(),
            version: self.version,
            title: self.name.clone(),
            licence: self.licence.clone(),
            deterministic: true,
            capabilities: vec![v1::Capability {
                name: v1::GENERATE_PART.into(),
                effect: v1::Effect::None,
                title: None,
                params: self.params.clone(),
                rules: self
                    .rules
                    .iter()
                    .map(|text| v1::Rule {
                        text: text.clone(),
                        fields: Vec::new(),
                    })
                    .collect(),
                presets: self.presets.clone(),
                part: Some(v1::PartOffer {
                    families: self.families.clone(),
                    scopes: self.scopes.clone(),
                    grammar: Vec::new(),
                    instance_fields: Vec::new(),
                }),
            }],
            needs: Vec::new(),
        }
    }
}

/// A generated part's state against the shipped module of its generator:
/// `superseded` (a newer version made the next revision of its lineages),
/// `not_shipped` (no module of its generator ships), `current` (the shipped
/// module is its own or the same version), `update_available` (a newer
/// version ships: making the part again regenerates it over the old
/// revision) or `shipped_older` (an older version ships, which never
/// downgrades the part).
fn state(
    module: ContentHash,
    version: Option<u32>,
    shipped: Option<(ContentHash, u32)>,
    superseded: bool,
) -> &'static str {
    if superseded {
        return "superseded";
    }
    match (shipped, version) {
        (None, _) => "not_shipped",
        (Some((hash, _)), _) if hash == module => "current",
        (Some((_, shipped)), Some(own)) if shipped > own => "update_available",
        (Some((_, shipped)), Some(own)) if shipped < own => "shipped_older",
        (Some(_), _) => "current",
    }
}

#[cfg(test)]
mod tests {
    use super::state;
    use sparksmith_core::ids::ContentHash;

    #[test]
    fn injected_runtime_is_used_by_sessions_forks_and_lending() {
        use sparksmith_generator_abi::{Failure, Generated, GeneratorRuntime, v1};
        use std::sync::Arc;

        struct RefusingRuntime;
        impl GeneratorRuntime for RefusingRuntime {
            fn describe(&self, _: &[u8]) -> std::result::Result<v1::Description, Failure> {
                Err(Failure::one("fixture.runtime", "Injected runtime".into()))
            }
            fn generate(
                &self,
                _: &[u8],
                _: &v1::Request,
            ) -> std::result::Result<Generated, Failure> {
                Err(Failure::one("fixture.runtime", "Injected runtime".into()))
            }
        }

        let runtime: Arc<dyn GeneratorRuntime + Send + Sync> = Arc::new(RefusingRuntime);
        let mut engine = sparksmith_core::Engine::default();
        engine.set_generators(runtime.clone());
        let mut session = super::Session::new(engine);
        session.ensure_generators();
        let refusal = session.describe(b"not a module").err();
        assert_eq!(
            refusal.as_ref().and_then(|e| e.diagnostic().rule),
            Some("fixture.runtime".into())
        );
        assert!(Arc::ptr_eq(&session.runtime(), &runtime));

        let mut fork = session.engine().fork();
        let mut lending = super::Session::lending();
        lending.lend(&mut fork, None, |session| {
            session.ensure_generators();
            let refusal = session.describe(b"not a module").err();
            assert_eq!(
                refusal.as_ref().and_then(|e| e.diagnostic().rule),
                Some("fixture.runtime".into())
            );
            assert!(Arc::ptr_eq(&session.runtime(), &runtime));
        });
        assert!(
            fork.generator_runtime()
                .is_some_and(|r| Arc::ptr_eq(r, &runtime))
        );
    }

    #[test]
    fn a_busy_description_keeps_its_rule_instead_of_becoming_a_bad_module_row() {
        let failure = sparksmith_generator_abi::Failure::one(
            "generator.busy",
            "Workers are busy. It was not run.".into(),
        );
        let error = crate::Error::Document(sparksmith_core::library::generated::failure(&failure));
        let refused = super::permanent_refusal(error).err();
        assert_eq!(
            refused.as_ref().and_then(|e| e.diagnostic().rule),
            Some("generator.busy".into())
        );
        let bad = crate::Error::Document(sparksmith_core::Error::Invalid(
            "generator.invalid_module: This module is invalid.".into(),
        ));
        assert_eq!(
            super::permanent_refusal(bad).ok().as_deref(),
            Some("This module is invalid.")
        );
    }

    #[test]
    fn a_newer_shipped_version_is_an_update_and_an_older_one_never_is() {
        let own = ContentHash([1; 32]);
        let other = ContentHash([2; 32]);
        assert_eq!(state(own, Some(1), Some((own, 1)), false), "current");
        assert_eq!(state(own, Some(1), Some((other, 1)), false), "current");
        assert_eq!(
            state(own, Some(1), Some((other, 2)), false),
            "update_available"
        );
        assert_eq!(
            state(own, Some(2), Some((other, 1)), false),
            "shipped_older"
        );
        assert_eq!(state(own, Some(1), None, false), "not_shipped");
        assert_eq!(state(own, Some(1), Some((other, 2)), true), "superseded");
    }
}
