//! The identity's NFTs: Jembe's gifts on Platform (contract `jembe-nft`, testnet). A `gift`
//! document is a numbered copy that changes hands; its `giftModel` holds the name and the
//! picture, written once by the issuer. The picture is stored packed (raw deflate, SVG or
//! GLB by `imageFormat`) with `imageHash`, the SHA-256 of the unpacked picture: it is unpacked
//! here only to check that hash, within a small limit, and drawn by the sandboxed renderer.
//!
//! The list is asked for in the background and kept; a snapshot shows the last one found and
//! asks again after [`REFRESH`].

use crate::art::Stored;
use evolite_api::{Duffs, Nft, NftOrigin, NftsStatus};
use evolite_render::Packing;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Jembe's NFT contract on testnet (`contracts/deployments.json`, `testnet:nft`, 2026-10-31).
pub const CONTRACT: &str = "8WdoxVsS2gh8enEhzSZPtGSxmbDs4SDadeLzqoRt3b3A";
const CONTRACT_NAME: &str = "jembe-nft";
/// Who issues these gifts: the contract is Jembe's (its owner writes the models).
const ISSUER: &str = "jembe";
/// How often the list is asked for again while the wallet is open.
const REFRESH: Duration = Duration::from_secs(60);
/// How long an ask may take: Platform that does not answer is a failure to show with its
/// reason, never an endless "Loading" (BUG-10); the list is asked for again after [`REFRESH`].
pub const ASK_TIMEOUT: Duration = Duration::from_secs(60);
/// An ask still "running" this long after it started is taken as lost (its task ended
/// without an answer): the next snapshot asks again.
const ASK_LOST: Duration = Duration::from_secs(90);
/// A model's picture is at most 3 072 bytes packed (the contract); unpacked, it stays far
/// under this. A "bomb" stops here.
const MAX_UNPACKED: usize = 1 << 20;

/// Why the list did not come: Platform gave no answer within [`ASK_TIMEOUT`].
pub fn no_answer() -> &'static str {
    tr!("Dash Platform did not answer within a minute")
}

/// A `giftModel` document, as far as it is read.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDoc {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Packed picture, as stored.
    pub image: Vec<u8>,
    pub image_format: u8,
    pub image_hash: Vec<u8>,
    pub supply: Option<u32>,
}

/// A `gift` document: one numbered copy, held by the wallet's identity.
#[derive(Clone, Debug, PartialEq)]
pub struct GiftDoc {
    pub id: String,
    pub model_id: String,
    pub serial: u32,
    /// Held by the contract's owner: only the owner may create gifts (`creationRestrictionMode`
    /// 1), so a gift anyone else holds was handed to them.
    pub held_by_issuer: bool,
    /// Milliseconds since the epoch, when the document keeps it.
    pub transferred_at: Option<u64>,
    /// Set by the holder to sell it, in credits.
    pub price: Option<u64>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The picture of `model`: its key and how it is drawn, and whether it matches `imageHash`.
/// The key is the hash of what is really there, so a picture that does not match is still
/// shown, marked, and never mistaken for the one it claims to be.
pub fn picture(model: &ModelDoc) -> (String, Stored, bool) {
    let stored = Stored {
        bytes: model.image.clone().into(),
        // 1 = SVG, 2 = GLB: both raw deflate; the renderer tells them apart by their bytes.
        packing: Packing::RawDeflate,
    };
    match miniz_oxide::inflate::decompress_to_vec_with_limit(&model.image, MAX_UNPACKED) {
        Ok(unpacked) => {
            let hash = Sha256::digest(&unpacked);
            let verified = model.image_hash.as_slice() == hash.as_slice();
            (hex(&hash), stored, verified)
        }
        // Not a deflate stream: keyed by its own bytes; the renderer will refuse it.
        Err(_) => (hex(&Sha256::digest(&model.image)), stored, false),
    }
}

/// One NFT as the interface shows it.
pub fn to_nft(gift: &GiftDoc, model: &ModelDoc, art: String, verified: bool, owner: &str) -> Nft {
    Nft {
        id: gift.id.clone(),
        // The issuer's words: shown without direction overrides or invisible characters.
        name: super::dashpay::shown(&model.name),
        description: super::dashpay::shown(&model.description),
        art,
        art_verified: verified,
        origin: if gift.held_by_issuer {
            NftOrigin::Release
        } else {
            NftOrigin::Received {
                at: gift.transferred_at.map(|ms| (ms / 1000) as i64),
            }
        },
        issuer: ISSUER.into(),
        contract: CONTRACT_NAME.into(),
        edition: (gift.serial, model.supply.unwrap_or(0)),
        owner: owner.into(),
        // 1 duff = 1 000 credits.
        price: gift.price.map(|credits| (credits / 1000) as Duffs),
    }
}

#[derive(Default)]
struct State {
    /// Whose list this is (base58).
    owner: Option<String>,
    nfts: Vec<Nft>,
    status: NftsStatus,
    asked: Option<Instant>,
    running: bool,
    /// Models never change: read once each.
    models: HashMap<String, ModelDoc>,
    pictures: HashMap<String, Stored>,
}

/// The NFTs of the shown identity, kept between snapshots.
#[derive(Clone, Default)]
pub struct Nfts(Arc<Mutex<State>>);

/// What a background ask found: the gifts, and the models not read before.
pub type Found = (Vec<GiftDoc>, Vec<ModelDoc>);

impl Nfts {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The list and where it stands, for `owner` (base58); `None`: no identity.
    pub fn shown(&self, owner: Option<&str>) -> (Vec<Nft>, NftsStatus) {
        let state = self.state();
        match owner {
            None => (Vec::new(), NftsStatus::NoIdentity),
            Some(o) if state.owner.as_deref() == Some(o) => {
                (state.nfts.clone(), state.status.clone())
            }
            Some(_) => (Vec::new(), NftsStatus::Loading),
        }
    }

    /// Should the list of `owner` be asked for now? Marks the ask as running when so.
    pub fn due(&self, owner: &str) -> bool {
        let mut state = self.state();
        if state.owner.as_deref() != Some(owner) {
            *state = State {
                owner: Some(owner.into()),
                status: NftsStatus::Loading,
                models: std::mem::take(&mut state.models),
                pictures: std::mem::take(&mut state.pictures),
                ..State::default()
            };
        }
        let due = match state.asked {
            None => true,
            Some(at) if state.running => {
                let lost = at.elapsed() >= ASK_LOST;
                if lost && matches!(state.status, NftsStatus::Loading) {
                    state.status = NftsStatus::Failed(no_answer().into());
                }
                lost
            }
            Some(at) => at.elapsed() >= REFRESH,
        };
        if due {
            state.running = true;
            state.asked = Some(Instant::now());
        }
        due
    }

    /// «Try again»: the next [`Self::due`] asks at once, unless an ask runs now. A list that
    /// failed and has nothing saved shows as loading again until the answer.
    pub fn ask_now(&self) {
        let mut state = self.state();
        if state.running {
            return;
        }
        state.asked = None;
        if state.nfts.is_empty() && matches!(state.status, NftsStatus::Failed(_)) {
            state.status = NftsStatus::Loading;
        }
    }

    /// The models already read, so an ask fetches only new ones.
    pub fn known_models(&self) -> Vec<String> {
        self.state().models.keys().cloned().collect()
    }

    /// The answer to an ask for `owner`, shown by `owner_name`.
    pub fn found(&self, owner: &str, owner_name: &str, result: Result<Found, String>) {
        let mut state = self.state();
        // An answer for an identity no longer shown: the ask running now is the new one's.
        if state.owner.as_deref() != Some(owner) {
            return;
        }
        state.running = false;
        let (gifts, models) = match result {
            Ok(found) => found,
            Err(e) => {
                state.status = NftsStatus::Failed(e);
                return;
            }
        };
        for model in models {
            state.models.insert(model.id.clone(), model);
        }
        let mut nfts = Vec::new();
        for gift in &gifts {
            // A gift whose model is gone cannot be shown: there is nothing to show.
            let Some(model) = state.models.get(&gift.model_id).cloned() else {
                continue;
            };
            let (art, stored, verified) = picture(&model);
            state.pictures.insert(art.clone(), stored);
            nfts.push(to_nft(gift, &model, art, verified, owner_name));
        }
        state.nfts = nfts;
        state.status = NftsStatus::Ready;
    }

    /// The picture with this key, for the renderer.
    pub fn picture(&self, art: &str) -> Option<Stored> {
        self.state().pictures.get(art).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG-10: an ask that never answers does not leave the Collection on "Loading" for good.
    #[test]
    fn a_lost_ask_fails_and_is_asked_again() {
        let nfts = Nfts::default();
        let owner = "owner";
        assert!(nfts.due(owner), "the first ask");
        assert!(!nfts.due(owner), "not twice while it runs");
        assert_eq!(nfts.shown(Some(owner)).1, NftsStatus::Loading);
        // Its task ended without an answer, long enough ago.
        let past = |d: Duration| Instant::now().checked_sub(d).unwrap();
        nfts.state().asked = Some(past(ASK_LOST - Duration::from_secs(1)));
        assert!(!nfts.due(owner), "still within its time");
        nfts.state().asked = Some(past(ASK_LOST));
        assert!(nfts.due(owner), "lost: asked again");
        assert_eq!(
            nfts.shown(Some(owner)).1,
            NftsStatus::Failed(no_answer().into()),
            "and said so"
        );
        // The new ask answers: an empty list is "no NFTs yet", not loading.
        nfts.found(owner, "@o", Ok((Vec::new(), Vec::new())));
        assert_eq!(nfts.shown(Some(owner)), (Vec::new(), NftsStatus::Ready));
        assert!(!nfts.due(owner), "the next ask after REFRESH");
        // A timed-out ask is a failure with its reason, asked again after REFRESH.
        nfts.state().asked = Some(past(REFRESH));
        assert!(nfts.due(owner));
        nfts.found(owner, "@o", Err(no_answer().into()));
        assert_eq!(
            nfts.shown(Some(owner)).1,
            NftsStatus::Failed(no_answer().into())
        );
        assert!(!nfts.due(owner));
    }

    /// CS-1a: «Try again» asks at once, not after REFRESH; never a second ask beside a running
    /// one; a failed list with nothing saved shows as loading until the answer.
    #[test]
    fn try_again_asks_at_once_and_only_once() {
        let nfts = Nfts::default();
        let owner = "owner";
        assert!(nfts.due(owner));
        nfts.ask_now();
        assert!(!nfts.due(owner), "an ask runs: none beside it");
        nfts.found(owner, "@o", Err(no_answer().into()));
        assert!(!nfts.due(owner), "the next one after REFRESH");
        nfts.ask_now();
        assert_eq!(nfts.shown(Some(owner)).1, NftsStatus::Loading);
        assert!(nfts.due(owner), "asked again now");
        assert!(!nfts.due(owner), "once");
        // A saved list stays and says it failed until the answer.
        nfts.found(owner, "@o", Ok((vec![gift(None)], vec![model(b"<svg/>")])));
        assert_eq!(nfts.shown(Some(owner)).0.len(), 1);
        nfts.state().running = false;
        nfts.state().status = NftsStatus::Failed("timeout".into());
        nfts.ask_now();
        assert_eq!(
            nfts.shown(Some(owner)).1,
            NftsStatus::Failed("timeout".into())
        );
        assert!(nfts.due(owner));
    }

    fn model(svg: &[u8]) -> ModelDoc {
        ModelDoc {
            id: "model".into(),
            name: "Coffin Dance".into(),
            description: "Dancing".into(),
            image: miniz_oxide::deflate::compress_to_vec(svg, 9),
            image_format: 1,
            image_hash: Sha256::digest(svg).to_vec(),
            supply: Some(3),
        }
    }

    fn gift(transferred_at: Option<u64>) -> GiftDoc {
        GiftDoc {
            id: "gift".into(),
            model_id: "model".into(),
            serial: 2,
            held_by_issuer: false,
            transferred_at,
            price: Some(5_000_000),
        }
    }

    const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;

    #[test]
    fn a_picture_matching_its_hash_is_verified_and_keyed_by_it() {
        let m = model(SVG);
        let (art, stored, verified) = picture(&m);
        assert!(verified);
        assert_eq!(art, hex(&Sha256::digest(SVG)));
        assert_eq!(stored.packing, Packing::RawDeflate);
        assert_eq!(&*stored.bytes, m.image.as_slice());
    }

    #[test]
    fn a_picture_that_does_not_match_is_marked_and_keyed_by_what_is_there() {
        let mut m = model(SVG);
        m.image_hash = vec![0; 32];
        let (art, _, verified) = picture(&m);
        assert!(!verified);
        assert_eq!(art, hex(&Sha256::digest(SVG)));
        // Not deflate at all.
        m.image = b"not deflate".to_vec();
        assert!(!picture(&m).2);
    }

    #[test]
    fn a_bomb_is_not_unpacked_past_the_limit() {
        let mut m = model(SVG);
        m.image = miniz_oxide::deflate::compress_to_vec(&vec![b' '; MAX_UNPACKED * 4], 9);
        assert!(!picture(&m).2);
    }

    #[test]
    fn a_gift_becomes_an_nft_as_platform_has_it() {
        let m = model(SVG);
        let nft = to_nft(
            &gift(Some(1_761_900_000_000)),
            &m,
            "k".into(),
            true,
            "@alice",
        );
        assert_eq!(nft.edition, (2, 3));
        assert_eq!(
            nft.origin,
            NftOrigin::Received {
                at: Some(1_761_900_000)
            }
        );
        // Platform did not keep when: received, the date unknown.
        assert_eq!(
            to_nft(&gift(None), &m, "k".into(), true, "").origin,
            NftOrigin::Received { at: None }
        );
        assert_eq!(nft.price, Some(5_000));
        assert_eq!(
            (nft.issuer.as_str(), nft.contract.as_str()),
            ("jembe", "jembe-nft")
        );
        // Still with the issuer, never handed over.
        let kept = GiftDoc {
            held_by_issuer: true,
            ..gift(None)
        };
        assert_eq!(
            to_nft(&kept, &m, "k".into(), true, "").origin,
            NftOrigin::Release
        );
        // No edition size set.
        let open = ModelDoc { supply: None, ..m };
        assert_eq!(
            to_nft(&gift(None), &open, "k".into(), true, "").edition,
            (2, 0)
        );
    }

    #[test]
    fn the_list_is_kept_per_identity_and_asked_for_again_later() {
        let nfts = Nfts::default();
        assert_eq!(nfts.shown(None).1, NftsStatus::NoIdentity);
        assert!(nfts.due("alice"));
        // Running: not asked twice.
        assert!(!nfts.due("alice"));
        assert_eq!(nfts.shown(Some("alice")).1, NftsStatus::Loading);
        let m = model(SVG);
        nfts.found("alice", "@alice", Ok((vec![gift(None)], vec![m.clone()])));
        let (list, status) = nfts.shown(Some("alice"));
        assert_eq!((list.len(), status), (1, NftsStatus::Ready));
        assert!(nfts.picture(&list[0].art).is_some());
        // Not due again before the refresh time.
        assert!(!nfts.due("alice"));
        // A failure keeps the last list.
        nfts.state().asked = None;
        assert!(nfts.due("alice"));
        nfts.found("alice", "@alice", Err("network".into()));
        let (list, status) = nfts.shown(Some("alice"));
        assert_eq!(
            (list.len(), status),
            (1, NftsStatus::Failed("network".into()))
        );
        // Another identity: its own list, models remembered.
        assert!(nfts.due("bob"));
        assert_eq!(nfts.known_models(), vec!["model".to_string()]);
        assert_eq!(nfts.shown(Some("bob")).0.len(), 0);
    }

    #[test]
    fn a_late_answer_for_another_identity_leaves_the_running_ask_alone() {
        let nfts = Nfts::default();
        assert!(nfts.due("alice"));
        // The person switches identity while alice's list is on its way.
        assert!(nfts.due("bob"));
        nfts.found("alice", "@alice", Ok((Vec::new(), Vec::new())));
        // Bob's ask is still running: not asked a second time.
        assert!(!nfts.due("bob"));
        assert_eq!(nfts.shown(Some("bob")).1, NftsStatus::Loading);
    }
}
