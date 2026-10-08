use gagadown_core::{AddRequest, Engine};
use gagadown_core::task::TaskRecord;

#[tokio::test]
async fn old_records_default_missing_route_latency() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(Some(tmp.path().join("state"))).unwrap();
    let mut settings = engine.settings();
    settings.download_dir = tmp.path().join("files");
    engine.set_settings(settings).unwrap();
    engine.add(AddRequest { url: "http://127.0.0.1:1/fixture.bin".into(), ..Default::default() }, None).unwrap();
    engine.shutdown().await;
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(tmp.path().join("state/state.json")).unwrap()).unwrap();
    let mut record = state["tasks"][0].clone();
    record.as_object_mut().unwrap().remove("route_latency_ms");
    let old: TaskRecord = serde_json::from_value(record.clone()).unwrap();
    assert_eq!(old.route_latency_ms, None);
    record["route_latency_ms"] = 37.into();
    let current: TaskRecord = serde_json::from_value(record).unwrap();
    assert_eq!(current.route_latency_ms, Some(37));
}
