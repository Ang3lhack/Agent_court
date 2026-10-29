import assert from 'node:assert/strict';
import test from 'node:test';
import { compareAttempts, MAX_ARCHIVE_BYTES, parseLocalEvidence, readLocalRun, sha256Text } from '../src/lib/localEvidence.ts';
import { archiveFixture, withDemoEvents } from './local-fixtures.mjs';

const parse = (archive) => parseLocalEvidence(JSON.stringify(archive));

test('valid local archive preserves exact bytes, diagnostics, statuses and chain', async () => {
  const archive = await archiveFixture(['FAIL', 'PASS', 'ERROR', 'NOT_RUN']);
  const data = await parse(archive);
  assert.deepEqual(data.attempts.map((a) => a.record.verdict), ['FAIL', 'PASS', 'ERROR', 'NOT_RUN']);
  assert.equal(data.attempts[0].artifact, archive.packages[0].artifact);
  assert.ok(data.attempts[0].artifact.endsWith('\r\n'));
  assert.equal(data.attempts[0].report.checks[0].detail, 'SIMULATED failure: left: 2, right: 1');
  assert.deepEqual(compareAttempts(...data.attempts.slice(0, 2)).map((h) => h.equal), [false, false, true]);
  assert.equal(data.events, null);
});

for (const file of ['artifact', 'criteria', 'payload', 'report']) {
  test(`rejects tampered ${file} bytes, including whitespace`, async () => {
    const archive = await archiveFixture();
    archive.packages[0][file] += ' ';
    await assert.rejects(parse(archive), /SHA-256/);
  });
}

test('rejects broken chains, duplicates, metadata changes, omissions and path-like IDs', async () => {
  for (const change of [
    (a) => { a.packages.pop(); },
    (a) => { a.packages.reverse(); },
    (a) => { const rows = a.history.trim().split('\n').map(JSON.parse); rows[1].previous_attempt_id = null; a.history = rows.map(JSON.stringify).join('\n'); },
    (a) => { a.history = [a.history.split('\n')[0], a.history.split('\n')[0]].join('\n'); },
    (a) => { a.history = a.history.replace('"FAIL"', '"PASS"'); },
    (a) => { a.history = a.history.replace('fixture-0', '../escape'); },
    (a) => { a.history = a.history.split('\n').slice(1).join('\n'); a.packages.shift(); },
    (a) => { const m = JSON.parse(a.packages[0].manifest); m.record.timestamp_unix_ms++; a.packages[0].manifest = JSON.stringify(m); },
  ]) {
    const archive = await archiveFixture(); change(archive);
    await assert.rejects(parse(archive));
  }
});

test('rejects report/payload contradictions even when all byte checksums match', async () => {
  const archive = await archiveFixture();
  const report = JSON.parse(archive.packages[0].report);
  report.checks[0].detail = 'Changed after evaluation';
  archive.packages[0].report = JSON.stringify(report);
  const manifest = JSON.parse(archive.packages[0].manifest);
  manifest.report_file_hash = await sha256Text(archive.packages[0].report);
  archive.packages[0].manifest = JSON.stringify(manifest);
  await assert.rejects(parse(archive), /payload/);
});

test('rejects non-string check verdicts even in an otherwise consistent package', async () => {
  const archive = await archiveFixture(['PASS']);
  const p = archive.packages[0];
  const report = JSON.parse(p.report);
  report.checks[0].status = ['PASS'];
  const { report_hash, ...payload } = report;
  assert.equal(report_hash.length, 64);
  p.payload = JSON.stringify(payload);
  report.report_hash = await sha256Text(p.payload);
  p.report = JSON.stringify(report);
  const record = JSON.parse(archive.history);
  record.report_hash = report.report_hash;
  archive.history = JSON.stringify(record);
  p.manifest = JSON.stringify({ record, report_file_hash: await sha256Text(p.report) });
  await assert.rejects(parse(archive), /Veredicto/);
});

test('rejects unsupported schemas, unsafe timestamps, huge inputs and empty archives', async () => {
  for (const change of [
    (a) => { a.format = 'future-format'; },
    (a) => { a.history = a.history.replaceAll('"evidence_format_version":1', '"evidence_format_version":99'); },
    (a) => { a.history = a.history.replace('1793228085556', '18446744073709551615'); },
    (a) => { a.history = ''; a.packages = []; },
    (a) => { a.certified_by_icp = true; },
  ]) {
    const archive = await archiveFixture(); change(archive);
    await assert.rejects(parse(archive));
  }
  await assert.rejects(parseLocalEvidence(' '.repeat(MAX_ARCHIVE_BYTES + 1)), /demasiado grande/);
  await assert.rejects(parseLocalEvidence('{ malformed JSON'));
});

test('demo log references only its two attempts and labels skipped builder and simulated source', async () => {
  const data = await parse(await withDemoEvents(await archiveFixture(['PASS', 'PASS', 'FAIL', 'PASS']), 2));
  assert.deepEqual(data.events.run.attemptIds, ['fixture-2', 'fixture-3']);
  assert.equal(data.events.run.stages[0].status, 'Omitido · candidato proporcionado');
  assert.match(data.events.run.stages[2].source, /SIMULATED/);
  assert.equal(data.events.run.stages[2].status, 'Análisis registrado');
  assert.equal(data.events.run.stages[3].status, 'Completado según log');
});

test('missing and truncated events never imply agent completion or a repair', async () => {
  const data = await parse(await archiveFixture(['PASS']));
  const run = readLocalRun('MISSION counter-v1; agents=REAL Codex CLI\nVERDICT PASS attempt_id=fixture-0 previous_attempt_id=None\n', data.attempts);
  assert.equal(run.stages[0].status, 'Sin evento registrado');
  assert.equal(run.stages[2].status, 'Sin evento registrado');
  assert.equal(run.stages[3].status, 'Sin evento registrado');
  assert.equal(readLocalRun('unrelated log', data.attempts), null);
  assert.equal(readLocalRun('MISSION counter-v1; agents=REAL\nBUILDER START agents=REAL', data.attempts), null);
});

test('FAIL after repair is retained and a start event does not claim completion', async () => {
  const data = await parse(await withDemoEvents(await archiveFixture(['FAIL', 'FAIL'])));
  assert.equal(data.attempts[1].record.verdict, 'FAIL');
  assert.equal(data.events.run.stages[3].status, 'Inicio registrado');
});

test('log hash, verdicts, predecessor references and IDs must match', async () => {
  const archive = await withDemoEvents(await archiveFixture());
  archive.events.text += '\n';
  await assert.rejects(parse(archive), /SHA-256/);
  for (const change of [
    (text) => text.replace('VERDICT FAIL', 'VERDICT PASS'),
    (text) => text.replace('attempt_id=fixture-1', 'attempt_id=missing'),
    (text) => text.replace('Some("fixture-0")', 'None'),
  ]) {
    const invalid = await withDemoEvents(await archiveFixture());
    invalid.events.text = change(invalid.events.text);
    invalid.events.sha256 = await sha256Text(invalid.events.text);
    await assert.rejects(parse(invalid), /contradice|ausente/);
  }
});
