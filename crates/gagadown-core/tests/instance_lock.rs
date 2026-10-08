use gagadown_core::Engine;

#[tokio::test]
async fn same_data_directory_cannot_start_two_engines() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    let first = Engine::new(Some(dir.clone())).unwrap();
    assert!(Engine::new(Some(dir)).is_err(), "second engine must fail before scheduling shared tasks");
    let isolated = Engine::new(Some(tmp.path().join("isolated"))).unwrap();
    isolated.shutdown().await;
    first.shutdown().await;
}
