//! Checking and finding DPNS names: every answer waits for Platform in one place.

use super::*;

/// How long a name lookup waits for the masternode lists before it gives up and says so. Long,
/// for a first sync; the wait is not expected once the lists are in.
const LISTS_WAIT: Duration = Duration::from_secs(600);
/// How long one Platform read of a name may take: the answer is a failure in words, not a
/// screen that says «Checking» for minutes (BUG-87).
const NAME_WAIT: Duration = Duration::from_secs(60);
/// How long the profile of a found name may take; the name is good without it.
const PROFILE_WAIT: Duration = Duration::from_secs(20);

/// What a read cut off at its limit fails with. These words are what the screen goes by
/// (`text::failure` in evolite-ui): the person reads that the network did not answer in time.
const TIMED_OUT: &str = "timed out";

/// One read of Platform, for at most `wait`: after that, [`TIMED_OUT`]. The read runs in a task
/// of its own and the limit is on the task, not on the read: a proof check blocks inside `poll`
/// while it waits for a quorum key (`provider.rs`, `block_in_place`), and a timer wrapped around
/// that `poll` would be looked at only once it returned. The read writes nothing, so one that
/// outlives its limit harms nothing.
async fn read_within<T: Send + 'static>(
    wait: Duration,
    read: impl Future<Output = Result<T, String>> + Send + 'static,
) -> Result<T, String> {
    let mut task = tokio::spawn(read);
    match tokio::time::timeout(wait, &mut task).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(stopped)) => Err(stopped.to_string()),
        Err(_) => {
            task.abort();
            Err(TIMED_OUT.to_owned())
        }
    }
}

/// Who holds a name, within [`NAME_WAIT`]. Only Platform's own answer that nobody does is
/// [`CoreError::NameNotFound`]: a read cut off says nothing about the name (BUG-87).
async fn name_holder(
    read: impl Future<Output = Result<Option<String>, String>> + Send + 'static,
) -> Result<String, CoreError> {
    match read_within(NAME_WAIT, read).await {
        Ok(Some(identity_id)) => Ok(identity_id),
        Ok(None) => Err(CoreError::NameNotFound),
        Err(e) => Err(CoreError::Failed(e)),
    }
}

/// Whether a name is free, within [`NAME_WAIT`]. Only Platform's own answer is
/// [`NameCheck::Available`] or [`NameCheck::Taken`]: a read cut off is neither (BUG-87).
async fn name_standing(
    read: impl Future<Output = Result<bool, String>> + Send + 'static,
) -> Result<NameCheck, CoreError> {
    match read_within(NAME_WAIT, read).await {
        Ok(false) => Ok(NameCheck::Taken),
        Ok(true) => Ok(NameCheck::Available),
        Err(e) => Err(CoreError::Failed(e)),
    }
}

/// The answer to a name's lookup that could not ask yet: nothing is known about the name.
fn lookup_not_synced(name: String) -> Event {
    Event::NameResolved {
        name,
        result: Err(CoreError::NotSynced),
    }
}

/// The answer to a name's check that could not ask yet: neither free nor taken.
fn check_not_synced(label: String) -> Event {
    Event::NameChecked {
        label,
        result: Err(CoreError::NotSynced),
    }
}

/// `question` once `ready` has happened; `give_up` if it has not after `wait`.
async fn when_ready(
    ready: impl Future<Output = ()>,
    wait: Duration,
    give_up: Event,
    question: impl Future<Output = Event>,
) -> Event {
    match tokio::time::timeout(wait, ready).await {
        Ok(()) => question.await,
        Err(_) => give_up,
    }
}

impl Real {
    /// Answers `question` once Platform can be asked ([`Chain::until_platform_ready`]), the one
    /// wait of the name lookups, with `give_up` after [`LISTS_WAIT`].
    fn answer_when_platform_ready(
        &self,
        give_up: Event,
        question: impl Future<Output = Event> + Send + 'static,
    ) {
        let chain = self.chain.clone();
        self.answer_later(async move {
            when_ready(chain.until_platform_ready(), LISTS_WAIT, give_up, question).await
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
                    let _ = self.out.send(lookup_not_synced(name.clone()));
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
                let give_up = lookup_not_synced(name.clone());
                self.answer_when_platform_ready(give_up, async move {
                    let read = {
                        let (chain, label) = (chain.clone(), label.clone());
                        async move { chain.resolve_name(&label).await }
                    };
                    let result = match name_holder(read).await {
                        Ok(identity_id) => {
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
                                    Ok(id) => {
                                        let read = async move { chain.profile_of(id).await };
                                        read_within(PROFILE_WAIT, read).await.ok().flatten()
                                    }
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
                        Err(e) => Err(e),
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
                    let _ = self.out.send(check_not_synced(asked.clone()));
                }
                let chain = self.chain.clone();
                let give_up = check_not_synced(asked.clone());
                self.answer_when_platform_ready(give_up, async move {
                    let read = async move { chain.name_available(&label).await };
                    let result = name_standing(read).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(label: &str, result: Result<NameCheck, CoreError>) -> Event {
        Event::NameChecked {
            label: label.into(),
            result,
        }
    }

    fn the_result(event: Event) -> Result<NameCheck, CoreError> {
        match event {
            Event::NameChecked { result, .. } => result,
            other => panic!("another event: {other:?}"),
        }
    }

    /// The masternode lists never come: the lookup says so instead of waiting for ever (BUG-87).
    #[tokio::test(start_paused = true)]
    async fn a_lookup_gives_up_on_the_lists_with_the_answer_it_was_given() {
        let event = tokio::time::timeout(
            LISTS_WAIT + Duration::from_secs(1),
            when_ready(
                std::future::pending::<()>(),
                LISTS_WAIT,
                checked("late", Err(CoreError::NotSynced)),
                async { checked("late", Ok(NameCheck::Available)) },
            ),
        )
        .await
        .expect("the wait has a limit");
        assert_eq!(the_result(event), Err(CoreError::NotSynced));
    }

    /// Lists that come in time: the wait does not cost the answer.
    #[tokio::test(start_paused = true)]
    async fn a_lookup_with_the_lists_in_asks_as_before() {
        let event = when_ready(
            async {},
            LISTS_WAIT,
            checked("now", Err(CoreError::NotSynced)),
            async { checked("now", Ok(NameCheck::Taken)) },
        )
        .await;
        assert_eq!(the_result(event), Ok(NameCheck::Taken));
    }

    /// The name's read never answers: not «no such name», whatever the name is (BUG-87).
    #[tokio::test(start_paused = true)]
    async fn a_lookup_cut_off_is_a_failure_and_not_no_such_name() {
        let answer = name_holder(std::future::pending()).await;
        assert_eq!(answer, Err(CoreError::Failed("timed out".to_owned())));
    }

    /// The check's read never answers: neither free nor taken, so nothing can be registered.
    #[tokio::test(start_paused = true)]
    async fn a_check_cut_off_is_a_failure_and_not_available() {
        let answer = name_standing(std::future::pending()).await;
        assert_eq!(answer, Err(CoreError::Failed("timed out".to_owned())));
    }

    /// Platform's own answers still say what they said before the limit came.
    #[tokio::test]
    async fn the_answers_within_the_limit_are_platforms() {
        assert_eq!(
            name_holder(async { Ok(None) }).await,
            Err(CoreError::NameNotFound)
        );
        assert_eq!(
            name_holder(async { Ok(Some("id".to_owned())) }).await,
            Ok("id".to_owned())
        );
        assert_eq!(
            name_holder(async { Err("no".to_owned()) }).await,
            Err(CoreError::Failed("no".to_owned()))
        );
        assert_eq!(
            name_standing(async { Ok(true) }).await,
            Ok(NameCheck::Available)
        );
        assert_eq!(
            name_standing(async { Ok(false) }).await,
            Ok(NameCheck::Taken)
        );
    }

    /// Giving up on the masternode lists says nothing about the name.
    #[tokio::test(start_paused = true)]
    async fn giving_up_on_the_lists_is_not_no_such_name() {
        let event = when_ready(
            std::future::pending::<()>(),
            LISTS_WAIT,
            lookup_not_synced("late".to_owned()),
            async { panic!("the lists never came") },
        )
        .await;
        match event {
            Event::NameResolved { result, .. } => {
                assert!(matches!(result, Err(CoreError::NotSynced)), "{result:?}")
            }
            other => panic!("another event: {other:?}"),
        }
        let event = when_ready(
            std::future::pending::<()>(),
            LISTS_WAIT,
            check_not_synced("late".to_owned()),
            async { panic!("the lists never came") },
        )
        .await;
        assert_eq!(the_result(event), Err(CoreError::NotSynced));
    }

    /// A read stuck inside `poll`, as a proof check is while it waits for a quorum key: the limit
    /// still ends the wait (without the task it would look at the clock only after `poll`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_limit_ends_a_read_blocked_inside_its_poll() {
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let read = async move {
            tokio::task::block_in_place(|| {
                let _ = blocked.recv_timeout(Duration::from_secs(5));
            });
            Ok(())
        };
        let answer = read_within(Duration::from_millis(50), read).await;
        drop(release);
        assert_eq!(answer, Err("timed out".to_owned()));
    }
}
