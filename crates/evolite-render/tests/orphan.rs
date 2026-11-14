//! BUG-168: a worker must not outlive the wallet that started it. Three times on 2026-11-14 a
//! `--render-worker` stayed after its parent was gone, holding `evolite.exe` (the next build
//! could not replace it). Here a parent — this test binary run again — starts a worker as the
//! wallet does and is then killed outright; the worker must be gone within seconds.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WORKER: &str = env!("CARGO_BIN_EXE_evolite-render-worker");
/// Set in the parent this test starts: there it starts the worker and waits to be killed.
const PARENT: &str = "EVOLITE_ORPHAN_PARENT";

/// The parent's part (a no-op in a normal run): start a worker, say its id, wait to be killed.
#[test]
fn as_a_parent_start_a_worker_and_wait() {
    if std::env::var_os(PARENT).is_none() {
        return;
    }
    let worker =
        evolite_render::sandbox::spawn(Path::new(WORKER), &[] as &[&str], 256 << 20, false)
            .expect("the worker starts");
    #[cfg(windows)]
    let in_job = worker.in_its_job();
    #[cfg(not(windows))]
    let in_job = true;
    println!("worker {} in-its-job {in_job}", worker.id());
    std::io::stdout().flush().unwrap();
    // Killed by the test long before this ends; `worker` is never dropped here.
    std::thread::sleep(Duration::from_secs(120));
    drop(worker);
}

#[test]
fn a_worker_ends_when_its_parent_is_killed() {
    let mut parent = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "as_a_parent_start_a_worker_and_wait",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PARENT, "1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("the parent starts");
    let mut lines = BufReader::new(parent.stdout.take().unwrap()).lines();
    let (id, in_job) = loop {
        let line = lines
            .next()
            .expect("the parent says the worker's id")
            .unwrap();
        // libtest may print «test … » before it on the same line.
        if let Some((_, rest)) = line.split_once("worker ") {
            let (id, rest) = rest.split_once(' ').unwrap();
            break (id.trim().parse::<u32>().unwrap(), rest.ends_with("true"));
        }
    };
    assert!(alive(id), "the worker {id} is running");
    assert!(
        in_job,
        "the worker {id} is in its own job, not only in one it inherited"
    );
    // As «End task» or a crash: no Drop, no goodbye.
    parent.kill().unwrap();
    parent.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(id) {
        if Instant::now() >= deadline {
            // The test's own orphan goes, or it would hold the executable for the next build.
            end(id);
            panic!("the worker {id} outlived its killed parent by 10 s (BUG-168)");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Whether the process `id` is still running.
#[cfg(windows)]
fn alive(id: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: a handle opened here and closed here, an out pointer for the code.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, id);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
fn alive(id: u32) -> bool {
    Command::new("kill")
        .args(["-0", &id.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The three orphans of 2026-11-14 (1.9 MB each: never run): the wallet ended between
/// creating the worker, suspended, and putting it in its job. Debug builds only (the point
/// where the parent ends is a debug hook).
#[cfg(all(windows, debug_assertions))]
#[test]
fn a_worker_ends_when_its_parent_ends_while_starting_it() {
    let mut parent = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "as_a_parent_start_a_worker_and_wait",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PARENT, "1")
        .env("EVOLITE_RENDER_EXIT_AFTER_CREATE", "1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("the parent starts");
    let id = BufReader::new(parent.stdout.take().unwrap())
        .lines()
        .map_while(Result::ok)
        .find_map(|line| {
            let (_, rest) = line.split_once("worker ")?;
            rest.split_once(' ')?.0.trim().parse::<u32>().ok()
        })
        .expect("the parent says the worker's id before it ends");
    parent.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(id) {
        if Instant::now() >= deadline {
            // The test's own orphan goes, or it would hold the executable for the next build.
            end(id);
            panic!(
                "the worker {id}, created as its parent ended, is still there 10 s later (BUG-168)"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Ends the worker `id` this test caused, once it has been shown to outlive its parent.
#[cfg(windows)]
fn end(id: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    // SAFETY: a handle opened here and closed here.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, id);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

#[cfg(not(windows))]
fn end(id: u32) {
    let _ = Command::new("kill").args(["-9", &id.to_string()]).status();
}
