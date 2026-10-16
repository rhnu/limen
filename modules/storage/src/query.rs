use std::{collections::BTreeMap, time::Instant};

use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value};

use crate::{
    Error, Filter, GroupBy, Outcome, Position, RecordState, RequestPage, RequestQuery,
    RequestRecord, Result, Retention, StorageOptions, TokenSum, TokenTotals, UsageGroup,
    UsageQuery, UsageSummary, database,
};

const PAGE_BYTES: usize = 1024 * 1024 - 4096;
const MAX_SCAN: u64 = 100_000;
const MAX_GROUPS: usize = 128;

fn predicate(filter: &Filter) -> Result<(String, Vec<Value>)> {
    if filter.from_ms >= filter.to_ms
        || [&filter.model, &filter.provider].iter().any(|value| {
            value
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > 128)
        })
        || chrono::DateTime::from_timestamp_millis(filter.from_ms).is_none()
        || chrono::DateTime::from_timestamp_millis(filter.to_ms).is_none()
    {
        return Err(Error::Invalid);
    }
    if filter
        .to_ms
        .checked_sub(filter.from_ms)
        .is_none_or(|range| range > 31 * 86_400_000)
    {
        return Err(Error::TooBroad);
    }
    let mut sql = "started_at_ms >= ? AND started_at_ms < ?".to_owned();
    let mut values = vec![Value::Integer(filter.from_ms), Value::Integer(filter.to_ms)];
    for (column, value) in [
        ("model", filter.model.as_deref()),
        ("provider", filter.provider.as_deref()),
        ("state", filter.state.map(RecordState::as_str)),
        ("outcome", filter.outcome.map(Outcome::as_str)),
    ] {
        if let Some(value) = value {
            sql.push_str(" AND ");
            sql.push_str(column);
            sql.push_str(" = ?");
            values.push(Value::Text(value.to_owned()));
        }
    }
    Ok((sql, values))
}

pub(crate) fn list(
    connection: &mut Connection,
    query: &RequestQuery,
    deadline: Instant,
) -> Result<RequestPage> {
    let (mut predicate, mut parameters) = predicate(&query.filter)?;
    if !(1..=100).contains(&query.limit)
        || query.high_watermark.is_some_and(|id| id < 0)
        || query.before.is_some_and(|position| position.record_id < 1)
    {
        return Err(Error::Invalid);
    }
    database::check_deadline(deadline)?;
    let transaction = connection.transaction()?;
    let high_watermark = match query.high_watermark {
        Some(value) => value,
        None => transaction.query_row(
            "SELECT COALESCE(MAX(record_id),0) FROM requests",
            [],
            |row| row.get(0),
        )?,
    };
    predicate.push_str(" AND record_id <= ?");
    parameters.push(Value::Integer(high_watermark));
    if let Some(before) = query.before {
        predicate.push_str(" AND (started_at_ms,record_id) < (?,?)");
        parameters.extend([
            Value::Integer(before.started_at_ms),
            Value::Integer(before.record_id),
        ]);
    }
    let sql = format!(
        "SELECT {} FROM requests WHERE {predicate} ORDER BY started_at_ms DESC,record_id DESC LIMIT ?",
        database::COLUMNS
    );
    parameters.push(Value::Integer(
        i64::try_from(query.limit + 1).map_err(|_error| Error::Invalid)?,
    ));
    let mut statement = transaction.prepare(&sql)?;
    let mut rows = statement.query(params_from_iter(parameters))?;
    let mut records = Vec::with_capacity(query.limit);
    let mut bytes = 0;
    let mut more = false;
    while let Some(row) = rows.next()? {
        database::check_deadline(deadline)?;
        let record = database::read_record(row)?;
        let size = serde_json::to_vec(&record)?.len() + 1;
        if records.len() == query.limit || bytes + size > PAGE_BYTES {
            more = true;
            break;
        }
        bytes += size;
        records.push(record);
    }
    let next = if more {
        records.last().map(|record| Position {
            started_at_ms: record.started_at_ms,
            record_id: record.record_id,
        })
    } else {
        None
    };
    Ok(RequestPage {
        records,
        high_watermark,
        next,
    })
}

pub(crate) fn get(connection: &Connection, request_id: &str) -> Result<Option<RequestRecord>> {
    if request_id.is_empty() || request_id.len() > 128 {
        return Err(Error::Invalid);
    }
    Ok(connection
        .query_row(
            &format!(
                "SELECT {} FROM requests WHERE request_id=?1",
                database::COLUMNS
            ),
            [request_id],
            database::read_record,
        )
        .optional()?)
}

pub(crate) fn usage(
    connection: &mut Connection,
    query: &UsageQuery,
    options: &StorageOptions,
    deadline: Instant,
) -> Result<UsageSummary> {
    if query.filter.state.is_some() || query.filter.outcome.is_some() {
        return Err(Error::Invalid);
    }
    let (predicate, parameters) = predicate(&query.filter)?;
    database::check_deadline(deadline)?;
    let transaction = connection.transaction()?;
    let oldest_started_at_ms =
        transaction.query_row("SELECT MIN(started_at_ms) FROM requests", [], |row| {
            row.get(0)
        })?;
    let mut statement = transaction.prepare(&format!(
        "SELECT {} FROM requests WHERE {predicate} LIMIT 100001",
        database::COLUMNS
    ))?;
    let mut rows = statement.query(params_from_iter(parameters))?;
    let mut groups = BTreeMap::new();
    let mut scanned = 0;
    while let Some(row) = rows.next()? {
        database::check_deadline(deadline)?;
        scanned += 1;
        if scanned > MAX_SCAN {
            return Err(Error::TooBroad);
        }
        let record = database::read_record(row)?;
        let key = match query.group_by {
            GroupBy::None => None,
            GroupBy::Model => record.model.clone(),
            GroupBy::Provider => record.provider.clone(),
            GroupBy::Day => Some(
                chrono::DateTime::from_timestamp_millis(record.started_at_ms)
                    .ok_or(Error::Invalid)?
                    .format("%Y-%m-%d")
                    .to_string(),
            ),
        };
        if !groups.contains_key(&key) && groups.len() >= MAX_GROUPS {
            return Err(Error::TooBroad);
        }
        let group = groups.entry(key.clone()).or_insert_with(|| new_group(key));
        accumulate(group, &record)?;
    }
    Ok(UsageSummary {
        from_ms: query.filter.from_ms,
        to_ms: query.filter.to_ms,
        group_by: query.group_by,
        retention: Retention {
            retention_days: options.retention_days,
            max_records: options.max_records,
            oldest_started_at_ms,
        },
        groups: groups.into_values().collect(),
    })
}

fn new_group(key: Option<String>) -> UsageGroup {
    UsageGroup {
        key,
        records: 0,
        states: BTreeMap::from([("pending", 0), ("finalized", 0), ("interrupted", 0)]),
        outcomes: BTreeMap::from([
            ("success", 0),
            ("failed", 0),
            ("cancelled", 0),
            ("unknown", 0),
        ]),
        qualities: BTreeMap::from([
            ("missing", 0),
            ("partial", 0),
            ("reported", 0),
            ("invalid", 0),
            ("conflicting", 0),
            ("unknown", 0),
        ]),
        upstream_completed: BTreeMap::from([("true", 0), ("false", 0), ("unknown", 0)]),
        tokens: TokenTotals::default(),
    }
}

fn increment(counts: &mut BTreeMap<&'static str, u64>, key: &'static str) {
    *counts.entry(key).or_default() += 1;
}

fn accumulate(group: &mut UsageGroup, record: &RequestRecord) -> Result<()> {
    group.records += 1;
    increment(&mut group.states, record.state.as_str());
    increment(
        &mut group.outcomes,
        record.outcome.map_or("unknown", Outcome::as_str),
    );
    increment(
        &mut group.qualities,
        record
            .usage_quality
            .map_or("unknown", crate::UsageQuality::as_str),
    );
    increment(
        &mut group.upstream_completed,
        match record.upstream_completed {
            Some(true) => "true",
            Some(false) => "false",
            None => "unknown",
        },
    );
    add(&mut group.tokens.input, record.input_tokens)?;
    add(&mut group.tokens.output, record.output_tokens)?;
    add(&mut group.tokens.total, record.total_tokens)?;
    add(&mut group.tokens.cached_input, record.cached_input_tokens)?;
    add(
        &mut group.tokens.reasoning_output,
        record.reasoning_output_tokens,
    )?;
    Ok(())
}

fn add(sum: &mut TokenSum, value: Option<u64>) -> Result<()> {
    if let Some(value) = value {
        sum.sum = Some(
            sum.sum
                .unwrap_or(0)
                .checked_add(u128::from(value))
                .ok_or(Error::TooBroad)?,
        );
        sum.samples += 1;
    }
    Ok(())
}
