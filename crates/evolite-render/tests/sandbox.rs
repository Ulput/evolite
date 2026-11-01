//! What a compromised decoder inside the sandbox can reach: nothing but its pipes. The same
//! attempts run first without the sandbox, to show they would have worked.

use std::io::Read;
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

const WORKER: &str = env!("CARGO_BIN_EXE_evolite-render-worker");

/// A user's file, a listening port and a writable folder, all in the user's temp dir.
fn targets(name: &str) -> (std::path::PathBuf, TcpListener, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("evolite-escape-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("wallet-secret.txt");
    std::fs::write(&file, b"secret").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    (file, listener, dir)
}

/// `mode` and where to try: the file, the port, the folder.
fn args(mode: &str, file: &Path, listener: &TcpListener, dir: &Path) -> Vec<String> {
    vec![
        mode.into(),
        file.display().to_string(),
        listener.local_addr().unwrap().port().to_string(),
        dir.display().to_string(),
    ]
}

#[test]
fn outside_the_sandbox_the_same_attempts_work() {
    let (file, listener, dir) = targets("plain");
    let out = Command::new(WORKER)
        .args(args("--test-escape", &file, &listener, &dir))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "read=ok net=ok write=ok exec=ok"
    );
}

/// Started as the wallet starts the worker, and closed as the worker closes itself.
#[test]
fn inside_the_sandbox_nothing_gets_through() {
    let (file, listener, dir) = targets("sandboxed");
    let worker = evolite_render::sandbox::spawn(
        Path::new(WORKER),
        &args("--test-escape-locked", &file, &listener, &dir),
        256 << 20,
        false,
    )
    .unwrap();
    let mut out = String::new();
    (&worker.stdout).read_to_string(&mut out).unwrap();
    assert_eq!(
        out.trim(),
        "read=denied net=denied write=denied exec=denied"
    );
    assert!(!dir.join("escaped.txt").exists());
}
