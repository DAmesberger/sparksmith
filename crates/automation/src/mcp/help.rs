//! Resource and completion adapters for the help queries. All reads use
//! the same desk and caller grant as the corresponding query tool.
use super::{Server, tools};
use rmcp::ErrorData;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

pub(super) const INDEX: &str = "sparksmith://help";
pub(super) const TEMPLATE: &str = "sparksmith://help/{id}";

pub(super) fn model<T: DeserializeOwned>(value: Value) -> Result<T, ErrorData> {
    serde_json::from_value(value).map_err(|e| ErrorData::internal_error(e.to_string(), None))
}

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
    pub(super) async fn help_resource(&self, uri: &str) -> Result<Value, ErrorData> {
        let value = if uri == INDEX {
            self.help_query("help.search", json!({"text":""})).await?
        } else if let Some(id) = uri.strip_prefix("sparksmith://help/") {
            self.help_query("help.topic", json!({"id":id})).await?
        } else {
            return Err(ErrorData::invalid_params(
                format!("Unknown resource {uri}"),
                None,
            ));
        };
        Ok(json!({"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}))
    }
    pub(super) async fn help_completion(
        &self,
        request: &impl Serialize,
    ) -> Result<Value, ErrorData> {
        let req = serde_json::to_value(request)
            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        if req["ref"]["uri"] != TEMPLATE || req["argument"]["name"] != "id" {
            return Err(ErrorData::invalid_params(
                "Unknown completion reference or argument",
                None,
            ));
        }
        let prefix = req["argument"]["value"].as_str().unwrap_or_default();
        let found = self.help_query("help.search", json!({"text":""})).await?;
        let ids: Vec<_> = found["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|hit| hit["id"].as_str())
            .filter(|id| id.starts_with(prefix))
            .collect();
        // MCP completion lists at most 100 values.
        let total = ids.len();
        let values: Vec<_> = ids.into_iter().take(100).collect();
        Ok(json!({"completion":{"values":values,"total":total,"hasMore":total>100}}))
    }
}
