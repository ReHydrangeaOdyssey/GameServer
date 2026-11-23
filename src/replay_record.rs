use crate::{
    protocol::game::{guildbattle::replay as r, privateapi as q},
    rpc::Failure,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use prost::Message;
use sha2::{Digest, Sha256};
const VERSION: u8 = 2;
const EVENT_PREFIX: &[u8] = b"GB-EVENT-V2\0";
const RECORD_PREFIX: &[u8] = b"GB-LOCAL-RECORD-V2\0";
const SIGN_PREFIX: &[u8] = b"GB-LOCAL-SIGN-V2\0";
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub version: u8,
    pub record_kind: u8,
    pub instance_id: String,
    pub signing_key_id: String,
    pub guild_battle_id: u64,
    pub event_index: u64,
    pub operation_id: String,
    pub envelope_wire: String,
    pub prev_event_hash: String,
    pub event_hash: String,
    pub private_api_method: String,
    pub private_request_wire: String,
    pub record_hash: String,
    pub signature: String,
}
impl Record {
    pub fn sign(&mut self, key: &SigningKey) -> Result<(), Failure> {
        self.version = VERSION;
        let (event, record) = self.bytes()?;
        self.event_hash = hex(&Sha256::digest(event));
        self.record_hash = hex(&Sha256::digest(record));
        let mut signed = SIGN_PREFIX.to_vec();
        signed.extend_from_slice(&unhex::<32>(&self.record_hash)?);
        self.signature = hex(&key.sign(&signed).to_bytes());
        Ok(())
    }
    pub fn verify(
        &self,
        key: &VerifyingKey,
        expected_index: u64,
        previous: [u8; 32],
    ) -> Result<(), Failure> {
        if self.event_index != expected_index || unhex::<32>(&self.prev_event_hash)? != previous {
            return Err(Failure::CONFLICT);
        }
        let (event, record) = self.bytes()?;
        let event_hash: [u8; 32] = Sha256::digest(event).into();
        let record_hash: [u8; 32] = Sha256::digest(record).into();
        if event_hash != unhex::<32>(&self.event_hash)?
            || record_hash != unhex::<32>(&self.record_hash)?
        {
            return Err(Failure::CONFLICT);
        }
        let mut signed = SIGN_PREFIX.to_vec();
        signed.extend_from_slice(&unhex::<32>(&self.record_hash)?);
        key.verify_strict(
            &signed,
            &Signature::from_bytes(&unhex::<64>(&self.signature)?),
        )
        .map_err(|_| Failure::CONFLICT)
    }
    fn bytes(&self) -> Result<(Vec<u8>, Vec<u8>), Failure> {
        if self.version != VERSION
            || !(1..=3).contains(&self.record_kind)
            || self.event_index == 0
            || self.guild_battle_id == 0
            || !safe_name(&self.instance_id)
            || !safe_name(&self.signing_key_id)
        {
            return Err(Failure::BAD_REQUEST);
        }
        let operation =
            uuid::Uuid::parse_str(&self.operation_id).map_err(|_| Failure::BAD_REQUEST)?;
        if operation.is_nil() || operation.to_string() != self.operation_id {
            return Err(Failure::BAD_REQUEST);
        }
        let wire = URL_SAFE_NO_PAD
            .decode(&self.envelope_wire)
            .map_err(|_| Failure::BAD_REQUEST)?;
        let request = URL_SAFE_NO_PAD
            .decode(&self.private_request_wire)
            .map_err(|_| Failure::BAD_REQUEST)?;
        let envelope = canonical::<r::GuildBattleReplayEnvelope>(&wire)?;
        if envelope.event_index != self.event_index {
            return Err(Failure::BAD_REQUEST);
        }
        validate_request(
            &envelope,
            &self.private_api_method,
            &request,
            &self.instance_id,
            self.guild_battle_id,
        )?;
        let previous = unhex::<32>(&self.prev_event_hash)?;
        if (self.event_index == 1 && previous != [0; 32])
            || (self.event_index > 1 && previous == [0; 32])
        {
            return Err(Failure::BAD_REQUEST);
        }
        let event = event_bytes(
            self.guild_battle_id,
            self.event_index,
            &self.operation_id,
            &wire,
            &previous,
        )?;
        let mut record = RECORD_PREFIX.to_vec();
        record.push(self.record_kind);
        append(&mut record, self.instance_id.as_bytes())?;
        append(&mut record, self.signing_key_id.as_bytes())?;
        append(&mut record, &event)?;
        append(&mut record, self.private_api_method.as_bytes())?;
        append(&mut record, &request)?;
        Ok((event, record))
    }
}
pub fn event_bytes(
    battle: u64,
    index: u64,
    operation: &str,
    envelope: &[u8],
    previous: &[u8; 32],
) -> Result<Vec<u8>, Failure> {
    let mut bytes = EVENT_PREFIX.to_vec();
    bytes.extend_from_slice(&battle.to_be_bytes());
    bytes.extend_from_slice(&index.to_be_bytes());
    append(&mut bytes, operation.as_bytes())?;
    append(&mut bytes, envelope)?;
    bytes.extend_from_slice(previous);
    Ok(bytes)
}
pub fn canonical<M: Message + Default>(bytes: &[u8]) -> Result<M, Failure> {
    if bytes.len() > crate::rpc::MAX_BODY {
        return Err(Failure::BAD_REQUEST);
    }
    let message = M::decode(bytes).map_err(|_| Failure::BAD_REQUEST)?;
    // prost emits fields in tag order; equality rejects unknown fields and noncanonical wire encodings.
    if message.encode_to_vec() != bytes {
        return Err(Failure::BAD_REQUEST);
    }
    Ok(message)
}
fn append(target: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Failure> {
    let length = u32::try_from(bytes.len()).map_err(|_| Failure::BAD_REQUEST)?;
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(bytes);
    Ok(())
}
fn validate_request(
    envelope: &r::GuildBattleReplayEnvelope,
    method: &str,
    bytes: &[u8],
    instance: &str,
    battle: u64,
) -> Result<(), Failure> {
    use r::guild_battle_replay_envelope::Payload;
    macro_rules! check {
        ($request:ty,$kind:literal,$payload:expr) => {{
            let request = canonical::<$request>(bytes)?;
            let payload = $payload;
            if envelope.process_type != $kind
                || request.event_index != envelope.event_index
                || payload.guild_battle_id != battle
                || request.payload.as_ref() != Some(payload)
                || envelope.event_index < 2
            {
                return Err(Failure::BAD_REQUEST);
            }
        }};
    }
    match (method, envelope.payload.as_ref()) {
        ("StartGuildBattle", Some(Payload::Create(payload))) => {
            let request = canonical::<q::StartGuildBattleRequest>(bytes)?;
            if envelope.process_type != 0
                || envelope.event_index != 1
                || payload.guild_battle_id != battle
                || request.game_server_instance_id != instance
                || request.create_log.as_ref() != Some(payload)
            {
                return Err(Failure::BAD_REQUEST);
            }
        }
        ("SaveGuildBattleSortieLog", Some(Payload::Sortie(p))) => {
            check!(q::SaveGuildBattleSortieLogRequest, 1, p)
        }
        ("SaveGuildBattleTacticsLog", Some(Payload::Tactics(p))) => {
            check!(q::SaveGuildBattleTacticsLogRequest, 2, p)
        }
        ("SaveGuildBattleHealLog", Some(Payload::Heal(p))) => {
            check!(q::SaveGuildBattleHealLogRequest, 3, p)
        }
        ("SaveGuildBattleReviveLog", Some(Payload::Revive(p))) => {
            check!(q::SaveGuildBattleReviveLogRequest, 4, p)
        }
        ("SaveGuildBattleItemLog", Some(Payload::Item(p))) => {
            check!(q::SaveGuildBattleItemLogRequest, 5, p)
        }
        ("SaveGuildBattleJoinLog", Some(Payload::Join(p))) => {
            check!(q::SaveGuildBattleJoinLogRequest, 6, p)
        }
        ("SaveGuildBattleTavernRecoveryLog", Some(Payload::TavernRecovery(p))) => {
            check!(q::SaveGuildBattleTavernRecoveryLogRequest, 7, p)
        }
        _ => return Err(Failure::BAD_REQUEST),
    }
    Ok(())
}
pub fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn unhex<const N: usize>(value: &str) -> Result<[u8; N], Failure> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Failure::BAD_REQUEST);
    }
    let mut bytes = [0u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| Failure::BAD_REQUEST)?;
        bytes[index] = u8::from_str_radix(text, 16).map_err(|_| Failure::BAD_REQUEST)?;
    }
    Ok(bytes)
}
