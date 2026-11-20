//! The registration's order and refusals on a network that is not one (FEAT-0038 module V1).

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters};
use dpp::platform_value::string_encoding::Encoding;
use dpp::prelude::Identifier;

use super::register::{Network, RegisterError, Seen, check, register};
use super::rules::Refusal;
use super::{Built, templates};

fn owner() -> Identifier {
    Identifier::from_string(
        "5Y98scgD8Bgp3hp1Doq667wYCBchPxVGFzrCWLje7tXc",
        Encoding::Base58,
    )
    .unwrap()
}

/// What the write does on the fake network.
#[derive(Clone, Copy, PartialEq)]
enum Write {
    /// Confirmed.
    Answers,
    /// Went through, the answer lost.
    LostAnswer,
    /// Refused: nothing registered.
    Refused,
}

struct Fake {
    credits: RefCell<Vec<u64>>,
    /// What the SDK has verified: nothing until the credits are read, then `network`.
    protocol: Cell<Option<u32>>,
    network: u32,
    newer: bool,
    key: bool,
    nonce: u64,
    write: Write,
    /// The contracts the network holds.
    held: RefCell<Vec<DataContract>>,
    created: Cell<u32>,
    kept: RefCell<Vec<Identifier>>,
    /// The order of what was asked.
    asked: RefCell<Vec<&'static str>>,
}

impl Fake {
    fn new(credits: &[u64], write: Write) -> Self {
        Fake {
            credits: RefCell::new(credits.to_vec()),
            // Nothing verified until a proved answer: the credits read gives it.
            protocol: Cell::new(None),
            network: 14,
            newer: false,
            key: true,
            nonce: 7,
            write,
            held: RefCell::default(),
            created: Cell::new(0),
            kept: RefCell::default(),
            asked: RefCell::default(),
        }
    }
}

impl Network for Fake {
    async fn credits(&self, _: &Identifier) -> Result<u64, String> {
        self.asked.borrow_mut().push("credits");
        self.protocol.set(Some(self.network));
        let mut c = self.credits.borrow_mut();
        Ok(if c.len() > 1 { c.remove(0) } else { c[0] })
    }

    fn protocol(&self) -> Seen {
        Seen {
            verified: self.protocol.get(),
            newer: self.newer,
        }
    }

    fn can_sign(&self, _: &Identifier) -> bool {
        self.key
    }

    async fn nonce(&self, _: &Identifier) -> Result<u64, String> {
        self.asked.borrow_mut().push("nonce");
        Ok(self.nonce)
    }

    async fn create(&self, owner: &Identifier, built: &Built) -> Result<DataContract, String> {
        self.asked.borrow_mut().push("create");
        self.created.set(self.created.get() + 1);
        let mut c = built.contract.clone();
        // The network's id: from the owner and the next nonce.
        c.set_id(DataContract::generate_data_contract_id_v0(
            owner,
            self.nonce + 1,
        ));
        match self.write {
            Write::Answers => {
                self.held.borrow_mut().push(c.clone());
                Ok(c)
            }
            Write::LostAnswer => {
                self.held.borrow_mut().push(c);
                Err("no complete response within 20s".into())
            }
            Write::Refused => Err("state transition broadcast error".into()),
        }
    }

    async fn fetch(&self, ids: &[Identifier]) -> Result<Vec<DataContract>, String> {
        self.asked.borrow_mut().push("fetch");
        Ok(self
            .held
            .borrow()
            .iter()
            .filter(|c| ids.contains(&c.id()))
            .cloned()
            .collect())
    }

    fn keep(&self, contract: Arc<DataContract>) {
        self.kept.borrow_mut().push(contract.id());
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(out) = f.as_mut().poll(&mut cx) {
            return out;
        }
    }
}

/// 0.25 DASH of credits: more than the collection's ≈ 0.1941.
const ENOUGH: u64 = 25_000_000_000;

/// Registered: built at the protocol the credits read named (14, not the seed 13: at 13 the
/// profile is refused), the id the network's, kept in the cache, the charge read after.
#[test]
fn a_registration_is_built_at_the_networks_protocol_and_kept() {
    let net = Fake::new(&[ENOUGH, ENOUGH - 19_163_897_000], Write::Answers);
    let done = block_on(register(&net, &owner(), &templates::nft_collection(None))).unwrap();
    assert_eq!(done.protocol, 14);
    let id = DataContract::generate_data_contract_id_v0(owner(), 8);
    assert_eq!(done.contract.id(), id);
    assert_eq!(*net.kept.borrow(), [id]);
    assert_eq!(done.charged, Some(19_163_897_000));
    assert!(done.fee.total() >= 19_163_897_000);
    assert!(!done.recovered);
    assert_eq!(
        *net.asked.borrow(),
        ["credits", "nonce", "create", "credits"]
    );
}

/// AC-T-W6: no CRITICAL key of this wallet — refused before anything is read or sent.
#[test]
fn no_key_is_refused_before_anything() {
    let mut net = Fake::new(&[ENOUGH], Write::Answers);
    net.key = false;
    let r = block_on(register(&net, &owner(), &templates::nft_collection(None)));
    assert_eq!(r.err(), Some(RegisterError::NoKey));
    assert!(net.asked.borrow().is_empty());
    let c = block_on(check(&net, &owner(), &templates::nft_collection(None)));
    assert_eq!(c.refusal, Some(RegisterError::NoKey));
}

/// Fewer credits than the most it can cost: refused with the fee shown, nothing sent.
#[test]
fn too_few_credits_are_refused_with_the_fee() {
    let net = Fake::new(&[19_000_000_000], Write::Answers);
    let c = block_on(check(&net, &owner(), &templates::nft_collection(None)));
    let fee = c.fee.expect("the fee is shown");
    assert_eq!(
        c.refusal,
        Some(RegisterError::NotEnough {
            have: 19_000_000_000,
            need: fee.total()
        })
    );
    let r = block_on(register(&net, &owner(), &templates::nft_collection(None)));
    assert!(matches!(r, Err(RegisterError::NotEnough { .. })));
    assert_eq!(net.created.get(), 0);
}

/// A draft refused by its rules: before the signature, nothing sent.
#[test]
fn a_refused_draft_is_not_sent() {
    let net = Fake::new(&[ENOUGH], Write::Answers);
    let mut draft = templates::nft_collection(None);
    draft.keywords = (0..6).map(|i| format!("word{i}")).collect();
    let r = block_on(register(&net, &owner(), &draft));
    assert!(matches!(
        r,
        Err(RegisterError::Refused(Refusal::Keyword(_)))
    ));
    assert_eq!(net.created.get(), 0);
}

/// The answer lost, the contract registered: found where the owner's nonce puts it, kept, one
/// write only.
#[test]
fn a_lost_answer_finds_the_contract_without_a_second_write() {
    let net = Fake::new(&[ENOUGH, ENOUGH - 19_163_897_000], Write::LostAnswer);
    let done = block_on(register(&net, &owner(), &templates::nft_collection(None))).unwrap();
    assert!(done.recovered);
    assert_eq!(net.created.get(), 1);
    let id = DataContract::generate_data_contract_id_v0(owner(), 8);
    assert_eq!(done.contract.id(), id);
    assert_eq!(*net.kept.borrow(), [id]);
}

/// An error and nothing found right after: not «nothing registered» but «not seen» (r1
/// Must 1, BUG-243) — with the nonce, for the core to keep and read again later; nothing kept
/// yet. (The mutant «None → Failed» fails here.)
#[test]
fn an_error_not_found_after_is_not_seen_rather_than_failed() {
    let net = Fake::new(&[ENOUGH], Write::Refused);
    let r = block_on(register(&net, &owner(), &templates::nft_collection(None)));
    assert_eq!(
        r.err(),
        Some(RegisterError::NotFound {
            nonce: 7,
            sent: "state transition broadcast error".into()
        })
    );
    assert!(net.kept.borrow().is_empty());
    assert_eq!(net.created.get(), 1);
}

/// A credits read whose answer named no protocol: nothing is built at a guess.
#[test]
fn no_verified_protocol_is_refused_before_signing() {
    struct Unverified(Fake);
    impl Network for Unverified {
        async fn credits(&self, _: &Identifier) -> Result<u64, String> {
            Ok(ENOUGH)
        }
        fn protocol(&self) -> Seen {
            Seen {
                verified: None,
                newer: false,
            }
        }
        fn can_sign(&self, o: &Identifier) -> bool {
            self.0.can_sign(o)
        }
        async fn nonce(&self, o: &Identifier) -> Result<u64, String> {
            self.0.nonce(o).await
        }
        async fn create(&self, o: &Identifier, b: &Built) -> Result<DataContract, String> {
            self.0.create(o, b).await
        }
        async fn fetch(&self, ids: &[Identifier]) -> Result<Vec<DataContract>, String> {
            self.0.fetch(ids).await
        }
        fn keep(&self, c: Arc<DataContract>) {
            self.0.keep(c)
        }
    }
    let net = Unverified(Fake::new(&[ENOUGH], Write::Answers));
    let r = block_on(register(&net, &owner(), &templates::nft_collection(None)));
    assert_eq!(r.err(), Some(RegisterError::VersionUnknown));
    assert_eq!(net.0.created.get(), 0);
}

/// The network past the versions this build knows: «Update evolite», nothing sent.
#[test]
fn a_newer_network_asks_for_an_update() {
    let mut net = Fake::new(&[ENOUGH], Write::Answers);
    net.network = 16;
    net.newer = true;
    let c = block_on(check(&net, &owner(), &templates::nft_collection(None)));
    assert_eq!(c.refusal, Some(RegisterError::NewerNetwork(16)));
    let r = block_on(register(&net, &owner(), &templates::nft_collection(None)));
    assert_eq!(r.err(), Some(RegisterError::NewerNetwork(16)));
    assert_eq!(net.created.get(), 0);
}
