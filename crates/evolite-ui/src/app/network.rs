//! FEAT-0037: testnet and mainnet. The network is fixed for the process
//! (`evolite_core::network`, a `OnceLock`, read by the SDK, the provider, the seeds and the
//! address checks), so a switch starts evolite again on the other one (the architect,
//! 2026-11-14): on the sign-in screens at one click, nothing being open; in Settings after a
//! question, the wallet locked first as by «Lock». The old window closes once the new one holds
//! its network's folder, or after 5 s.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use evolite_api::{Network, VaultState};

use super::App;
use crate::components::{Part, button, button_row_destructive, dialog_with_footer, notice};
use crate::network_choice;
use crate::theme::Weight;

/// How long the old window waits for the new one to hold its folder.
const HANDOVER: Duration = Duration::from_secs(5);
/// How long the switch waits for the wallet to lock (the demo never does).
const LOCK_WAIT: Duration = Duration::from_secs(3);

/// The networks of this run: the other one's folder and its wallets there.
#[derive(Default)]
pub(crate) struct Networks {
    /// This network's data folder; `None` in the demo.
    pub own_dir: Option<PathBuf>,
    /// The other network's data folder (the same one with `--data-dir`); `None` in the demo.
    pub other_dir: Option<PathBuf>,
    /// The other network's wallets in its folder, for the switch.
    pub other_wallets: usize,
    /// The app's common folder: `last-network` is written there; `None` in the demo.
    pub app_dir: Option<PathBuf>,
    /// This run was started by a switch.
    pub switched: bool,
}

/// A switch under way.
pub(crate) struct Switching {
    to: Network,
    /// The window's place and size, for the new one.
    window: Option<[f32; 4]>,
    phase: Phase,
    /// Whether the new run has ended already (review r1, Should 1): set once it is started.
    ended: Option<Ended>,
}

/// Asks whether a started run has ended (the process, or a test's stand-in).
pub(crate) type Ended = Box<dyn FnMut() -> bool>;

enum Phase {
    /// The wallet locks first (Settings), since then: at most [`LOCK_WAIT`], as this window
    /// ends with its keys anyway.
    Locking(Instant),
    /// Nothing open (the sign-in screens): straight on.
    Ready,
    /// The new run started then: the old window waits for it to hold its folder.
    Started(Instant),
}

/// The other network.
pub(crate) fn other(network: Network) -> Network {
    match network {
        Network::Testnet => Network::Mainnet,
        Network::Mainnet => Network::Testnet,
    }
}

/// A network's name: a name, the same in every language (the designer's frames, AC-A8).
pub(crate) fn name(network: Network) -> &'static str {
    match network {
        Network::Testnet => "Testnet",
        Network::Mainnet => "Mainnet",
    }
}

impl App {
    /// Starts a switch to `to`. Refused, and said, when another window holds that network's
    /// folder (AC-T6е): nothing here is locked or touched then.
    pub(crate) fn switch_network(&mut self, ctx: &egui::Context, to: Network, unlocked: bool) {
        if self.net_switch.is_some() {
            return;
        }
        // The demo has no keys to lock: straight on.
        let unlocked = unlocked && self.networks.own_dir.is_some();
        if let (Some(own), Some(target)) = (&self.networks.own_dir, &self.networks.other_dir)
            && own != target
            && network_choice::held(target)
        {
            self.switch_held = Some(to);
            return;
        }
        let window = ctx.input(|i| {
            let v = i.viewport();
            let at = v.outer_rect?.min;
            let size = v.inner_rect?.size();
            Some([at.x, at.y, size.x, size.y])
        });
        self.net_switch = Some(Switching {
            to,
            window,
            ended: None,
            phase: if unlocked {
                Phase::Locking(Instant::now())
            } else {
                Phase::Ready
            },
        });
        if unlocked {
            // As «Lock» does: the keys go, the forms with what was typed are wiped as the
            // snapshot says the vault is closed, the clipboard when this window ends.
            self.core.send(evolite_api::Command::Lock);
        }
        ctx.request_repaint();
    }

    /// Each frame: the lock done, the last network is kept and evolite starts again on it; then
    /// this window closes once the new one holds its folder (or after [`HANDOVER`]).
    pub(crate) fn tick_switch(&mut self, ctx: &egui::Context, vault: VaultState) {
        let Some(switching) = &self.net_switch else {
            return;
        };
        let (to, window) = (switching.to, switching.window);
        match switching.phase {
            Phase::Locking(since)
                if vault == VaultState::Unlocked && since.elapsed() < LOCK_WAIT => {}
            Phase::Locking(_) | Phase::Ready => {
                let args: Vec<OsString> = std::env::args_os().skip(1).collect();
                let again = network_choice::restart_args(&args, to, window);
                match self.start_again_with(again) {
                    Ok(ended) => {
                        // Kept once the new run is under way (review r1, P3 2): a run that did
                        // not start leaves the last network as it was.
                        self.keep_last(to);
                        if let Some(s) = &mut self.net_switch {
                            s.phase = Phase::Started(Instant::now());
                            s.ended = Some(ended);
                        }
                    }
                    Err(e) => {
                        eprintln!("evolite did not start again: {e}");
                        self.net_switch = None;
                        self.switch_failed = Some(to);
                        return;
                    }
                }
            }
            Phase::Started(at) => {
                // The new run ended before it took its folder (it fell at its start): this
                // window stays, on its network, and says so (review r1, Should 1).
                let ended = self
                    .net_switch
                    .as_mut()
                    .and_then(|s| s.ended.as_mut())
                    .is_some_and(|ended| ended());
                if ended {
                    self.keep_last(other(to));
                    self.net_switch = None;
                    self.switch_failed = Some(to);
                    return;
                }
                let handed = match (&self.networks.own_dir, &self.networks.other_dir) {
                    // One folder for both (`--data-dir`): the new run waits for this one to
                    // let it go.
                    (Some(own), Some(target)) if own == target => true,
                    (_, Some(target)) => network_choice::held(target),
                    _ => true,
                };
                if handed || at.elapsed() >= HANDOVER {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// Keeps `network` as the last chosen, for the next start.
    fn keep_last(&self, network: Network) {
        if let Some(dir) = &self.networks.app_dir
            && let Err(e) = network_choice::write_last(dir, network)
        {
            eprintln!("the network chosen was not kept: {e}");
        }
    }

    /// Starts this program again with `args`, and a way to ask whether that run has ended; a
    /// test keeps the command instead, its run ending when the test says (AC-T6б, review r1).
    fn start_again_with(&mut self, args: Vec<OsString>) -> std::io::Result<Ended> {
        #[cfg(test)]
        {
            self.restarted = Some(args);
            let ends = self.restart_ends.clone();
            Ok(Box::new(move || {
                ends.load(std::sync::atomic::Ordering::Relaxed)
            }))
        }
        #[cfg(not(test))]
        {
            crate::relaunch::start(args)
        }
    }

    /// The switch on the sign-in screens, at the top right (L1): the other network at one click.
    pub(crate) fn sign_in_switch(&mut self, ctx: &egui::Context, here: Network, wallets: usize) {
        let counts = match here {
            Network::Testnet => [wallets, self.networks.other_wallets],
            Network::Mainnet => [self.networks.other_wallets, wallets],
        };
        let top = if here == Network::Mainnet {
            band_height(ctx) + 20.0
        } else {
            20.0
        };
        let chosen = egui::Area::new(egui::Id::new("network-switch"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-24.0, top))
            .show(ctx, |ui| {
                crate::components::network_switch(ui, &mut self.a, here, counts)
            })
            .inner;
        if let Some(to) = chosen {
            self.switch_network(ctx, to, false);
        }
    }

    /// Settings → Network asked for the other network: «Switch to …?» (N2d, N2t), «Cancel»
    /// first; the answer, the switch.
    pub(crate) fn switch_dialog(&mut self, ui: &mut egui::Ui, wallet: Option<&str>) {
        let Some(to) = self.settings.switch_to else {
            return;
        };
        let p = self.a.p;
        let (title, words, go) = match to {
            Network::Mainnet => (
                tr!("Switch to mainnet?"),
                tr!(
                    "evolite signs out of the wallet “{}” and opens the mainnet sign-in. Your testnet wallets stay on this computer — switch back on the sign-in screen."
                ),
                tr!("Sign out and go to mainnet"),
            ),
            Network::Testnet => (
                tr!("Switch to testnet?"),
                tr!(
                    "evolite signs out of the wallet “{}” and opens the testnet sign-in. Your mainnet wallets stay on this computer — switch back on the sign-in screen."
                ),
                tr!("Sign out and go to testnet"),
            ),
        };
        let name = crate::i18n::own(wallet.unwrap_or_default());
        let mut answer = None;
        let closed = dialog_with_footer(
            ui,
            &mut self.a,
            "network-switch",
            match to {
                Network::Testnet => "testnet",
                Network::Mainnet => "mainnet",
            },
            title,
            440.0,
            |ui, a, part| {
                if part == Part::Footer {
                    let mut close = false;
                    // Signing out cannot be undone here: «Cancel» first (the designer, N2d).
                    button_row_destructive(ui, |ui| {
                        if button(
                            ui,
                            a,
                            tr!("Cancel"),
                            crate::components::ButtonKind::Ghost,
                            None,
                        )
                        .clicked()
                        {
                            close = true;
                        }
                        if button(ui, a, go, crate::components::ButtonKind::Primary, None).clicked()
                        {
                            answer = Some(to);
                        }
                    });
                    return close;
                }
                ui.spacing_mut().item_spacing.y = 16.0;
                ui.add(crate::components::para(
                    &fill(words, &[&name]),
                    13.0,
                    Weight::Regular,
                    p.text,
                ));
                if to == Network::Mainnet {
                    notice(
                        ui,
                        a,
                        "warning",
                        p.accent,
                        tr!(
                            "Mainnet means real DASH. This is a beta: keep only small amounts here."
                        ),
                    );
                }
                false
            },
        );
        if closed {
            self.settings.switch_to = None;
        }
        if let Some(to) = answer {
            self.settings.switch_to = None;
            self.switch_network(ui.ctx(), to, true);
        }
    }

    /// Another window holds the network asked for (AC-A6, AC-T6е): said, and nothing changed.
    pub(crate) fn held_dialog(&mut self, ui: &mut egui::Ui, here: Network) {
        let Some(to) = self.switch_held else {
            return;
        };
        let p = self.a.p;
        // Lowercase inside a sentence and on a button (the analyst, FEAT-0037).
        let words = format!(
            tr!(
                "{} is already open in another evolite window. Switch to that window or go back to {}."
            ),
            name(to),
            name(here).to_lowercase(),
        );
        let back = format!(tr!("Back to {}"), name(here).to_lowercase());
        let closed = dialog_with_footer(
            ui,
            &mut self.a,
            "network-held",
            "warning",
            name(to),
            420.0,
            |ui, a, part| {
                if part == Part::Footer {
                    return button(ui, a, &back, crate::components::ButtonKind::Primary, None)
                        .clicked();
                }
                ui.add(crate::components::para(
                    &words,
                    13.0,
                    Weight::Regular,
                    p.text,
                ));
                false
            },
        );
        if closed {
            self.switch_held = None;
        }
    }

    /// The switch to `to` did not start evolite there, or that run ended at its start (review r1,
    /// Should 1): this window stays on `here`, and says so.
    pub(crate) fn failed_dialog(&mut self, ui: &mut egui::Ui, here: Network) {
        let Some(to) = self.switch_failed else {
            return;
        };
        let p = self.a.p;
        let words = format!(
            tr!(
                "evolite did not start on {}. You are still on {}. Try switching again; if it fails again, close evolite and open it again."
            ),
            name(to).to_lowercase(),
            name(here).to_lowercase(),
        );
        let title = format!(tr!("Could not switch to {}"), name(to).to_lowercase());
        let closed = dialog_with_footer(
            ui,
            &mut self.a,
            "network-failed",
            "warning",
            &title,
            420.0,
            |ui, a, part| {
                if part == Part::Footer {
                    return button(
                        ui,
                        a,
                        tr!("Got it"),
                        crate::components::ButtonKind::Primary,
                        None,
                    )
                    .clicked();
                }
                ui.add(crate::components::para(
                    &words,
                    13.0,
                    Weight::Regular,
                    p.text,
                ));
                false
            },
        );
        if closed {
            self.switch_failed = None;
        }
    }

    /// While evolite starts again on the other network: «Connecting to the wallet core…» over
    /// this window, as the new one shows it first.
    pub(crate) fn switching_screen(&mut self, ctx: &egui::Context) -> bool {
        if self.net_switch.is_none() {
            return false;
        }
        let p = self.a.p;
        egui::Area::new(egui::Id::new("network-switching"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::Pos2::ZERO)
            .show(ctx, |ui| {
                let rect = ctx.content_rect();
                ui.painter().rect_filled(rect, 0.0, p.bg);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    crate::i18n::t(tr!("Connecting to the wallet core…")),
                    crate::theme::sans(15.0, Weight::Medium),
                    p.muted,
                );
            });
        true
    }
}

/// `template` (English, the catalog's key) with its `{}` filled in turn; the component that draws
/// it translates it, as `format!(tr!(…))` would.
fn fill(template: &str, values: &[&str]) -> String {
    let mut out = template.to_owned();
    for v in values {
        out = out.replacen("{}", v, 1);
    }
    out
}

/// The mainnet band's height (N3): 34 from a 640 px high window, 28 below.
pub(crate) fn band_height(ctx: &egui::Context) -> f32 {
    if ctx.content_rect().height() >= 640.0 {
        34.0
    } else {
        28.0
    }
}

/// N3: the band over every mainnet screen, the sign-in too — «BETA» in a frame and the words,
/// in a group in the middle; `accent` with the page's colour for the words (the designer;
/// designer-52 is with the customer). 34 high from a 640 high window, 28 below; the short
/// words below 900 wide. Not closed. The reader hears the words once, a status.
pub(crate) fn mainnet_band(ui: &mut egui::Ui, a: &mut crate::components::Assets) {
    let p = a.p;
    let ctx = ui.ctx().clone();
    let height = band_height(&ctx);
    let short = ctx.content_rect().width() < 900.0;
    let words = if short {
        tr!("Mainnet: real DASH. Keep only small amounts here.")
    } else {
        tr!(
            "Mainnet: real DASH. Keep only small amounts here and check the address before every payment."
        )
    };
    let size = if height >= 34.0 { 13.0 } else { 12.0 };
    egui::Panel::top("mainnet")
        .exact_size(height)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(p.accent))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let ink = p.bg;
            let label = crate::i18n::t(tr!("BETA"));
            let label_font = crate::theme::sans(11.0, Weight::Bold);
            let words_font = crate::theme::sans(size, Weight::SemiBold);
            let shown = crate::i18n::t(words);
            let label_w = crate::components::text_width(ui, &label, label_font.clone()) + 14.0;
            let words_w = crate::components::text_width(ui, &shown, words_font.clone());
            let left = rect.center().x - (label_w + 10.0 + words_w) / 2.0;
            let tag = egui::Rect::from_min_size(
                egui::pos2(left, rect.center().y - 10.0),
                egui::vec2(label_w, 20.0),
            );
            ui.painter().rect_stroke(
                tag,
                egui::CornerRadius::same(5),
                egui::Stroke::new(1.25, ink),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                tag.center(),
                egui::Align2::CENTER_CENTER,
                label,
                label_font,
                ink,
            );
            ui.painter().text(
                egui::pos2(tag.right() + 10.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                shown,
                words_font,
                ink,
            );
            let resp = ui.interact(rect, egui::Id::new("mainnet-band"), egui::Sense::hover());
            let spoken = crate::i18n::spoken(words);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
            ui.ctx().accesskit_node_builder(resp.id, |node| {
                node.set_role(egui::accesskit::Role::Status);
                node.set_label(spoken.clone());
            });
        });
}
