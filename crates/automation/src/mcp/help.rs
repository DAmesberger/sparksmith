//! Resource and completion adapters for the help queries. All reads use
//! the same desk and caller grant as the corresponding query tool.
use super::{Server, tools};
use rmcp::ErrorData;
use rmcp::model::{
    CacheScope, CompleteRequestParams, CompleteResult, CompletionInfo, ReadResourceResult,
    ResourceContents,
};
use serde_json::{Value, json};

pub(super) const INDEX: &str = "sparksmith://help";
pub(super) const TEMPLATE: &str = "sparksmith://help/{id}";

impl Server {
    async fn help_query(&self, name: &'static str, params: Value) -> Result<Value, ErrorData> {
        let reply = self
            .desk
            .run(move |desk| {
                tools::dispatch(desk, tools::QUERY, json!({"name":name,"params":params}))
            })
            .await?
            .ok_or_else(|| ErrorData::internal_error("Help query is unavailable", None))?;
        if reply["ok"] != true {
            return Err(ErrorData::invalid_params(
                reply["error"]["message"]
                    .as_str()
                    .unwrap_or("Help unavailable")
                    .to_owned(),
                Some(reply["error"].clone()),
            ));
        }
        Ok(reply["result"].clone())
    }
    pub(super) async fn help_resource(&self, uri: &str) -> Result<ReadResourceResult, ErrorData> {
        let value = if uri == INDEX {
            self.help_query("help.search", json!({"text":""})).await?
        } else if let Some(id) = uri.strip_prefix("sparksmith://help/") {
            // RFC 6570 simple expansion escapes the two reserved characters
            // permitted in a HelpId. Also accept the readable URI in topic replies.
            let id = id
                .replace("%2F", "/")
                .replace("%2f", "/")
                .replace("%23", "#");
            self.help_query("help.topic", json!({"id":id})).await?
        } else {
            return Err(ErrorData::invalid_params(
                format!("Unknown resource {uri}"),
                None,
            ));
        };
        let content =
            ResourceContents::text(value.to_string(), uri).with_mime_type("application/json");
        // In a window the active profile can change between two reads of the same URI.
        Ok(ReadResourceResult::new(vec![content])
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private))
    }
    pub(super) async fn help_completion(
        &self,
        request: &CompleteRequestParams,
    ) -> Result<CompleteResult, ErrorData> {
        if request.r#ref.as_resource_uri() != Some(TEMPLATE) || request.argument.name != "id" {
            return Err(ErrorData::invalid_params(
                "Unknown completion reference or argument",
                None,
            ));
        }
        let prefix = request.argument.value.as_str();
        let found = self.help_query("help.search", json!({"text":""})).await?;
        let ids: Vec<_> = found["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|hit| hit["id"].as_str())
            .filter(|id| id.starts_with(prefix))
            .collect();
        let total = ids.len();
        let values = ids
            .into_iter()
            .take(CompletionInfo::MAX_VALUES)
            .map(str::to_owned)
            .collect();
        let completion = CompletionInfo::with_pagination(
            values,
            u32::try_from(total).ok(),
            total > CompletionInfo::MAX_VALUES,
        )
        .map_err(|e| ErrorData::internal_error(e, None))?;
        Ok(CompleteResult::new(completion))
    }
}
