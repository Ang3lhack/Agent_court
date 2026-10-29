// Synthetic packages for automated tests only; never presented as real runs.
import { LOCAL_FORMAT, sha256Text } from '../src/lib/localEvidence.ts';

export async function archiveFixture(verdicts = ['FAIL', 'PASS']) {
  const records = [];
  const packages = [];
  for (const [index, verdict] of verdicts.entries()) {
    const artifact = `// SIMULATED test fixture ${index}\r\n`;
    const criteria = 'Synthetic counter criteria';
    const payload = JSON.stringify({
      schema_version: 2, mission_id: 'counter-v1', verdict,
      checks: [{ name: 'fixture_check', status: verdict, detail: verdict === 'FAIL' ? 'SIMULATED failure: left: 2, right: 1' : `SIMULATED ${verdict}` }],
      criteria_hash: await sha256Text(criteria), artifact_hash: await sha256Text(artifact),
      timestamp_unix_ms: 1793228085556 + index, isolation: 'SIMULATED TEST FIXTURE; no Docker or agents executed',
    });
    const report = { ...JSON.parse(payload), report_hash: await sha256Text(payload) };
    const record = {
      mission_id: report.mission_id, attempt_id: `fixture-${index}`, previous_attempt_id: records.at(-1)?.attempt_id ?? null,
      criteria_hash: report.criteria_hash, artifact_hash: report.artifact_hash, report_hash: report.report_hash,
      verdict, timestamp_unix_ms: report.timestamp_unix_ms, evidence_format_version: 1,
    };
    const reportText = JSON.stringify(report, null, 2);
    records.push(record);
    packages.push({ attempt_id: record.attempt_id, artifact, criteria, payload, report: reportText,
      manifest: JSON.stringify({ record, report_file_hash: await sha256Text(reportText) }) });
  }
  return { format: LOCAL_FORMAT, history: records.map((r) => JSON.stringify(r)).join('\n') + '\n', packages, events: null };
}

export async function withDemoEvents(archive, first = 0) {
  const records = archive.history.trim().split('\n').map((line) => JSON.parse(line)).slice(first);
  const verdictLine = (r) => `VERDICT ${r.verdict} attempt_id=${r.attempt_id} previous_attempt_id=${r.previous_attempt_id === null ? 'None' : `Some("${r.previous_attempt_id}")`}`;
  const text = [
    'MISSION counter-v1; agents=SIMULATED fixture; no Codex real',
    'PUBLICATION LOCAL (sin --publish)',
    'DEMO CONTROLADA — candidato inicial proporcionado',
    'BUILDER OMITIDO (--demo-seed)',
    'EVALUATOR 1/2 START', verdictLine(records[0]),
    'REVIEWER START agents=SIMULATED fixture; no Codex real',
    'REVIEWER analysis="SIMULATED: increment by one"',
    'REPAIR START agents=SIMULATED fixture; no Codex real',
    'EVALUATOR 2/2 START', verdictLine(records[1]),
    ...(records[1].verdict === 'PASS' ? ['REPAIR completado; segunda evaluación PASS'] : []),
    `MISSION END verdict=${records[1].verdict} evaluations=2 repaired=true`,
  ].join('\n');
  archive.events = { text, sha256: await sha256Text(text) };
  return archive;
}
