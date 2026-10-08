use gagadown_core::{AddRequest, Engine};
use gagadown_core::task::RemoveMode;

#[tokio::test]
async fn permanent_delete_is_retryable_and_removes_partial_in_one_operation() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(Some(tmp.path().join("data"))).unwrap();
    let out = engine.add(AddRequest {
        url: "http://127.0.0.1:1/file.bin".into(),
        start_paused: true,
        ..Default::default()
    }, None).unwrap();
    let path = tmp.path().join("file.bin");
    let part = tmp.path().join("file.bin.fdpart");
    engine.inner.tasks.read()[0].with(|r| r.path = Some(path.clone()));
    // A directory where a file should be forces a deterministic deletion error.
    std::fs::create_dir(&part).unwrap();
    engine.remove(out.id, RemoveMode::DeleteFiles).await;
    assert!(engine.views(0).iter().any(|v| v.id == out.id), "failed deletion must retain the task");
    std::fs::remove_dir(&part).unwrap();
    std::fs::write(&part, b"partial download").unwrap();
    engine.remove(out.id, RemoveMode::DeleteFiles).await;
    assert!(!part.exists());
    assert!(!engine.views(0).iter().any(|v| v.id == out.id));
    // Missing files / repeated requests are harmless.
    engine.remove(out.id, RemoveMode::DeleteFiles).await;
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removal_waits_for_open_writer_beyond_old_timeout() {
    use gagadown_core::download::Shared;
    use gagadown_core::engine::RunHandle;
    use std::sync::Arc;
    use std::time::Duration;

    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(Some(tmp.path().join("data"))).unwrap();
    let out = engine.add(AddRequest {
        url: "http://127.0.0.1:1/file.bin".into(),
        start_paused: true,
        ..Default::default()
    }, None).unwrap();
    let path = tmp.path().join("file.bin");
    let part = tmp.path().join("file.bin.fdpart");
    let file = std::fs::File::create(&part).unwrap();
    let entry = engine.inner.tasks.read()[0].clone();
    entry.with(|r| r.path = Some(path));
    let shared = Arc::new(Shared::new());
    let cancel = shared.cancel.clone();
    let worker_entry = entry.clone();
    let join = tokio::spawn(async move {
        cancel.cancelled().await;
        tokio::time::sleep(Duration::from_secs(12)).await;
        // Simulate a preallocation/write which cannot stop immediately.
        file.set_len(4096).unwrap();
        drop(file);
        *worker_entry.run.lock() = None;
    });
    *entry.run.lock() = Some(RunHandle { shared, join: Some(join) });
    let deleting = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.remove(out.id, RemoveMode::DeleteFiles).await })
    };
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert!(!deleting.is_finished(), "must not detach writer after ten seconds");
    assert!(part.exists(), "must not unlink an open writer's file");
    engine.resume(out.id); // Must not restart a task being deleted.
    tokio::time::timeout(Duration::from_secs(10), deleting).await.unwrap().unwrap();
    assert!(!part.exists());
    assert!(!engine.views(0).iter().any(|v| v.id == out.id));
    engine.shutdown().await;
}
