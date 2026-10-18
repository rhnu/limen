use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Row, params};

use crate::{Error, FinalRecord, PendingRecord, RequestRecord, Result, StorageOptions};

const APPLICATION_ID: i64 = 0x4C49_4D4E;
const SCHEMA_VERSION: i64 = 2;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(crate) const COLUMNS: &str = "r.record_id,r.request_id,r.run_id,r.started_at_ms,r.finished_at_ms,r.state,r.outcome,r.error_code,r.error_phase,r.error_step,r.model,r.provider,r.configured_upstream_model,r.final_upstream_model,r.client_stream,r.upstream_status,r.upstream_media,r.upstream_completed,r.input_tokens,r.output_tokens,r.total_tokens,r.cached_input_tokens,r.reasoning_output_tokens,r.usage_source,r.usage_quality,r.input_bytes,r.outbound_bytes,r.upstream_bytes,r.output_bytes,t.request_us,CASE WHEN t.upstream_headers_wait_us IS NOT NULL THEN t.pre_send_us+t.upstream_headers_wait_us END,t.downstream_first_event_us,t.metrics";
pub(crate) const SOURCE: &str = "requests r LEFT JOIN request_timings t ON t.record_id=r.record_id";

pub(crate) fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(Duration::ZERO)?;
    connection.execute_batch(
        "PRAGMA foreign_keys=ON; PRAGMA cache_size=-8192; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY;",
    )?;
    connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024)?;
    connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 64 * 1024)?;
    connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_COLUMN, 64)?;
    connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 64)?;
    Ok(())
}

pub(crate) fn initialize(
    connection: &mut Connection,
    fresh: bool,
    options: &StorageOptions,
    deadline: Instant,
) -> Result<()> {
    configure(connection)?;
    check_deadline(deadline)?;
    if fresh {
        connection.pragma_update(None, "page_size", 4096)?;
        let transaction = connection.transaction()?;
        transaction.execute_batch("CREATE TABLE requests (
            record_id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id TEXT NOT NULL UNIQUE,
            run_id TEXT NOT NULL,
            started_at_ms INTEGER NOT NULL,
            finished_at_ms INTEGER,
            state TEXT NOT NULL CHECK(state IN ('pending','finalized','interrupted')),
            outcome TEXT CHECK(outcome IN ('success','failed','cancelled')),
            error_code TEXT, error_phase TEXT, error_step TEXT,
            model TEXT, provider TEXT, configured_upstream_model TEXT, final_upstream_model TEXT,
            client_stream INTEGER NOT NULL,
            upstream_status INTEGER, upstream_media TEXT, upstream_completed INTEGER,
            input_tokens INTEGER, output_tokens INTEGER, total_tokens INTEGER,
            cached_input_tokens INTEGER, reasoning_output_tokens INTEGER,
            usage_source TEXT, usage_quality TEXT,
            input_bytes INTEGER, outbound_bytes INTEGER, upstream_bytes INTEGER, output_bytes INTEGER,
            client_parameters TEXT NOT NULL DEFAULT '{}', upstream_parameters TEXT
        ) STRICT;
        CREATE INDEX requests_time ON requests(started_at_ms,record_id);
        CREATE INDEX requests_model ON requests(model,started_at_ms,record_id);
        CREATE INDEX requests_provider ON requests(provider,started_at_ms,record_id);")?;
        create_detail_schema(&transaction)?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;
    } else {
        let application_id: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
        if application_id != APPLICATION_ID || version != SCHEMA_VERSION || page_size != 4096 {
            return Err(Error::Schema);
        }
    }
    let page_count: u64 = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
    if page_count > options.max_db_bytes / 4096 {
        return Err(Error::Capacity);
    }
    connection.pragma_update(None, "max_page_count", options.max_db_bytes / 4096)?;
    let mode: String =
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    // In-memory databases use SQLite's memory journal in deterministic unit tests.
    if mode != "wal" && mode != "memory" {
        return Err(Error::Schema);
    }
    connection.execute_batch("PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=1000; PRAGMA journal_size_limit=16777216;")?;
    loop {
        check_deadline(deadline)?;
        let recovered = connection.execute("UPDATE requests SET state='interrupted' WHERE record_id IN (SELECT record_id FROM requests WHERE state='pending' LIMIT 500)", [])?;
        if recovered == 0 {
            break;
        }
    }
    while prune_age(connection, options, now_ms()?)? != 0 {
        check_deadline(deadline)?;
    }
    loop {
        check_deadline(deadline)?;
        let count = count(connection)?;
        if count <= options.max_records {
            break;
        }
        if prune_oldest(connection, (count - options.max_records).min(500))? == 0 {
            return Err(Error::Capacity);
        }
    }
    Ok(())
}

pub(crate) fn begin(
    connection: &mut Connection,
    record: &PendingRecord,
    options: &StorageOptions,
) -> Result<()> {
    validate_pending(record)?;
    let transaction = connection.transaction()?;
    if count(&transaction)? >= options.max_records {
        prune_oldest(&transaction, 500)?;
        if count(&transaction)? >= options.max_records {
            return Err(Error::Capacity);
        }
    }
    transaction.execute("INSERT INTO requests(request_id,run_id,started_at_ms,state,client_stream,input_bytes,client_parameters) VALUES (?1,?2,?3,'pending',?4,?5,?6)", params![record.request_id,record.run_id,record.started_at_ms,record.client_stream,record.input_bytes,serde_json::to_string(&record.client_parameters)?])?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn finish(connection: &Connection, record: &FinalRecord) -> Result<()> {
    validate_final(record)?;
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute("UPDATE requests SET state='finalized',finished_at_ms=?2,outcome=?3,error_code=?4,error_phase=?5,error_step=?6,model=?7,provider=?8,configured_upstream_model=?9,final_upstream_model=?10,upstream_status=?11,upstream_media=?12,upstream_completed=?13,input_tokens=?14,output_tokens=?15,total_tokens=?16,cached_input_tokens=?17,reasoning_output_tokens=?18,usage_source=?19,usage_quality=?20,outbound_bytes=?21,upstream_bytes=?22,output_bytes=?23,upstream_parameters=?24 WHERE request_id=?1 AND state='pending'",
        params![record.request_id,record.finished_at_ms,record.outcome.map(crate::Outcome::as_str),record.error_code,record.error_phase,record.error_step,record.model,record.provider,record.configured_upstream_model,record.final_upstream_model,record.upstream_status,record.upstream_media,record.upstream_completed,record.input_tokens,record.output_tokens,record.total_tokens,record.cached_input_tokens,record.reasoning_output_tokens,record.usage_source,record.usage_quality.map(crate::UsageQuality::as_str),record.outbound_bytes,record.upstream_bytes,record.output_bytes,record.upstream_parameters.as_ref().map(serde_json::to_string).transpose()?])?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    write_details(&transaction, record)?;
    transaction.commit()?;
    Ok(())
}

fn validate_pending(record: &PendingRecord) -> Result<()> {
    if record.request_id.len() > 128
        || record.run_id.len() > 64
        || record.request_id.is_empty()
        || record.run_id.is_empty()
        || record.input_bytes > MAX_SAFE_INTEGER
        || chrono::DateTime::from_timestamp_millis(record.started_at_ms).is_none()
        || !crate::parameters::validate_parameters(&record.client_parameters)
        || serde_json::to_vec(record)?.len() > MAX_RECORD_BYTES
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn validate_final(record: &FinalRecord) -> Result<()> {
    let counts = [
        record.input_tokens,
        record.output_tokens,
        record.total_tokens,
        record.cached_input_tokens,
        record.reasoning_output_tokens,
    ];
    let numeric = [
        record.outbound_bytes,
        record.upstream_bytes,
        record.output_bytes,
    ];
    let identifiers = [
        &record.model,
        &record.provider,
        &record.error_code,
        &record.error_phase,
        &record.error_step,
        &record.usage_source,
        &record.upstream_media,
    ];
    validate_details(record)?;
    if record.request_id.is_empty()
        || record.request_id.len() > 128
        || record.outcome.is_none()
        || chrono::DateTime::from_timestamp_millis(record.finished_at_ms).is_none()
        || counts
            .iter()
            .flatten()
            .any(|count| *count > MAX_SAFE_INTEGER)
        || numeric
            .iter()
            .flatten()
            .any(|count| *count > MAX_SAFE_INTEGER)
        || identifiers
            .iter()
            .any(|value| value.as_ref().is_some_and(|value| value.len() > 128))
        || [
            &record.configured_upstream_model,
            &record.final_upstream_model,
        ]
        .iter()
        .any(|value| value.as_ref().is_some_and(|value| value.len() > 1024))
        || (matches!(
            record.usage_quality,
            Some(crate::UsageQuality::Invalid | crate::UsageQuality::Conflicting)
        ) && counts.iter().any(Option::is_some))
        || serde_json::to_vec(record)?.len() > MAX_RECORD_BYTES
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

pub(crate) fn prune_age(
    connection: &Connection,
    options: &StorageOptions,
    now_ms: i64,
) -> Result<usize> {
    let days_ms = i64::try_from(options.retention_days)
        .map_err(|_error| Error::Invalid)?
        .checked_mul(86_400_000)
        .ok_or(Error::Invalid)?;
    Ok(connection.execute("DELETE FROM requests WHERE record_id IN (SELECT record_id FROM requests WHERE state != 'pending' AND started_at_ms < ?1 ORDER BY started_at_ms,record_id LIMIT 500)", [now_ms.saturating_sub(days_ms)])?)
}

fn prune_oldest(connection: &Connection, limit: u64) -> Result<usize> {
    Ok(connection.execute("DELETE FROM requests WHERE record_id IN (SELECT record_id FROM requests WHERE state != 'pending' ORDER BY started_at_ms,record_id LIMIT ?1)", [limit])?)
}

fn count(connection: &Connection) -> Result<u64> {
    Ok(connection.query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0))?)
}

pub(crate) fn now_ms() -> Result<i64> {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_error| Error::Invalid)?;
    i64::try_from(time.as_millis()).map_err(|_error| Error::Invalid)
}

pub(crate) fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Error::Timeout)
    } else {
        Ok(())
    }
}

pub(crate) const TIMING_COLUMNS: &[(&str, &str)] = &[
    ("request_us", "requestUs"),
    ("request_decode_us", "requestDecodeUs"),
    ("storage_begin_us", "storageBeginUs"),
    ("pre_send_us", "preSendUs"),
    ("routing_us", "routingUs"),
    ("request_encode_us", "requestEncodeUs"),
    ("upstream_headers_wait_us", "upstreamHeadersWaitUs"),
    ("upstream_first_byte_us", "upstreamFirstByteUs"),
    ("upstream_first_chunk_us", "upstreamFirstChunkUs"),
    ("upstream_total_us", "upstreamTotalUs"),
    ("downstream_first_event_us", "downstreamFirstEventUs"),
    ("upstream_read_wait_us", "upstreamReadWaitUs"),
    ("downstream_queue_wait_us", "downstreamQueueWaitUs"),
    ("downstream_drain_us", "downstreamDrainUs"),
    ("cleanup_us", "cleanupUs"),
];
const GENERATION_COLUMNS: &[(&str, &str)] = &[
    ("upstream_first_content_us", "upstreamFirstContentUs"),
    ("upstream_last_content_us", "upstreamLastContentUs"),
    ("upstream_first_text_us", "upstreamFirstTextUs"),
    ("downstream_first_content_us", "downstreamFirstContentUs"),
    ("downstream_first_text_us", "downstreamFirstTextUs"),
    (
        "upstream_content_observations",
        "upstreamContentObservations",
    ),
    ("choice_count", "choiceCount"),
    ("observed_reasoning", "observedReasoning"),
    ("observed_audio", "observedAudio"),
    ("content_qualified", "contentQualified"),
    ("downstream_buffered", "downstreamBuffered"),
    ("finish_stop", "finishStop"),
    ("finish_length", "finishLength"),
    ("finish_tool", "finishTool"),
    ("finish_filtered", "finishFiltered"),
    ("finish_other", "finishOther"),
];

fn create_detail_schema(connection: &Connection) -> Result<()> {
    let columns = TIMING_COLUMNS
        .iter()
        .chain(GENERATION_COLUMNS)
        .map(|(column, _)| format!("{column} INTEGER"))
        .collect::<Vec<_>>()
        .join(",");
    connection.execute_batch(&format!("CREATE TABLE request_timings (record_id INTEGER PRIMARY KEY REFERENCES requests(record_id) ON DELETE CASCADE,{columns},metrics TEXT NOT NULL) STRICT;
        CREATE TABLE request_phases (record_id INTEGER NOT NULL REFERENCES requests(record_id) ON DELETE CASCADE,phase TEXT NOT NULL,scope TEXT NOT NULL,executor TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(record_id,phase,scope,executor)) STRICT;"))?;
    Ok(())
}

fn validate_details(record: &FinalRecord) -> Result<()> {
    for object in [
        serde_json::to_value(&record.timings)?,
        serde_json::to_value(&record.generation)?,
    ] {
        if object.as_object().is_none_or(|object| {
            object
                .values()
                .any(|value| value.as_u64().is_some_and(|value| value > MAX_SAFE_INTEGER))
        }) {
            return Err(Error::Invalid);
        }
    }
    if record
        .upstream_parameters
        .as_ref()
        .is_some_and(|value| !crate::parameters::validate_parameters(value))
        || record.phases.len() > 60
    {
        return Err(Error::Invalid);
    }
    let mut keys = std::collections::BTreeSet::new();
    for phase in &record.phases {
        if !keys.insert((&phase.phase, phase.scope, phase.executor)) {
            return Err(Error::Invalid);
        }
        validate_phase(phase)?;
    }
    Ok(())
}

fn validate_phase(phase: &crate::PhaseTiming) -> Result<()> {
    use crate::{PhaseExecutor, PhaseScope};
    let known = [
        "onRequest",
        "beforeSend",
        "afterReceive",
        "onResponse",
        "onUpstreamEvent",
        "onStreamEvent",
        "onContent",
        "onErrorResponse",
        "onError",
        "onFinish",
    ]
    .contains(&phase.phase.as_str());
    let global_only = ["onRequest", "onError", "onFinish"].contains(&phase.phase.as_str());
    let global_allowed = global_only || phase.phase == "onErrorResponse";
    if !known
        || (global_only && phase.scope != PhaseScope::Global)
        || (phase.scope == PhaseScope::Global && !global_allowed)
        || (phase.executor == PhaseExecutor::Native
            && (phase.scope == PhaseScope::Global
                || !["beforeSend", "onContent"].contains(&phase.phase.as_str())))
        || [
            phase.calls,
            phase.failed_calls,
            phase.interrupted_calls,
            phase.reported_calls,
            phase.skipped_budget_calls,
            phase.unavailable_calls,
        ]
        .iter()
        .any(|count| *count > MAX_SAFE_INTEGER)
        || phase.failed_calls > phase.calls
        || phase.interrupted_calls > phase.calls
        || phase.reported_calls > phase.calls
        || (phase.calls == 0) != phase.wall_us.is_none()
        || phase.wall_us.is_some_and(|value| value > MAX_SAFE_INTEGER)
    {
        return Err(Error::Invalid);
    }
    for sample in [
        &phase.queue,
        &phase.initial_call,
        &phase.pending,
        &phase.input_bridge,
        &phase.output_bridge,
    ] {
        if (sample.samples == 0) != sample.total_us.is_none()
            || sample.samples > phase.reported_calls
            || sample
                .total_us
                .is_some_and(|value| value > MAX_SAFE_INTEGER)
            || (phase.executor == PhaseExecutor::Native && sample.samples != 0)
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}

fn write_details(connection: &Connection, record: &FinalRecord) -> Result<()> {
    let (record_id, client_stream, client_size): (i64,bool,usize) = connection.query_row("SELECT record_id,client_stream,length(client_parameters) FROM requests WHERE request_id=?1", [&record.request_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
    if serde_json::to_vec(record)?.len() + client_size > MAX_RECORD_BYTES {
        return Err(Error::Invalid);
    }
    let timings = serde_json::to_value(&record.timings)?;
    let generation = serde_json::to_value(&record.generation)?;
    let columns = TIMING_COLUMNS
        .iter()
        .chain(GENERATION_COLUMNS)
        .map(|(column, _)| *column)
        .collect::<Vec<_>>()
        .join(",");
    let mut parameters = vec![rusqlite::types::Value::Integer(record_id)];
    for (fields, object) in [
        (TIMING_COLUMNS, &timings),
        (GENERATION_COLUMNS, &generation),
    ] {
        for (_, key) in fields {
            let value = &object[*key];
            parameters.push(if let Some(value) = value.as_bool() {
                rusqlite::types::Value::Integer(i64::from(value))
            } else if let Some(value) = value.as_u64() {
                rusqlite::types::Value::Integer(
                    i64::try_from(value).map_err(|_error| Error::Invalid)?,
                )
            } else {
                rusqlite::types::Value::Null
            });
        }
    }
    parameters.push(rusqlite::types::Value::Text(serde_json::to_string(
        &crate::derive_metrics(record, client_stream),
    )?));
    let placeholders = std::iter::repeat_n("?", parameters.len())
        .collect::<Vec<_>>()
        .join(",");
    connection.execute(
        &format!(
            "INSERT INTO request_timings(record_id,{columns},metrics) VALUES ({placeholders})"
        ),
        rusqlite::params_from_iter(parameters),
    )?;
    for phase in &record.phases {
        connection.execute("INSERT INTO request_phases(record_id,phase,scope,executor,payload) VALUES (?1,?2,?3,?4,?5)",params![record_id,phase.phase,serde_json::to_value(phase.scope)?.as_str(),serde_json::to_value(phase.executor)?.as_str(),serde_json::to_string(phase)?])?;
    }
    Ok(())
}

pub(crate) fn read_details(
    connection: &Connection,
    record_id: i64,
) -> Result<(
    crate::RequestTimings,
    crate::GenerationFacts,
    Vec<crate::PhaseTiming>,
)> {
    use rusqlite::OptionalExtension;
    let columns = TIMING_COLUMNS
        .iter()
        .chain(GENERATION_COLUMNS)
        .map(|(column, _)| *column)
        .collect::<Vec<_>>()
        .join(",");
    let data = connection
        .query_row(
            &format!("SELECT {columns} FROM request_timings WHERE record_id=?1"),
            [record_id],
            |row| {
                let mut objects = [serde_json::Map::new(), serde_json::Map::new()];
                let mut index = 0;
                for (object, fields) in objects.iter_mut().zip([TIMING_COLUMNS, GENERATION_COLUMNS])
                {
                    for (_, key) in fields {
                        let value: Option<u64> = row.get(index)?;
                        let boolean = [
                            "observedReasoning",
                            "observedAudio",
                            "contentQualified",
                            "downstreamBuffered",
                        ]
                        .contains(key);
                        object.insert(
                            (*key).into(),
                            if boolean {
                                serde_json::json!(value.map(|value| value != 0))
                            } else {
                                serde_json::json!(value)
                            },
                        );
                        index += 1;
                    }
                }
                Ok(objects)
            },
        )
        .optional()?;
    let (timings, generation) = match data {
        Some([timings, generation]) => (
            serde_json::from_value(serde_json::Value::Object(timings))?,
            serde_json::from_value(serde_json::Value::Object(generation))?,
        ),
        None => (
            crate::RequestTimings::default(),
            crate::GenerationFacts::default(),
        ),
    };
    let mut statement = connection.prepare(
        "SELECT payload FROM request_phases WHERE record_id=?1 ORDER BY phase,scope,executor",
    )?;
    let mut rows = statement.query([record_id])?;
    let mut phases = Vec::new();
    while let Some(row) = rows.next()? {
        if phases.len() >= 60 {
            return Err(Error::Invalid);
        }
        phases.push(serde_json::from_str(&row.get::<_, String>(0)?)?);
    }
    Ok((timings, generation, phases))
}

pub(crate) fn read_record(row: &Row<'_>) -> rusqlite::Result<RequestRecord> {
    Ok(RequestRecord {
        record_id: row.get(0)?,
        request_id: row.get(1)?,
        run_id: row.get(2)?,
        started_at_ms: row.get(3)?,
        finished_at_ms: row.get(4)?,
        state: parse_column(row, 5, crate::RecordState::parse)?
            .ok_or(rusqlite::Error::InvalidQuery)?,
        outcome: parse_column(row, 6, crate::Outcome::parse)?,
        error_code: row.get(7)?,
        error_phase: row.get(8)?,
        error_step: row.get(9)?,
        model: row.get(10)?,
        provider: row.get(11)?,
        configured_upstream_model: row.get(12)?,
        final_upstream_model: row.get(13)?,
        client_stream: row.get(14)?,
        upstream_status: row.get(15)?,
        upstream_media: row.get(16)?,
        upstream_completed: row.get(17)?,
        input_tokens: row.get(18)?,
        output_tokens: row.get(19)?,
        total_tokens: row.get(20)?,
        cached_input_tokens: row.get(21)?,
        reasoning_output_tokens: row.get(22)?,
        usage_source: row.get(23)?,
        usage_quality: parse_column(row, 24, crate::UsageQuality::parse)?,
        input_bytes: row.get(25)?,
        outbound_bytes: row.get(26)?,
        upstream_bytes: row.get(27)?,
        output_bytes: row.get(28)?,
        request_ms: row
            .get::<_, Option<u64>>(29)?
            .map(crate::metrics::milliseconds),
        upstream_headers_ms: row
            .get::<_, Option<u64>>(30)?
            .map(crate::metrics::milliseconds),
        first_output_ms: row
            .get::<_, Option<u64>>(31)?
            .map(crate::metrics::milliseconds),
        metrics: match row.get::<_, Option<String>>(32)? {
            Some(value) => serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    32,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            None => crate::derive_metrics(&FinalRecord::default(), row.get(14)?),
        },
    })
}

fn parse_column<T>(
    row: &Row<'_>,
    index: usize,
    parse: fn(&str) -> Option<T>,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| parse(&value).ok_or(rusqlite::Error::InvalidQuery))
        .transpose()
}

#[cfg(test)]
mod tests;
