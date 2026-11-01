//! A stand-alone worker, for the crate's own tests (the app runs the worker from its own
//! executable with `--render-worker`). The `--test-*` modes misbehave on purpose, so the tests
//! can check that the client survives them.

use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_default();
    // As the real worker: a crash (the memory cap) ends the process at once.
    evolite_render::worker::quiet_crashes();
    if mode.is_empty() {
        evolite_render::worker::run();
    }
    // Tries what a compromised decoder would: read a user's file, reach the network, write a
    // file, make memory executable. Prints what got through, e.g. `read=denied net=denied …`.
    // `--test-escape-locked` first closes itself as the real worker does.
    if mode == "--test-escape" || mode == "--test-escape-locked" {
        let args: Vec<String> = std::env::args().skip(2).collect();
        let [file, port, dir] = args.as_slice() else {
            return ExitCode::FAILURE;
        };
        if mode == "--test-escape-locked"
            && let Err(e) = evolite_render::worker::lock_down()
        {
            println!("lockdown failed: {e}");
            return ExitCode::FAILURE;
        }
        let read = std::fs::read(file).is_ok();
        let net = port
            .parse::<u16>()
            .ok()
            .and_then(|p| {
                std::net::TcpStream::connect_timeout(
                    &(std::net::Ipv4Addr::LOCALHOST, p).into(),
                    std::time::Duration::from_secs(2),
                )
                .ok()
            })
            .is_some();
        let write = std::fs::write(std::path::Path::new(dir).join("escaped.txt"), b"x").is_ok();
        let word = |ok: bool| if ok { "ok" } else { "denied" };
        println!(
            "read={} net={} write={} exec={}",
            word(read),
            word(net),
            word(write),
            word(executable_memory())
        );
        return ExitCode::SUCCESS;
    }
    // Every test mode waits for a request first, like the real worker.
    let mut request = [0u8; 16];
    if std::io::stdin().read_exact(&mut request).is_err() {
        return ExitCode::FAILURE;
    }
    match mode.as_str() {
        "--test-hang" => loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        },
        "--test-crash" => ExitCode::from(3),
        // Over the Job Object's memory cap: Windows ends the process. Without the cap it would
        // hang here, and the client would see a timeout instead of a crash.
        "--test-allocate" => {
            let mut hog: Vec<Vec<u8>> = Vec::new();
            for _ in 0..16 {
                hog.push(vec![1; 64 * 1024 * 1024]);
            }
            std::hint::black_box(&hog);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        // An answer claiming frames far over the limits.
        "--test-lie" => {
            let id = u64::from_le_bytes(request[4..12].try_into().unwrap());
            let mut out = std::io::stdout();
            let mut answer = b"EVR1".to_vec();
            answer.extend(id.to_le_bytes());
            answer.push(0);
            for _ in 0..3 {
                answer.extend(u32::MAX.to_le_bytes());
            }
            let _ = out.write_all(&answer);
            let _ = out.flush();
            std::thread::sleep(std::time::Duration::from_secs(60));
            ExitCode::SUCCESS
        }
        _ => ExitCode::FAILURE,
    }
}

/// Whether this process may make memory executable — what shellcode needs. The sandbox's
/// "no dynamic code" mitigation refuses it.
#[cfg(windows)]
fn executable_memory() -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
    };
    // SAFETY: a fresh page is asked for and released at once; nothing is written or run.
    unsafe {
        let page = VirtualAlloc(
            std::ptr::null(),
            4096,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_EXECUTE_READWRITE,
        );
        if page.is_null() {
            return false;
        }
        VirtualFree(page, 0, MEM_RELEASE);
        true
    }
}

#[cfg(unix)]
fn executable_memory() -> bool {
    // SAFETY: a fresh anonymous page is asked for and unmapped at once; nothing is run.
    unsafe {
        let page = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        if page == libc::MAP_FAILED {
            return false;
        }
        libc::munmap(page, 4096);
        true
    }
}
