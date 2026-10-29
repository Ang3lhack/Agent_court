// Read-only export. The evaluator's existing verifier remains authoritative;
// the portable validator checks the captured bytes again before any output.
import { execFileSync } from 'node:child_process';
import { lstat, open, readFile, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { LOCAL_FORMAT, MAX_ARCHIVE_BYTES, MAX_ATTEMPTS, MAX_FILE_BYTES, parseLocalEvidence, sha256Text } from '../src/lib/localEvidence.ts';

async function readText(path, limit = MAX_FILE_BYTES) {
  const info = await lstat(path);
  if (!info.isFile() || info.size > limit) throw new Error(`Archivo regular requerido, hasta ${limit} bytes: ${path}`);
  const bytes = await readFile(path);
  if (bytes.length > limit) throw new Error(`Archivo demasiado grande: ${path}`);
  // Preserve BOMs and CRLF exactly; reject invalid UTF-8 instead of replacing it.
  return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes);
}
function inside(parent, child) {
  const path = relative(parent, child);
  return path === '' || (!path.startsWith(`..${sep}`) && path !== '..' && !isAbsolute(path));
}
async function main() {
  const { values } = parseArgs({ options: {
    'evidence-dir': { type: 'string' }, events: { type: 'string' }, output: { type: 'string' },
  } });
  if (!values['evidence-dir'] || !values.output) {
    throw new Error('Uso: node frontend/scripts/export-evidence.mjs --evidence-dir <almacén> [--events <orchestrate.log>] --output <archivo-nuevo.json>');
  }
  const root = await realpath(values['evidence-dir']);
  const output = join(await realpath(dirname(resolve(values.output))), resolve(values.output).split(sep).at(-1));
  if (inside(root, output) || output.split(sep).includes('.agent-court')) {
    throw new Error('La exportación debe quedar fuera del almacén y de .agent-court.');
  }
  const history = await readText(join(root, 'evidence.jsonl'), 2 * MAX_FILE_BYTES);
  const records = history.split('\n').filter((line) => line.trim()).map((line) => JSON.parse(line));
  if (!records.length || records.length > MAX_ATTEMPTS) throw new Error(`Se requieren entre 1 y ${MAX_ATTEMPTS} intentos.`);
  const manifestPath = fileURLToPath(new URL('../../evaluator/Cargo.toml', import.meta.url));
  const packages = [];
  let capturedBytes = Buffer.byteLength(history);
  for (const record of records) {
    if (typeof record.attempt_id !== 'string' || !/^[a-zA-Z0-9_-]{1,128}$/.test(record.attempt_id)) throw new Error('Identificador inválido.');
    // This command only reads evidence. It never evaluates, runs agents or publishes.
    execFileSync('cargo', ['run', '--quiet', '--locked', '--manifest-path', manifestPath, '--', '--evidence-dir', root, 'verify', record.attempt_id], {
      stdio: ['ignore', 'pipe', 'pipe'], timeout: 120_000, maxBuffer: MAX_FILE_BYTES,
    });
    const directory = join(root, 'attempts', record.attempt_id);
    if (!(await lstat(join(root, 'attempts'))).isDirectory() || !(await lstat(directory)).isDirectory()) throw new Error('Directorio de paquetes inválido o enlace simbólico.');
    const entry = { attempt_id: record.attempt_id };
    for (const [key, name] of Object.entries({ manifest: 'manifest.json', report: 'report.json', payload: 'report-payload.json', artifact: 'artifact.rs', criteria: 'criteria.txt' })) {
      entry[key] = await readText(join(directory, name));
      capturedBytes += Buffer.byteLength(entry[key]);
      if (capturedBytes > MAX_ARCHIVE_BYTES) throw new Error('Exportación demasiado grande (máximo 8 MiB).');
    }
    packages.push(entry);
  }
  const log = values.events ? await readText(values.events) : null;
  const archive = JSON.stringify({ format: LOCAL_FORMAT, history, packages, events: log === null ? null : { text: log, sha256: await sha256Text(log) } }, null, 2) + '\n';
  if (Buffer.byteLength(archive) > MAX_ARCHIVE_BYTES) throw new Error('Exportación demasiado grande (máximo 8 MiB).');
  const verified = await parseLocalEvidence(archive);
  // A new path only; never truncate originals, previous exports or symlinks.
  const file = await open(output, 'wx', 0o600);
  try { await file.writeFile(archive); } finally { await file.close(); }
  console.log(`${verified.attempts.length} paquetes verificados → ${output}\nCopia local; no certifica ejecución ni publicación en ICP. Originales conservados.`);
}
main().catch((error) => {
  console.error(`Exportación rechazada: ${error.message}`);
  process.exitCode = 1;
});
