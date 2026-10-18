use crate::{
    FinalRecord, GenerationFacts, MetricStatus, RequestTimings, UsageQuality, derive_metrics,
};

fn record() -> FinalRecord {
    FinalRecord {
        usage_source: Some("sse".into()),
        upstream_completed: Some(true),
        output_tokens: Some(11),
        usage_quality: Some(UsageQuality::Reported),
        timings: RequestTimings {
            upstream_total_us: Some(2_000_000),
            ..RequestTimings::default()
        },
        generation: GenerationFacts {
            upstream_first_content_us: Some(200_000),
            upstream_last_content_us: Some(1_200_000),
            upstream_first_text_us: Some(300_000),
            downstream_first_content_us: Some(250_000),
            downstream_first_text_us: Some(350_000),
            upstream_content_observations: 2,
            choice_count: 1,
            content_qualified: true,
            ..GenerationFacts::default()
        },
        ..FinalRecord::default()
    }
}

#[test]
fn rate_denominators_and_content_clocks_are_independent() {
    let result = derive_metrics(&record(), true);
    assert_eq!(result.upstream_ttft_ms, Some(200.0));
    assert_eq!(result.downstream_first_text_ms, Some(350.0));
    assert_eq!(result.average_output_tps, Some(5.5));
    assert_eq!(result.generation_tps, Some(10.0));
    assert_eq!(result.generation_tps_status, MetricStatus::Available);
}

#[test]
fn json_completion_has_an_average_but_no_native_generation_rate() {
    let mut record = record();
    record.usage_source = Some("json".into());
    let result = derive_metrics(&record, false);
    assert_eq!(result.upstream_ttft_ms, None);
    assert_eq!(result.average_output_tps, Some(5.5));
    assert_eq!(result.generation_tps_status, MetricStatus::NotSse);
    assert_eq!(result.downstream_mode, crate::DownstreamMode::NotStreamed);
    assert_eq!(
        derive_metrics(&FinalRecord::default(), true).generation_tps_status,
        MetricStatus::NotReached
    );
}

#[test]
fn missing_invalid_or_unfinished_usage_is_never_a_rate_sample() {
    for quality in [
        None,
        Some(UsageQuality::Missing),
        Some(UsageQuality::Invalid),
        Some(UsageQuality::Conflicting),
    ] {
        let mut record = record();
        record.usage_quality = quality;
        assert_eq!(derive_metrics(&record, true).average_output_tps, None);
    }
    let mut record = record();
    record.upstream_completed = Some(false);
    record.timings.upstream_total_us = None;
    assert_eq!(
        derive_metrics(&record, true).average_output_tps_status,
        MetricStatus::Incomplete
    );
    record.upstream_completed = Some(true);
    record.timings.upstream_total_us = Some(2_000_000);
    record.usage_quality = Some(UsageQuality::Partial);
    assert_eq!(derive_metrics(&record, true).average_output_tps, Some(5.5));
}

#[test]
fn generation_rate_exclusions_preserve_observed_latency() {
    let mut cases = Vec::new();
    let mut multiple = record();
    multiple.generation.choice_count = 2;
    cases.push((multiple, MetricStatus::MultipleChoices));
    let mut one_token = record();
    one_token.output_tokens = Some(1);
    cases.push((one_token, MetricStatus::InsufficientTokens));
    let mut coalesced = record();
    coalesced.generation.upstream_content_observations = 1;
    cases.push((coalesced, MetricStatus::InsufficientObservations));
    let mut zero = record();
    zero.generation.upstream_last_content_us = zero.generation.upstream_first_content_us;
    cases.push((zero, MetricStatus::ZeroDuration));
    let mut hidden = record();
    hidden.reasoning_output_tokens = Some(2);
    cases.push((hidden, MetricStatus::TokenScopeMismatch));
    let mut audio = record();
    audio.generation.observed_audio = true;
    cases.push((audio, MetricStatus::TokenScopeMismatch));
    for (record, status) in cases {
        let result = derive_metrics(&record, true);
        assert_eq!(result.generation_tps, None);
        assert_eq!(result.generation_tps_status, status);
        assert_eq!(result.upstream_ttft_ms, Some(200.0));
    }
}
