//! "Report a problem": a few fields, the letter shown
//! whole before anything leaves, then the person's own mail program with it filled in
//! (`mailto:`, no server, no password) or a folder with the report and the journal to attach.
//! Nothing private goes in but what the person types — and a phrase is refused even then.

use super::txt;
use crate::components::{
    Assets, ButtonKind, FieldKind, button, button_enabled, checkbox, dialog, field_label, notice,
    segmented, text_area, text_field,
};
use crate::context::Ctx;
use crate::theme::{Weight, mono};
use egui::Ui;
use std::path::{Path, PathBuf};

/// Where reports go.
pub const TO: &str = "ulput@protonmail.com";
/// A `mailto:` link longer than this may not open everywhere: the body is cut to fit.
const MAILTO_MAX: usize = 1900;
/// This many words of a phrase in a row in what was typed: refused.
const PHRASE_WORDS: usize = 6;

const OFTEN: [&str; 3] = [tr!("Once"), tr!("Sometimes"), tr!("Every time")];

pub struct ReportForm {
    pub summary: String,
    pub steps: String,
    pub expected: String,
    pub actual: String,
    /// By [`OFTEN`].
    pub often: usize,
    pub contact: String,
    /// "Attach system details".
    pub system: bool,
    /// What the last "Open in email" or "Save report to folder" did.
    pub done: Option<Result<String, String>>,
    /// The second step: the letter whole, then where it goes.
    pub letter: bool,
}

impl Default for ReportForm {
    fn default() -> Self {
        Self {
            summary: String::new(),
            steps: String::new(),
            expected: String::new(),
            actual: String::new(),
            often: 0,
            contact: String::new(),
            system: true,
            done: None,
            letter: false,
        }
    }
}

impl ReportForm {
    /// Whether something typed holds a piece of a recovery phrase.
    pub fn holds_phrase(&self) -> bool {
        [
            &self.summary,
            &self.steps,
            &self.expected,
            &self.actual,
            &self.contact,
        ]
        .iter()
        .any(|text| evolite_core::diag::phrase_words(text) >= PHRASE_WORDS)
    }
}

/// What the letter says of the system, label and value.
pub type System = Vec<(&'static str, String)>;

/// The system part of a report, from what the window knows now.
pub fn system(cx: &Ctx, dark: bool, scale: f32) -> System {
    let snap = cx.snap;
    let s = &snap.sync;
    let (os, os_version) = crate::diagnostics::os();
    let yes = |b: bool| if b { tr!("yes") } else { tr!("no") };
    vec![
        (
            tr!("Version"),
            format!(
                "{} · {} · {}",
                env!("CARGO_PKG_VERSION"),
                crate::diagnostics::COMMIT,
                crate::diagnostics::profile()
            ),
        ),
        (tr!("System"), format!("{os} {os_version}")),
        (
            tr!("Network"),
            format!(
                tr!("{} · own node: {}"),
                match snap.network {
                    evolite_api::Network::Testnet => "testnet",
                    evolite_api::Network::Mainnet => "mainnet",
                },
                yes(s.core_node.is_some())
            ),
        ),
        (
            tr!("Interface"),
            format!(
                tr!("{} · {} theme · scale {} %"),
                crate::i18n::current().code(),
                if dark { tr!("dark") } else { tr!("light") },
                (scale * 100.0).round()
            ),
        ),
        (
            tr!("Sync"),
            format!(
                tr!("{:?} · block {} of {} · ChainLock: {} · peers {}"),
                s.stage,
                s.headers_height,
                s.tip_height,
                yes(s.chainlocked),
                s.core_peers
            ),
        ),
        (
            tr!("Wallet"),
            if crate::is_demo(snap) {
                tr!("demo")
            } else {
                tr!("real")
            }
            .to_string(),
        ),
    ]
}

/// `[evolite 0.1.0] what happened`.
pub fn subject(form: &ReportForm) -> String {
    let summary: String = form.summary.trim().chars().take(90).collect();
    format!("[evolite {}] {summary}", env!("CARGO_PKG_VERSION"))
}

/// The letter as plain text: sections under capital headings, the system as "• name: value"
/// lines, then what is never in it. Mail programs show plain text in a proportional font, so
/// nothing relies on columns: underlines and space-aligned tables come out ragged there.
pub fn body(form: &ReportForm, system: Option<&System>) -> String {
    let mut out = String::from(tr!("Problem report for evolite\n"));
    let mut section = |title: &str, text: &str| {
        out.push('\n');
        out.push_str(&title.to_uppercase());
        out.push('\n');
        let text = text.trim();
        out.push_str(if text.is_empty() { "—" } else { text });
        out.push('\n');
    };
    section(tr!("What happened"), &form.summary);
    section(tr!("What I was doing"), &form.steps);
    section(tr!("What I expected"), &form.expected);
    section(tr!("What happened instead"), &form.actual);
    section(tr!("How often"), OFTEN[form.often.min(OFTEN.len() - 1)]);
    section(
        tr!("Contact for a reply"),
        if form.contact.trim().is_empty() {
            tr!("not given")
        } else {
            &form.contact
        },
    );
    if let Some(system) = system {
        let lines: String = system
            .iter()
            .map(|(label, value)| format!("• {label}: {value}\n"))
            .collect();
        section(tr!("System details"), &lines);
        section(
            tr!("Journal"),
            tr!(
                "Not part of the email. “Save report to folder” in evolite puts the journal next to the report — attach the files to the email."
            ),
        );
    }
    out.push_str(
        tr!("\n—\nThe phrase, keys, passwords, addresses, amounts and names are not included in the report: only what you wrote above.\n"),
    );
    out
}

/// Percent-encoding for a `mailto:` link: everything but unreserved characters.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.replace('\n', "\r\n").bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The `mailto:` link, the body cut to fit [`MAILTO_MAX`] when needed (and then saying so).
pub fn mailto(subject: &str, body: &str) -> String {
    let link = |body: &str| {
        format!(
            "mailto:{TO}?subject={}&body={}",
            encode(subject),
            encode(body)
        )
    };
    let full = link(body);
    if full.len() <= MAILTO_MAX {
        return full;
    }
    let note =
        tr!("\n\n[Email shortened: the full text is in report.txt — “Save report to folder”.]");
    let mut chars: Vec<char> = body.chars().collect();
    loop {
        let cut = chars.len().saturating_sub(40);
        chars.truncate(cut);
        let short: String = chars.iter().collect::<String>() + note;
        let candidate = link(&short);
        if candidate.len() <= MAILTO_MAX || chars.is_empty() {
            return candidate;
        }
    }
}

/// "Save report to folder": `report.txt` and copies of the journal in a new folder in
/// `parent`. Returns the folder.
pub fn save(
    parent: &Path,
    subject: &str,
    body: &str,
    journal: &[PathBuf],
) -> std::io::Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    let folder = parent.join(format!("evolite-report-{stamp}"));
    std::fs::create_dir_all(&folder)?;
    std::fs::write(
        folder.join("report.txt"),
        format!(
            tr!("To: {TO}\nSubject: {subject}\n\n{body}"),
            TO = TO,
            subject = subject,
            body = body
        ),
    )?;
    for file in journal {
        if let Some(name) = file.file_name() {
            std::fs::copy(file, folder.join(name))?;
        }
    }
    Ok(folder)
}

/// Opens `url` with the system's handler (a `mailto:` link: the mail program).
fn open(url: &str) -> Result<(), String> {
    if crate::links::open(url) {
        Ok(())
    } else {
        Err(
            tr!("the mail app did not open: save the report to a folder and send it yourself")
                .into(),
        )
    }
}

/// Opens `folder` in the file manager.
pub fn show(folder: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(folder).spawn();
}

/// The report window, in two steps — the fields, then the letter whole with where it goes —
/// so the letter is always seen before it leaves; true when it should close. Each step
/// scrolls between the title and the buttons, so the window fits a 1280 × 800 screen at 150 %.
pub fn report_dialog(ui: &mut Ui, a: &mut Assets, cx: &mut Ctx, form: &mut ReportForm) -> bool {
    let scale = ui.ctx().pixels_per_point();
    let system = system(cx, a.p.dark, scale);
    // The title, the buttons, a notice and the margins take about this much.
    let room = (ui.ctx().content_rect().height() - 230.0).max(140.0);
    dialog(
        ui,
        a,
        "report",
        "help",
        tr!("Report a problem"),
        560.0,
        |ui, a| {
            if form.letter {
                letter_step(ui, a, form, &system, room)
            } else {
                fields_step(ui, a, form, room)
            }
        },
    )
}

/// The phrase warning under either step; whether a phrase was found.
fn phrase_notice(ui: &mut Ui, a: &mut Assets, form: &ReportForm) -> bool {
    let phrase = form.holds_phrase();
    if phrase {
        notice(
            ui,
            a,
            "warning",
            a.p.danger,
            tr!(
                "The text seems to contain words of a recovery phrase. Remove them — such a report cannot be sent."
            ),
        );
    }
    phrase
}

/// Step 1: what happened. "Next" shows the letter.
fn fields_step(ui: &mut Ui, a: &mut Assets, form: &mut ReportForm, room: f32) -> bool {
    let p = a.p;
    let mut close = false;
    egui::ScrollArea::vertical()
            .id_salt("report-fields")
            .max_height(room)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // Room for the scroll bar on the right.
                ui.set_max_width(ui.available_width() - 14.0);
                ui.spacing_mut().item_spacing.y = 14.0;
                ui.add(
                    egui::Label::new(txt(
                        &format!(tr!("The email goes to the developers at {TO} from your mail app. You will see all of it before sending."), TO = TO),
                        13.0,
                        Weight::Regular,
                        p.muted,
                    ))
                    .wrap(),
                );
                let field = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        field_label(ui, &p, label);
                        add(ui);
                    });
                };
                field(ui, tr!("What happened"), &mut |ui| {
                    text_field(ui, &p, "report-summary", &mut form.summary, tr!("In one line: a payment does not go through, the window closed…"), FieldKind::Text, |_| {});
                });
                field(ui, tr!("What you were doing"), &mut |ui| {
                    text_area(ui, &p, "report-steps", &mut form.steps, tr!("Steps in order: opened “Send”, pasted an address…"), 2);
                });
                field(ui, tr!("What you expected"), &mut |ui| {
                    text_area(ui, &p, "report-expected", &mut form.expected, tr!("The payment goes out and shows in History"), 1);
                });
                field(ui, tr!("What happened instead"), &mut |ui| {
                    text_area(ui, &p, "report-actual", &mut form.actual, tr!("The error text, if there was one"), 2);
                });
                field(ui, tr!("How often"), &mut |ui| {
                    segmented(ui, &p, &OFTEN, &mut form.often, false);
                });
                field(ui, tr!("Contact for a reply (optional)"), &mut |ui| {
                    text_field(ui, &p, "report-contact", &mut form.contact, tr!("email or Telegram, if you want a reply"), FieldKind::Text, |_| {});
                });
                checkbox(ui, a, &mut form.system, tr!("Attach system details: version, OS, network, sync"));
                notice(ui, a, "shield", p.muted, tr!("Do not write your recovery phrase, password or keys here: nobody asks for them, and the report does not need them."));
            });
    let phrase = phrase_notice(ui, a, form);
    let ready = !form.summary.trim().is_empty() && !phrase;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if button_enabled(
            ui,
            a,
            tr!("Next: review the email"),
            ButtonKind::Primary,
            None,
            ready,
        )
        .clicked()
        {
            form.letter = true;
            form.done = None;
        }
        if button(ui, a, tr!("Cancel"), ButtonKind::Ghost, None).clicked() {
            close = true;
        }
    });
    close
}

/// Step 2: the letter whole, as the recipient sees it, then the mail program or a folder.
fn letter_step(
    ui: &mut Ui,
    a: &mut Assets,
    form: &mut ReportForm,
    system: &System,
    room: f32,
) -> bool {
    let p = a.p;
    let (subject, body) = (subject(form), body(form, form.system.then_some(system)));
    ui.add(
        egui::Label::new(txt(
            tr!("The whole email, as the recipient will see it. If everything is right, open it in your mail app or save it together with the journal."),
            13.0,
            Weight::Regular,
            p.muted,
        ))
        .wrap(),
    );
    let preview = format!(
        tr!("To: {TO}\nSubject: {subject}\n\n{body}"),
        TO = TO,
        subject = subject,
        body = body
    );
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("report-letter")
                .max_height(room - 110.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width() - 10.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(crate::i18n::t(&preview))
                                .font(mono(12.0, Weight::Regular))
                                .color(p.text),
                        )
                        .wrap(),
                    );
                });
        });
    let phrase = phrase_notice(ui, a, form);
    match &form.done {
        Some(Ok(what)) => notice(ui, a, "confirmed", p.accent, what),
        Some(Err(why)) => notice(ui, a, "warning", p.danger, why),
        None => {}
    }
    let ready = !form.summary.trim().is_empty() && !phrase;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if button_enabled(ui, a, tr!("Open in email"), ButtonKind::Primary, None, ready).clicked()
        {
            form.done = Some(open(&mailto(&subject, &body)).map(|()| {
                tr!("The email is open in your mail app: check it and press “Send”.")
                    .to_string()
            }));
        }
        if button_enabled(
            ui,
            a,
            tr!("Save report to folder"),
            ButtonKind::Secondary,
            None,
            ready,
        )
        .clicked()
        {
            let parent = evolite_core::diag::dir()
                .and_then(|d| d.parent().map(Path::to_path_buf))
                .unwrap_or_else(std::env::temp_dir)
                .join("reports");
            form.done = Some(
                match save(&parent, &subject, &body, &evolite_core::diag::files()) {
                    Ok(folder) => {
                        show(&folder);
                        Ok(
                            tr!("Report and journal saved: the folder is open, attach the files to the email.")
                                .to_string(),
                        )
                    }
                    Err(e) => Err(format!(
                        tr!("Not saved: {}"),
                        evolite_core::diag::io_error_kind(&e)
                    )),
                },
            );
        }
        if button(ui, a, tr!("Back"), ButtonKind::Ghost, None).clicked() {
            form.letter = false;
            form.done = None;
        }
    });
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> ReportForm {
        ReportForm {
            summary: "A payment does not go through".into(),
            steps: "Opened \"Send\"\npasted an address".into(),
            expected: "The payment goes out".into(),
            actual: "\"Not enough for the amount and the fee\"".into(),
            often: 2,
            ..Default::default()
        }
    }

    #[test]
    fn the_letter_has_its_sections_and_the_system_as_lines() {
        let system: System = vec![
            (tr!("Version"), "0.1.0".into()),
            (tr!("Sync"), "done".into()),
        ];
        let letter = body(&form(), Some(&system));
        for part in [
            "WHAT HAPPENED\nA payment does not go through",
            "HOW OFTEN\nEvery time",
            "CONTACT FOR A REPLY\nnot given",
        ] {
            assert!(letter.contains(part), "{part}\n{letter}");
        }
        // Readable in a proportional font: no column alignment to break.
        assert!(
            letter.contains("• Version: 0.1.0\n• Sync: done\n"),
            "{letter}"
        );
        assert!(!letter.contains('─') && !letter.contains("  "), "{letter}");
        assert!(!body(&form(), None).contains("SYSTEM DETAILS"));
        assert_eq!(
            subject(&form()),
            format!(
                "[evolite {}] A payment does not go through",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn the_mail_link_is_encoded_and_fits() {
        let link = mailto("[evolite] a&b", "line 1\nline 2");
        assert!(
            link.starts_with("mailto:ulput@protonmail.com?subject=%5Bevolite%5D%20a%26b&body=")
        );
        assert!(link.contains("%0D%0A"), "line breaks as CRLF");
        let long = "long text ".repeat(500);
        let link = mailto("subject", &long);
        assert!(link.len() <= MAILTO_MAX, "{}", link.len());
        assert!(link.contains(&encode("Email shortened")));
    }

    #[test]
    fn a_report_with_a_phrase_is_refused() {
        let mut f = form();
        assert!(!f.holds_phrase());
        f.steps = "here is my phrase: abandon ability able about above absent absorb".into();
        assert!(f.holds_phrase());
    }

    #[test]
    fn saves_the_report_with_the_journal() {
        let dir = std::env::temp_dir().join(format!("evolite-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("evolite-2026-11-01.log");
        std::fs::write(&log, b"{\"k\":\"start\"}\n").unwrap();
        let folder = save(&dir.join("reports"), "subject", "text", &[log]).unwrap();
        let report = std::fs::read_to_string(folder.join("report.txt")).unwrap();
        assert!(report.starts_with("To: ulput@protonmail.com\nSubject: subject"));
        assert!(folder.join("evolite-2026-11-01.log").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
