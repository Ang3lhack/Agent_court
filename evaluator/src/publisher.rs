//! Publish immutable, verified metadata; this module never writes local evidence.
use candid::{CandidType, Principal};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::Path,
};

use crate::{
    evidence::{self, EvidenceRecord, Verification},
    Status,
};

mod icp;
pub use icp::IcpCli;

#[derive(Clone, Copy, Debug, CandidType, Deserialize, PartialEq, Eq)]
// These labels must exactly match backend/backend.did.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
pub enum Verdict {
    PASS,
    FAIL,
    ERROR,
    NOT_RUN,
}

#[derive(Clone, Debug, CandidType, Deserialize, PartialEq, Eq)]
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

impl AttemptInput {
    fn from_record(record: EvidenceRecord) -> Result<Self, PublishError> {
        let input = Self {
            mission_id: record.mission_id,
            attempt_id: record.attempt_id,
            previous_attempt_id: record.previous_attempt_id,
            criteria_hash: record.criteria_hash,
            artifact_hash: record.artifact_hash,
            report_hash: record.report_hash,
            verdict: match record.verdict {
                Status::Pass => Verdict::PASS,
                Status::Fail => Verdict::FAIL,
                Status::Error => Verdict::ERROR,
                Status::NotRun => Verdict::NOT_RUN,
            },
            timestamp_unix_ms: u64::try_from(record.timestamp_unix_ms).map_err(|_| {
                PublishError::Validation("timestamp_unix_ms fuera del rango nat64".into())
            })?,
        };
        input.validate()?;
        Ok(input)
    }

    fn validate(&self) -> Result<(), PublishError> {
        if !evidence::valid_id(&self.mission_id)
            || !evidence::valid_id(&self.attempt_id)
            || self
                .previous_attempt_id
                .as_deref()
                .is_some_and(|id| !evidence::valid_id(id))
            || [&self.criteria_hash, &self.artifact_hash, &self.report_hash]
                .iter()
                .any(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(PublishError::Validation("ID o SHA-256 inválido".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, CandidType, Deserialize)]
pub struct Mission {
    mission_id: String,
    name: String,
    description: String,
    criteria_hash: String,
    created_at: u64,
    registered_by: Principal,
}

#[derive(Clone, Debug, CandidType, Deserialize, PartialEq, Eq)]
pub struct RemoteAttempt {
    mission_id: String,
    attempt_id: String,
    previous_attempt_id: Option<String>,
    criteria_hash: String,
    artifact_hash: String,
    report_hash: String,
    verdict: Verdict,
    timestamp_unix_ms: u64,
    registered_at: u64,
    registered_by: Principal,
}

impl RemoteAttempt {
    fn original(&self) -> AttemptInput {
        AttemptInput {
            mission_id: self.mission_id.clone(),
            attempt_id: self.attempt_id.clone(),
            previous_attempt_id: self.previous_attempt_id.clone(),
            criteria_hash: self.criteria_hash.clone(),
            artifact_hash: self.artifact_hash.clone(),
            report_hash: self.report_hash.clone(),
            verdict: self.verdict,
            timestamp_unix_ms: self.timestamp_unix_ms,
        }
    }
}

#[derive(Clone, Debug, CandidType, Deserialize, PartialEq, Eq)]
pub enum RegistryError {
    Unauthorized,
    NotFound,
    AlreadyExists,
    ChainConflict {
        expected_previous_attempt_id: Option<String>,
    },
    InvalidInput(String),
    CapacityReached,
    StorageError(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliFailure {
    Connection,
    InvalidExpiry,
    IdentityUnavailable,
    Process,
    InteractiveProcess,
}

#[derive(Debug)]
pub enum PublishError {
    Validation(String),
    Registry(RegistryError),
    Cli(CliFailure),
    InvalidResponse(String),
    Conflict(String),
    ChainConflict {
        provided: Option<String>,
        expected: Option<String>,
    },
    Unconfirmed(String),
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message) | Self::InvalidResponse(message) | Self::Conflict(message) => f.write_str(message),
            Self::Registry(error) => match error {
                RegistryError::Unauthorized => f.write_str("Unauthorized: la identidad debe ser controladora del canister"),
                RegistryError::NotFound => f.write_str("NotFound: la misión o el recurso no existe"),
                RegistryError::AlreadyExists => f.write_str("AlreadyExists: el ID existe pero no pudo confirmarse como idéntico"),
                RegistryError::ChainConflict { expected_previous_attempt_id } => write!(f, "ChainConflict del canister: predecesor esperado {expected_previous_attempt_id:?}; no se modifica la evidencia"),
                RegistryError::InvalidInput(message) => write!(f, "InvalidInput: {message}"),
                RegistryError::CapacityReached => f.write_str("CapacityReached: el registro alcanzó su capacidad"),
                RegistryError::StorageError(message) => write!(f, "StorageError: {message}"),
            },
            Self::Cli(kind) => f.write_str(match kind {
                CliFailure::Connection => "error de conexión con ICP local; comprueba la red existente",
                CliFailure::InvalidExpiry => "Invalid request expiry: comprueba la sincronización del reloj del host/WSL y de la réplica; no se altera el timestamp de la evidencia",
                CliFailure::IdentityUnavailable => "identidad o contraseña no disponible: ejecuta publish desde una terminal interactiva y utiliza el desbloqueo oficial de icp-cli",
                CliFailure::Process => "icp-cli falló o no está disponible; comprueba icp --version y la configuración local",
                CliFailure::InteractiveProcess => "icp-cli terminó con error; revisa el diagnóstico mostrado en la terminal. Si indica identidad/contraseña, utiliza el desbloqueo oficial; si indica Invalid request expiry, comprueba el reloj del host/WSL y la réplica; si indica conexión, comprueba la red local existente",
            }),
            Self::ChainConflict { provided, expected } => write!(f, "ChainConflict: predecesor local {provided:?}, esperado en ICP {expected:?}; publica en orden o genera un intento en un almacén independiente vacío, sin reescribir este intento"),
            Self::Unconfirmed(message) => write!(f, "estado de publicación sin confirmar: {message}; no se reenvía automáticamente; vuelve a ejecutar publish con el mismo ID para reconciliar antes de escribir"),
        }
    }
}

pub trait Registry {
    fn get_mission(&mut self, id: &str) -> Result<Option<Mission>, PublishError>;
    fn get_attempt(&mut self, id: &str) -> Result<Option<RemoteAttempt>, PublishError>;
    fn list_attempts(
        &mut self,
        mission: &str,
        cursor: Option<String>,
    ) -> Result<Vec<RemoteAttempt>, PublishError>;
    fn register_attempt(&mut self, input: &AttemptInput) -> Result<RemoteAttempt, PublishError>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Registered,
    AlreadyRegistered,
    ConfirmedAfterUncertainResponse,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Registered => "REGISTRADO",
            Self::AlreadyRegistered => "YA REGISTRADO (sin nueva escritura)",
            Self::ConfirmedAfterUncertainResponse => {
                "REGISTRO CONFIRMADO tras respuesta incierta (sin reenvío)"
            }
        })
    }
}

fn same_original(remote: &RemoteAttempt, input: &AttemptInput) -> Result<(), PublishError> {
    if remote.original() != *input {
        return Err(PublishError::Conflict(format!(
            "conflicto: el intento {} tiene metadatos remotos diferentes",
            input.attempt_id
        )));
    }
    Ok(())
}

// list_attempts sorts by client timestamp, not registration order. Recover the
// actual tip from predecessor links, including when clocks go backwards.
fn remote_tip(
    registry: &mut impl Registry,
    input: &AttemptInput,
) -> Result<Option<String>, PublishError> {
    let mut links = HashMap::new();
    let mut cursor = None;
    loop {
        let page = registry.list_attempts(&input.mission_id, cursor.clone())?;
        let size = page.len();
        if size > 100 {
            return Err(PublishError::InvalidResponse(
                "página ICP mayor que el límite solicitado".into(),
            ));
        }
        for remote in page {
            remote.original().validate()?;
            if remote.mission_id != input.mission_id
                || remote.criteria_hash != input.criteria_hash
                || links
                    .insert(remote.attempt_id.clone(), remote.previous_attempt_id)
                    .is_some()
            {
                return Err(PublishError::InvalidResponse(
                    "historial remoto incoherente o paginación repetida".into(),
                ));
            }
            cursor = Some(remote.attempt_id);
        }
        if links.len() > 10_000 {
            return Err(PublishError::InvalidResponse(
                "historial remoto excede la capacidad del contrato".into(),
            ));
        }
        if size < 100 {
            break;
        }
    }
    if links.is_empty() {
        return Ok(None);
    }
    let mut referenced = HashSet::new();
    let mut roots = 0;
    for previous in links.values() {
        if let Some(previous) = previous {
            if !links.contains_key(previous) || !referenced.insert(previous.clone()) {
                return Err(PublishError::InvalidResponse(
                    "historial remoto con predecesor ausente o bifurcación".into(),
                ));
            }
        } else {
            roots += 1;
        }
    }
    let tips: Vec<_> = links
        .keys()
        .filter(|id| !referenced.contains(*id))
        .cloned()
        .collect();
    if roots != 1 || tips.len() != 1 {
        return Err(PublishError::InvalidResponse(
            "historial remoto sin una cadena única".into(),
        ));
    }
    let tip = tips[0].clone();
    let mut visited = HashSet::new();
    let mut current = Some(tip.clone());
    while let Some(id) = current {
        if !visited.insert(id.clone()) {
            return Err(PublishError::InvalidResponse(
                "ciclo en el historial remoto".into(),
            ));
        }
        current = links[&id].clone();
    }
    if visited.len() != links.len() {
        return Err(PublishError::InvalidResponse(
            "historial remoto desconectado".into(),
        ));
    }
    Ok(Some(tip))
}

pub fn publish(
    root: &Path,
    id: &str,
    registry: &mut impl Registry,
) -> Result<Outcome, PublishError> {
    let record = match evidence::verify_attempt(root, id).map_err(PublishError::Validation)? {
        Verification::Verified(record) => record,
        Verification::Legacy(_) => {
            return Err(PublishError::Validation(
                "registro histórico sin paquete: no verificable ni publicable".into(),
            ))
        }
    };
    let input = AttemptInput::from_record(record)?;
    let mission = registry
        .get_mission(&input.mission_id)?
        .ok_or(PublishError::Registry(RegistryError::NotFound))?;
    if mission.mission_id != input.mission_id {
        return Err(PublishError::InvalidResponse(
            "get_mission devolvió otra misión".into(),
        ));
    }
    if mission.criteria_hash != input.criteria_hash {
        return Err(PublishError::Validation(
            "criteria_hash no coincide con la misión ICP".into(),
        ));
    }
    if let Some(remote) = registry.get_attempt(id)? {
        same_original(&remote, &input)?;
        return Ok(Outcome::AlreadyRegistered);
    }
    let expected = remote_tip(registry, &input)?;
    if input.previous_attempt_id != expected {
        return Err(PublishError::ChainConflict {
            provided: input.previous_attempt_id,
            expected,
        });
    }
    match registry.register_attempt(&input) {
        Ok(response) => {
            same_original(&response, &input)?;
            let confirmed = registry
                .get_attempt(id)
                .map_err(|error| PublishError::Unconfirmed(error.to_string()))?
                .ok_or_else(|| {
                    PublishError::Unconfirmed(
                        "register_attempt respondió Ok pero get_attempt no encuentra el ID".into(),
                    )
                })?;
            same_original(&confirmed, &input)?;
            if confirmed != response {
                return Err(PublishError::InvalidResponse(
                    "get_attempt difiere de la respuesta de register_attempt".into(),
                ));
            }
            Ok(Outcome::Registered)
        }
        Err(PublishError::Registry(RegistryError::AlreadyExists)) => {
            let remote = registry
                .get_attempt(id)?
                .ok_or(PublishError::Registry(RegistryError::AlreadyExists))?;
            same_original(&remote, &input)?;
            Ok(Outcome::AlreadyRegistered)
        }
        // A definite canister rejection, especially ChainConflict, stays an error.
        Err(error @ PublishError::Registry(_)) => Err(error),
        Err(error) => match registry.get_attempt(id) {
            Ok(Some(remote)) => {
                same_original(&remote, &input)?;
                Ok(Outcome::ConfirmedAfterUncertainResponse)
            }
            Ok(None) => Err(PublishError::Unconfirmed(format!(
                "{error}; get_attempt no encuentra el ID"
            ))),
            Err(read_error) => Err(PublishError::Unconfirmed(format!(
                "{error}; get_attempt también falló: {read_error}"
            ))),
        },
    }
}

#[cfg(test)]
mod tests;
