use limen_config::{LogFormat, LogLevel};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Admin, Error,
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

impl Admin {
    pub(super) fn configuration(&self, parameters: &Parameters) -> Result<Value, Error> {
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
            if cursor.run_id != self.run_id {
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
            let server = &self.server;
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
            return Ok(
                json!({"data":{"listen":server.listen.to_string(),"apiKeyConfigured":true,"managerKeyConfigured":server.auth.manager_key.is_some(),"limits":server.limits,
                "storage":{"retentionDays":server.storage.retention_days,"maxRecords":server.storage.max_records,"maxDbBytes":server.storage.max_db_bytes.to_string(),"operationMs":server.storage.operation_ms},
                "log":{"level":level,"format":format,"maxFileBytes":server.log.max_file_bytes.to_string(),"maxFiles":server.log.max_files},
                "management":{"concurrency":server.management.concurrency,"timeoutMs":server.management.timeout_ms}}}),
            );
        }
        let limit = query::limit(parameters, 50, 100)?;
        let after = cursor.as_ref().map(|cursor| cursor.after.as_str());
        let mut rows:Vec<(String,Value)>=match section {
            "models"=>self.business.models.iter().filter(|(id,_)|after.is_none_or(|after|id.as_str()>after)).map(|(id,model)|(id.clone(),json!({"alias":id,"provider":model.provider,"configuredUpstreamModel":model.upstream}))).collect(),
            "providers"=>self.business.providers.iter().filter(|(id,_)|after.is_none_or(|after|id.as_str()>after)).map(|(id,provider)| {
                let origin=url::Url::parse(&provider.endpoint).map(|url|url.origin().ascii_serialization()).unwrap_or_default();
                (id.clone(),json!({"id":id,"origin":origin}))
            }).collect(),
            _=>return Err(Error::InvalidQuery),
        };
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more {
            rows.last()
                .map(|(id, _)| {
                    query::encode(&Cursor {
                        version: 1,
                        run_id: self.run_id.clone(),
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
