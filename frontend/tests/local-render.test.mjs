import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { registerHooks } from 'node:module';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import ts from 'typescript';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { archiveFixture, withDemoEvents } from './local-fixtures.mjs';
import { parseLocalEvidence, sha256Text } from '../src/lib/localEvidence.ts';

// Use the installed compiler to render the real components, without jsdom or
// a new framework. Browser interactions still deserve a manual smoke check.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier.startsWith('.') && context.parentURL?.includes('/src/')) {
      for (const extension of ['.ts', '.tsx', '.js']) {
        const url = new URL(specifier + extension, context.parentURL);
        if (existsSync(url)) return { url: url.href, shortCircuit: true };
      }
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url.endsWith('.css')) return { format: 'module', source: '', shortCircuit: true };
    if (url.endsWith('.tsx') || url.endsWith('.ts')) return { format: 'module', shortCircuit: true, source: ts.transpileModule(readFileSync(fileURLToPath(url), 'utf8'), {
      compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    }).outputText };
    return nextLoad(url, context);
  },
});
const { default: LocalEvidencePanel, EvidenceComparison } = await import('../src/LocalEvidencePanel.tsx');

test('initial view requires explicit import and never claims ICP certification', () => {
  const html = renderToStaticMarkup(createElement(LocalEvidencePanel));
  assert.match(html, /SIN CERTIFICACIÓN ICP/);
  assert.match(html, /type="file"/);
  assert.match(html, /no se sube ni se publica/);
  assert.doesNotMatch(html, /Integridad local comprobada/);
});

test('app starts locally and does not mount the ICP registry until requested', async () => {
  const { default: App } = await import('../src/App.tsx');
  const html = renderToStaticMarkup(createElement(App));
  assert.match(html, /Evidencia local/);
  assert.match(html, /Registro ICP/);
  assert.doesNotMatch(html, /Comprobando conexión|Consultando misiones|Estado del registro/);
});

test('comparison renders complete hashes, actual checks and local-only provenance', async () => {
  const evidence = await parseLocalEvidence(JSON.stringify(await withDemoEvents(await archiveFixture())));
  const html = renderToStaticMarkup(createElement(EvidenceComparison, { evidence, left: evidence.attempts[0], right: evidence.attempts[1] }));
  for (const attempt of evidence.attempts) assert.ok(html.includes(attempt.record.artifact_hash));
  assert.match(html, /SIMULATED failure: left: 2, right: 1/);
  assert.match(html, /Omitido · candidato proporcionado/);
  assert.match(html, /Análisis registrado/);
  assert.match(html, /ni está certificado por ICP/);
  assert.match(html, /Cambió/);
  assert.match(html, /Igual/);
});

test('local log text is escaped and unavailable agent events stay unknown', async () => {
  const archive = await archiveFixture();
  archive.events = { text: '<img src=x onerror=alert(1)>', sha256: await sha256Text('<img src=x onerror=alert(1)>') };
  const evidence = await parseLocalEvidence(JSON.stringify(archive));
  const html = renderToStaticMarkup(createElement(EvidenceComparison, { evidence, left: evidence.attempts[0], right: evidence.attempts[1] }));
  assert.match(html, /&lt;img/);
  assert.doesNotMatch(html, /<img/);
  assert.match(html, /Sin sesión de agentes vinculable/);
  assert.doesNotMatch(html, /Completado según log/);
});
