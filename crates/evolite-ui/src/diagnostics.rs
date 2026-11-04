//! The window's part of the journal ([`evolite_core::diag`]): it starts it, writes down what
//! the app is and where it runs, panics with their stack, screens and long frames.

use evolite_core::diag::{self, Build, Record};
use std::path::Path;

/// The commit this build is from (`build.rs`).
pub const COMMIT: &str = env!("EVOLITE_COMMIT");

pub fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Starts the journal in `dir` and catches panics into it (the default report still follows).
pub fn start(dir: &Path) {
    diag::init(dir);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        diag::record_now(Record::Panic {
            location: info.location().map(|l| l.to_string()).unwrap_or_default(),
            message,
            backtrace: std::backtrace::Backtrace::force_capture().to_string(),
        });
        previous(info);
    }));
}

/// The system's name and version, e.g. ("windows", "10.0.26200").
pub fn os() -> (&'static str, String) {
    (std::env::consts::OS, os_version())
}

#[cfg(windows)]
fn os_version() -> String {
    // RtlGetVersion tells the truth; GetVersionEx answers 6.2 to an app without a manifest.
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }
    let mut info = OsVersionInfo {
        size: size_of::<OsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // SAFETY: a structure of the documented layout with its size set, written by the call.
    if unsafe { RtlGetVersion(&mut info) } == 0 {
        format!("{}.{}.{}", info.major, info.minor, info.build)
    } else {
        String::new()
    }
}

#[cfg(target_os = "macos")]
fn os_version() -> String {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn os_version() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// The start record.
pub fn build(
    language: &'static str,
    dark: bool,
    scale: f32,
    network: evolite_api::Network,
    own_node: bool,
    demo: bool,
) -> Build {
    let (os, os_version) = os();
    Build {
        version: env!("CARGO_PKG_VERSION"),
        commit: COMMIT,
        profile: profile(),
        os,
        os_version,
        language,
        theme: if dark { "dark" } else { "light" },
        scale,
        network: match network {
            evolite_api::Network::Testnet => "testnet",
            evolite_api::Network::Mainnet => "mainnet",
        },
        own_node,
        demo,
    }
}

/// A screen's name for the journal.
pub fn screen_name(screen: crate::screens::Screen) -> &'static str {
    use crate::screens::Screen::*;
    match screen {
        Welcome => "welcome",
        Unlock => "unlock",
        Overview => "overview",
        Send => "send",
        Receive => "receive",
        Activity => "activity",
        Identity => "identity",
        Contacts => "contacts",
        Shielded => "shielded",
        Apps => "apps",
        Collection => "collection",
        Settings => "settings",
        CreateIdentity => "create_identity",
        Onboarding(_) => "onboarding",
        #[cfg(feature = "dev-tools")]
        Probe => "probe",
    }
}

/// Screens and long frames, as the window goes.
#[derive(Default)]
pub struct FrameWatch {
    screen: Option<&'static str>,
    last_slow: Option<std::time::Instant>,
}

impl FrameWatch {
    pub fn screen(&mut self, name: &'static str) {
        if self.screen != Some(name) {
            self.screen = Some(name);
            diag::record(Record::Screen { name });
        }
    }

    /// A frame of `took`: noted past 250 ms, once in 10 s at most.
    pub fn frame(&mut self, took: std::time::Duration) {
        let slow = took >= std::time::Duration::from_millis(250);
        let quiet = self
            .last_slow
            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(10));
        if slow && quiet {
            self.last_slow = Some(std::time::Instant::now());
            diag::record(Record::SlowFrame {
                ms: took.as_millis() as u64,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_the_system_and_the_build() {
        let (os, version) = os();
        assert!(!os.is_empty());
        if cfg!(windows) {
            assert!(version.starts_with("10."), "{version}");
        }
        assert!(!COMMIT.is_empty());
    }
}
