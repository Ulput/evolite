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
    let expected = "read=ok net=ok write=ok exec=ok chmod=ok truncate=ok";
    // `clone3=other`: it made a child, where inside the sandbox it must answer ENOSYS.
    #[cfg(target_os = "linux")]
    let expected = format!(
        "{expected} kill=ok tkill=ok tgkill=ok owner=ok limit=ok fork=ok clone3=other \
         thread=ok own_signal=ok"
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected);
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
    println!("{}", out.trim());
    // macOS promises nothing about memory made executable: the profile has no rule of its own
    // for it, so what the system does is printed above and not checked.
    #[cfg(target_os = "macos")]
    let out = out.replace(" exec=ok", " exec=denied");
    let expected = "read=denied net=denied write=denied exec=denied chmod=denied truncate=denied";
    // On Linux also: no reaching other processes and no new ones, while the worker keeps its
    // own threads and its own abort (BUG-96).
    #[cfg(target_os = "linux")]
    let expected = format!(
        "{expected} kill=denied tkill=denied tgkill=denied owner=denied limit=denied \
         fork=denied clone3=enosys thread=ok own_signal=ok"
    );
    assert_eq!(out.trim(), expected);
    assert!(!dir.join("escaped.txt").exists());
}

/// `abort()` is how a worker ends over the memory cap. It raises SIGABRT by `tgkill` on itself;
/// if the filter stopped that, glibc would fall through to a bad instruction, a different death.
#[cfg(target_os = "linux")]
#[test]
fn an_abort_inside_the_sandbox_is_an_abort() {
    let worker = evolite_render::sandbox::spawn(
        Path::new(WORKER),
        &["--test-abort-locked"],
        256 << 20,
        false,
    )
    .unwrap();
    let mut out = String::new();
    (&worker.stdout).read_to_string(&mut out).unwrap();
    assert!(worker.wait());
    assert_eq!(worker.exit_code(), Some(134));
}
