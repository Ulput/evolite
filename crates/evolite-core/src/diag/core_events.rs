//! What the journal keeps of the traffic between the UI and the core: the name of each command,
//! what each answer was and how it ended, and changes of the sync and of the vault. Never a
//! command's contents (amounts, addresses, names, passwords), never an answer's.

use super::Record;
use evolite_api::{Command, CoreError, Event, Spend, SyncStage, VaultState};

/// A command by name only.
pub fn command(command: &Command) -> Record {
    let name = match command {
        Command::NewPhrase { .. } => "new_phrase",
        Command::UsePhrase { .. } => "use_phrase",
        Command::CreateWallet { .. } => "create_wallet",
        Command::Unlock { .. } => "unlock",
        Command::Lock => "lock",
        Command::AutoLock { .. } => "auto_lock",
        Command::Touch => "touch",
        Command::ChangePassword { .. } => "change_password",
        Command::ResolveName { .. } => "resolve_name",
        Command::Spend { op, .. } => spend(op),
        Command::SetPasswordEachSend { .. } => "set_password_each_send",
        Command::DisconnectApp { .. } => "disconnect_app",
        Command::CheckName { .. } => "check_name",
        Command::OpenAppLink { .. } => "open_app_link",
        Command::DeclineApp { .. } => "decline_app",
        Command::DrawArt { .. } => "draw_art",
        Command::RevealPhrase { .. } => "reveal_phrase",
        Command::HideRequest { .. } => "hide_request",
        Command::PreviewPayment { .. } => "preview_payment",
        Command::FindIdentities => "find_identities",
        Command::ChooseIdentity { .. } => "choose_identity",
        Command::DeleteWallet { .. } => "delete_wallet",
        Command::AskPhraseCheck { .. } => "ask_phrase_check",
        Command::CheckPhraseWords { .. } => "check_phrase_words",
        Command::Resync => "resync",
        Command::SaveAddress { .. } => "save_address",
        Command::ForgetAddress { .. } => "forget_address",
        Command::CoreNode { .. } => "core_node",
        Command::FiatRate { .. } => "fiat_rate",
        Command::ReceiveAddress { .. } => "receive_address",
        // Which wallet, and its name, stay off the journal.
        Command::SwitchWallet { .. } => "switch_wallet",
        Command::AddWallet => "add_wallet",
        Command::CancelAddWallet => "cancel_add_wallet",
        Command::RenameWallet { .. } => "rename_wallet",
    };
    Record::Command { name }
}

/// A spend by the name its command had before the password gate (R-27), so old and new
/// journals read alike.
fn spend(op: &Spend) -> &'static str {
    match op {
        Spend::Send { .. } => "send",
        Spend::TopUpIdentity { .. } => "top_up_identity",
        Spend::ResumeTopUp => "resume_top_up",
        Spend::CreateIdentity { .. } => "create_identity",
        Spend::ResumeIdentity => "resume_identity",
        Spend::AddName { .. } => "add_name",
        Spend::Shield { .. } => "shield",
        Spend::Unshield { .. } => "unshield",
        Spend::TopUpFromPool { .. } => "top_up_from_pool",
        Spend::AddContact { .. } => "add_contact",
        Spend::AcceptContact { .. } => "accept_contact",
        Spend::SaveProfile { .. } => "save_profile",
        Spend::ApproveApp { .. } => "approve_app",
        Spend::GiftNft { .. } => "gift_nft",
    }
}

/// The kind of an error, without its text.
pub fn error_kind(error: &CoreError) -> &'static str {
    match error {
        CoreError::NotAvailable(_) => "not_available",
        CoreError::WrongPassword => "wrong_password",
        CoreError::BadPhrase => "bad_phrase",
        CoreError::NameNotFound => "name_not_found",
        CoreError::BadName => "bad_name",
        CoreError::BadAmount => "bad_amount",
        CoreError::BadAddress => "bad_address",
        CoreError::NotEnough => "not_enough",
        CoreError::NotSynced => "not_synced",
        CoreError::Failed(_) => "failed",
        CoreError::BadLink => "bad_link",
    }
}

fn outcome<T>(result: &Result<T, CoreError>) -> &'static str {
    result.as_ref().map_or_else(error_kind, |_| "ok")
}

/// Why the renderer did not draw a picture, from its fixed reasons (`art.rs`): words those
/// English texts contain, matched in the core's own text (never shown, so not `tr!`).
fn art_reason(error: &CoreError) -> &'static str {
    let CoreError::Failed(text) = error else {
        return error_kind(error);
    };
    [
        ("too large", "limit"),
        ("not supported", "unsupported"),
        ("damaged", "invalid"),
        ("too long", "timeout"),
        ("stopped", "crashed"),
    ]
    .iter()
    .find(|(words, _)| text.contains(words))
    .map_or("other", |(_, reason)| reason)
}

/// What an answer was and how it ended; snapshots, progress and drawn pictures are not
/// answers (snapshots go through [`Watch`]).
pub fn answer(event: &Event) -> Option<Record> {
    let (what, outcome) = match event {
        Event::Snapshot(_) | Event::Progress { .. } => return None,
        Event::Art { result, .. } => {
            return result.as_ref().err().map(|e| Record::Art {
                reason: art_reason(e),
            });
        }
        Event::NewPhrase(_) => ("new_phrase", "ok"),
        Event::PhraseAccepted(r) => ("phrase_accepted", outcome(r)),
        Event::NameResolved { result, .. } => ("name_resolved", outcome(result)),
        Event::Done { what, result } => (*what, outcome(result)),
        Event::PhraseRevealed(r) => ("phrase_revealed", outcome(r)),
        Event::Sent(r) => ("sent", outcome(r)),
        Event::NameChecked { result, .. } => ("name_checked", outcome(result)),
        Event::AppRequest(r) => ("app_request", outcome(r)),
        Event::PaymentPreview { result, .. } => ("payment_preview", outcome(result)),
        Event::PhraseCheck(r) => ("phrase_check", outcome(r)),
        Event::PhraseChecked { wrong } => (
            "phrase_checked",
            if wrong.is_some() { "wrong" } else { "ok" },
        ),
    };
    Some(Record::Answer { what, outcome })
}

fn stage_name(stage: SyncStage) -> &'static str {
    match stage {
        SyncStage::Connecting => "connecting",
        SyncStage::Headers => "headers",
        SyncStage::Masternodes => "masternodes",
        SyncStage::Filters => "filters",
        SyncStage::Transactions => "transactions",
        SyncStage::Platform => "platform",
        SyncStage::ChainLock => "chainlock",
        SyncStage::Done => "done",
    }
}

/// The last snapshot's sync stage, peers, error and vault: a record only when one changes.
#[derive(Default)]
pub struct Watch {
    stage: Option<(SyncStage, u32)>,
    error: Option<String>,
    vault: Option<VaultState>,
}

impl Watch {
    pub fn snapshot(&mut self, snapshot: &evolite_api::Snapshot) -> Vec<Record> {
        let mut out = Vec::new();
        let s = &snapshot.sync;
        if self.stage != Some((s.stage, s.core_peers)) {
            self.stage = Some((s.stage, s.core_peers));
            out.push(Record::Sync {
                stage: stage_name(s.stage),
                headers: s.headers_height,
                filters: s.filters_height,
                tip: s.tip_height,
                chainlocked: s.chainlocked,
                peers: s.core_peers,
            });
        }
        if s.error != self.error {
            self.error = s.error.clone();
            if let Some(text) = &s.error {
                out.push(Record::SyncError { text: text.clone() });
            }
        }
        if self.vault != Some(snapshot.vault) {
            self.vault = Some(snapshot.vault);
            out.push(Record::Vault {
                state: match snapshot.vault {
                    VaultState::NoWallet => "no_wallet",
                    VaultState::Locked => "locked",
                    VaultState::Unlocked => "unlocked",
                },
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolite_api::{Password, Source};

    #[test]
    fn a_command_is_its_name_and_nothing_of_what_it_carries() {
        let record = command(&Command::Spend {
            op: Spend::Send {
                from: Source::Core,
                to: "yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ".into(),
                amount: 25_000_000,
            },
            password: Some(Password::new("hunter2".into())),
        });
        assert_eq!(record, Record::Command { name: "send" });
        let line = record.line(std::time::UNIX_EPOCH);
        for secret in ["yMrDQ", "25000000", "hunter2"] {
            assert!(!line.contains(secret), "{line}");
        }
    }

    #[test]
    fn an_answer_is_its_kind_and_how_it_ended() {
        let done = Event::Done {
            what: "send",
            result: Err(CoreError::Failed("insufficient funds at yMrDQ…".into())),
        };
        assert_eq!(
            answer(&done),
            Some(Record::Answer {
                what: "send",
                outcome: "failed"
            })
        );
        let art = Event::Art {
            art: "k".into(),
            side: 64,
            partial: false,
            result: Err(CoreError::Failed(
                tr!("the picture takes too long to draw").into(),
            )),
        };
        assert_eq!(answer(&art), Some(Record::Art { reason: "timeout" }));
        // Every reason `art.rs` gives is told apart.
        for (text, reason) in [
            (tr!("the picture is too large"), "limit"),
            (tr!("the picture's format is not supported"), "unsupported"),
            (tr!("the picture is damaged"), "invalid"),
            (tr!("the renderer stopped"), "crashed"),
        ] {
            assert_eq!(
                art_reason(&CoreError::Failed(text.into())),
                reason,
                "{text}"
            );
        }
    }
}
