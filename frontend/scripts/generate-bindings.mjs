import { generate } from '@icp-sdk/bindgen/core';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const didFile = fileURLToPath(new URL('../../backend/backend.did', import.meta.url));
const output = fileURLToPath(new URL('../src/backend/api/', import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), 'agent-court-bindings-'));

try {
  await generate({ didFile, outDir: temporary });
  const missing = [];
  for (const name of ['backend.ts', 'declarations/backend.did.js', 'declarations/backend.did.d.ts']) {
    const generated = await readFile(join(temporary, name));
    const target = join(output, name);
    let existing;
    try {
      existing = await readFile(target);
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
    if (existing && !existing.equals(generated)) {
      throw new Error(`Los bindings de ${target} difieren de backend.did. Revisa las diferencias antes de regenerarlos; no se sobrescribió ningún archivo existente.`);
    }
    if (!existing) missing.push({ target, generated });
  }
  for (const { target, generated } of missing) {
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, generated, { flag: 'wx' });
  }
  console.log(`Bindings Candid verificados; ${missing.length} archivos generados, archivos existentes conservados.`);
} finally {
  // This directory belongs exclusively to this invocation.
  await rm(temporary, { recursive: true, force: true });
}
