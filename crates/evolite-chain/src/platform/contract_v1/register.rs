//! Registering a contract (FEAT-0038, FEAT-0019 T-4): the checks before the signature, the
//! write, and what a lost answer leaves. The network is a [`Network`], so the order and the
//! refusals are tested without one (`Chain` is the real one).

use std::sync::Arc;

use dpp::balances::credits::Credits;
use dpp::data_contract::DataContract;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::prelude::Identifier;

use super::fee::{self, Fee};
use super::{Built, Draft, Refusal, build, version_for};

/// What registering asks of the network, with proofs where it reads. Its futures are not
/// required `Send` here: the chain's are, and the core spawns them as they are.
#[allow(async_fn_in_trait)]
pub trait Network {
    /// The owner's credits, read with proof now. Asked first: the SDK knows the network's
    /// protocol only from a proved answer (before one it holds its seed, 13 on testnet).
    async fn credits(&self, owner: &Identifier) -> Result<Credits, String>;
    /// The network's protocol as a proved, signed answer named it (the SDK's
    /// `protocol_version_info`): never its seed.
    fn protocol(&self) -> Seen;
    /// The owner has an AUTHENTICATION · CRITICAL · ECDSA key this wallet can sign with: the
    /// only key a contract's registration takes (platform-wallet).
    fn can_sign(&self, owner: &Identifier) -> bool;
    /// The owner's identity nonce, read with proof now.
    async fn nonce(&self, owner: &Identifier) -> Result<u64, String>;
    /// Sign and send `built`; the contract as the network confirmed it.
    async fn create(&self, owner: &Identifier, built: &Built) -> Result<DataContract, String>;
    /// The contracts of `ids` the network has, read with proof.
    async fn fetch(&self, ids: &[Identifier]) -> Result<Vec<DataContract>, String>;
    /// A contract registered: into the contracts cache and the provider (FEAT-0031 C5), so the
    /// first document written to it is checked without another read.
    fn keep(&self, contract: Arc<DataContract>);
}

/// The network's protocol, as the SDK has verified it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    /// From a proved, signed answer; `None` before one.
    pub verified: Option<u32>,
    /// The network is past the versions this build knows.
    pub newer: bool,
}

/// Why a registration did not happen or is not known to have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// No CRITICAL key of this wallet on the owner (FEAT-0038 AC-T-W6: K5 adds one).
    NoKey,
    /// No proved answer has named the network's protocol yet: nothing is built at a guess.
    VersionUnknown,
    /// The network runs a protocol newer than this build knows: «Update evolite».
    NewerNetwork(u32),
    /// The draft is refused before the signature.
    Refused(Refusal),
    /// The owner's credits are below the most it can cost.
    NotEnough { have: Credits, need: Credits },
    /// Not sent, or sent and refused: nothing was registered.
    Failed(String),
    /// The send ended with an error and the contract is not where the owner's nonce `nonce`
    /// puts it, read once right after: «not seen» is not «not done» (BUG-243). The core tells
    /// a refusal before sending (nothing registered) from the rest (may still land).
    NotFound { nonce: u64, sent: String },
    /// The send ended with an error and the read after it failed too: where the owner's nonce
    /// `nonce` puts it is not known (r2: on a bad network the commonest case). The core keeps
    /// it as [`Self::NotFound`] is kept.
    Unclear {
        nonce: u64,
        sent: String,
        read: String,
    },
}

/// A registration done.
#[derive(Clone, Debug)]
pub struct Registered {
    pub contract: Arc<DataContract>,
    /// What was estimated before the signature.
    pub fee: Fee,
    /// What the owner's credits went down by, read with proof after it; `None` if that read
    /// failed.
    pub charged: Option<Credits>,
    /// The contract's size, for the journal's calibration.
    pub bytes: usize,
    /// The protocol it was built and priced at.
    pub protocol: u32,
    /// Found after a lost answer rather than confirmed by it.
    pub recovered: bool,
}

/// What [`check`] says before the password: the fee, and why it may not go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub fee: Option<Fee>,
    pub refusal: Option<RegisterError>,
}

/// How many nonces past the one read a contract of the owner's may have taken: the SDK bumps
/// its own cache, which may be ahead of the network's by an attempt or two.
const NONCES_AHEAD: u64 = 3;

/// The checks before the password, read fresh: the key, the draft, the fee, the credits.
pub async fn check(net: &impl Network, owner: &Identifier, draft: &Draft) -> Checked {
    match prepare(net, owner, draft).await {
        Ok((_, fee, _, _)) => Checked {
            fee: Some(fee),
            refusal: None,
        },
        Err((fee, refusal)) => Checked {
            fee,
            refusal: Some(refusal),
        },
    }
}

/// Register `draft` as `owner`: the same checks first, then the write; after a lost answer,
/// the contract where the owner's nonce puts it, read with proof, or [`RegisterError::Unclear`].
pub async fn register(
    net: &impl Network,
    owner: &Identifier,
    draft: &Draft,
) -> Result<Registered, RegisterError> {
    let (built, fee, before, protocol) = prepare(net, owner, draft).await.map_err(|(_, e)| e)?;
    let nonce = net.nonce(owner).await.map_err(RegisterError::Failed)?;
    let (contract, recovered) = match net.create(owner, &built).await {
        Ok(contract) => (contract, false),
        Err(sent) => match find(net, owner, nonce).await {
            Ok(Some(contract)) => (contract, true),
            Ok(None) => return Err(RegisterError::NotFound { nonce, sent }),
            Err(read) => return Err(RegisterError::Unclear { nonce, sent, read }),
        },
    };
    let contract = Arc::new(contract);
    net.keep(contract.clone());
    let charged = net
        .credits(owner)
        .await
        .ok()
        .map(|after| before.saturating_sub(after));
    Ok(Registered {
        contract,
        fee,
        charged,
        bytes: built.bytes,
        protocol,
        recovered,
    })
}

/// The key, the credits (read first: the protocol comes with them), the draft built at the
/// network's verified protocol, its fee against the credits.
// Private, and its error is taken apart at once by `check` and `register`.
#[allow(clippy::result_large_err)]
async fn prepare(
    net: &impl Network,
    owner: &Identifier,
    draft: &Draft,
) -> Result<(Built, Fee, Credits, u32), (Option<Fee>, RegisterError)> {
    if !net.can_sign(owner) {
        return Err((None, RegisterError::NoKey));
    }
    let credits = net
        .credits(owner)
        .await
        .map_err(|e| (None, RegisterError::Failed(e)))?;
    let refused = |r: Refusal| (None, RegisterError::Refused(r));
    let seen = net.protocol();
    let protocol = seen.verified.ok_or((None, RegisterError::VersionUnknown))?;
    if seen.newer {
        return Err((None, RegisterError::NewerNetwork(protocol)));
    }
    let pv = version_for(protocol).map_err(refused)?;
    let built = build(draft, *owner, pv).map_err(refused)?;
    let fee = fee::estimate(&built, pv).map_err(|e| (None, RegisterError::Failed(e)))?;
    if credits < fee.total() {
        return Err((
            Some(fee),
            RegisterError::NotEnough {
                have: credits,
                need: fee.total(),
            },
        ));
    }
    Ok((built, fee, credits, protocol))
}

/// The owner's contract made with a nonce after `nonce`, if the network has it.
pub async fn find(
    net: &impl Network,
    owner: &Identifier,
    nonce: u64,
) -> Result<Option<DataContract>, String> {
    let ids: Vec<Identifier> = (1..=NONCES_AHEAD)
        .map(|ahead| DataContract::generate_data_contract_id_v0(owner, nonce + ahead))
        .collect();
    Ok(net
        .fetch(&ids)
        .await?
        .into_iter()
        .find(|c| c.owner_id() == *owner && ids.contains(&c.id())))
}
