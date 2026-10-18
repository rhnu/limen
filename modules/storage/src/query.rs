use std::{collections::BTreeMap, time::Instant};

use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value};

use crate::{
    Error, Filter, GroupBy, Outcome, Position, RecordState, RequestDetail, RequestPage,
    RequestQuery, RequestRecord, Result, Retention, StorageOptions, TokenSum, TokenTotals,
    UsageGroup, UsageQuery, UsageSummary, database,
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
    if filter
        .upstream_media
        .as_deref()
        .is_some_and(|media| !["json", "sse", "other"].contains(&media))
    {
        return Err(Error::Invalid);
    }
    let mut sql = "r.started_at_ms >= ? AND r.started_at_ms < ?".to_owned();
    let mut values = vec![Value::Integer(filter.from_ms), Value::Integer(filter.to_ms)];
    for (column, value) in [
        ("model", filter.model.as_deref()),
        ("provider", filter.provider.as_deref()),
        ("state", filter.state.map(RecordState::as_str)),
        ("outcome", filter.outcome.map(Outcome::as_str)),
    ] {
        if let Some(value) = value {
            sql.push_str(" AND ");
            sql.push_str("r.");
            sql.push_str(column);
            sql.push_str(" = ?");
            values.push(Value::Text(value.to_owned()));
        }
    }
    if let Some(value) = filter.client_stream {
        sql.push_str(" AND r.client_stream = ?");
        values.push(Value::Integer(i64::from(value)));
    }
    if let Some(value) = &filter.upstream_media {
        sql.push_str(" AND r.upstream_media = ?");
        values.push(Value::Text(value.clone()));
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
    predicate.push_str(" AND r.record_id <= ?");
    parameters.push(Value::Integer(high_watermark));
    if let Some(before) = query.before {
        predicate.push_str(" AND (r.started_at_ms,r.record_id) < (?,?)");
        parameters.extend([
            Value::Integer(before.started_at_ms),
            Value::Integer(before.record_id),
        ]);
    }
    let sql = format!(
        "SELECT {} FROM {} WHERE {predicate} ORDER BY r.started_at_ms DESC,r.record_id DESC LIMIT ?",
        database::COLUMNS,
        database::SOURCE
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

pub(crate) fn get(connection: &Connection, request_id: &str) -> Result<Option<RequestDetail>> {
    if request_id.is_empty() || request_id.len() > 128 {
        return Err(Error::Invalid);
    }
    let transaction = connection.unchecked_transaction()?;
    let record = transaction
        .query_row(
            &format!(
                "SELECT {} FROM {} WHERE r.request_id=?1",
                database::COLUMNS,
                database::SOURCE
            ),
            [request_id],
            database::read_record,
        )
        .optional()?;
    let Some(record) = record else {
        return Ok(None);
    };
    let (timings, generation, phases) = database::read_details(&transaction, record.record_id)?;
    let (client, upstream): (String, Option<String>) = transaction.query_row(
        "SELECT client_parameters,upstream_parameters FROM requests WHERE record_id=?1",
        [record.record_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(Some(RequestDetail {
        record,
        timings,
        generation,
        phases,
        parameters: crate::ParameterSnapshots {
            client: serde_json::from_str(&client)?,
            upstream: upstream.as_deref().map(serde_json::from_str).transpose()?,
        },
    }))
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
    let timing_columns = database::TIMING_COLUMNS
        .iter()
        .map(|(column, _)| format!("t.{column}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut statement = transaction.prepare(&format!(
        "SELECT {},{timing_columns} FROM {} WHERE {predicate} LIMIT 100001",
        database::COLUMNS,
        database::SOURCE
    ))?;
    let mut rows = statement.query(params_from_iter(parameters.clone()))?;
    let mut groups = BTreeMap::new();
    let mut overall = Accumulator::new(None);
    let mut scanned = 0;
    while let Some(row) = rows.next()? {
        database::check_deadline(deadline)?;
        scanned += 1;
        if scanned > MAX_SCAN {
            return Err(Error::TooBroad);
        }
        let record = database::read_record(row)?;
        let key = group_key(&record, query.group_by)?;
        if !groups.contains_key(&key) && groups.len() >= MAX_GROUPS {
            return Err(Error::TooBroad);
        }
        let group = groups
            .entry(key.clone())
            .or_insert_with(|| Accumulator::new(key));
        accumulate(&mut group.group, &record)?;
        accumulate(&mut overall.group, &record)?;
        if performance_matches(query.performance_outcome, record.outcome, record.state) {
            group.performance(&record);
            overall.performance(&record);
            for (index, (_, name)) in database::TIMING_COLUMNS.iter().enumerate() {
                let value: Option<u64> = row.get(33 + index)?;
                let key = name.trim_end_matches("Us");
                add_duration(group.stages.entry(key.into()).or_default(), value, 1)?;
                add_duration(overall.stages.entry(key.into()).or_default(), value, 1)?;
            }
        }
    }
    drop(rows);
    drop(statement);
    scan_phases(
        &transaction,
        &predicate,
        parameters,
        query,
        &mut (&mut groups, &mut overall),
        deadline,
    )?;
    let mut finished = Vec::with_capacity(groups.len());
    for group in groups.into_values() {
        finished.push(group.finish(deadline)?);
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
        groups: finished,
        overall: overall.finish(deadline)?,
        performance_outcome: query.performance_outcome,
    })
}

fn scan_phases(
    connection: &Connection,
    predicate: &str,
    parameters: Vec<Value>,
    query: &UsageQuery,
    targets: &mut (&mut BTreeMap<Option<String>, Accumulator>, &mut Accumulator),
    deadline: Instant,
) -> Result<()> {
    // Request-count and fixed phase-count bounds apply independently: no event history is read.
    let mut statement = connection.prepare(&format!("SELECT r.model,r.provider,r.started_at_ms,r.state,r.outcome,p.payload FROM requests r JOIN request_phases p ON p.record_id=r.record_id WHERE {predicate} LIMIT 6000001"))?;
    let mut rows = statement.query(params_from_iter(parameters))?;
    let mut phase_rows = 0;
    while let Some(row) = rows.next()? {
        database::check_deadline(deadline)?;
        phase_rows += 1;
        if phase_rows > MAX_SCAN * 60 {
            return Err(Error::TooBroad);
        }
        let outcome: Option<String> = row.get(4)?;
        let state: String = row.get(3)?;
        if !performance_matches(
            query.performance_outcome,
            outcome.as_deref().and_then(Outcome::parse),
            RecordState::parse(&state).ok_or(Error::Invalid)?,
        ) {
            continue;
        }
        let key = match query.group_by {
            GroupBy::None => None,
            GroupBy::Model => row.get(0)?,
            GroupBy::Provider => row.get(1)?,
            GroupBy::Day => Some(
                chrono::DateTime::from_timestamp_millis(row.get(2)?)
                    .ok_or(Error::Invalid)?
                    .format("%Y-%m-%d")
                    .to_string(),
            ),
        };
        let phase: crate::PhaseTiming = serde_json::from_str(&row.get::<_, String>(5)?)?;
        targets
            .0
            .get_mut(&key)
            .ok_or(Error::Invalid)?
            .phase(&phase)?;
        targets.1.phase(&phase)?;
    }

    Ok(())
}

fn group_key(record: &RequestRecord, by: GroupBy) -> Result<Option<String>> {
    Ok(match by {
        GroupBy::None => None,
        GroupBy::Model => record.model.clone(),
        GroupBy::Provider => record.provider.clone(),
        GroupBy::Day => Some(
            chrono::DateTime::from_timestamp_millis(record.started_at_ms)
                .ok_or(Error::Invalid)?
                .format("%Y-%m-%d")
                .to_string(),
        ),
    })
}

fn performance_matches(
    filter: crate::PerformanceOutcome,
    outcome: Option<Outcome>,
    state: RecordState,
) -> bool {
    state == RecordState::Finalized
        && match filter {
            crate::PerformanceOutcome::All => true,
            crate::PerformanceOutcome::Success => outcome == Some(Outcome::Success),
            crate::PerformanceOutcome::Failed => outcome == Some(Outcome::Failed),
            crate::PerformanceOutcome::Cancelled => outcome == Some(Outcome::Cancelled),
        }
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
        performance: crate::PerformanceSummary::default(),
        stages: BTreeMap::new(),
        phases: Vec::new(),
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

#[derive(Default)]
struct DurationAccumulator {
    sum: u128,
    samples: u64,
}

fn add_duration(
    accumulator: &mut DurationAccumulator,
    value: Option<u64>,
    samples: u64,
) -> Result<()> {
    if let Some(value) = value {
        accumulator.sum = accumulator
            .sum
            .checked_add(u128::from(value))
            .ok_or(Error::TooBroad)?;
        accumulator.samples = accumulator
            .samples
            .checked_add(samples)
            .ok_or(Error::TooBroad)?;
    }
    Ok(())
}

impl DurationAccumulator {
    #[expect(
        clippy::cast_precision_loss,
        reason = "duration sums use checked integer accumulation and are converted only for the floating-point millisecond API"
    )]
    fn finish(self) -> crate::DurationAggregate {
        let sum_ms = (self.samples > 0).then_some(self.sum as f64 / 1000.0);
        crate::DurationAggregate {
            sum_ms,
            mean_ms: sum_ms.map(|sum| sum / self.samples as f64),
            samples: self.samples,
        }
    }
}

struct PhaseAccumulator {
    value: crate::PhaseAggregate,
    durations: [DurationAccumulator; 6],
}

impl PhaseAccumulator {
    fn new(phase: &crate::PhaseTiming) -> Self {
        Self {
            value: crate::PhaseAggregate {
                phase: phase.phase.clone(),
                scope: phase.scope,
                executor: phase.executor,
                calls: 0,
                failed_calls: 0,
                interrupted_calls: 0,
                reported_calls: 0,
                skipped_budget_calls: 0,
                unavailable_calls: 0,
                wall: crate::DurationAggregate::default(),
                queue: crate::DurationAggregate::default(),
                initial_call: crate::DurationAggregate::default(),
                pending: crate::DurationAggregate::default(),
                input_bridge: crate::DurationAggregate::default(),
                output_bridge: crate::DurationAggregate::default(),
            },
            durations: std::array::from_fn(|_| DurationAccumulator::default()),
        }
    }

    fn add(&mut self, phase: &crate::PhaseTiming) -> Result<()> {
        for (target, value) in [
            (&mut self.value.calls, phase.calls),
            (&mut self.value.failed_calls, phase.failed_calls),
            (&mut self.value.interrupted_calls, phase.interrupted_calls),
            (&mut self.value.reported_calls, phase.reported_calls),
            (
                &mut self.value.skipped_budget_calls,
                phase.skipped_budget_calls,
            ),
            (&mut self.value.unavailable_calls, phase.unavailable_calls),
        ] {
            *target = target.checked_add(value).ok_or(Error::TooBroad)?;
        }
        add_duration(&mut self.durations[0], phase.wall_us, 1)?;
        for (index, sample) in [
            &phase.queue,
            &phase.initial_call,
            &phase.pending,
            &phase.input_bridge,
            &phase.output_bridge,
        ]
        .iter()
        .enumerate()
        {
            add_duration(
                &mut self.durations[index + 1],
                sample.total_us,
                sample.samples,
            )?;
        }
        Ok(())
    }

    fn finish(mut self) -> crate::PhaseAggregate {
        let [wall, queue, initial, pending, input, output] = self.durations;
        self.value.wall = wall.finish();
        self.value.queue = queue.finish();
        self.value.initial_call = initial.finish();
        self.value.pending = pending.finish();
        self.value.input_bridge = input.finish();
        self.value.output_bridge = output.finish();
        self.value
    }
}

struct Accumulator {
    group: UsageGroup,
    values: [Vec<f64>; 7],
    stages: BTreeMap<String, DurationAccumulator>,
    phases: BTreeMap<(String, crate::PhaseScope, crate::PhaseExecutor), PhaseAccumulator>,
}

impl Accumulator {
    fn new(key: Option<String>) -> Self {
        Self {
            group: new_group(key),
            values: std::array::from_fn(|_| Vec::new()),
            stages: BTreeMap::new(),
            phases: BTreeMap::new(),
        }
    }

    fn performance(&mut self, record: &RequestRecord) {
        self.group.performance.records += 1;
        let metric = &record.metrics;
        let native_downstream = metric.downstream_mode == crate::DownstreamMode::NativeStream;
        for (values, value) in self.values.iter_mut().zip([
            record.request_ms,
            metric.upstream_ttft_ms,
            metric.downstream_ttft_ms.filter(|_| native_downstream),
            metric.upstream_first_text_ms,
            metric
                .downstream_first_text_ms
                .filter(|_| native_downstream),
            metric.average_output_tps,
            metric.generation_tps,
        ]) {
            if let Some(value) = value {
                values.push(value);
            }
        }
    }

    fn phase(&mut self, phase: &crate::PhaseTiming) -> Result<()> {
        let key = (phase.phase.clone(), phase.scope, phase.executor);
        if !self.phases.contains_key(&key) && self.phases.len() >= 60 {
            return Err(Error::TooBroad);
        }
        self.phases
            .entry(key)
            .or_insert_with(|| PhaseAccumulator::new(phase))
            .add(phase)
    }

    fn finish(mut self, deadline: Instant) -> Result<UsageGroup> {
        let mut summaries = Vec::with_capacity(7);
        for mut values in self.values {
            database::check_deadline(deadline)?;
            if values.is_empty() {
                summaries.push(crate::MetricSummary::default());
                continue;
            }
            let sum = values.iter().sum::<f64>();
            values.sort_unstable_by(f64::total_cmp);
            database::check_deadline(deadline)?;
            // Nearest rank: a real observation, with no interpolated synthetic latency.
            let percentile = |percent: usize| values[(values.len() * percent).div_ceil(100) - 1];
            summaries.push(crate::MetricSummary {
                samples: values.len() as u64,
                mean: Some(
                    sum / f64::from(u32::try_from(values.len()).map_err(|_error| Error::TooBroad)?),
                ),
                min: values.first().copied(),
                max: values.last().copied(),
                p50: Some(percentile(50)),
                p95: Some(percentile(95)),
                p99: Some(percentile(99)),
            });
        }
        let mut summaries = summaries.into_iter();
        let performance = &mut self.group.performance;
        performance.request_ms = summaries.next().ok_or(Error::Invalid)?;
        performance.upstream_ttft_ms = summaries.next().ok_or(Error::Invalid)?;
        performance.downstream_ttft_ms = summaries.next().ok_or(Error::Invalid)?;
        performance.upstream_first_text_ms = summaries.next().ok_or(Error::Invalid)?;
        performance.downstream_first_text_ms = summaries.next().ok_or(Error::Invalid)?;
        performance.average_output_tps = summaries.next().ok_or(Error::Invalid)?;
        performance.generation_tps = summaries.next().ok_or(Error::Invalid)?;
        self.group.stages = self
            .stages
            .into_iter()
            .map(|(key, value)| (key, value.finish()))
            .collect();
        self.group.phases = self
            .phases
            .into_values()
            .map(PhaseAccumulator::finish)
            .collect();
        Ok(self.group)
    }
}
