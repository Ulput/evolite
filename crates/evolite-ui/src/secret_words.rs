//! BUG-228 (option C, the customer 2026-11-16): the words of a recovery phrase, a private key
//! and a password shown stay out of the accessibility tree. UI Automation is an ordinary channel
//! of Windows: screen readers use it, and so do automation tools and AI agents that send what they
//! read to the cloud — that is how a QA phrase leaked. Capture exclusion does not close it.
//!
//! By default no node names, values or describes a word. Each screen of secret words has its
//! own [`Reading`] switch, «Read the words to a screen reader»: off, for one showing only, with a
//! warning, and said on screen while on. Then the words go into the tree one per node.

use egui::{Color32, Context, FontId, Id, Response, Sense, Ui};

use crate::components::Assets;
use crate::theme::Weight;

/// The «read the words» switch of one screen, for one showing of it: any frame the screen is
/// not drawn — left, closed, hidden by its timer, locked, another wallet — turns it off, and it
/// comes back off.
#[derive(Default)]
pub struct Reading {
    on: bool,
    /// The pass the words were last drawn in.
    last_pass: Option<u64>,
}

impl Reading {
    /// Called on each pass the words are drawn: whether they are read now. A pass without the
    /// words between two with them ends the showing.
    /// Whether reading is on: what the last [`Self::showing`] of this pass said.
    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn showing(&mut self, ctx: &Context) -> bool {
        let pass = ctx.cumulative_pass_nr();
        if self.last_pass.is_none_or(|last| pass > last + 1) {
            self.on = false;
        }
        self.last_pass = Some(pass);
        self.on
    }

    /// Off at once: the words left the screen.
    pub fn off(&mut self) {
        self.on = false;
        self.last_pass = None;
    }
}

/// The switch and, while it is on, what it means and that it is on for this showing only.
/// Drawn first on the screen when Windows says a screen reader runs ([`screen_reader_running`]),
/// so it comes first by Tab; it never turns itself on.
pub fn toggle(ui: &mut Ui, a: &mut Assets, reading: &mut Reading) {
    let p = a.p;
    let was = reading.on;
    crate::components::checkbox(
        ui,
        a,
        &mut reading.on,
        tr!("Read the words to a screen reader"),
    );
    if reading.on != was {
        // This pass drew the words as they were: redone in the same frame, so the tree that goes
        // out says what the switch says now (review r1, M2).
        ui.ctx().request_discard("BUG-228: reading changed");
    }
    if reading.on {
        crate::components::notice(
            ui,
            a,
            "info",
            p.danger,
            tr!("Any program on this computer can read the words now, not only a screen reader."),
        );
        ui.label(crate::components::txt(
            tr!("The words are read aloud for this showing only: leaving the screen turns it off."),
            12.0,
            Weight::Medium,
            p.text,
        ));
    }
}

/// Whether Windows says a screen reader is running (`SPI_GETSCREENREADER`), asked at most every
/// two seconds. Elsewhere: no.
pub fn screen_reader_running() -> bool {
    #[cfg(windows)]
    {
        use std::sync::Mutex;
        use std::time::{Duration, Instant};
        static ASKED: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
        let mut asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, running)) = *asked
            && at.elapsed() < Duration::from_secs(2)
        {
            return running;
        }
        let running = windows_says_a_reader_runs();
        *asked = Some((Instant::now(), running));
        running
    }
    #[cfg(not(windows))]
    false
}

#[cfg(windows)]
fn windows_says_a_reader_runs() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETSCREENREADER, SystemParametersInfoW};
    let mut on: i32 = 0;
    // SAFETY: SPI_GETSCREENREADER writes one BOOL to the pointer given.
    let ok = unsafe { SystemParametersInfoW(SPI_GETSCREENREADER, 0, (&raw mut on).cast(), 0) };
    ok != 0 && on != 0
}

/// A text field's words out of the tree: no value, and its text run's letters as `*` (one per
/// letter, so the run still says how long it is). Called right after the field is drawn.
pub fn mute(ctx: &Context, id: Id) {
    ctx.accesskit_node_builder(id, |node| node.clear_value());
    // A one-line field's text is one run: egui's `parent.with(row).with(chunk)`. That is egui's
    // inside, not its API: `the_restore_fields_reach_the_tree_only_when_read` (`none_in_tree`
    // looks at every value) holds it — run it on every egui update.
    let run = id.with(0usize).with(0usize);
    ctx.accesskit_node_builder(run, |node| {
        let letters = node.value().map_or(0, |v| v.chars().count());
        node.set_value("*".repeat(letters));
        node.set_character_lengths(vec![1; letters]);
    });
}

/// `text` painted, with no words of it in the tree: one node named `name` instead (it says what
/// is there, not what it says).
pub fn painted(ui: &mut Ui, text: &str, font: FontId, color: Color32, name: &str) -> Response {
    let galley = ui
        .painter()
        .layout(text.to_owned(), font, color, ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    crate::components::named(&resp, egui::WidgetType::Label, true, name);
    resp
}

/// The words of `text` one per node, while reading is on: each its own label, in a wrapped row,
/// as the line looks.
pub fn word_by_word(ui: &mut Ui, text: &str, font: FontId, color: Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let space = ui
            .painter()
            .layout_no_wrap(" ".into(), font.clone(), color)
            .size()
            .x;
        for word in text.split_whitespace() {
            // Not selectable: Ctrl+C would copy the word to the ordinary clipboard, past
            // `copy_secret` (BUG-228 review r1, M1; SEC-2 B2).
            ui.add(
                egui::Label::new(egui::RichText::new(word).font(font.clone()).color(color))
                    .selectable(false),
            );
            ui.add_space(space);
        }
    });
}

/// Э4 (SEC-7b): the window is in the background or minimized — its words are hidden then, on
/// every screen of a phrase or a key, and come back only by «Show».
pub fn window_left(ctx: &Context) -> bool {
    !ctx.input(|i| i.focused)
}
