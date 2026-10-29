import { useRef, useState } from "react";
import { compareAttempts, MAX_ARCHIVE_BYTES, parseLocalEvidence, type LocalAttempt, type LocalEvidence } from "./lib/localEvidence";
import { formatTimestamp } from "./lib/registry";

function Verdict({ value }: { value: string }) {
  return <span className={`verdict verdict-${value.toLowerCase().replaceAll("_", "-")}`}>{value}</span>;
}

function AttemptReport({ attempt, side }: { attempt: LocalAttempt; side: string }) {
  const { record, report } = attempt;
  return <article className="attempt-card evidence-report" aria-label={`Informe ${side}`}>
    <div className="attempt-heading"><h3>Intento {side}</h3><Verdict value={record.verdict} /></div>
    <code className="identifier">{record.attempt_id}</code>
    <p className="attempt-date">{formatTimestamp(BigInt(record.timestamp_unix_ms), "ms").label}</p>
    <p className="description">{report.checks.length} resultado(s) conservado(s) en el informe. Se muestra el detalle registrado, sin desglosar pruebas que no se guardaron por separado.</p>
    {report.checks.map((check, i) => <div className="check-result" key={i}>
      <div className="attempt-heading"><code>{check.name}</code><Verdict value={check.status} /></div>
      <pre className="evidence-text">{check.detail}</pre>
    </div>)}
    {!report.checks.length && <p className="description">El informe no contiene resultados de pruebas.</p>}
    <details><summary>Aislamiento registrado</summary><p className="evidence-text">{report.isolation}</p></details>
    <details><summary>Criterios originales</summary><pre className="evidence-text">{attempt.criteria}</pre></details>
    <details><summary>Código evaluado · artifact.rs</summary><pre className="evidence-text source-code">{attempt.artifact}</pre></details>
  </article>;
}

// Exported for render tests without a browser or an additional test dependency.
export function EvidenceComparison({ evidence, left, right }: { evidence: LocalEvidence; left: LocalAttempt; right: LocalAttempt }) {
  const labels = { artifact_hash: "Artefacto", report_hash: "Informe (payload)", criteria_hash: "Criterios" };
  return <>
    <div className="comparison-summary">
      <span><Verdict value={left.record.verdict} /> <span aria-hidden="true">→</span> <Verdict value={right.record.verdict} /></span>
      <span>{left.record.attempt_id === right.record.attempt_id ? "Mismo intento seleccionado" : right.record.previous_attempt_id === left.record.attempt_id ? "B continúa directamente la cadena de A" : "Comparación libre · B no es el sucesor directo de A"}</span>
    </div>
    {left.record.mission_id !== right.record.mission_id && <p className="description">Son misiones distintas; sus resultados no son directamente comparables.</p>}
    <div className="table-scroll" role="region" aria-label="Comparación de hashes SHA-256" tabIndex={0}>
      <table className="evidence-table hash-comparison"><caption>Hashes SHA-256 completos · A frente a B</caption><thead><tr><th scope="col">Contenido</th><th scope="col">Intento A</th><th scope="col">Intento B</th><th scope="col">Comparación</th></tr></thead>
        <tbody>{compareAttempts(left, right).map((hash) => <tr key={hash.key}><th scope="row">{labels[hash.key]}</th><td><code className="hash">{hash.left}</code></td><td><code className="hash">{hash.right}</code></td><td><span className={hash.equal ? "hash-equal" : "hash-changed"}>{hash.equal ? "Igual" : "Cambió"}</span></td></tr>)}</tbody>
      </table>
    </div>
    <div className="report-columns"><AttemptReport attempt={left} side="A" /><AttemptReport attempt={right} side="B" /></div>
    <section className="local-events" aria-labelledby="events-title">
      <div className="subsection-heading"><h3 id="events-title">Builder → Evaluator → Reviewer → Repair</h3><span className="source-tag">EVENTOS LOCALES</span></div>
      <p className="description">Los estados siguientes proceden del log importado. El log no forma parte de los paquetes de evidencia ni está certificado por ICP. Su hash comprueba la copia exportada; no autentica quién lo escribió.</p>
      {evidence.events?.run ? <>
        <p className="description">Esta sesión solo referencia {evidence.events.run.attemptIds.length} intento(s). Los demás no tienen eventos de agentes asociados en este log.</p>
        {![left, right].every((attempt) => evidence.events?.run?.attemptIds.includes(attempt.record.attempt_id)) && <p className="description">La selección A/B incluye intentos fuera de esta sesión. Los estados de agentes de abajo corresponden únicamente a los identificadores listados.</p>}
        <ul className="event-attempts">{evidence.events.run.attemptIds.map((id) => <li key={id}><code className="identifier">{id}</code></li>)}</ul>
        <div className="agent-stages">{evidence.events.run.stages.map((stage) => <article className="agent-stage" key={stage.role}><p className="eyebrow">{stage.role}</p><strong>{stage.status}</strong><p className="description">{stage.source}</p></article>)}</div>
        {evidence.events.run.analysis && <details open><summary>Análisis de Reviewer según el log</summary><pre className="evidence-text">{evidence.events.run.analysis}</pre></details>}
      </> : <p className="notice local-notice">Sin sesión de agentes vinculable. No se infieren ejecuciones de Builder, Reviewer ni Repair a partir de un veredicto.</p>}
      {evidence.events && <details><summary>Log original importado y hash de la copia</summary><code className="hash">{evidence.events.sha256}</code><pre className="evidence-text">{evidence.events.text}</pre></details>}
    </section>
  </>;
}

export default function LocalEvidencePanel() {
  const [evidence, setEvidence] = useState<LocalEvidence | null>(null);
  const [filename, setFilename] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [selection, setSelection] = useState([0, 0]);
  const request = useRef(0);
  async function importFile(file?: File) {
    if (!file) return;
    const current = ++request.current;
    setBusy(true); setError(null); setEvidence(null); setFilename("");
    try {
      if (file.size > MAX_ARCHIVE_BYTES) throw new Error("El JSON supera el límite de 8 MiB.");
      const data = await parseLocalEvidence(new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(await file.arrayBuffer()));
      if (current !== request.current) return;
      setEvidence(data); setFilename(file.name);
      setSelection([Math.max(0, data.attempts.length - 2), data.attempts.length - 1]);
    } catch (failure) {
      if (current === request.current) setError(failure instanceof Error ? failure.message : "No se pudo leer el archivo.");
    } finally {
      if (current === request.current) setBusy(false);
    }
  }
  return <section className="panel local-panel" id="local-evidence" aria-labelledby="local-title">
    <div className="section-heading"><div><p className="eyebrow">EVIDENCIA LOCAL · SOLO LECTURA</p><h2 id="local-title">Del resultado a la evidencia</h2></div><span className="source-tag">SIN CERTIFICACIÓN ICP</span></div>
    <p className="description">Importa una exportación para comparar intentos, hashes y pruebas conservadas. El archivo se procesa en este navegador; no se sube ni se publica.</p>
    <div className="import-controls"><label className="file-label">JSON de evidencia local<input type="file" accept=".json,application/json" disabled={busy} onChange={(event) => { void importFile(event.target.files?.[0]); event.target.value = ""; }} /></label>{evidence && <button className="button" onClick={() => { ++request.current; setEvidence(null); setFilename(""); }}>Cerrar archivo</button>}</div>
    {busy && <p className="loading-message" role="status">Verificando SHA-256, manifiestos, informes y cadena…</p>}
    {error && <div className="notice notice-error" role="alert"><strong>Archivo rechazado</strong><p>{error}</p><p>No se muestran datos de una importación fallida.</p></div>}
    {!evidence && !busy && <div className="empty-state"><span className="empty-symbol" aria-hidden="true">↔</span><h3>Compara dos intentos verificados localmente</h3><p>Exporta el almacén con <code>frontend/scripts/export-evidence.mjs</code> y selecciona el JSON. El historial original permanece intacto.</p></div>}
    {evidence && <>
      <div className="notice local-notice" role="status"><strong>Integridad local comprobada · {evidence.attempts.length} intentos · {filename}</strong><p>Hashes de los archivos, contenido de los informes, manifiestos y cadena coherentes. Esto no demuestra autoría ni ejecución auténtica, y no acredita registro o certificación en ICP.</p></div>
      <div className="table-scroll" role="region" aria-label="Historial local" tabIndex={0}><table className="evidence-table"><caption>Cadena completa del almacén importado</caption><thead><tr><th scope="col">#</th><th scope="col">Intento / misión</th><th scope="col">Veredicto</th><th scope="col">Predecesor</th><th scope="col">Eventos</th></tr></thead><tbody>{evidence.attempts.map((attempt, i) => <tr key={attempt.record.attempt_id} className={selection.includes(i) ? "selected-row" : ""}><td>{i + 1}</td><td><code className="identifier">{attempt.record.attempt_id}</code><br /><span>{attempt.record.mission_id}</span></td><td><Verdict value={attempt.record.verdict} /></td><td><code className="identifier">{attempt.record.previous_attempt_id ?? "Inicio de cadena"}</code></td><td>{evidence.events?.run?.attemptIds.includes(attempt.record.attempt_id) ? "Referenciado en log local" : "Sin eventos asociados"}</td></tr>)}</tbody></table></div>
      <div className="comparison-controls">{[0, 1].map((side) => <label key={side}>Intento {side === 0 ? "A" : "B"}<select value={selection[side]} onChange={(event) => setSelection((previous) => previous.map((value, i) => i === side ? Number(event.target.value) : value))}>{evidence.attempts.map((attempt, i) => <option key={attempt.record.attempt_id} value={i}>#{i + 1} · {attempt.record.verdict} · {attempt.record.attempt_id}</option>)}</select></label>)}</div>
      <EvidenceComparison evidence={evidence} left={evidence.attempts[selection[0]]} right={evidence.attempts[selection[1]]} />
    </>}
  </section>;
}
