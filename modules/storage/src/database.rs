use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Row, params};

use crate::{Error, FinalRecord, PendingRecord, RequestRecord, Result, StorageOptions};

const APPLICATION_ID: i64 = 0x4C49_4D4E;
const SCHEMA_VERSION: i64 = 1;
const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(crate) const COLUMNS: &str = "record_id,request_id,run_id,started_at_ms,finished_at_ms,state,outcome,error_code,error_phase,error_step,model,provider,configured_upstream_model,final_upstream_model,client_stream,upstream_status,upstream_media,upstream_completed,input_tokens,output_tokens,total_tokens,cached_input_tokens,reasoning_output_tokens,usage_source,usage_quality,input_bytes,outbound_bytes,upstream_bytes,output_bytes,request_ms,upstream_headers_ms,first_output_ms";

pub(crate) fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(Duration::ZERO)?;
    connection.execute_batch(
        "PRAGMA cache_size=-8192; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY;",
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
            request_ms INTEGER, upstream_headers_ms INTEGER, first_output_ms INTEGER
        ) STRICT;
        CREATE INDEX requests_time ON requests(started_at_ms,record_id);
        CREATE INDEX requests_model ON requests(model,started_at_ms,record_id);
        CREATE INDEX requests_provider ON requests(provider,started_at_ms,record_id);")?;
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
    transaction.execute("INSERT INTO requests(request_id,run_id,started_at_ms,state,client_stream,input_bytes) VALUES (?1,?2,?3,'pending',?4,?5)", params![record.request_id,record.run_id,record.started_at_ms,record.client_stream,record.input_bytes])?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn finish(connection: &Connection, record: &FinalRecord) -> Result<()> {
    validate_final(record)?;
    let changed = connection.execute("UPDATE requests SET state='finalized',finished_at_ms=?2,outcome=?3,error_code=?4,error_phase=?5,error_step=?6,model=?7,provider=?8,configured_upstream_model=?9,final_upstream_model=?10,upstream_status=?11,upstream_media=?12,upstream_completed=?13,input_tokens=?14,output_tokens=?15,total_tokens=?16,cached_input_tokens=?17,reasoning_output_tokens=?18,usage_source=?19,usage_quality=?20,outbound_bytes=?21,upstream_bytes=?22,output_bytes=?23,request_ms=?24,upstream_headers_ms=?25,first_output_ms=?26 WHERE request_id=?1 AND state='pending'",
        params![record.request_id,record.finished_at_ms,record.outcome.map(crate::Outcome::as_str),record.error_code,record.error_phase,record.error_step,record.model,record.provider,record.configured_upstream_model,record.final_upstream_model,record.upstream_status,record.upstream_media,record.upstream_completed,record.input_tokens,record.output_tokens,record.total_tokens,record.cached_input_tokens,record.reasoning_output_tokens,record.usage_source,record.usage_quality.map(crate::UsageQuality::as_str),record.outbound_bytes,record.upstream_bytes,record.output_bytes,record.request_ms,record.upstream_headers_ms,record.first_output_ms])?;
    if changed != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn validate_pending(record: &PendingRecord) -> Result<()> {
    if record.request_id.len() > 128
        || record.run_id.len() > 64
        || record.request_id.is_empty()
        || record.run_id.is_empty()
        || record.input_bytes > MAX_SAFE_INTEGER
        || chrono::DateTime::from_timestamp_millis(record.started_at_ms).is_none()
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
        record.request_ms,
        record.upstream_headers_ms,
        record.first_output_ms,
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
        request_ms: row.get(29)?,
        upstream_headers_ms: row.get(30)?,
        first_output_ms: row.get(31)?,
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
