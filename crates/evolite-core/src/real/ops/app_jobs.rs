//! App sign-in in the core's loop: a link becomes a request shown to the person, "Allow" does
//! the work in the background, "Disconnect" takes the app's session keys back. Only App Connect
//! v2 ([`super::app_connect2`]): v1 (`dash-key:` v1, `dash-st:`) is not supported (the
//! customer, 2026-11-11), and the v1 sign-ins in older files are neither listed nor touched;
//! their keys stay on the identity for key management (FEAT-0020).

use evolite_api::*;
use evolite_chain::app_connect_v2;

use super::app_connect2;
use super::identity::own_spend;
use crate::real::connected_apps::App;
use crate::real::snapshot::acting_identity;
use crate::real::{AppJob, Real, journal, loaded, now};

/// A request evolite no longer has.
fn out_of_date() -> CoreError {
    CoreError::NotAvailable(tr!("The request is out of date: open the app's link again").into())
}

/// The links of App Connect v1, refused in words of their own.
fn is_v1_link(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.starts_with("dash-key:") || lower.starts_with("dash-st:")
}

/// The apps signed in as `identity` (base58) by App Connect v2, as the screens show them, with
/// their session keys and what Platform last said of them. Older v1 entries are left out.
pub(in crate::real) fn shown_apps(
    apps: &[App],
    identity: Option<&str>,
    reads: &app_connect2::Reads,
) -> Vec<ConnectedApp> {
    let now_ms = (now().max(0) as u64).saturating_mul(1_000);
    apps.iter()
        .filter(|a| Some(a.identity.as_str()) == identity && a.session.is_some())
        .map(|a| ConnectedApp {
            id: a.contract.clone(),
            name: a.name.clone(),
            label: a
                .session
                .as_ref()
                .map(|s| s.label.clone())
                .unwrap_or_default(),
            domain: String::new(),
            last_used: a.last_used,
            session: app_connect2::shown_session(a, reads, now_ms),
            disconnected_at: a.session.as_ref().and_then(|s| s.disconnected_at),
        })
        .collect()
}

impl Real {
    /// Opens an app's link: checks it, asks Platform what is needed to show it, and answers
    /// `AppRequest` — at once when the link itself is refused, else from the background.
    pub(in crate::real) fn open_app_link(&self, text: &str) -> Result<(), CoreError> {
        let ms = (now().max(0) as u64).saturating_mul(1_000);
        match app_connect_v2::parse(text, app_connect_v2::Net::current(), ms) {
            Ok(link) => self.open_connect2(link),
            // App Connect v1 is not supported (the customer, 2026-11-11): told as another version.
            Err(app_connect_v2::Refusal::NotOurs) if is_v1_link(text) => {
                Err(CoreError::AppLink(AppLinkRefusal::Version {
                    v: app_connect2::link_param(text, "v").unwrap_or_else(|| "1".into()),
                }))
            }
            Err(refused) => Err(app_connect2::refusal(refused, text, false)),
        }
    }

    /// The person's answer to request `id`: declined at once, approved in the background.
    /// `Ok(true)`: the answer comes later as `Done { what: DoneWhat::App }`.
    pub(in crate::real) fn answer_app(
        &self,
        id: u64,
        approve: bool,
        grant: Option<SessionGrant>,
    ) -> Result<bool, CoreError> {
        let job = self
            .app_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if !approve {
            return Ok(false);
        }
        let AppJob::Connect2(job) = job.ok_or_else(out_of_date)?;
        self.approve_connect2(*job, grant).map(|()| true)
    }

    /// "Remove from the list" (D2): a disconnected app of the shown identity leaves the list;
    /// nothing is spent and nothing goes to Platform. `Done { what: DoneWhat::ForgetApp }`.
    pub(in crate::real) fn forget_app(&self, contract: &str) -> Result<(), CoreError> {
        let (wallet_id, keys) = self.dashpay_ready()?;
        let (chain, apps, refresh) = (self.chain.clone(), self.apps.clone(), self.refresher());
        let contract = contract.to_owned();
        self.answer_later("task:forget_app", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                let (shown, _) = evolite_chain::chain::Chain::first_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                use dpp::identity::accessors::IdentityGettersV0;
                let me = app_connect2::base58(&shown.id().to_buffer());
                app_connect2::forget(&apps, &contract, &me)
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::ForgetApp,
                result,
            }
        });
        Ok(())
    }

    /// "Disconnect" (a spend: it passes the password gate first, BUG-42): the sign-in's live
    /// session keys are disabled in a MASTER-signed update, never its encryption keys, its
    /// response is deleted, and the app leaves the list. `Done { what: DoneWhat::Disconnect }`.
    pub(in crate::real) fn disconnect_app(&self, contract: &str) -> Result<(), CoreError> {
        let (wallet_id, keys) = self.dashpay_ready()?;
        let (chain, apps, refresh) = (self.chain.clone(), self.apps.clone(), self.refresher());
        let journal = self.journal.clone();
        let contract = contract.to_owned();
        self.answer_later("task:disconnect_app", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                let who = chain
                    .sign_in_identity(&wallet, &keys)
                    .await
                    .map_err(CoreError::NotAvailable)?;
                let me = app_connect2::base58(&who.id_bytes());
                let app = apps
                    .get(&contract, &me)
                    .filter(|a| a.session.is_some())
                    .ok_or(CoreError::NotAvailable(
                        tr!("This app is no longer in the list").into(),
                    ))?;
                // BUG-145: what disabling the keys cost, from the credits before and after.
                let acting = acting_identity(&wallet).await;
                let before = chain.first_identity_credits(&wallet, false).await;
                let done = app_connect2::disconnect(&chain, &wallet, &keys, who, &app, &apps).await;
                let after = match done {
                    Ok(()) => chain.first_identity_credits(&wallet, true).await,
                    Err(_) => None,
                };
                let label = app_connect2::app_label(&app);
                let kind = journal::Kind::AppDisconnect { app: label };
                if let Some(line) = own_spend(&done, kind, before, after, acting) {
                    journal.add(line);
                }
                done
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::Disconnect,
                result,
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::real::connected_apps::Session;

    /// FEAT-0023, the customer's 2026-11-11 decision 1a: an older file's v1 entries do not show
    /// up among the apps, and they do not disturb the v2 sign-ins of the same identity.
    #[test]
    fn only_v2_sign_ins_of_the_shown_identity_are_listed() {
        let app = |contract: &str, identity: &str, session: Option<Session>| App {
            contract: contract.into(),
            name: "@yappr".into(),
            identity: identity.into(),
            generation: 3,
            keys: Some((6, 7)),
            last_used: 5,
            session,
        };
        let apps = [
            app("V1", "I1", None),
            app("V2", "I1", Some(Session::default())),
            app("V2", "I2", Some(Session::default())),
        ];
        let shown = shown_apps(&apps, Some("I1"), &Default::default());
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, "V2");
        assert!(shown_apps(&apps, None, &Default::default()).is_empty());
    }

    #[test]
    fn v1_links_are_told_apart() {
        assert!(is_v1_link("dash-key:3mJr7AoUXx2Wqd?n=t&v=1"));
        assert!(is_v1_link("  DASH-ST:abc"));
        assert!(!is_v1_link("dashpay://connect?v=2"));
        assert!(!is_v1_link("dash:yXq?amount=1"));
    }
}
