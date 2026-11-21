//! The room a model's picture has in its one write (the architect's «Fit into 19 KB», answer 4):
//! the model's document and its transition are signed as one, at most
//! `max_state_transition_size` = 20 480 bytes, and besides the picture they carry the name (up
//! to 256 bytes of UTF-8), the description (up to 1 120), the type, the supply, the document's
//! and the transition's own fields and the signature. A long name in four-byte letters leaves
//! less than [`PARTS_TOTAL`], so the room is measured, not assumed:
//!
//! room = 20 480 − (the same transition without the picture, with a stand-in signature) − 64,
//! and never more than [`PARTS_TOTAL`]; each part stays within [`FIELD`] (`pack`).
//!
//! The transition is built here by DPP's own constructor, offline: the profile's contract from
//! the module V1 template, the document from the same fields the write sends.

use async_trait::async_trait;
use dpp::ProtocolError;
use dpp::address_funds::AddressWitness;
use dpp::data_contract::accessors::v0::DataContractV0Getters;
use dpp::data_contract::document_type::methods::DocumentTypeV0Methods;
use dpp::document::DocumentV0Getters;
use dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dpp::identity::signer::Signer;
use dpp::identity::{IdentityPublicKey, KeyType, Purpose, SecurityLevel};
use dpp::platform_value::{BinaryData, Value};
use dpp::prelude::Identifier;
use dpp::serialization::PlatformSerializable;
use dpp::state_transition::batch_transition::BatchTransition;
use dpp::state_transition::batch_transition::methods::v0::DocumentsBatchTransitionMethodsV0;
use dpp::version::PlatformVersion;

use super::image::{ImageType, PARTS_TOTAL};
use crate::contract_v1::{build, templates};

/// Platform's `max_state_transition_size`.
pub const MAX_TRANSITION: usize = 20_480;

/// Kept free besides what is measured: the keys of the picture's fields and their lengths.
pub const MARGIN: usize = 64;

/// A signer that signs nothing: 65 bytes, as long as a real ECDSA signature.
#[derive(Debug)]
struct StandIn;

#[async_trait]
impl Signer<IdentityPublicKey> for StandIn {
    async fn sign(
        &self,
        _key: &IdentityPublicKey,
        _data: &[u8],
    ) -> Result<BinaryData, ProtocolError> {
        Ok(BinaryData::new(vec![0x1F; 65]))
    }

    async fn sign_create_witness(
        &self,
        _key: &IdentityPublicKey,
        _data: &[u8],
    ) -> Result<AddressWitness, ProtocolError> {
        Err(ProtocolError::Generic("not for addresses".into()))
    }

    fn can_sign_with(&self, _key: &IdentityPublicKey) -> bool {
        true
    }
}

/// The size of the model's write — its «image» document with these fields and `parts` (none,
/// one «image», or `part0…`) — as the network gets it, signed by a stand-in.
pub async fn model_write_size(
    name: &str,
    description: &str,
    supply: u32,
    image_type: ImageType,
    parts: &[Vec<u8>],
    pv: &PlatformVersion,
) -> Result<usize, String> {
    let owner = Identifier::new([0xA5; 32]);
    let built = build(&templates::nft_collection(None), owner, pv)
        .map_err(|r| format!("the template: {r:?}"))?;
    let contract = built.contract;
    let doc_type = contract
        .document_type_for_name("image")
        .map_err(|e| e.to_string())?;
    let mut fields = vec![
        (Value::Text("name".into()), Value::Text(name.into())),
        (
            Value::Text("description".into()),
            Value::Text(description.into()),
        ),
        (Value::Text("imageType".into()), Value::U8(image_type as u8)),
        (Value::Text("supply".into()), Value::U32(supply)),
    ];
    match parts {
        [] => {}
        [one] => fields.push((Value::Text("image".into()), Value::Bytes(one.clone()))),
        many => {
            for (i, part) in many.iter().enumerate() {
                fields.push((Value::Text(format!("part{i}")), Value::Bytes(part.clone())));
            }
        }
    }
    let document = doc_type
        .create_document_from_data(Value::Map(fields), owner, 1, 1, [7; 32], pv)
        .map_err(|e| e.to_string())?;
    debug_assert_eq!(document.owner_id(), owner);
    let key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
        id: 2,
        purpose: Purpose::AUTHENTICATION,
        security_level: SecurityLevel::HIGH,
        contract_bounds: None,
        key_type: KeyType::ECDSA_SECP256K1,
        read_only: false,
        data: BinaryData::new(vec![0x02; 33]),
        disabled_at: None,
    });
    // A nonce as large as a long-used identity's: its encoding is never shorter than this.
    let nonce = u64::MAX >> 8;
    let transition = BatchTransition::new_document_creation_transition_from_document(
        document, doc_type, [7; 32], &key, nonce, 0, None, &StandIn, pv, None,
    )
    .await
    .map_err(|e| e.to_string())?;
    transition
        .serialize_to_bytes()
        .map(|b| b.len())
        .map_err(|e| e.to_string())
}

/// The room for the picture of a model with these fields: [`MAX_TRANSITION`] less the write
/// without its picture and [`MARGIN`], at most [`PARTS_TOTAL`]. The same number for the scale
/// of W5, the check before the password and «Fit into the record».
pub async fn picture_room(
    name: &str,
    description: &str,
    supply: u32,
    image_type: ImageType,
    pv: &PlatformVersion,
) -> Result<usize, String> {
    let without = model_write_size(name, description, supply, image_type, &[], pv).await?;
    Ok(MAX_TRANSITION
        .saturating_sub(without + MARGIN)
        .min(PARTS_TOTAL))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nft_series::image::FIELD;

    fn pv() -> &'static PlatformVersion {
        PlatformVersion::latest()
    }

    /// Picture bytes of `len` split as `pack` stores them: one field, or parts of [`FIELD`].
    fn parts_of(len: usize) -> Vec<Vec<u8>> {
        let bytes = vec![0xAB; len];
        if len <= FIELD {
            vec![bytes]
        } else {
            bytes.chunks(FIELD).map(<[u8]>::to_vec).collect()
        }
    }

    /// The architect's test: a name of 64 and a description of 280 four-byte letters leave less
    /// than 19 000; a picture exactly that large still makes a write within 20 480.
    #[tokio::test]
    async fn long_four_byte_texts_leave_less_room_and_the_room_still_fits() {
        let name = "𝕏".repeat(64);
        let description = "😀".repeat(280);
        let room = picture_room(&name, &description, 1_000_000, ImageType::Jpeg, pv())
            .await
            .unwrap();
        assert!(room < PARTS_TOTAL, "{room}");
        let size = model_write_size(
            &name,
            &description,
            1_000_000,
            ImageType::Jpeg,
            &parts_of(room),
            pv(),
        )
        .await
        .unwrap();
        assert!(size <= MAX_TRANSITION, "{size} for a picture of {room}");
    }

    /// Short texts: the room is the full 19 000, and a picture of 19 000 in four parts fits.
    #[tokio::test]
    async fn short_texts_have_the_full_room() {
        let room = picture_room("Pin", "A pin", 10, ImageType::Png, pv())
            .await
            .unwrap();
        assert_eq!(room, PARTS_TOTAL);
        let size = model_write_size("Pin", "A pin", 10, ImageType::Png, &parts_of(room), pv())
            .await
            .unwrap();
        assert!(size <= MAX_TRANSITION, "{size}");
    }
}
