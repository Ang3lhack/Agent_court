use candid::{CandidType, Principal};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    DefaultMemoryImpl, StableBTreeMap,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::cell::RefCell;

type Memory = VirtualMemory<DefaultMemoryImpl>;

const MISSIONS_MEMORY_ID: MemoryId = MemoryId::new(0);
const ATTEMPTS_MEMORY_ID: MemoryId = MemoryId::new(1);
const LATEST_ATTEMPTS_MEMORY_ID: MemoryId = MemoryId::new(2);
const MAX_MISSIONS: u64 = 100;
const MAX_ATTEMPTS: u64 = 10_000;
const MAX_PAGE_SIZE: u16 = 100;

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    static MISSIONS: RefCell<StableBTreeMap<String, Vec<u8>, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|manager| manager.borrow().get(MISSIONS_MEMORY_ID))
        ));

    static ATTEMPTS: RefCell<StableBTreeMap<String, Vec<u8>, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|manager| manager.borrow().get(ATTEMPTS_MEMORY_ID))
        ));

    static LATEST_ATTEMPTS: RefCell<StableBTreeMap<String, String, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|manager| manager.borrow().get(LATEST_ATTEMPTS_MEMORY_ID))
        ));
}

#[derive(Clone, Debug, CandidType, Deserialize, Serialize)]
pub struct Mission {
    pub mission_id: String,
    pub name: String,
    pub description: String,
    pub criteria_hash: String,
    pub created_at: u64,
    pub registered_by: Principal,
}

#[derive(Clone, Copy, Debug, CandidType, Deserialize, Serialize)]
#[allow(non_camel_case_types)]
pub enum Verdict {
    PASS,
    FAIL,
    ERROR,
    NOT_RUN,
}

#[derive(Clone, Debug, CandidType, Deserialize, Serialize)]
pub struct EvaluationAttempt {
    pub mission_id: String,
    pub attempt_id: String,
    pub previous_attempt_id: Option<String>,
    pub criteria_hash: String,
    pub artifact_hash: String,
    pub report_hash: String,
    pub verdict: Verdict,
    pub timestamp_unix_ms: u64,
    pub registered_at: u64,
    pub registered_by: Principal,
}

#[derive(CandidType, Deserialize)]
pub struct MissionInput {
    mission_id: String,
    name: String,
    description: String,
    criteria_hash: String,
}

#[derive(CandidType, Deserialize)]
pub struct AttemptInput {
    mission_id: String,
    attempt_id: String,
    previous_attempt_id: Option<String>,
    criteria_hash: String,
    artifact_hash: String,
    report_hash: String,
    verdict: Verdict,
    timestamp_unix_ms: u64,
}

#[derive(Debug, CandidType, Deserialize)]
pub enum RegistryError {
    Unauthorized,
    InvalidInput(String),
    AlreadyExists,
    NotFound,
    ChainConflict {
        expected_previous_attempt_id: Option<String>,
    },
    CapacityReached,
    StorageError(String),
}

fn encode<T: CandidType>(value: &T) -> Result<Vec<u8>, RegistryError> {
    candid::encode_one(value).map_err(|error| RegistryError::StorageError(error.to_string()))
}

fn decode<T: CandidType + DeserializeOwned>(bytes: &[u8]) -> Result<T, RegistryError> {
    candid::decode_one(bytes).map_err(|error| RegistryError::StorageError(error.to_string()))
}

fn ensure_controller() -> Result<Principal, RegistryError> {
    let caller = ic_cdk::api::msg_caller();
    if ic_cdk::api::is_controller(&caller) {
        Ok(caller)
    } else {
        Err(RegistryError::Unauthorized)
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn load_mission(mission_id: &str) -> Result<Option<Mission>, RegistryError> {
    MISSIONS.with(|missions| {
        missions
            .borrow()
            .get(&mission_id.to_string())
            .map(|bytes| decode(&bytes))
            .transpose()
    })
}

fn load_attempt(attempt_id: &str) -> Result<Option<EvaluationAttempt>, RegistryError> {
    ATTEMPTS.with(|attempts| {
        attempts
            .borrow()
            .get(&attempt_id.to_string())
            .map(|bytes| decode(&bytes))
            .transpose()
    })
}

fn latest_attempt_id(mission_id: &str) -> Result<Option<String>, RegistryError> {
    Ok(LATEST_ATTEMPTS.with(|latest| latest.borrow().get(&mission_id.to_string())))
}

fn validate_previous_attempt(
    provided: &Option<String>,
    expected: Option<String>,
) -> Result<(), RegistryError> {
    if *provided == expected {
        Ok(())
    } else {
        Err(RegistryError::ChainConflict {
            expected_previous_attempt_id: expected,
        })
    }
}

#[ic_cdk::init]
fn init() {}

#[ic_cdk::post_upgrade]
fn post_upgrade() {
    // StableBTreeMap is restored directly from stable memory.
}

#[ic_cdk::update]
fn register_mission(input: MissionInput) -> Result<Mission, RegistryError> {
    let caller = ensure_controller()?;
    if !valid_id(&input.mission_id)
        || input.name.trim().is_empty()
        || input.name.len() > 120
        || input.description.len() > 512
        || !valid_hash(&input.criteria_hash)
    {
        return Err(RegistryError::InvalidInput(
            "invalid mission ID, name, description, or SHA-256 criteria hash".into(),
        ));
    }

    if load_mission(&input.mission_id)?.is_some() {
        return Err(RegistryError::AlreadyExists);
    }
    if MISSIONS.with(|missions| missions.borrow().len()) >= MAX_MISSIONS {
        return Err(RegistryError::CapacityReached);
    }

    let mission = Mission {
        mission_id: input.mission_id.clone(),
        name: input.name,
        description: input.description,
        criteria_hash: input.criteria_hash,
        created_at: ic_cdk::api::time(),
        registered_by: caller,
    };
    let bytes = encode(&mission)?;
    MISSIONS.with(|missions| {
        missions.borrow_mut().insert(input.mission_id, bytes);
    });
    Ok(mission)
}

#[ic_cdk::update]
fn register_attempt(input: AttemptInput) -> Result<EvaluationAttempt, RegistryError> {
    let caller = ensure_controller()?;
    let Some(mission) = load_mission(&input.mission_id)? else {
        return Err(RegistryError::NotFound);
    };
    if !valid_id(&input.attempt_id)
        || !valid_hash(&input.criteria_hash)
        || !valid_hash(&input.artifact_hash)
        || !valid_hash(&input.report_hash)
    {
        return Err(RegistryError::InvalidInput(
            "invalid attempt ID or SHA-256 hash".into(),
        ));
    }
    if input.criteria_hash != mission.criteria_hash {
        return Err(RegistryError::InvalidInput(
            "attempt criteria hash does not match mission".into(),
        ));
    }
    if load_attempt(&input.attempt_id)?.is_some() {
        return Err(RegistryError::AlreadyExists);
    }
    if ATTEMPTS.with(|attempts| attempts.borrow().len()) >= MAX_ATTEMPTS {
        return Err(RegistryError::CapacityReached);
    }

    validate_previous_attempt(
        &input.previous_attempt_id,
        latest_attempt_id(&input.mission_id)?,
    )?;

    let attempt = EvaluationAttempt {
        mission_id: input.mission_id,
        attempt_id: input.attempt_id,
        previous_attempt_id: input.previous_attempt_id,
        criteria_hash: input.criteria_hash,
        artifact_hash: input.artifact_hash,
        report_hash: input.report_hash,
        verdict: input.verdict,
        timestamp_unix_ms: input.timestamp_unix_ms,
        registered_at: ic_cdk::api::time(),
        registered_by: caller,
    };
    let bytes = encode(&attempt)?;
    ATTEMPTS.with(|attempts| {
        attempts
            .borrow_mut()
            .insert(attempt.attempt_id.clone(), bytes);
    });
    LATEST_ATTEMPTS.with(|latest| {
        latest
            .borrow_mut()
            .insert(attempt.mission_id.clone(), attempt.attempt_id.clone());
    });
    Ok(attempt)
}

#[ic_cdk::query]
fn get_mission(mission_id: String) -> Result<Option<Mission>, RegistryError> {
    load_mission(&mission_id)
}

#[ic_cdk::query]
fn get_attempt(attempt_id: String) -> Result<Option<EvaluationAttempt>, RegistryError> {
    load_attempt(&attempt_id)
}

#[ic_cdk::query]
fn list_missions(start_after: Option<String>, limit: u16) -> Result<Vec<Mission>, RegistryError> {
    validate_page_size(limit)?;
    let mut result = Vec::new();
    MISSIONS.with(|missions| {
        for entry in missions.borrow().iter() {
            let mission_id = entry.key();
            let bytes = entry.value();
            if start_after
                .as_deref()
                .is_some_and(|cursor| mission_id.as_str() <= cursor)
            {
                continue;
            }
            result.push(decode(&bytes)?);
            if result.len() >= usize::from(limit) {
                break;
            }
        }
        Ok::<(), RegistryError>(())
    })?;
    Ok(result)
}

#[ic_cdk::query]
fn list_attempts(
    mission_id: String,
    start_after: Option<String>,
    limit: u16,
) -> Result<Vec<EvaluationAttempt>, RegistryError> {
    validate_page_size(limit)?;
    let mut result = Vec::new();
    ATTEMPTS.with(|attempts| {
        for entry in attempts.borrow().iter() {
            let bytes = entry.value();
            let attempt: EvaluationAttempt = decode(&bytes)?;
            if attempt.mission_id == mission_id {
                result.push(attempt);
            }
        }
        Ok::<(), RegistryError>(())
    })?;
    result.sort_by_key(|attempt| (attempt.timestamp_unix_ms, attempt.attempt_id.clone()));
    if let Some(cursor) = start_after {
        let cursor_index = result
            .iter()
            .position(|attempt| attempt.attempt_id == cursor);
        if let Some(cursor_index) = cursor_index {
            result.drain(..=cursor_index);
        } else {
            return Err(RegistryError::InvalidInput(
                "attempt cursor is not part of this mission".into(),
            ));
        }
    }
    result.truncate(usize::from(limit));
    Ok(result)
}

fn validate_page_size(limit: u16) -> Result<(), RegistryError> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        Err(RegistryError::InvalidInput(format!(
            "page size must be between 1 and {MAX_PAGE_SIZE}"
        )))
    } else {
        Ok(())
    }
}

ic_cdk::export_candid!();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_bounded_slug_ids_and_sha256_hashes() {
        assert!(valid_id("counter-v1"));
        assert!(!valid_id("../counter"));
        assert!(!valid_id(&"x".repeat(129)));
        assert!(valid_hash(&"a".repeat(64)));
        assert!(!valid_hash(&"z".repeat(64)));
        assert!(!valid_hash("short"));
    }

    #[test]
    fn attempt_candid_encoding_round_trips_metadata() {
        let attempt = EvaluationAttempt {
            mission_id: "counter-v1".into(),
            attempt_id: "attempt-1".into(),
            previous_attempt_id: None,
            criteria_hash: "a".repeat(64),
            artifact_hash: "b".repeat(64),
            report_hash: "c".repeat(64),
            verdict: Verdict::PASS,
            timestamp_unix_ms: 1_800_000_000_000,
            registered_at: 1_800_000_000_000_000_000,
            registered_by: Principal::anonymous(),
        };
        let bytes = encode(&attempt).unwrap();
        let decoded: EvaluationAttempt = decode(&bytes).unwrap();
        assert_eq!(decoded.attempt_id, attempt.attempt_id);
        assert!(matches!(decoded.verdict, Verdict::PASS));
    }

    #[test]
    fn previous_attempt_must_extend_the_current_tip() {
        assert!(validate_previous_attempt(&None, None).is_ok());
        assert!(validate_previous_attempt(&Some("a1".into()), Some("a1".into())).is_ok());
        assert!(matches!(
            validate_previous_attempt(&None, Some("a1".into())),
            Err(RegistryError::ChainConflict { .. })
        ));
    }
}
