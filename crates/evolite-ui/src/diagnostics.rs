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
    /// When the screen shown was opened, a wheel turned since, and the scroll causes told.
    opened: Option<std::time::Instant>,
    wheel_here: bool,
    told: Vec<&'static str>,
}

/// How long after a screen opens its scroll input goes to the journal (BUG-71).
const SCROLL_WATCH: std::time::Duration = std::time::Duration::from_secs(1);

/// The kinds of input in a frame that can scroll a screen (BUG-71).
#[derive(Clone, Copy, Debug, Default)]
pub struct ScrollInput {
    /// A wheel event in this frame.
    pub raw_wheel: bool,
    /// Scroll egui hands out this frame, from the wheel's smoothing.
    pub smooth_wheel: bool,
    pub drag: bool,
    pub accesskit_scroll: bool,
    pub accesskit_focus: bool,
    /// Tab, an arrow, Page Up or Down, Home, End or Space.
    pub key: bool,
    /// Text typed, pasted or deleted: a focused field then scrolls to its cursor.
    pub text: bool,
    /// Our code asked for a scroll (`scroll_to_me`, `scroll_to_rect`, [`ask_scroll`]). Set by
    /// [`Self::of_frame`].
    pub scroll_asked: bool,
    /// A text field with focus took text, and so scrolls to its cursor. Set by
    /// [`Self::of_frame`].
    pub typing: bool,
}

/// Where [`ask_scroll`] marks this frame.
fn asked_id() -> egui::Id {
    egui::Id::new("diag-scroll-asked")
}

/// BUG-71: our code is about to ask egui to scroll to something; the journal hears of it while a
/// screen is new.
pub fn ask_scroll(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(asked_id(), true));
}

impl ScrollInput {
    /// This frame's input, with the scrolls our code or a focused field asked for since the last.
    pub fn of_frame(ctx: &egui::Context) -> Self {
        let mut seen = ctx.input(Self::of);
        seen.scroll_asked = ctx.data_mut(|d| d.remove_temp::<bool>(asked_id()).unwrap_or(false));
        seen.typing = seen.text && ctx.memory(|m| m.focused().is_some());
        seen
    }

    pub fn of(i: &egui::InputState) -> Self {
        use egui::accesskit::Action;
        let mut seen = Self {
            smooth_wheel: i.smooth_scroll_delta != egui::Vec2::ZERO,
            drag: i.pointer.is_decidedly_dragging(),
            ..Default::default()
        };
        for event in &i.events {
            match event {
                egui::Event::MouseWheel { .. } => seen.raw_wheel = true,
                egui::Event::AccessKitActionRequest(request) => match request.action {
                    Action::Focus => seen.accesskit_focus = true,
                    Action::ScrollIntoView
                    | Action::ScrollDown
                    | Action::ScrollUp
                    | Action::ScrollLeft
                    | Action::ScrollRight
                    | Action::ScrollToPoint
                    | Action::SetScrollOffset => seen.accesskit_scroll = true,
                    _ => {}
                },
                egui::Event::Key {
                    key, pressed: true, ..
                } => {
                    use egui::Key::*;
                    if matches!(
                        key,
                        Tab | ArrowUp | ArrowDown | PageUp | PageDown | Home | End | Space
                    ) {
                        seen.key = true;
                    }
                    if matches!(key, Backspace | Delete) {
                        seen.text = true;
                    }
                }
                egui::Event::Text(_) | egui::Event::Paste(_) => seen.text = true,
                _ => {}
            }
        }
        seen
    }
}

impl FrameWatch {
    pub fn screen(&mut self, name: &'static str) {
        self.screen_at(name, std::time::Instant::now());
    }

    fn screen_at(&mut self, name: &'static str, now: std::time::Instant) {
        if self.screen != Some(name) {
            self.screen = Some(name);
            self.opened = Some(now);
            self.wheel_here = false;
            self.told.clear();
            diag::record(Record::Screen { name });
        }
    }

    /// BUG-71: the input of this frame that can scroll the screen, to the journal while it is
    /// new, each cause once.
    pub fn scroll_input(&mut self, seen: ScrollInput) {
        let Some(screen) = self.screen else {
            return;
        };
        for cause in self.new_causes(seen, std::time::Instant::now()) {
            diag::record(Record::ScrollMoved { screen, cause });
        }
    }

    /// The causes in `seen` not told yet for this screen, while it is in its first second.
    fn new_causes(&mut self, seen: ScrollInput, now: std::time::Instant) -> Vec<&'static str> {
        if self
            .opened
            .is_none_or(|at| now.duration_since(at) >= SCROLL_WATCH)
        {
            return Vec::new();
        }
        if seen.raw_wheel {
            self.wheel_here = true;
        }
        let causes = [
            (seen.raw_wheel, "wheel"),
            // Smoothing of a turn that began before the screen opened.
            (seen.smooth_wheel && !self.wheel_here, "wheel_before"),
            (seen.drag, "drag"),
            (seen.accesskit_scroll, "accesskit_scroll"),
            (seen.accesskit_focus, "accesskit_focus"),
            (seen.key, "key"),
            (seen.scroll_asked, "scroll_asked"),
            (seen.typing, "typing"),
        ];
        let mut new = Vec::new();
        for (on, cause) in causes {
            if on && !self.told.contains(&cause) {
                self.told.push(cause);
                new.push(cause);
            }
        }
        new
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

    /// BUG-71: in a screen's first second each kind of scroll input is told once; a turn of the
    /// wheel from before it opened is told apart from one on it; after that second, nothing.
    #[test]
    fn scroll_input_is_told_once_each_in_a_screens_first_second() {
        let t0 = std::time::Instant::now();
        let at = |ms| t0 + std::time::Duration::from_millis(ms);
        let mut watch = FrameWatch::default();
        watch.screen_at("activity", t0);
        let smooth = ScrollInput {
            smooth_wheel: true,
            ..Default::default()
        };
        assert_eq!(watch.new_causes(smooth, at(50)), ["wheel_before"]);
        assert!(watch.new_causes(smooth, at(80)).is_empty(), "once");
        let turned = ScrollInput {
            raw_wheel: true,
            smooth_wheel: true,
            key: true,
            ..Default::default()
        };
        assert_eq!(watch.new_causes(turned, at(100)), ["wheel", "key"]);
        // Review r1 P3: our scroll requests and typing are told apart.
        let asked = ScrollInput {
            scroll_asked: true,
            typing: true,
            ..Default::default()
        };
        assert_eq!(watch.new_causes(asked, at(120)), ["scroll_asked", "typing"]);
        let late = ScrollInput {
            drag: true,
            ..Default::default()
        };
        assert!(
            watch.new_causes(late, at(1_200)).is_empty(),
            "after a second"
        );
        watch.screen_at("identity", at(1_300));
        let turned_here = ScrollInput {
            raw_wheel: true,
            smooth_wheel: true,
            ..Default::default()
        };
        assert_eq!(
            watch.new_causes(turned_here, at(1_350)),
            ["wheel"],
            "a turn on this screen is no turn from before"
        );
        assert_eq!(watch.new_causes(smooth, at(1_400)), Vec::<&str>::new());
    }

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
