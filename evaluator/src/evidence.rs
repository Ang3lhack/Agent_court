use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions, Permissions},
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{sha256, Evaluation, EvaluationReport, Status, REPORT_SCHEMA_VERSION};

pub const EVIDENCE_FORMAT_VERSION: u32 = 1;
const LOG_NAME: &str = "evidence.jsonl";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub mission_id: String,
    pub attempt_id: String,
    pub previous_attempt_id: Option<String>,
    pub criteria_hash: String,
    pub artifact_hash: String,
    pub report_hash: String,
    pub verdict: Status,
    pub timestamp_unix_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_format_version: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    record: EvidenceRecord,
    // Extra checksum for the complete report, including whitespace and report_hash.
    report_file_hash: String,
}

pub enum Verification {
    Verified(EvidenceRecord),
    Legacy(EvidenceRecord),
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "{} must be a regular file, not a symlink or directory",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn history_bytes(root: &Path) -> Result<Vec<u8>, String> {
    let path = root.join(LOG_NAME);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("cannot inspect evidence log: {error}")),
        Ok(_) => read_file(&path),
    }
}

fn parse_history(bytes: &[u8]) -> Result<Vec<EvidenceRecord>, String> {
    let mut records = Vec::new();
    let mut ids = HashSet::new();
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let record: EvidenceRecord = serde_json::from_slice(line).map_err(|error| {
            format!(
                "invalid evidence log at line {}: {error}; history was not repaired",
                index + 1
            )
        })?;
        if !valid_id(&record.attempt_id) || !valid_id(&record.mission_id) {
            return Err(format!("invalid ID in evidence log at line {}", index + 1));
        }
        if !ids.insert(record.attempt_id.clone()) {
            return Err(format!(
                "duplicate attempt ID in evidence log: {}",
                record.attempt_id
            ));
        }
        records.push(record);
    }
    Ok(records)
}

struct WriterLock(File);

impl Drop for WriterLock {
    fn drop(&mut self) {
        // Release the shared file-description lock explicitly. A concurrently
        // spawned child can briefly inherit the FD before close-on-exec; merely
        // closing our handle would keep the lock alive until that child execs.
        let _ = self.0.unlock();
    }
}

fn writer_lock(root: &Path) -> Result<WriterLock, String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("cannot create evidence directory: {error}"))?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".writer.lock"))
        .map_err(|error| format!("cannot open evidence writer lock: {error}"))?;
    lock.try_lock().map_err(|error| {
        format!("cannot acquire evidence writer lock (one writer at a time): {error}")
    })?;
    Ok(WriterLock(lock)) // Process exit still closes the underlying OS handle.
}

fn write_new_file(
    path: &Path,
    bytes: &[u8],
    permissions: Option<Permissions>,
) -> Result<(), String> {
    let mut file = File::create_new(path).map_err(|error| {
        format!(
            "cannot create {} without overwriting: {error}",
            path.display()
        )
    })?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)
            .map_err(|error| format!("cannot preserve log permissions: {error}"))?;
    }
    file.sync_all()
        .map_err(|error| format!("cannot sync {}: {error}", path.display()))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("cannot sync directory {}: {error}", path.display()))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn check_hash(name: &str, bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = sha256(bytes);
    if actual != expected {
        return Err(format!(
            "SHA-256 mismatch for {name}: expected {expected}, actual {actual}"
        ));
    }
    Ok(())
}

fn validate_evaluation(evaluation: &Evaluation) -> Result<(), String> {
    // serde_json::Value cannot represent u128 integers above u64::MAX. Reject
    // them before hash_payload's json! conversion, rather than panic on input.
    if u64::try_from(evaluation.report.timestamp_unix_ms).is_err() {
        return Err("timestamp_unix_ms fuera del rango nat64".into());
    }
    check_hash(
        "criteria.txt",
        &evaluation.criteria,
        &evaluation.report.criteria_hash,
    )?;
    check_hash(
        "artifact.rs",
        &evaluation.artifact,
        &evaluation.report.artifact_hash,
    )?;
    check_hash(
        "report-payload.json",
        &evaluation.report_payload,
        &evaluation.report.report_hash,
    )?;
    let payload: serde_json::Value = serde_json::from_slice(&evaluation.report_payload)
        .map_err(|error| format!("invalid report payload: {error}"))?;
    if evaluation.report.schema_version != REPORT_SCHEMA_VERSION
        || payload != evaluation.report.hash_payload()
    {
        return Err("report does not match its hash payload or supported schema".into());
    }
    Ok(())
}

fn commit_record(
    root: &Path,
    previous_bytes: &[u8],
    record: &EvidenceRecord,
) -> Result<(), String> {
    let mut bytes = previous_bytes.to_vec();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend(
        serde_json::to_vec(record)
            .map_err(|error| format!("cannot serialize evidence record: {error}"))?,
    );
    bytes.push(b'\n');
    let log_path = root.join(LOG_NAME);
    let permissions = fs::metadata(&log_path)
        .ok()
        .map(|metadata| metadata.permissions());
    let pending = root.join(format!(".evidence-{}.pending", record.attempt_id));
    write_new_file(&pending, &bytes, permissions)?;
    // Same directory/filesystem: readers see the whole old or whole new index.
    // No historical bytes are reserialized or removed.
    fs::rename(&pending, &log_path).map_err(|error| {
        format!("cannot commit evidence index: {error}; package remains uncommitted")
    })?;
    sync_directory(root)
}

pub fn record_attempt(root: &Path, evaluation: &Evaluation) -> Result<EvidenceRecord, String> {
    let _lock = writer_lock(root)?;
    let previous_bytes = history_bytes(root)?;
    let records = parse_history(&previous_bytes)?;
    validate_evaluation(evaluation)?;
    if records.iter().any(|record| {
        record
            .evidence_format_version
            .is_some_and(|version| version != EVIDENCE_FORMAT_VERSION)
    }) {
        return Err("history contains an unsupported evidence format; refusing to append".into());
    }
    let report = &evaluation.report;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_nanos();
    let record = EvidenceRecord {
        mission_id: report.mission_id.clone(),
        attempt_id: format!(
            "attempt-{}-{}-{nonce:x}",
            report.timestamp_unix_ms,
            std::process::id()
        ),
        previous_attempt_id: records
            .iter()
            .rev()
            .find(|record| record.mission_id == report.mission_id)
            .map(|record| record.attempt_id.clone()),
        criteria_hash: report.criteria_hash.clone(),
        artifact_hash: report.artifact_hash.clone(),
        report_hash: report.report_hash.clone(),
        verdict: report.verdict,
        timestamp_unix_ms: report.timestamp_unix_ms,
        evidence_format_version: Some(EVIDENCE_FORMAT_VERSION),
    };
    let attempts = root.join("attempts");
    fs::create_dir_all(&attempts)
        .map_err(|error| format!("cannot create attempts directory: {error}"))?;
    sync_directory(root)?;
    let directory = attempts.join(&record.attempt_id);
    fs::create_dir(&directory)
        .map_err(|error| format!("cannot create new attempt directory: {error}"))?;
    let report_bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| format!("cannot serialize report: {error}"))?;
    for (name, bytes) in [
        ("criteria.txt", evaluation.criteria.as_slice()),
        ("artifact.rs", evaluation.artifact.as_slice()),
        ("report-payload.json", evaluation.report_payload.as_slice()),
        ("report.json", report_bytes.as_slice()),
    ] {
        write_new_file(&directory.join(name), bytes, None)?;
    }
    let manifest = Manifest {
        record: record.clone(),
        report_file_hash: sha256(&report_bytes),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("cannot serialize manifest: {error}"))?;
    write_new_file(&directory.join("manifest.json"), &manifest_bytes, None)?;
    sync_directory(&directory)?;
    sync_directory(&attempts)?;
    // Only a complete, synced package may enter the authoritative index.
    commit_record(root, &previous_bytes, &record)?;
    Ok(record)
}

pub fn verify_attempt(root: &Path, attempt_id: &str) -> Result<Verification, String> {
    if !valid_id(attempt_id) {
        return Err(
            "invalid attempt ID; use a slug of at most 128 ASCII letters, digits, '-' or '_'"
                .into(),
        );
    }
    let records = parse_history(&history_bytes(root)?)?;
    let index = records.iter().position(|record| record.attempt_id == attempt_id)
        .ok_or_else(|| format!("attempt {attempt_id} is not committed in evidence.jsonl; absent or incomplete package"))?;
    let record = &records[index];
    match record.evidence_format_version {
        None => return Ok(Verification::Legacy(record.clone())),
        Some(EVIDENCE_FORMAT_VERSION) => {}
        Some(version) => return Err(format!("unsupported evidence format {version}")),
    }
    let expected_previous = records[..index]
        .iter()
        .rev()
        .find(|previous| previous.mission_id == record.mission_id)
        .map(|previous| &previous.attempt_id);
    if record.previous_attempt_id.as_ref() != expected_previous {
        return Err("previous_attempt_id does not extend this mission's history".into());
    }
    let directory = root.join("attempts").join(attempt_id);
    let metadata = fs::symlink_metadata(&directory).map_err(|error| {
        format!(
            "cannot inspect attempt directory {}: {error}",
            directory.display()
        )
    })?;
    if !metadata.file_type().is_dir() {
        return Err("attempt package must be a directory, not a symlink".into());
    }
    let manifest: Manifest = serde_json::from_slice(&read_file(&directory.join("manifest.json"))?)
        .map_err(|error| format!("invalid manifest.json: {error}"))?;
    if manifest.record != *record {
        return Err("manifest.json metadata does not match evidence.jsonl".into());
    }
    let criteria = read_file(&directory.join("criteria.txt"))?;
    let artifact = read_file(&directory.join("artifact.rs"))?;
    let report_payload = read_file(&directory.join("report-payload.json"))?;
    let report_bytes = read_file(&directory.join("report.json"))?;
    check_hash("criteria.txt", &criteria, &record.criteria_hash)?;
    check_hash("artifact.rs", &artifact, &record.artifact_hash)?;
    check_hash("report-payload.json", &report_payload, &record.report_hash)?;
    check_hash("report.json", &report_bytes, &manifest.report_file_hash)?;
    let report: EvaluationReport = serde_json::from_slice(&report_bytes)
        .map_err(|error| format!("invalid report.json: {error}"))?;
    if report.mission_id != record.mission_id
        || report.verdict != record.verdict
        || report.criteria_hash != record.criteria_hash
        || report.artifact_hash != record.artifact_hash
        || report.report_hash != record.report_hash
        || report.timestamp_unix_ms != record.timestamp_unix_ms
    {
        return Err("report.json metadata does not match the manifest and evidence.jsonl".into());
    }
    validate_evaluation(&Evaluation {
        report,
        criteria,
        artifact,
        report_payload,
    })?;
    Ok(Verification::Verified(record.clone()))
}

#[cfg(test)]
mod tests;
