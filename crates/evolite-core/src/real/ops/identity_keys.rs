//! An identity's keys (FEAT-0020): whose each key is, for the keys screen (K1) and for what it
//! may do — show a private key (R-10), disable, replace.

use crate::real::*;
use evolite_api::KeyOwner;
use evolite_api::key_plan::{self, KeyRefusal, KeyRequest};
use evolite_chain::identity::{Owner, key_owner, this_wallet_keys, this_wallet_secret};
use std::collections::HashMap;

/// This phrase's public keys of each identity, by (identity, revision): derived once while the
/// wallet is open (АРХ-1). Public keys only; while it is closed owners are not known anyway.
#[derive(Default)]
pub(in crate::real) struct ThisKeys(std::sync::Mutex<HashMap<(String, u64), PublicKeys>>);

/// One identity's: its key numbers with this phrase's public key at each.
type PublicKeys = HashMap<u32, [u8; 33]>;

/// The key ids the apps signed in as `identity` on this computer left on it, with each app's
/// name: an older sign-in's two keys, an App Connect v2 sign-in's session keys.
fn app_keys(apps: &[connected_apps::App], identity: &str) -> Vec<(u32, String)> {
    apps.iter()
        .filter(|a| a.identity == identity)
        .flat_map(|a| {
            let older = a.keys.into_iter().flat_map(|(auth, enc)| [auth, enc]);
            let session = a
                .session
                .iter()
                .flat_map(|s| s.keys.iter().map(|k| k.id))
                .collect::<Vec<_>>();
            older
                .chain(session)
                .map(|id| (id, a.name.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

impl Real {
    /// Says whose each of `api`'s keys is (АРХ-1): this phrase's DIP-13 key of the identity's
    /// slot, an app's signed in here, or another's; not known while the wallet is closed or the
    /// identity is not one of the phrase's slots.
    pub(in crate::real) fn fill_key_owners(
        &self,
        api: &mut evolite_api::Identity,
        managed: &platform_wallet::ManagedIdentity,
    ) {
        use dpp::identity::accessors::IdentityGettersV0;
        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        let keys = self.unlocked.as_ref().and_then(|u| u.keys.as_ref());
        let (Some(keys), Some(slot)) = (keys, managed.identity_index) else {
            return;
        };
        let identity = &managed.identity;
        let this = {
            let mut cache = self.this_keys.0.lock().unwrap_or_else(|e| e.into_inner());
            cache
                .entry((api.id.clone(), identity.revision()))
                .or_insert_with(|| {
                    this_wallet_keys(keys, slot, identity.public_keys().keys().copied())
                })
                .clone()
        };
        let apps = app_keys(&self.apps.all(), &api.id);
        for key in &mut api.keys {
            let Some(public) = identity.public_keys().get(&key.id) else {
                continue;
            };
            key.owner = match key_owner(
                public.key_type(),
                key.id,
                public.data().as_slice(),
                this.get(&key.id),
                &apps,
            ) {
                Owner::This => KeyOwner::This,
                Owner::App(name) => KeyOwner::App(name),
                Owner::Other => KeyOwner::Other,
            };
        }
    }

    /// R-10: the private key `key_id` of `identity` as WIF and hex, after the password
    /// ([`Real::checked_vault`]). Refused before the password: a closed wallet, an identity
    /// that is not one of this phrase's, a disabled key, an app's key (App Connect keys are
    /// not the phrase's) or another wallet's. After it, the key derived by DIP-13 is given only
    /// when its public key is the identity's (АРХ-1). Nothing of it is logged.
    pub(in crate::real) async fn reveal_identity_key(
        &mut self,
        identity: &str,
        key_id: u32,
        password: &Password,
    ) -> Result<RevealedKey, CoreError> {
        use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let keys = open
            .keys
            .clone()
            .ok_or(CoreError::NotAvailable(tr!("The keys are not open").into()))?;
        let wallet_id = open.id;
        let wallet = loaded(&self.chain, &wallet_id).await?;
        let (slot, kind, data, disabled) = {
            let state = wallet.state().await;
            let managed = state
                .identity_manager
                .managed_identities()
                .find(|m| {
                    m.wallet_id == Some(wallet_id)
                        && m.identity.id().to_string(Encoding::Base58) == identity
                })
                .ok_or(CoreError::NotAvailable(NOT_FROM_PHRASE.into()))?;
            let slot = managed
                .identity_index
                .ok_or(CoreError::NotAvailable(NOT_FROM_PHRASE.into()))?;
            let key =
                managed
                    .identity
                    .public_keys()
                    .get(&key_id)
                    .ok_or(CoreError::NotAvailable(
                        tr!("The identity has no such key").into(),
                    ))?;
            (
                slot,
                key.key_type(),
                key.data().to_vec(),
                key.disabled_at().is_some(),
            )
        };
        let this = this_wallet_keys(&keys, slot, [key_id]);
        let apps = app_keys(&self.apps.all(), identity);
        may_reveal(
            disabled,
            key_owner(kind, key_id, &data, this.get(&key_id), &apps),
        )?;
        self.checked_vault(password).await?;
        let words = this_wallet_secret(&keys, slot, key_id, kind, &data)
            .ok_or(CoreError::NotAvailable(NOT_FROM_PHRASE.into()))?;
        Ok(RevealedKey {
            identity: identity.to_owned(),
            key_id,
            wif: words.wif,
            hex: words.hex,
        })
    }
}

/// R-10: whether a key may be shown at all, before the password: still acting, and this
/// phrase's — not an app's (App Connect keys are not made from the phrase) nor another's.
fn may_reveal(disabled: bool, owner: Owner) -> Result<(), CoreError> {
    let refusal = match owner {
        _ if disabled => tr!("The key is disabled: its private key is not shown"),
        Owner::This => return Ok(()),
        Owner::App(_) => tr!("An app's key is not shown: it is not made from the phrase"),
        Owner::Other => NOT_FROM_PHRASE,
    };
    Err(CoreError::NotAvailable(refusal.into()))
}

/// Said of a key this phrase does not give.
const NOT_FROM_PHRASE: &str =
    tr!("This key is not from this wallet's phrase: evolite cannot show it");

/// The keys of `managed` as the interface has them, whose each is told (АРХ-1): for a job that
/// cannot reach [`Real::fill_key_owners`]'s cache.
fn api_keys(
    managed: &platform_wallet::ManagedIdentity,
    keys: &key_wallet::Wallet,
    apps: &[(u32, String)],
) -> Vec<evolite_api::IdentityKey> {
    use dpp::identity::accessors::IdentityGettersV0;
    use dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    let mut api = identity::to_api(managed);
    let Some(slot) = managed.identity_index else {
        return api.keys;
    };
    let public_keys = managed.identity.public_keys();
    let this = this_wallet_keys(keys, slot, public_keys.keys().copied());
    for key in &mut api.keys {
        if let Some(public) = public_keys.get(&key.id) {
            key.owner = match key_owner(
                public.key_type(),
                key.id,
                public.data().as_slice(),
                this.get(&key.id),
                apps,
            ) {
                Owner::This => KeyOwner::This,
                Owner::App(name) => KeyOwner::App(name),
                Owner::Other => KeyOwner::Other,
            };
        }
    }
    api.keys
}

/// A key change refused before the password, in words (R-2, R-3, AC-T2).
pub(in crate::real) fn key_refusal(refusal: &KeyRefusal) -> CoreError {
    CoreError::NotAvailable(evolite_common::key_refusal::words(refusal))
}

impl Real {
    /// The shown identity's keys whose each is told, from what the wallet holds now.
    async fn current_keys(&self, identity: &str) -> Option<Vec<evolite_api::IdentityKey>> {
        let open = self.unlocked.as_ref()?;
        let keys = open.keys.as_ref()?;
        let wallet = loaded(&self.chain, &open.id).await.ok()?;
        let state = wallet.state().await;
        let managed = state.identity_manager.managed_identities().find(|m| {
            m.wallet_id == Some(open.id) && m.identity.id().to_string(Encoding::Base58) == identity
        })?;
        Some(api_keys(
            managed,
            keys,
            &app_keys(&self.apps.all(), identity),
        ))
    }

    /// `ReadIdentityKeys` (АРХ-4): the identity read from Platform again in the background, the
    /// next snapshot from it; the keys screen says «checking…» until the answer.
    pub(in crate::real) fn read_identity_keys(&self, identity: String) -> Result<(), CoreError> {
        let open = self
            .unlocked
            .as_ref()
            .ok_or(CoreError::NotAvailable(tr!("The wallet is locked").into()))?;
        let wallet_id = open.id;
        let id = Identifier::from_string(&identity, Encoding::Base58)
            .map_err(|_| CoreError::NotAvailable(NOT_FROM_PHRASE.into()))?;
        let (chain, refresh) = (self.chain.clone(), self.refresher());
        let (journal, unclear, keys) = (
            self.journal.clone(),
            self.unclear.clone(),
            open.keys.clone(),
        );
        self.answer_later("task:read_identity_keys", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                wallet
                    .identity()
                    .refresh_identity(&id)
                    .await
                    .map(|_| ())
                    .map_err(|e| CoreError::Failed(e.to_string()))?;
                // BUG-184 p. 2: a change whose outcome was not known, now shown: its line.
                let pending = unclear.waits(&identity, |w| matches!(w, sent::Watch::Keys(_)));
                if let (true, Some(keys)) = (pending, keys.as_ref())
                    && let Ok(again) = chain.sign_in_identity(&wallet, keys).await
                    && let Some(done) = take_if_done(&unclear, &identity, |plan| {
                        again.id == id && evolite_chain::key_update::shows(&again, plan)
                    })
                {
                    let after = chain.identity_credits(&wallet, &id, true).await;
                    if let Some(kind) = done.kind {
                        journal.add(own_line(kind, done.before, after, done.acting));
                    }
                }
                Ok(())
            }
            .await;
            refresh();
            Event::Done {
                what: DoneWhat::KeysRead,
                result,
            }
        });
        Ok(())
    }

    /// AC-T2: what [`Self::update_keys`] would refuse, said before the password is asked; the
    /// job plans again on the identity read from Platform.
    pub(in crate::real) async fn key_update_refused(
        &self,
        identity: &str,
        add: &[KeyRequest],
        disable: &[u32],
    ) -> Option<CoreError> {
        if self.identity_busy.is_busy() {
            return Some(identity_busy());
        }
        let keys = self.current_keys(identity).await?;
        key_plan::plan(&keys, add, disable)
            .err()
            .map(|r| key_refusal(&r))
    }

    /// `Spend::UpdateKeys` (АРХ-5): in the background, the identity read again, the change
    /// planned on it, sent, the identity read again to see it done (R-7). After an error it is
    /// read again too, and nothing is sent twice by itself (AC-T4).
    pub(in crate::real) fn update_keys(
        &self,
        identity: String,
        add: Vec<KeyRequest>,
        disable: Vec<u32>,
    ) -> Result<(), CoreError> {
        let (wallet_id, keys) = self.dashpay_ready()?;
        // TECH-platform-errors review r1 S1, as App Connect (BUG-208 S1): an earlier change of
        // these keys not known to have gone through may still land; signed again now, the same
        // change could be refused with a fee. The person waits, then looks at the keys.
        if self.unclear.keys_on_their_way(&identity) {
            return Err(CoreError::NotAvailable(
                tr!("The earlier change of keys may still go through: wait two minutes, then look at the keys again").into(),
            ));
        }
        // One change of the identity at a time (the review's Should 1): two would number their
        // keys alike and the second be refused by Platform, with a fee.
        let Some(busy) = self.identity_busy.take() else {
            return Err(identity_busy());
        };
        let (chain, apps, refresh) = (self.chain.clone(), self.apps.clone(), self.refresher());
        let (journal, unclear) = (self.journal.clone(), self.unclear.clone());
        self.answer_later("task:update_keys", async move {
            let result = async {
                let wallet = loaded(&chain, &wallet_id).await?;
                // Shared with the send's own task (TECH-after-send-outcome), never copied: it holds
                // the private halves of the identity's keys.
                let who = Arc::new(
                    chain
                        .sign_in_identity(&wallet, &keys)
                        .await
                        .map_err(CoreError::NotAvailable)?,
                );
                if who.id.to_string(Encoding::Base58) != identity {
                    return Err(CoreError::NotAvailable(
                        tr!("Another identity is shown now: open its keys again").into(),
                    ));
                }
                let fresh = {
                    let state = wallet.state().await;
                    let managed = state.identity_manager.managed_identity(&who.id).ok_or(
                        CoreError::NotAvailable(tr!("The wallet has no identity").into()),
                    )?;
                    api_keys(managed, &keys, &app_keys(&apps.all(), &identity))
                };
                let plan = key_plan::plan(&fresh, &add, &disable).map_err(|r| key_refusal(&r))?;
                // BUG-145: what the update cost, from the credits before and after.
                // The identity whose keys change, not the one shown by now.
                let acting = Some(identity.clone());
                let before = chain.identity_credits(&wallet, &who.id, false).await;
                // TECH-after-send-outcome: by the one rule. Every error has the identity read
                // again, as before (BUG-184): showing the change, it is done; never sent twice.
                let send = {
                    let (chain, wallet, keys) = (chain.clone(), wallet.clone(), keys.clone());
                    let (who, plan) = (who.clone(), plan.clone());
                    async move { chain.update_keys(&wallet, &keys, &who, &plan).await }
                };
                let outcome = sent::run(
                    send,
                    sent::SEND_WAIT,
                    |e: &String| {
                        // The kind of the error, never its text (BUG-184 p. 4).
                        crate::diag::platform_failed("update_keys", e);
                        // TECH-platform-errors (audit Д4): a refusal before the send — no
                        // credits, a key, not ready — is said as such, not as «not known».
                        sent::refused_before(e)
                    },
                    || async { chain.sign_in_identity(&wallet, &keys).await.ok() },
                    |again| done_after_error(Some(again), &who, &plan),
                )
                .await;
                let kind = journal::Kind::KeysChanged {
                    added: plan.add.iter().map(|k| k.id).collect(),
                    disabled: plan.disable.clone(),
                    bound: plan.add.iter().filter(|k| k.dashpay).count() as u32,
                };
                let after_send = match outcome {
                    sent::Outcome::Done(after) => after,
                    sent::Outcome::Shown(again) => {
                        crate::diag::platform_failed("update_keys_unclear", "Done");
                        again
                    }
                    sent::Outcome::Refused(e) => return Err(super::super::platform_failure(e)),
                    sent::Outcome::Unclear { late } => {
                        // Not known: kept, for the read that shows it, or the send's own late
                        // answer, to write its line (BUG-184 p. 2) — by the one rule (Д4).
                        let pending = sent::Pending::new(
                            identity.clone(),
                            sent::Watch::Keys(plan.clone()),
                            Some(kind),
                            before,
                            acting,
                        );
                        let (chain, id) = (chain.clone(), who.id);
                        return Err(unclear.not_known(
                            pending,
                            late,
                            journal.clone(),
                            move |_, p| async move {
                                let after = chain.identity_credits(&wallet, &id, true).await;
                                let kind = p.kind.expect("a key change's kind is known");
                                own_line(kind, p.before, after, p.acting)
                            },
                        ));
                    }
                };
                let after = chain.identity_credits(&wallet, &who.id, true).await;
                journal.add(own_line(kind, before, after, acting));
                sent_answer(evolite_chain::key_update::shows(&after_send, &plan))
            }
            .await;
            drop(busy);
            refresh();
            Event::Done {
                what: DoneWhat::Keys,
                result,
            }
        });
        Ok(())
    }
}

/// BUG-200: the answer to a key change Platform executed (the send proved it): done. The
/// identity read right after may still be the old one — a DAPI node a block behind — and that
/// is only journaled (`update_keys_stale_read`); the next read of the keys catches up. Answering
/// it as a failure made the dialog plan again on the new keys («−9, +10») with its button on: a
/// second paid change of the main key one click away.
pub(in crate::real) fn sent_answer(read_shows_it: bool) -> Result<(), CoreError> {
    if !read_shows_it {
        crate::diag::platform_failed("update_keys_stale_read", "Stale");
    }
    Ok(())
}

/// BUG-184 p. 1: after an error of the send, whether `again` (the identity read again, if it
/// was) is the same identity and shows `plan` done.
pub(in crate::real) fn done_after_error(
    again: Option<&evolite_chain::app_chain::SignInIdentity>,
    who: &evolite_chain::app_chain::SignInIdentity,
    plan: &key_plan::KeyPlan,
) -> bool {
    again.is_some_and(|a| a.id == who.id && evolite_chain::key_update::shows(a, plan))
}

/// BUG-184 p. 2: the unclear change of `identity`, taken out once `shows` says a read of it
/// shows the change done; its line is then written, once.
pub(in crate::real) fn take_if_done(
    unclear: &sent::Unclear,
    identity: &str,
    shows: impl Fn(&key_plan::KeyPlan) -> bool,
) -> Option<sent::Pending> {
    unclear.take_if(identity, |watch| match watch {
        sent::Watch::Keys(plan) => shows(plan),
        _ => false,
    })
}

/// Said while another change of the identity is under way (the review's Should 1).
pub(in crate::real) fn identity_busy() -> CoreError {
    CoreError::NotAvailable(
        tr!("The identity is busy: wait for the current operation to end").into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(identity: &str, keys: Option<(u32, u32)>, session: &[u32]) -> connected_apps::App {
        connected_apps::App {
            contract: "C".into(),
            name: "Jembe".into(),
            identity: identity.into(),
            generation: 0,
            keys,
            last_used: 0,
            session: (!session.is_empty()).then(|| connected_apps::Session {
                connection: None,
                keys: session
                    .iter()
                    .map(|&id| connected_apps::SessionKey {
                        id,
                        critical: false,
                        kind: "contract".into(),
                        target: "C".into(),
                        document_type: None,
                        budget: 0,
                        expires_at: 0,
                    })
                    .collect(),
                response: None,
                label: "Jembe".into(),
                disconnected_at: None,
                spent_seen: None,
            }),
        }
    }

    /// FEAT-0020 AC-T1: an app's keys are the ids its sign-in left on this identity — an older
    /// sign-in's pair and an App Connect v2 session's keys; another identity's apps are not
    /// this one's.
    #[test]
    fn an_apps_keys_are_the_ids_its_sign_in_left() {
        let apps = [
            app("id1", Some((7, 8)), &[]),
            app("id1", None, &[9, 10]),
            app("id2", Some((11, 12)), &[]),
        ];
        let found: Vec<u32> = app_keys(&apps, "id1")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(found, [7, 8, 9, 10]);
    }

    /// FEAT-0020 AC-T11: refused before the password — a disabled key (also this wallet's), an
    /// app's, another wallet's; this wallet's acting key goes on to the password.
    #[test]
    fn only_this_wallets_acting_key_goes_on_to_the_password() {
        let refused = |disabled, owner| match may_reveal(disabled, owner) {
            Err(CoreError::NotAvailable(words)) => words,
            other => panic!("{other:?}"),
        };
        assert!(may_reveal(false, Owner::This).is_ok());
        assert!(refused(true, Owner::This).contains("disabled"));
        assert!(refused(false, Owner::App("Jembe".into())).contains("app"));
        assert!(refused(false, Owner::Other).contains("not from this wallet"));
        assert!(refused(true, Owner::Other).contains("disabled"));
    }

    /// A key `id` of the identity, disabled or not.
    fn key(id: u32, disabled: bool) -> dpp::identity::IdentityPublicKey {
        dpp::identity::identity_public_key::v0::IdentityPublicKeyV0 {
            id,
            purpose: dpp::identity::Purpose::AUTHENTICATION,
            security_level: dpp::identity::SecurityLevel::CRITICAL,
            contract_bounds: None,
            key_type: dpp::identity::KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: dpp::platform_value::BinaryData::new(vec![2; 33]),
            disabled_at: disabled.then_some(1_793_000_000_000),
        }
        .into()
    }

    fn identity(
        n: u8,
        keys: Vec<dpp::identity::IdentityPublicKey>,
    ) -> evolite_chain::app_chain::SignInIdentity {
        evolite_chain::app_chain::SignInIdentity::for_tests(Identifier::from([n; 32]), keys)
    }

    /// BUG-184 AC-T1: the send failed, and the identity read again shows key 2 disabled: done.
    /// AC-T2: not read again, or not showing it, or another identity: not known.
    #[test]
    fn a_change_the_identity_shows_after_an_error_is_done() {
        let plan = key_plan::KeyPlan {
            add: Vec::new(),
            disable: vec![2],
        };
        let who = identity(1, vec![key(0, false), key(2, false)]);
        let done = identity(1, vec![key(0, false), key(2, true)]);
        assert!(done_after_error(Some(&done), &who, &plan));
        assert!(
            !done_after_error(Some(&who), &who, &plan),
            "key 2 acts still"
        );
        assert!(!done_after_error(None, &who, &plan), "not read again");
        let other = identity(9, vec![key(0, false), key(2, true)]);
        assert!(!done_after_error(Some(&other), &who, &plan));
    }

    /// BUG-184 AC-T2: a change not known waits; the read that shows it done takes it once, for
    /// its one line; another identity's read does not.
    #[test]
    fn a_late_line_is_written_once() {
        let slot = sent::Unclear::default();
        slot.keep(sent::Pending::new(
            "A".into(),
            sent::Watch::Keys(key_plan::KeyPlan {
                add: Vec::new(),
                disable: vec![2],
            }),
            Some(journal::Kind::KeysChanged {
                added: Vec::new(),
                disabled: vec![2],
                bound: 0,
            }),
            Some(1_331_644),
            None,
        ));
        assert!(
            take_if_done(&slot, "A", |_| false).is_none(),
            "not shown yet"
        );
        assert!(
            take_if_done(&slot, "B", |_| true).is_none(),
            "another identity"
        );
        let done = take_if_done(&slot, "A", |plan| plan.disable == [2]).expect("shown now");
        assert_eq!(done.before, Some(1_331_644));
        assert!(take_if_done(&slot, "A", |_| true).is_none(), "once");
    }

    /// TECH-platform-errors (audit Д4): a change of keys refused before it was sent — no
    /// credits, a key — is said as that refusal, not as «not known» after a read of the keys
    /// again; a failure of the network after it was sent is still not known. In the path, the
    /// refusal is asked of `sent::refused_before` and answered by `platform_failure`, and «not
    /// known» kept by `Unclear::not_known`. Mutants: the old `false`; the hand-kept slot.
    #[test]
    fn a_refused_change_of_keys_says_why() {
        let not_enough = "Current credits balance 1 is not enough to pay 2 fee";
        assert!(sent::refused_before(not_enough));
        assert_eq!(
            super::super::super::platform_failure(not_enough.into()),
            CoreError::NotEnough
        );
        assert!(
            !sent::refused_before("connection reset by peer"),
            "not known"
        );
        let source = include_str!("identity_keys.rs");
        let f = &source[source.find("fn update_keys(").expect("update_keys")..];
        let f = &f[..f.find("#[cfg(test)]").expect("its end")];
        assert!(
            f.contains("sent::refused_before(e)"),
            "the refusal not asked"
        );
        assert!(
            f.contains(
                "sent::Outcome::Refused(e) => return Err(super::super::platform_failure(e))"
            ),
            "the refusal not said"
        );
        assert!(f.contains("unclear.not_known("), "not by the one rule");
        assert!(!f.contains("unclear.keep("), "kept by hand");
    }

    /// TECH-platform-errors review r1 S1: a change of keys while an earlier one of the same
    /// identity may still land (`Unclear::keys_on_their_way`, BUG-208 S1) is not signed: the
    /// check comes before the identity is taken and before the task that sends it.
    /// Mutant: the check removed, or put after `sent::run(`.
    #[test]
    fn a_change_of_keys_waits_for_one_on_its_way() {
        let source = include_str!("identity_keys.rs");
        let f = &source[source.find("fn update_keys(").expect("update_keys")..];
        let f = &f[..f.find("#[cfg(test)]").expect("its end")];
        let check = f
            .find("self.unclear.keys_on_their_way(&identity)")
            .expect("the check");
        let busy = f
            .find("self.identity_busy.take()")
            .expect("the identity taken");
        let task = f.find("self.answer_later(").expect("the task");
        let update = f.find("sent::run(").expect("the update");
        assert!(check < busy && busy < task && task < update);
    }

    /// BUG-200: a send Platform executed is done, whether or not the identity read right after
    /// already shows it; its History line is the send's (`own_spend` on `Ok`), one.
    #[test]
    fn a_proved_send_is_done_even_when_the_read_after_is_stale() {
        assert_eq!(sent_answer(true), Ok(()));
        assert_eq!(sent_answer(false), Ok(()));
        let sent: Result<(), String> = Ok(());
        let kind = journal::Kind::KeysChanged {
            added: vec![10],
            disabled: vec![9],
            bound: 0,
        };
        let line = own_spend(&sent, kind, Some(1_000_000_000), Some(982_754_000), None);
        assert_eq!(line.map(|l| l.spent), Some(17_246));
    }
}
