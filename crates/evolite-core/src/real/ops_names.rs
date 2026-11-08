//! Checking and finding DPNS names: every answer waits for Platform in one place.

use super::*;

impl Real {
    /// Answers `question` once Platform can be asked ([`Chain::until_platform_ready`]): the one
    /// wait of the name lookups. A time limit on that wait goes here, with the answer to give
    /// up with as a parameter: only the caller knows which [`Event`] that is.
    fn answer_when_platform_ready(&self, question: impl Future<Output = Event> + Send + 'static) {
        let chain = self.chain.clone();
        self.answer_later(async move {
            chain.until_platform_ready().await;
            question.await
        });
    }

    /// Finds the identity of a DPNS name, with its profile and where it stands among our contacts.
    pub(super) async fn resolve_name(&mut self, name: String) -> (Option<Event>, bool) {
        let event = match dpns::label(&name) {
            Err(e) => Event::NameResolved {
                name,
                result: Err(e),
            },
            Ok(label) => {
                if !self.chain.platform_ready() {
                    let _ = self.out.send(Event::NameResolved {
                        name: name.clone(),
                        result: Err(CoreError::NotSynced),
                    });
                }
                let chain = self.chain.clone();
                // Where the name stands among our contacts, as of the last snapshot.
                let states: HashMap<String, ContactState> = self
                    .contacts
                    .iter()
                    .map(|c| (c.identity_id.clone(), c.state))
                    .collect();
                let avatars = self.avatars.clone();
                let refresh = self.refresher();
                let names = self.names.clone();
                let renamed = self.refresher();
                self.answer_when_platform_ready(async move {
                    let result = match chain.resolve_name(&label).await {
                        Ok(Some(identity_id)) => {
                            // A contact's name known from here on: a payment to `@name`
                            // finds the contact without waiting for its own name lookup.
                            let known = names
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(identity_id.clone(), Some(label.clone()));
                            if known != Some(Some(label.clone())) {
                                renamed();
                            }
                            // A profile that cannot be read is no reason to refuse the name.
                            let profile =
                                match Identifier::from_string(&identity_id, Encoding::Base58) {
                                    Ok(id) => chain.profile_of(id).await.ok().flatten(),
                                    Err(_) => None,
                                };
                            let profile = profile.map(|p| {
                                let key = avatars.want(
                                    &identity_id,
                                    p.avatar_url.as_deref(),
                                    p.avatar_hash,
                                    refresh,
                                );
                                dashpay::profile_of(&p, key)
                            });
                            Ok(ResolvedName {
                                name: label,
                                display: profile.as_ref().and_then(|p| p.display.clone()),
                                contact: states.get(&identity_id).copied(),
                                identity_id,
                                profile,
                            })
                        }
                        Ok(None) => Err(CoreError::NameNotFound),
                        Err(e) => Err(CoreError::Failed(e)),
                    };
                    Event::NameResolved { name, result }
                });
                return (None, false);
            }
        };
        (Some(event), true)
    }

    /// Whether a DPNS name can be taken.
    pub(super) async fn check_name(&mut self, asked: String) -> (Option<Event>, bool) {
        let event = match dpns::label(&asked) {
            Err(e) => Event::NameChecked {
                label: asked,
                result: Err(e),
            },
            // A contested name is told by its letters alone: said at once, with no wait for
            // Platform and no price shown first. evolite cannot take one either way.
            Ok(label) if dpns::is_contested(&label) => Event::NameChecked {
                label: asked,
                result: Ok(NameCheck::Contested),
            },
            Ok(label) => {
                if !self.chain.platform_ready() {
                    let _ = self.out.send(Event::NameChecked {
                        label: asked.clone(),
                        result: Err(CoreError::NotSynced),
                    });
                }
                let chain = self.chain.clone();
                self.answer_when_platform_ready(async move {
                    let result = match chain.name_available(&label).await {
                        Ok(false) => Ok(NameCheck::Taken),
                        Ok(true) => Ok(NameCheck::Available),
                        Err(e) => Err(CoreError::Failed(e)),
                    };
                    Event::NameChecked {
                        label: asked,
                        result,
                    }
                });
                return (None, false);
            }
        };
        (Some(event), true)
    }
}
