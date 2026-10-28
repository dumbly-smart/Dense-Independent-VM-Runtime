use densevm::qmp::Qmp;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime};

#[test]
fn validates_linux_unix_socket_path_length() {
    assert!(
        densevm::qmp::validate_unix_socket_path(Path::new(&format!("/{}", "s".repeat(106))))
            .is_ok()
    );
    let error =
        densevm::qmp::validate_unix_socket_path(Path::new(&format!("/{}", "s".repeat(107))))
            .unwrap_err();
    assert!(error.contains("108 bytes"), "{error}");
}

#[test]
fn qmp_handshake_and_event_do_not_confuse_command_reply() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("densevm-qmp-{}-{stamp}.sock", std::process::id()));
    let server = UnixListener::bind(&path).unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream.write_all(b"{\"QMP\":{\"version\":{}}}\n").unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let first: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(first["execute"], "qmp_capabilities");
        stream.write_all(b"{\"return\":{}}\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let second: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(second["execute"], "query-status");
        stream
            .write_all(b"{\"event\":\"STOP\"}\n{\"return\":{\"status\":\"paused\"}}\n")
            .unwrap();
    });
    let mut qmp = Qmp::connect(&path, Duration::from_secs(2)).unwrap();
    assert_eq!(
        qmp.execute("query-status", None).unwrap(),
        json!({"status":"paused"})
    );
    handle.join().unwrap();
    std::fs::remove_file(path).unwrap();
}
