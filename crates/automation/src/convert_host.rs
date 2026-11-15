//! The host the format 5 converter needs (`sparksmith_io::convert::v4`):
//! the shipped generators and Solidcraft, for packages whose typed form a
//! version 4 file stored. `sparksmith convert` passes one in its options;
//! the work is `sparksmith_solidcraft::models::retired`'s.
use serde_json::Value;
use sparksmith_core::ids::ContentHash;
use sparksmith_core::library::standards::Context;
use sparksmith_io::convert::v4::{ConvertedModel, Host, PackageReply, PackageRequest};
use sparksmith_io::shipped::ShippedLibrary;
use sparksmith_solidcraft::models::retired::{self, PartInfo};
use std::sync::Arc;

/// The converter host over the shipped library.
pub struct ShippedHost {
    context: Arc<Context>,
    /// Each bundled part file's manifest entry, by hash.
    parts: Vec<(ContentHash, PartInfo)>,
}

impl ShippedHost {
    /// The host of the shipped library `shipped`.
    pub fn new(shipped: &ShippedLibrary) -> crate::Result<Self> {
        let context = sparksmith_io::standards::context(shipped)?;
        let mut parts = Vec::new();
        for name in shipped.generators()? {
            let bundle = shipped.generator(&name)?;
            for part in bundle.manifest.parts {
                let file = part
                    .file
                    .rsplit('/')
                    .next()
                    .unwrap_or(&part.file)
                    .to_owned();
                parts.push((
                    part.sha256,
                    PartInfo {
                        file,
                        title: part.title,
                        licence: part.licence,
                    },
                ));
            }
        }
        Ok(Self { context, parts })
    }

    /// The host of the shipped library found as the program finds it.
    pub fn shipped() -> crate::Result<Self> {
        Self::new(&ShippedLibrary::locate()?)
    }

    /// The converter options that carry this host.
    pub fn options(self) -> sparksmith_io::convert::Options {
        sparksmith_io::convert::Options {
            host: Some(sparksmith_io::convert::HostRef(Arc::new(self))),
            ..sparksmith_io::convert::Options::default()
        }
    }
}

impl Host for ShippedHost {
    fn package(&self, request: &PackageRequest<'_>) -> Result<PackageReply, String> {
        let _scope = self.context.enter();
        let info = |_: &str, hash: ContentHash| {
            self.parts
                .iter()
                .find(|(h, _)| *h == hash)
                .map(|(_, i)| i.clone())
        };
        let got = retired::package(
            &self.context,
            &info,
            request.form,
            request.model,
            request.envelope,
            &request.terminals,
            &request.markings,
        )?;
        let model = got
            .model
            .map(|m| {
                Ok::<_, String>(ConvertedModel {
                    definition: serde_json::to_value(&m.definition).map_err(|e| e.to_string())?,
                    name: m.name,
                    bytes: m.bytes,
                    values: serde_json::to_value(&m.values).map_err(|e| e.to_string())?,
                    derived: serde_json::to_value(&m.derived).map_err(|e| e.to_string())?,
                })
            })
            .transpose()?;
        Ok(PackageReply {
            leads: got.leads,
            model,
            refusals: got.refusals,
            notes: got.notes,
        })
    }

    fn row(&self, values: &Value) -> Result<(String, Value), String> {
        retired::row(values)
    }
}
