use std::collections::BTreeSet;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use limen_storage::{
    Filter, GroupBy, Outcome, PerformanceOutcome, Position, RecordState, RequestQuery, UsageQuery,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::Error;

pub(super) type Parameters = Vec<(String, String)>;

pub(super) fn parameters(raw: &str) -> Result<Parameters, Error> {
    if raw.len() > 4096 {
        return Err(Error::InvalidQuery);
    }
    let mut bytes = raw.as_bytes().iter();
    while let Some(byte) = bytes.next() {
        if *byte == b'%'
            && (!bytes.next().is_some_and(u8::is_ascii_hexdigit)
                || !bytes.next().is_some_and(u8::is_ascii_hexdigit))
        {
            return Err(Error::InvalidQuery);
        }
    }
    let mut seen = BTreeSet::new();
    let mut pairs = Vec::new();
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        if key.contains('\u{fffd}') || value.contains('\u{fffd}') || !seen.insert(key.to_string()) {
            return Err(Error::InvalidQuery);
        }
        pairs.push((key.into_owned(), value.into_owned()));
    }
    Ok(pairs)
}

pub(super) fn get<'a>(parameters: &'a Parameters, key: &str) -> Option<&'a str> {
    parameters
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

pub(super) fn allowed(parameters: &Parameters, names: &[&str]) -> Result<(), Error> {
    if parameters
        .iter()
        .all(|(key, _)| names.contains(&key.as_str()))
    {
        Ok(())
    } else {
        Err(Error::InvalidQuery)
    }
}

pub(super) fn limit(
    parameters: &Parameters,
    default: usize,
    maximum: usize,
) -> Result<usize, Error> {
    get(parameters, "limit").map_or(Ok(default), |value| {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::InvalidQuery);
        }
        value
            .parse()
            .ok()
            .filter(|value| (1..=maximum).contains(value))
            .ok_or(Error::InvalidQuery)
    })
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<String, Error> {
    let result =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).map_err(|_error| Error::InvalidQuery)?);
    if result.len() > 2048 {
        return Err(Error::InvalidQuery);
    }
    Ok(result)
}

pub(super) fn decode<T: DeserializeOwned>(value: &str) -> Result<T, Error> {
    if value.len() > 2048 {
        return Err(Error::InvalidQuery);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_error| Error::InvalidQuery)?;
    serde_json::from_slice(&bytes).map_err(|_error| Error::InvalidQuery)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RequestCursor {
    pub version: u8,
    pub run_id: String,
    pub filter: Filter,
    pub high_watermark: i64,
    pub position: Position,
}

pub(super) struct Resolved {
    pub filter: Filter,
}

pub(super) fn requests(
    parameters: &Parameters,
    run: &str,
) -> Result<(RequestQuery, Resolved), Error> {
    allowed(
        parameters,
        &[
            "from",
            "to",
            "model",
            "provider",
            "state",
            "outcome",
            "clientStream",
            "upstreamMedia",
            "limit",
            "cursor",
        ],
    )?;
    let limit = limit(parameters, 50, 100)?;
    let (filter, high_watermark, before) = if let Some(encoded) = get(parameters, "cursor") {
        let cursor: RequestCursor = decode(encoded)?;
        if cursor.version != 1 {
            return Err(Error::InvalidQuery);
        }
        if cursor.run_id != run {
            return Err(Error::CursorExpired);
        }
        validate_filter(&cursor.filter)?;
        if cursor.high_watermark < 0
            || cursor.position.record_id < 1
            || cursor.position.record_id > cursor.high_watermark
            || cursor.position.started_at_ms < cursor.filter.from_ms
            || cursor.position.started_at_ms >= cursor.filter.to_ms
        {
            return Err(Error::InvalidQuery);
        }
        let filter = filter(parameters, Some(&cursor.filter))?;
        if filter != cursor.filter {
            return Err(Error::InvalidQuery);
        }
        (filter, Some(cursor.high_watermark), Some(cursor.position))
    } else {
        (filter(parameters, None)?, None, None)
    };
    Ok((
        RequestQuery {
            filter: filter.clone(),
            limit,
            high_watermark,
            before,
        },
        Resolved { filter },
    ))
}

pub(super) fn usage(parameters: &Parameters) -> Result<UsageQuery, Error> {
    allowed(
        parameters,
        &[
            "from",
            "to",
            "model",
            "provider",
            "groupBy",
            "clientStream",
            "upstreamMedia",
            "performanceOutcome",
        ],
    )?;
    let group_by = get(parameters, "groupBy")
        .map_or(Some(GroupBy::Day), GroupBy::parse)
        .ok_or(Error::InvalidQuery)?;
    Ok(UsageQuery {
        filter: filter(parameters, None)?,
        group_by,
        performance_outcome: get(parameters, "performanceOutcome")
            .map_or(Some(PerformanceOutcome::Success), PerformanceOutcome::parse)
            .ok_or(Error::InvalidQuery)?,
    })
}

fn filter(parameters: &Parameters, base: Option<&Filter>) -> Result<Filter, Error> {
    let (from_ms, to_ms) = match (get(parameters, "from"), get(parameters, "to")) {
        (Some(from), Some(to)) => (time(from)?, time(to)?),
        (None, None) => base.map_or_else(
            || {
                let to = chrono::Utc::now().timestamp_millis();
                (to - 86_400_000, to)
            },
            |filter| (filter.from_ms, filter.to_ms),
        ),
        _ => return Err(Error::InvalidQuery),
    };
    let exact = |name: &str, prior: Option<&String>| {
        get(parameters, name)
            .map(str::to_owned)
            .or_else(|| prior.cloned())
    };
    let state = get(parameters, "state")
        .map(|value| RecordState::parse(value).ok_or(Error::InvalidQuery))
        .transpose()?
        .or(base.and_then(|filter| filter.state));
    let outcome = get(parameters, "outcome")
        .map(|value| Outcome::parse(value).ok_or(Error::InvalidQuery))
        .transpose()?
        .or(base.and_then(|filter| filter.outcome));
    let filter = Filter {
        from_ms,
        to_ms,
        model: exact("model", base.and_then(|filter| filter.model.as_ref())),
        provider: exact("provider", base.and_then(|filter| filter.provider.as_ref())),
        state,
        outcome,
        client_stream: get(parameters, "clientStream")
            .map(|value| match value {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(Error::InvalidQuery),
            })
            .transpose()?
            .or(base.and_then(|filter| filter.client_stream)),
        upstream_media: exact(
            "upstreamMedia",
            base.and_then(|filter| filter.upstream_media.as_ref()),
        ),
    };
    validate_filter(&filter)?;
    Ok(filter)
}

fn time(value: &str) -> Result<i64, Error> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp_millis())
        .map_err(|_error| Error::InvalidQuery)
}

fn validate_filter(filter: &Filter) -> Result<(), Error> {
    if filter
        .upstream_media
        .as_deref()
        .is_some_and(|value| !matches!(value, "json" | "sse" | "other"))
    {
        return Err(Error::InvalidQuery);
    }
    if filter.from_ms >= filter.to_ms
        || chrono::DateTime::from_timestamp_millis(filter.from_ms).is_none()
        || chrono::DateTime::from_timestamp_millis(filter.to_ms).is_none()
    {
        return Err(Error::InvalidQuery);
    }
    if filter
        .to_ms
        .checked_sub(filter.from_ms)
        .is_none_or(|span| span > 31 * 86_400_000)
    {
        return Err(Error::TooBroad);
    }
    if [&filter.model, &filter.provider].iter().any(|value| {
        value
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 128)
    }) {
        return Err(Error::InvalidQuery);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
