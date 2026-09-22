use densevm::ksm::KsmSnapshot;

#[test]
fn snapshot_serializes_the_run_state() {
    let snapshot = KsmSnapshot {
        run: 0,
        pages_to_scan: 100,
        sleep_millisecs: 20,
    };
    assert_eq!(snapshot.run, 0);
    assert_eq!(snapshot.pages_to_scan, 100);
    assert_eq!(snapshot.sleep_millisecs, 20);
}
