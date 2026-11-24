use std::collections::BTreeMap;

use crate::sml_engine::MasternodeListEngine;
use crate::storage::BlockHeaderStorage;
use dashcore::bls_sig_utils::BlsScheme;
use dashcore::network::message_qrinfo::QuorumSnapshot;
use dashcore::sml::llmq_entry_verification::LLMQEntryVerificationStatus;
use dashcore::sml::llmq_type::QUORUM_MEMBER_LIST_OFFSET;
use dashcore::sml::masternode_list_entry::qualified_masternode_list_entry::QualifiedMasternodeListEntry;
use dashcore::sml::quorum_entry::qualified_quorum_entry::{
    QualifiedQuorumEntry, VerifyingChainLockSignaturesType,
};
use dashcore::sml::quorum_entry::quorum_modifier_type::LLMQModifierType;
use dashcore::sml::quorum_validation_error::QuorumValidationError;
use dashcore::{BlockHash, QuorumHash};

impl<H: BlockHeaderStorage> MasternodeListEngine<H> {
    /// Validates a non-rotating quorum against the members the list at its
    /// work block selects (DIP-6).
    pub(super) async fn validate_quorum(
        &self,
        quorum: &QualifiedQuorumEntry,
    ) -> Result<(), QuorumValidationError> {
        quorum.quorum_entry.validate_structure()?;
        let quorum_hash = quorum.quorum_entry.quorum_hash;
        let height = self.height_if_known(&quorum_hash).await.ok_or_else(|| {
            QuorumValidationError::RequiredBlockNotPresent(quorum_hash, "quorum height".to_string())
        })?;
        let work_height = height.saturating_sub(QUORUM_MEMBER_LIST_OFFSET);
        let masternode_list = self
            .masternode_lists
            .get(&work_height)
            .ok_or(QuorumValidationError::RequiredMasternodeListNotPresent(work_height))?;
        let Some(VerifyingChainLockSignaturesType::NonRotating(chain_lock_sig)) =
            quorum.verifying_chain_lock_signature
        else {
            return Err(QuorumValidationError::RequiredChainLockNotPresent(
                work_height,
                masternode_list.block_hash,
            ));
        };
        let quorum_modifier_type = LLMQModifierType::new_quorum_modifier_type(
            quorum.quorum_entry.llmq_type,
            masternode_list.block_hash,
            work_height,
            chain_lock_sig,
            self.network,
        )?;
        let members: Vec<_> = masternode_list.valid_masternodes_for_quorum(
            quorum,
            quorum_modifier_type,
            self.network,
        );
        validate_signed_by(quorum, &members)
    }

    /// The status of each rotated quorum, by quorum hash.
    pub(super) async fn rotated_quorum_statuses(
        &self,
        quorums: &[&QualifiedQuorumEntry],
        snapshots: &BTreeMap<BlockHash, QuorumSnapshot>,
    ) -> BTreeMap<QuorumHash, LLMQEntryVerificationStatus> {
        let members = self.find_rotated_masternodes_for_quorums(quorums, snapshots).await;
        quorums
            .iter()
            .zip(members)
            .map(|(quorum, members)| {
                let quorum_hash = quorum.quorum_entry.quorum_hash;
                let result = quorum.quorum_entry.validate_structure().and_then(|()| {
                    if !quorum.quorum_entry.llmq_type.is_rotating_quorum_type() {
                        return Err(QuorumValidationError::ExpectedOnlyRotatedQuorums(
                            quorum_hash,
                            quorum.quorum_entry.llmq_type,
                        ));
                    }
                    validate_signed_by(quorum, &members?)
                });
                let status = match result {
                    Ok(()) => LLMQEntryVerificationStatus::Verified,
                    Err(e) => e.into(),
                };
                (quorum_hash, status)
            })
            .collect()
    }
}

/// Checks `quorum`'s signatures against the `members` that signed it.
///
/// As in Dash Core's `CFinalCommitment::Verify`, a bit past the members names
/// no masternode and must be clear: a signer bit there would count toward the
/// threshold while no key of it enters the aggregate check.
///
/// A masternode, keyed by its ProRegTx hash as Dash Core keys quorum members,
/// signs at most once: rebuilt from a peer's snapshots, the members can name
/// one masternode in many slots, and each of its signer bits would count
/// toward the threshold on that one operator key.
fn validate_signed_by(
    quorum: &QualifiedQuorumEntry,
    members: &[&QualifiedMasternodeListEntry],
) -> Result<(), QuorumValidationError> {
    let entry = &quorum.quorum_entry;
    let members_count = members.len();
    if let Some(index) = entry.signers.iter().skip(members_count).position(|&bit| bit) {
        return Err(QuorumValidationError::SignerBeyondMembers {
            index: members_count + index,
            members_count,
        });
    }
    if let Some(index) = entry.valid_members.iter().skip(members_count).position(|&bit| bit) {
        return Err(QuorumValidationError::ValidMemberBeyondMembers {
            index: members_count + index,
            members_count,
        });
    }
    let mut signed_at = BTreeMap::new();
    for (index, member) in members.iter().enumerate() {
        if entry.signers.get(index) != Some(&true) {
            continue;
        }
        let pro_tx_hash = member.masternode_list_entry.pro_reg_tx_hash;
        if let Some(first_index) = signed_at.insert(pro_tx_hash, index) {
            return Err(QuorumValidationError::DuplicateSigner {
                index,
                first_index,
            });
        }
    }
    let required = entry.llmq_type.threshold() as u64;
    let found = entry.signers.iter().take(members_count).filter(|&&signed| signed).count() as u64;
    if found < required {
        return Err(QuorumValidationError::InsufficientSigners {
            required,
            found,
        });
    }
    let signers = members.iter().enumerate().filter_map(|(i, member)| {
        quorum.quorum_entry.signers.get(i)?.then_some(&member.masternode_list_entry)
    });
    quorum.validate(signers, BlsScheme::Modern)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use dashcore::hashes::Hash;

    use super::*;
    use crate::sml_engine::test_support::{quorum_entry, TestEngine};
    use dashcore::bls_sig_utils::{BLSPublicKey, BLSSignature};
    use dashcore::hash_types::ConfirmedHash;
    use dashcore::sml::llmq_entry_verification::LLMQEntryVerificationStatus;
    use dashcore::sml::llmq_type::LLMQType;
    use dashcore::sml::masternode_list_entry::{
        EntryMasternodeType, MasternodeListEntry, MasternodeNetInfo,
    };
    use dashcore::sml::quorum_entry::qualified_quorum_entry::QualifiedQuorumEntry;
    use dashcore::Network;
    use dashcore::{ProTxHash, PubkeyHash};

    fn member(byte: u8) -> QualifiedMasternodeListEntry {
        MasternodeListEntry {
            version: 2,
            pro_reg_tx_hash: ProTxHash::from_byte_array([byte; 32]),
            confirmed_hash: Some(ConfirmedHash::from_byte_array([byte; 32])),
            service_address: MasternodeNetInfo::Legacy(SocketAddr::from(([127, 0, 0, 1], 9999))),
            operator_public_key: BLSPublicKey::from([byte; 48]),
            key_id_voting: PubkeyHash::from_byte_array([byte; 20]),
            is_valid: true,
            mn_type: EntryMasternodeType::Regular,
        }
        .into()
    }

    /// A 60_75 commitment whose bitsets span the type's size with these bits
    /// set, and whose keys and signatures are not zeroed.
    fn commitment(signers: &[usize], valid_members: &[usize]) -> QualifiedQuorumEntry {
        let llmq_type = LLMQType::Llmqtype60_75;
        let size = llmq_type.size() as usize;
        let bits = |set: &[usize]| (0..size).map(|i| set.contains(&i)).collect::<Vec<bool>>();
        let mut entry = quorum_entry(llmq_type, QuorumHash::from_byte_array([1; 32]), Some(0));
        entry.signers = bits(signers);
        entry.valid_members = bits(valid_members);
        entry.quorum_public_key = BLSPublicKey::from([1; 48]);
        entry.threshold_sig = BLSSignature::from([1; 96]);
        entry.all_commitment_aggregated_signature = BLSSignature::from([1; 96]);
        entry.into()
    }

    /// Members rebuilt short, 16 of 60, with one member's bit set and the rest
    /// of the threshold set past the members. The structure check counts all
    /// 45; only the one member's key would enter the aggregate check.
    #[test]
    fn signer_bits_past_the_members_are_refused() {
        let entries: Vec<QualifiedMasternodeListEntry> = (1..=16).map(member).collect();
        let members: Vec<&QualifiedMasternodeListEntry> = entries.iter().collect();
        let bits: Vec<usize> = std::iter::once(0).chain(16..60).collect();
        let quorum = commitment(&bits, &bits);
        assert!(quorum.quorum_entry.validate_structure().is_ok(), "it counts every set bit");

        assert_eq!(
            validate_signed_by(&quorum, &members),
            Err(QuorumValidationError::SignerBeyondMembers {
                index: 16,
                members_count: 16,
            }),
        );
    }

    #[test]
    fn valid_member_bits_past_the_members_are_refused() {
        let entries: Vec<QualifiedMasternodeListEntry> = (1..=50).map(member).collect();
        let members: Vec<&QualifiedMasternodeListEntry> = entries.iter().collect();
        let signers: Vec<usize> = (0..45).collect();
        let valid_members: Vec<usize> = (0..51).collect();
        let quorum = commitment(&signers, &valid_members);

        assert_eq!(
            validate_signed_by(&quorum, &members),
            Err(QuorumValidationError::ValidMemberBeyondMembers {
                index: 50,
                members_count: 50,
            }),
        );
    }

    #[test]
    fn signers_among_the_members_must_reach_the_threshold() {
        let entries: Vec<QualifiedMasternodeListEntry> = (1..=60).map(member).collect();
        let members: Vec<&QualifiedMasternodeListEntry> = entries.iter().collect();
        let signers: Vec<usize> = (0..44).collect();
        let quorum = commitment(&signers, &signers);

        assert_eq!(
            validate_signed_by(&quorum, &members),
            Err(QuorumValidationError::InsufficientSigners {
                required: 45,
                found: 44,
            }),
        );
    }

    /// Snapshots that name one masternode in every slot of the three older
    /// quarters rebuild it 45 times. Its signer bits there reach the threshold
    /// with no bit past the members, all on its one operator key.
    #[test]
    fn a_masternode_signing_at_more_than_one_index_is_refused() {
        let repeated = member(1);
        let others: Vec<QualifiedMasternodeListEntry> = (2..=16).map(member).collect();
        let members: Vec<&QualifiedMasternodeListEntry> =
            std::iter::repeat_n(&repeated, 45).chain(&others).collect();
        let signers: Vec<usize> = (0..45).collect();
        let quorum = commitment(&signers, &signers);
        assert!(quorum.quorum_entry.validate_structure().is_ok(), "it counts every set bit");

        assert_eq!(
            validate_signed_by(&quorum, &members),
            Err(QuorumValidationError::DuplicateSigner {
                index: 1,
                first_index: 0,
            }),
        );
    }

    /// A list too small to fill the quorum leaves fewer members than the
    /// type's size, as Dash Core allows. With the bits past them clear, the
    /// commitment goes on to the signature check, here failing on dummy bytes.
    #[test]
    fn fewer_members_than_the_size_with_clear_bits_past_them_reach_the_signature_check() {
        let entries: Vec<QualifiedMasternodeListEntry> = (1..=50).map(member).collect();
        let members: Vec<&QualifiedMasternodeListEntry> = entries.iter().collect();
        let signers: Vec<usize> = (0..45).collect();
        let valid_members: Vec<usize> = (0..50).collect();
        let quorum = commitment(&signers, &valid_members);

        let result = validate_signed_by(&quorum, &members);
        assert!(
            matches!(
                result,
                Err(QuorumValidationError::AllCommitmentAggregatedSignatureNotValid(_))
            ),
            "expected the signature check to be reached, got {result:?}",
        );
    }

    fn rotating_quorum(
        quorum_hash: QuorumHash,
        quorum_index: i16,
        valid_structure: bool,
    ) -> QualifiedQuorumEntry {
        let mut entry =
            quorum_entry(LLMQType::LlmqtypeTestDIP0024, quorum_hash, Some(quorum_index));
        if valid_structure {
            entry.signers = vec![true; 4];
            entry.valid_members = vec![true; 4];
            entry.quorum_public_key = BLSPublicKey::from([1; 48]);
            entry.threshold_sig = BLSSignature::from([1; 96]);
            entry.all_commitment_aggregated_signature = BLSSignature::from([1; 96]);
        }
        entry.into()
    }

    #[tokio::test]
    async fn rotation_cycle_statuses_classify_infra_error_as_skipped_and_preserve_invalid() {
        let engine = TestEngine::empty(Network::Testnet);

        let broken_hash = QuorumHash::from_byte_array([1; 32]);
        let unknown_hash = QuorumHash::from_byte_array([2; 32]);
        let second_unknown_hash = QuorumHash::from_byte_array([3; 32]);
        let broken = rotating_quorum(broken_hash, 0, false);
        let unknown_block = rotating_quorum(unknown_hash, 1, true);
        let second_unknown_block = rotating_quorum(second_unknown_hash, 2, true);

        let statuses = engine
            .rotated_quorum_statuses(
                &[&broken, &unknown_block, &second_unknown_block],
                &BTreeMap::new(),
            )
            .await;

        assert!(
            matches!(statuses.get(&broken_hash), Some(LLMQEntryVerificationStatus::Invalid(_))),
            "structurally-broken quorum must keep an Invalid status, got {:?}",
            statuses.get(&broken_hash),
        );
        for hash in [unknown_hash, second_unknown_hash] {
            assert!(
                matches!(statuses.get(&hash), Some(LLMQEntryVerificationStatus::Skipped(_))),
                "each unresolvable quorum must surface as Skipped on its own, got {:?} for {:?}",
                statuses.get(&hash),
                hash,
            );
        }
    }
}
