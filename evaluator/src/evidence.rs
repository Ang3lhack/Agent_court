use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{sha256, EvaluationReport, Status};

const CRITERIA: &str = "counter-v1: starts at zero; increment increases by one; reset returns zero";

#[derive(Debug, Deserialize, Serialize)]
pub struct EvidenceRecord {
    pub mission_id: String,
    pub attempt_id: String,
    pub previous_attempt_id: Option<String>,
    pub criteria_hash: String,
    pub artifact_hash: String,
    pub report_hash: String,
    pub verdict: Status,
    pub timestamp_unix_ms: u128,
}

pub fn record_attempt(path: &Path, report: &EvaluationReport) -> Result<EvidenceRecord, String> {
    let previous_attempt_id = last_attempt_id(path)?;
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_millis();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_nanos();
    let attempt_id = format!(
        "attempt-{timestamp_unix_ms}-{}-{nonce:x}",
        std::process::id()
    );

    let record = EvidenceRecord {
        mission_id: report.mission_id.to_string(),
        attempt_id,
        previous_attempt_id,
        criteria_hash: sha256(CRITERIA.as_bytes()),
        artifact_hash: report.artifact_hash.clone(),
        report_hash: report.report_hash.clone(),
        verdict: match report.verdict {
            Status::Pass => Status::Pass,
            Status::Fail => Status::Fail,
            Status::Error => Status::Error,
            Status::NotRun => Status::NotRun,
        },
        timestamp_unix_ms,
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create evidence directory: {error}"))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open evidence log: {error}"))?;
    serde_json::to_writer(&mut file, &record)
        .map_err(|error| format!("cannot serialize evidence record: {error}"))?;
    file.write_all(b"\n")
        .and_then(|()| file.sync_data())
        .map_err(|error| format!("cannot persist evidence record: {error}"))?;

    Ok(record)
}

fn last_attempt_id(path: &Path) -> Result<Option<String>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read evidence log: {error}")),
    };

    let mut last = None;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| format!("cannot read evidence log line: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let record: EvidenceRecord = serde_json::from_str(&line)
            .map_err(|error| format!("evidence log is invalid: {error}"))?;
        last = Some(record.attempt_id);
    }
    Ok(last)
}
