use super::*;

#[test]
fn rejects_duplicate_unknown_and_malformed_parameters() {
    for raw in ["limit=1&limit=2", "model=%gg", "provider=%ff"] {
        assert!(parameters(raw).is_err());
    }
    assert!(requests(&parameters("sql=select").unwrap(), "run").is_err());
    for raw in [
        "limit=0",
        "limit=101",
        "from=2026-01-01T00:00:00Z",
        "state=unknown",
        "outcome=pending",
    ] {
        assert!(requests(&parameters(raw).unwrap(), "run").is_err());
    }
}

#[test]
fn cursor_reuses_resolved_time_and_rejects_changed_filters() {
    let parameters =
        parameters("from=2026-10-01T00:00:00Z&to=2026-10-02T00:00:00Z&model=chat").unwrap();
    let (request, _) = requests(&parameters, "run").unwrap();
    let encoded = encode(&RequestCursor {
        version: 1,
        run_id: "run".into(),
        filter: request.filter.clone(),
        high_watermark: 20,
        position: Position {
            record_id: 10,
            started_at_ms: request.filter.from_ms,
        },
    })
    .unwrap();
    let page = vec![("cursor".into(), encoded), ("limit".into(), "2".into())];
    let (next, _) = requests(&page, "run").unwrap();
    assert_eq!(next.filter, request.filter);
    assert_eq!(next.limit, 2);
    assert!(matches!(
        requests(&page, "other"),
        Err(Error::CursorExpired)
    ));
    let mut page = page;
    page.push(("model".into(), "changed".into()));
    assert!(matches!(requests(&page, "run"), Err(Error::InvalidQuery)));
}

#[test]
fn usage_has_one_dimension_and_no_success_filter() {
    let query = usage(&parameters("groupBy=provider").unwrap()).unwrap();
    assert_eq!(query.group_by, GroupBy::Provider);
    assert!(query.filter.outcome.is_none());
    assert!(usage(&parameters("state=finalized").unwrap()).is_err());
    assert!(usage(&parameters("groupBy=day,model").unwrap()).is_err());
}
