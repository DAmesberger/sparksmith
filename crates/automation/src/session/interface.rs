//! `interface.publish_preview` and `interface.publish`, the board
//! interface's host requests (docs/architecture/board-interface.md,
//! "Commands"; slice B3): the preview reads the document and the published
//! sidecar; the publish builds the design from the core's candidate sidecar
//! with Solidcraft, commits the design and sidecar through a recoverable journal and
//! records the publish with `interface.record_publish`, which, like a save,
//! is no undo step. An invalid board outline, or a broken reference between
//! the board and the linked design, blocks the publish, writing nothing.
use super::Session;
use crate::Result;
use serde_json::{Value, json};
use sparksmith_core::Operation;
use sparksmith_core::ids::ContentHash;
use sparksmith_core::interface::publish;
use sparksmith_core::interface::{InterfaceCommand, PublishParams, PublishPreviewParams};
use sparksmith_core::model::interface::Tier;
use sparksmith_io::interface as files;
use sparksmith_io::project_models::portable;
use std::path::{Path, PathBuf};

/// The linked design file a publish is bound to: where it is and what
/// its bytes hash to.
type Linked = (PathBuf, ContentHash);

/// The token binding a publish to its preview (review findings 12 and
/// 18): the board revision (fill copper stored since is none; a Full
/// publish binds the copper itself), the tier, the candidate the preview showed
/// (its datum table's hash), where it is published (the destination,
/// resolved) and that destination's design and sidecar bytes, and the
/// linked design file's identity and bytes. Anything else about the
/// request may differ; any of these differing refuses the publish.
fn publish_token(
    project: &sparksmith_core::model::Project,
    revision: u64,
    tier: Tier,
    candidate: ContentHash,
    path: &Path,
    destination: (Option<ContentHash>, Option<ContentHash>),
    linked: Option<&Linked>,
) -> Result<String> {
    use sha2::{Digest, Sha256};
    // Packed assets already carry their content hashes. Bind them by those
    // hashes without encoding their potentially large bodies as base64.
    let mut document = project.clone();
    for asset in &mut document.library.assets {
        asset.data = Default::default();
    }
    let mut contents = Sha256::new();
    serde_json::to_writer(&mut contents, &document)?;
    if tier == Tier::Full {
        // Computed copper is exported but omitted from the persisted project.
        for fill in project.fills.iter() {
            serde_json::to_writer(&mut contents, &fill.computed)?;
        }
    }
    let bound = json!({
        "project": ContentHash(contents.finalize().into()),
        "revision": revision,
        "tier": tier,
        "candidate": candidate,
        "path": resolved(path).to_string_lossy(),
        "design": destination.0,
        "sidecar": destination.1,
        "linked": linked.map(|(path, hash)| json!({
            "path": path.to_string_lossy(),
            "sha256": hash,
        })),
    });
    let digest = Sha256::digest(serde_json::to_vec(&bound)?);
    Ok(format!("publish-{}", ContentHash(digest.into())))
}

/// `path` with its folder resolved (links and `..` followed), so two
/// spellings of one destination are one destination; the file itself
/// need not exist yet.
fn resolved(path: &Path) -> PathBuf {
    let folder = match path.parent() {
        Some(p) if p.as_os_str().is_empty() => Path::new("."),
        Some(p) => p,
        None => return path.to_path_buf(),
    };
    match (folder.canonicalize(), path.file_name()) {
        (Ok(folder), Some(name)) => folder.join(name),
        _ => path.to_path_buf(),
    }
}

impl Session {
    /// `design` as the project keeps it: relative to the project's folder
    /// when it lies inside it, `/`-separated on every platform, else as
    /// given.
    fn relative_to_project(&self, design: &Path) -> String {
        let folder = self.path.as_deref().and_then(Path::parent);
        let relative = match (folder, design.is_absolute()) {
            (Some(folder), true) if !folder.as_os_str().is_empty() => {
                let folder = folder
                    .canonicalize()
                    .unwrap_or_else(|_| folder.to_path_buf());
                let file = design
                    .parent()
                    .and_then(|p| p.canonicalize().ok())
                    .zip(design.file_name())
                    .map_or_else(|| design.to_path_buf(), |(p, n)| p.join(n));
                file.strip_prefix(&folder).ok().map(Path::to_path_buf)
            }
            _ => None,
        };
        let kept = relative.unwrap_or_else(|| design.to_path_buf());
        if kept.is_absolute() {
            kept.to_string_lossy().into_owned()
        } else {
            portable(&kept)
        }
    }

    /// The diff publishing would make, against the sidecar beside `path`
    /// when it is there, else the recorded publish. Reads only.
    pub(super) fn interface_preview(&mut self, params: PublishPreviewParams) -> Result<Value> {
        let baseline = match params.path.as_deref() {
            Some(path) => {
                let path = Path::new(path);
                files::check_design_path(path)?;
                Some(files::Publication::open(path)?.baseline()?)
            }
            None => None,
        };
        #[cfg(test)]
        if let Some(after_read) = self.after_interface_read.take() {
            after_read()?;
        }
        let previous = baseline.as_ref().and_then(|b| b.sidecar.as_ref());
        let mut preview = publish::preview(
            self.engine.project(),
            params.tier,
            previous,
            self.engine.revision(),
        )?;
        // With the linked design named, the dry run on a scratch copy.
        let project = self.engine.project_arc().clone();
        let mut dry_run_read = None;
        if let (Some(linked_design), Some(design), Some(source_id), Some(baseline)) = (
            params.assembly_path.as_deref(),
            params.path.as_deref(),
            project.interface.source_id,
            baseline.as_ref(),
        ) {
            let candidate = publish::candidate(&project, preview.tier, source_id)?;
            let (dry_run, read) = self.dry_run(linked_design, design, candidate, baseline)?;
            preview.dry_run = dry_run;
            dry_run_read = Some(read);
        }
        let Some(destination) = params.path.as_deref() else {
            return Ok(serde_json::to_value(preview)?);
        };
        if (project.assembly_link.is_some() || params.assembly_path.is_some())
            && preview.dry_run.is_none()
        {
            return Ok(serde_json::to_value(preview)?);
        }
        // The token binds the linked design's bytes the dry run read.
        let linked = match params.assembly_path.as_deref() {
            Some(path) => Some(self.linked_file(path, dry_run_read)?),
            None => None,
        };
        preview.token = Some(publish_token(
            &project,
            self.engine.content_revision(),
            preview.tier,
            preview.hash,
            Path::new(destination),
            baseline
                .as_ref()
                .map_or((None, None), files::Baseline::identity),
            linked.as_ref(),
        )?);
        Ok(serde_json::to_value(preview)?)
    }

    /// The linked design file at `given` (relative to the project's
    /// folder) and its bytes' hash: `read`, when a dry run read them, else
    /// read now. Refused when the file is not there.
    fn linked_file(&self, given: &str, read: Option<ContentHash>) -> Result<Linked> {
        let file = self.assembly_path(given);
        let hash = match read {
            Some(hash) => hash,
            None => sparksmith_io::assembly_link::hash(&file)?.ok_or_else(|| {
                sparksmith_io::Error::Invalid(format!(
                    "assembly.missing: The linked design file was not found at {}",
                    file.display()
                ))
            })?,
        };
        let file = file.canonicalize().unwrap_or(file);
        Ok((file, hash))
    }

    /// Refuse a publish that is not the one its preview showed (review
    /// finding 12): with a linked design linked, or named, the preview's
    /// token is required, and it must still bind this board revision,
    /// tier, destination and linked design file.
    fn check_publish_token(
        &self,
        params: &PublishParams,
        baseline: &files::Baseline,
    ) -> Result<Option<Linked>> {
        let linked_project = self.engine.project().assembly_link.is_some();
        let refuse = |message: &str| -> crate::Error {
            sparksmith_core::diagnostics::Error::Conflict(message.to_owned()).into()
        };
        if linked_project && params.assembly_path.is_none() {
            return Err(refuse(
                "interface.token: a linked design is linked; preview the publish with it \
                 (assembly_path) and publish with the preview's assembly_path and token",
            ));
        }
        let Some(token) = params.token.as_deref() else {
            if params.assembly_path.is_some() {
                return Err(refuse(
                    "interface.token: publish with the token of the preview that dry-ran the \
                     linked design",
                ));
            }
            return Ok(None);
        };
        let linked = match params.assembly_path.as_deref() {
            Some(path) => Some(self.linked_file(path, None)?),
            None => None,
        };
        // The candidate's hash is its datum table's, whatever source the
        // sidecar is stamped with.
        let project = self.engine.project();
        let source = project.interface.source_id.unwrap_or_default();
        let candidate = publish::candidate(project, params.tier, source)?;
        let now = publish_token(
            project,
            self.engine.content_revision(),
            params.tier,
            candidate.hash,
            Path::new(&params.path),
            baseline.identity(),
            linked.as_ref(),
        )?;
        if now != token {
            return Err(refuse(
                "interface.preview_changed: the board, the destination, the published files or \
                 the linked design differ from the preview; preview again",
            ));
        }
        if let Some(path) = params.assembly_path.as_deref() {
            let link = project.assembly_link.as_deref().ok_or_else(|| {
                refuse("interface.token: the reviewed assembly is no longer linked")
            })?;
            self.same_files(link, Some(path), &params.path)?;
            if baseline.sidecar.is_none() || project.interface.source_id.is_none() {
                return Err(refuse(
                    "interface.token: preview the linked design against its published board",
                ));
            }
        }
        Ok(linked)
    }

    /// Write the interface design and its sidecar at `path`, then record
    /// the publish. The document's revision was checked before this runs
    /// (`Request::expected_revision`), so a stale request writes nothing.
    /// Refused, writing nothing, when the files there belong to another
    /// board's interface.
    pub(super) fn interface_publish(&mut self, params: PublishParams) -> Result<Value> {
        let (operation, published) = self.interface_write(&params)?;
        let mut reply = self.apply(params.expected_revision, operation)?;
        reply["published"] = published;
        Ok(reply)
    }

    /// The files of `interface.publish`, without recording it: build the
    /// design from the document's candidate sidecar, put it and the sidecar
    /// in place as a recoverable pair, and return `interface.record_publish` with what
    /// was written (no undo step, like a save) and the reply's `published`
    /// summary. The desktop runs this off the UI thread, on a session over
    /// a fork of the window's engine, and applies the record on its own
    /// engine; nothing else differs from the request. Refused, writing
    /// nothing, while the board outline is invalid or a reference between
    /// the board and the linked design is broken.
    pub fn interface_write(&self, params: &PublishParams) -> Result<(Operation, Value)> {
        let design = Path::new(&params.path);
        files::check_design_path(design)?;
        let project = self.engine.project_arc().clone();
        sparksmith_core::interface::assembly_link::check_publish(&project)?;
        let baseline = files::Publication::open(design)?.baseline()?;
        let reviewed = self.check_publish_token(params, &baseline)?;
        let previous = baseline.sidecar.as_ref();
        let recorded = project.interface.source_id;
        let source_id = match (recorded, previous) {
            (Some(own), Some(there)) if there.source_id != own => None,
            (Some(own), _) => Some(own),
            (None, Some(_)) => None,
            (None, None) => Some(files::new_source_id()?),
        }
        .ok_or_else(|| {
            sparksmith_io::Error::Invalid(
                "interface.source: the interface published there belongs to another board; \
                 choose another file"
                    .into(),
            )
        })?;
        if let (Some(there), Some(record)) = (previous, project.interface.published.as_ref())
            && (there.revision > record.revision
                || (there.revision == record.revision && there.hash != record.hash))
        {
            return Err(sparksmith_io::Error::Invalid(
                "interface.changed: a newer or different revision is already published; reopen the current board before publishing".into(),
            ).into());
        }
        let sidecar = publish::candidate(&project, params.tier, source_id)?;
        let exported = sparksmith_solidcraft::interface::build(&project, sidecar)?;
        let check = exported.check.clone();
        // A test's model that does not fit its plan.
        #[cfg(test)]
        let check = self.interface_check.clone().or(check);
        // The standalone export's fit policy: bodies that differ from the
        // board plan are refused, writing nothing, unless asked otherwise.
        if let Some(check) = check.as_ref() {
            sparksmith_solidcraft::export::check_fit(
                &project,
                exported.sidecar.tier,
                check,
                if params.write_anyway {
                    sparksmith_solidcraft::export::Unfit::Write
                } else {
                    sparksmith_solidcraft::export::Unfit::Refuse
                },
                Default::default(),
            )?;
        }
        let text = exported.sidecar.text()?;
        let temporary =
            sparksmith_io::persistence::StagedNew::new(&files::temporary_design(design)?)?;
        let temporary_text = temporary.path().to_string_lossy().into_owned();
        exported.stage_new(crate::native_files::new_file(&temporary_text), &params.path)
            .map_err(|error| -> crate::Error {
                match error {
                    sparksmith_solidcraft::Error::Unsynced(message) => sparksmith_io::Error::Io(
                        std::io::Error::other(format!("The interface publication was not committed; native staging could not be synchronized: {message}")),
                    ).into(),
                    other => other.into(),
                }
            })?;
        // The hash SolidCraft keeps on a linked design's link to the file.
        let written = files::design_hash(temporary.path())?;
        let mut record = publish::record(&project, &exported.sidecar)?;
        record.design_sha256 = Some(written);
        // Where it was written, kept with the project as the next
        // publish's default: relative to the project's folder when inside it.
        record.design_hint = Some(self.relative_to_project(design));
        let mut published = json!({
            "path": params.path,
            "sidecar": files::sidecar_path(design).to_string_lossy(),
            "revision": exported.sidecar.revision,
            "hash": exported.sidecar.hash,
            "tier": exported.sidecar.tier,
            "datums": exported.sidecar.datums.len(),
        });
        // Section 2 checked against the board plan (A5): reported; past
        // the fit policy only when written anyway.
        if let Some(check) = check {
            let report = sparksmith_solidcraft::export::report(
                exported.sidecar.tier,
                0,
                Some(check),
                Default::default(),
            );
            let checked = super::export_reply(&project, &report, json!({}));
            published["check"] = checked["check"].clone();
        }
        #[cfg(test)]
        if let Some(staged) = self
            .after_interface_stage
            .lock()
            .ok()
            .and_then(|mut hook| hook.take())
        {
            staged()?;
        }
        // What the preview approved is checked again at installation: the
        // linked design or the destination may have changed while the
        // native design was built.
        files::put_reviewed_in_place(
            design,
            temporary.path(),
            &text,
            &baseline,
            reviewed.as_ref(),
        )?;
        Ok((
            Operation::Interface(InterfaceCommand::RecordPublish { source_id, record }),
            published,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::portable;
    use std::path::PathBuf;

    /// Built from parts with the host's separator, kept with `/`.
    #[test]
    fn a_relative_hint_is_slash_separated_on_every_host() {
        let path: PathBuf = ["shared", "boards", "board.solidcraft"].iter().collect();
        assert_eq!(portable(&path), "shared/boards/board.solidcraft");
        assert_eq!(
            portable(&PathBuf::from("board.solidcraft")),
            "board.solidcraft"
        );
    }
}
