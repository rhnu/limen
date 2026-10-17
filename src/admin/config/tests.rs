use super::*;

fn snapshot() -> AdminConfigSnapshot {
    let server=ServerConfig::parse("[auth]\napi_key='client-secret-01234567890123456789'\nmanager_key='manager-secret-01234567890123456789'\n",limen_config::ServerFormat::Toml).unwrap();
    let business=BusinessConfig::from_value(json!({"providers":{"private":{"endpoint":"https://fictional.invalid/private?key=query-secret","auth":{"bearer":"upstream-secret"}}},"models":{"z":{"provider":"private","upstream":"last"},"a":{"provider":"private","upstream":"first"},"m":{"provider":"private","upstream":"middle"}}}),&server.limits).unwrap();
    AdminConfigSnapshot::new(&server, &business)
}

#[test]
fn approved_snapshot_hides_credentials_and_pages_selected_sorted_rows() {
    let snapshot = snapshot();
    let server = snapshot
        .page(&query::parameters("").unwrap(), "run-1")
        .unwrap();
    assert_eq!(server["data"]["apiKeyConfigured"], true);
    assert_eq!(server["data"]["managerKeyConfigured"], true);
    let providers = snapshot
        .page(&query::parameters("section=providers").unwrap(), "run-1")
        .unwrap();
    assert_eq!(
        providers["data"],
        json!([{"id":"private","origin":"https://fictional.invalid"}])
    );
    let serialized = format!("{server}{providers}");
    for secret in [
        "client-secret",
        "manager-secret",
        "upstream-secret",
        "query-secret",
        "/private?",
    ] {
        assert!(!serialized.contains(secret));
    }
    let first = snapshot
        .page(
            &query::parameters("section=models&limit=1").unwrap(),
            "run-1",
        )
        .unwrap();
    assert_eq!(
        first["data"],
        json!([{"alias":"a","provider":"private","configuredUpstreamModel":"first"}])
    );
    let cursor = first["nextCursor"].as_str().unwrap();
    let parameters = query::parameters(&format!("cursor={cursor}&limit=2")).unwrap();
    let rest = snapshot.page(&parameters, "run-1").unwrap();
    assert_eq!(
        rest["data"],
        json!([{"alias":"m","provider":"private","configuredUpstreamModel":"middle"},{"alias":"z","provider":"private","configuredUpstreamModel":"last"}])
    );
    assert_eq!(rest["nextCursor"], Value::Null);
    assert_eq!(
        snapshot.page(&parameters, "run-2").unwrap_err(),
        Error::CursorExpired
    );
}
