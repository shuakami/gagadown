use gagadown_core::{AddRequest, Engine};

/// The GUI calls engine methods from its own (non-tokio) thread.
#[test]
fn resume_from_non_runtime_thread() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let dir = std::env::temp_dir().join(format!("gagadown-test-{}", std::process::id()));
    let engine = {
        let _g = rt.enter();
        Engine::new(Some(dir.clone())).unwrap()
    };
    let e = engine.clone();
    std::thread::spawn(move || {
        let out = e
            .add(AddRequest { url: "http://127.0.0.1:9/file.bin".into(), start_paused: true, ..Default::default() }, None)
            .unwrap();
        e.resume(out.id);
        e.pause(out.id);
        e.resume_all();
    })
    .join()
    .expect("engine calls from a non-runtime thread must not panic");
    drop(rt);
    let _ = std::fs::remove_dir_all(dir);
}
