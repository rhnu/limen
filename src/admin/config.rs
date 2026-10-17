use std::collections::BTreeMap;

use limen_config::{BusinessConfig, LogFormat, LogLevel, ServerConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Error,
    query::{self, Parameters},
};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cursor {
    version: u8,
    run_id: String,
    section: String,
    after: String,
}

/// Approved configuration data only; credentials and source objects never enter Admin.
pub(crate) struct AdminConfigSnapshot {
    server: Value,
    models: BTreeMap<String, Value>,
    providers: BTreeMap<String, Value>,
}

impl AdminConfigSnapshot {
    pub(crate) fn new(server: &ServerConfig, business: &BusinessConfig) -> Self {
        let level = match server.log.level {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        let format = match server.log.format {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
        };
        let snapshot = json!({"listen":server.listen.to_string(),"apiKeyConfigured":true,"managerKeyConfigured":server.auth.manager_key.is_some(),"limits":server.limits,
            "storage":{"retentionDays":server.storage.retention_days,"maxRecords":server.storage.max_records,"maxDbBytes":server.storage.max_db_bytes.to_string(),"operationMs":server.storage.operation_ms},
            "log":{"level":level,"format":format,"maxFileBytes":server.log.max_file_bytes.to_string(),"maxFiles":server.log.max_files},
            "management":{"concurrency":server.management.concurrency,"timeoutMs":server.management.timeout_ms}});
        let models = business.models.iter().map(|(id,model)|(id.clone(),json!({"alias":id,"provider":model.provider,"configuredUpstreamModel":model.upstream}))).collect();
        let providers = business
            .providers
            .iter()
            .map(|(id, provider)| {
                let origin = url::Url::parse(&provider.endpoint)
                    .map(|url| url.origin().ascii_serialization())
                    .unwrap_or_default();
                (id.clone(), json!({"id":id,"origin":origin}))
            })
            .collect();
        Self {
            server: snapshot,
            models,
            providers,
        }
    }
}

impl AdminConfigSnapshot {
    pub(super) fn page(&self, parameters: &Parameters, run_id: &str) -> Result<Value, Error> {
        query::allowed(parameters, &["section", "limit", "cursor"])?;
        let cursor: Option<Cursor> = query::get(parameters, "cursor")
            .map(query::decode)
            .transpose()?;
        if let Some(cursor) = &cursor {
            if cursor.version != 1
                || !matches!(cursor.section.as_str(), "models" | "providers")
                || cursor.after.is_empty()
                || cursor.after.len() > 128
            {
                return Err(Error::InvalidQuery);
            }
            if cursor.run_id != run_id {
                return Err(Error::CursorExpired);
            }
        }
        let section = query::get(parameters, "section")
            .or_else(|| cursor.as_ref().map(|cursor| cursor.section.as_str()))
            .unwrap_or("server");
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.section != section)
        {
            return Err(Error::InvalidQuery);
        }
        if section == "server" {
            if cursor.is_some() || query::get(parameters, "limit").is_some() {
                return Err(Error::InvalidQuery);
            }
            return Ok(json!({"data":self.server}));
        }
        let limit = query::limit(parameters, 50, 100)?;
        let after = cursor.as_ref().map(|cursor| cursor.after.as_str());
        let entries = match section {
            "models" => &self.models,
            "providers" => &self.providers,
            _ => return Err(Error::InvalidQuery),
        };
        let mut selected = entries
            .iter()
            .filter(|(id, _)| after.is_none_or(|after| id.as_str() > after));
        let rows: Vec<_> = selected
            .by_ref()
            .take(limit)
            .map(|(id, row)| (id.clone(), row.clone()))
            .collect();
        let more = selected.next().is_some();
        let next = if more {
            rows.last()
                .map(|(id, _)| {
                    query::encode(&Cursor {
                        version: 1,
                        run_id: run_id.to_owned(),
                        section: section.to_owned(),
                        after: id.clone(),
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(json!({"data":rows.into_iter().map(|(_,row)|row).collect::<Vec<_>>(),"nextCursor":next}))
    }
}

#[cfg(test)]
mod tests;
