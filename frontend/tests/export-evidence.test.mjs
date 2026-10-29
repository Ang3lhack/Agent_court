import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { archiveFixture, withDemoEvents } from './local-fixtures.mjs';
import { parseLocalEvidence } from '../src/lib/localEvidence.ts';

const script = fileURLToPath(new URL('../scripts/export-evidence.mjs', import.meta.url));
const names = { manifest: 'manifest.json', report: 'report.json', payload: 'report-payload.json', artifact: 'artifact.rs', criteria: 'criteria.txt' };
async function setup(t, withEvents = true) {
  const temporary = await mkdtemp(join(tmpdir(), 'court-export-test-'));
  t.after(() => rm(temporary, { recursive: true, force: true }));
  const root = join(temporary, 'evidence');
  const archive = await archiveFixture();
  if (withEvents) await withDemoEvents(archive);
  await mkdir(root);
  await writeFile(join(root, 'evidence.jsonl'), archive.history);
  for (const p of archive.packages) {
    const directory = join(root, 'attempts', p.attempt_id);
    await mkdir(directory, { recursive: true });
    for (const [key, name] of Object.entries(names)) await writeFile(join(directory, name), p[key]);
  }
  const log = join(temporary, 'orchestrate.log');
  if (archive.events) await writeFile(log, archive.events.text);
  const output = join(temporary, 'export.json');
  const run = (out = output) => {
    const result = spawnSync(process.execPath, [script, '--evidence-dir', root, ...(withEvents ? ['--events', log] : []), '--output', out], { encoding: 'utf8', timeout: 120_000 });
    assert.ifError(result.error); // A blocked subprocess is not a successful rejection test.
    return result;
  };
  return { temporary, root, archive, output, run };
}

test('export reuses Rust verify, preserves all originals and does not require agents, Docker or ICP', async (t) => {
  const { root, archive, output, run } = await setup(t);
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  const exported = JSON.parse(await readFile(output, 'utf8'));
  assert.deepEqual(exported, archive);
  assert.equal((await parseLocalEvidence(JSON.stringify(exported))).attempts.length, 2);
  assert.equal(await readFile(join(root, 'evidence.jsonl'), 'utf8'), archive.history);
  assert.deepEqual((await readdir(root)).sort(), ['attempts', 'evidence.jsonl']);
  for (const p of archive.packages) for (const [key, name] of Object.entries(names)) {
    assert.equal(await readFile(join(root, 'attempts', p.attempt_id, name), 'utf8'), p[key]);
  }
  const again = run();
  assert.notEqual(again.status, 0);
  assert.deepEqual(JSON.parse(await readFile(output, 'utf8')), archive);
});

test('export without log keeps agent events absent', async (t) => {
  const { output, run } = await setup(t, false);
  assert.equal(run().status, 0);
  assert.equal(JSON.parse(await readFile(output, 'utf8')).events, null);
});

test('a tampered package fails existing Rust verification before creating an export', async (t) => {
  const { root, output, run } = await setup(t);
  await writeFile(join(root, 'attempts', 'fixture-0', 'artifact.rs'), 'tampered');
  const result = run();
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /SHA-256 mismatch/);
  await assert.rejects(readFile(output), { code: 'ENOENT' });
});

test('export rejects evidence destinations and symlinked files without changing them', async (t) => {
  const { temporary, root, output, run, archive } = await setup(t);
  const history = join(root, 'evidence.jsonl');
  assert.notEqual(run(history).status, 0);
  assert.equal(await readFile(history, 'utf8'), archive.history);
  await symlink(root, join(temporary, 'alias'), 'dir');
  assert.notEqual(run(join(temporary, 'alias', 'export.json')).status, 0);
  await symlink(history, output);
  assert.notEqual(run().status, 0);
  assert.equal(await readFile(history, 'utf8'), archive.history);
  await rm(join(root, 'attempts', 'fixture-0', 'artifact.rs'));
  await symlink(history, join(root, 'attempts', 'fixture-0', 'artifact.rs'));
  assert.notEqual(run(join(temporary, 'other.json')).status, 0);
});
