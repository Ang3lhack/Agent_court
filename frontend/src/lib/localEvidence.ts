// Portable local archive, deliberately separate from the ICP metadata model.
// Hashes prove internal consistency, not authorship, execution or publication.
export const MAX_ARCHIVE_BYTES = 8 * 1024 * 1024;
export const MAX_FILE_BYTES = 1024 * 1024;
export const MAX_ATTEMPTS = 100;
export const LOCAL_FORMAT = "agent-court-local-v1";
export type LocalVerdict = "PASS" | "FAIL" | "ERROR" | "NOT_RUN";
export interface LocalRecord {
  mission_id: string;
  attempt_id: string;
  previous_attempt_id: string | null;
  criteria_hash: string;
  artifact_hash: string;
  report_hash: string;
  verdict: LocalVerdict;
  timestamp_unix_ms: number;
  evidence_format_version: 1;
}
export interface LocalReport {
  schema_version: 2;
  mission_id: string;
  verdict: LocalVerdict;
  checks: { name: string; status: LocalVerdict; detail: string }[];
  criteria_hash: string;
  artifact_hash: string;
  report_hash: string;
  timestamp_unix_ms: number;
  isolation: string;
}
export interface LocalAttempt {
  record: LocalRecord;
  report: LocalReport;
  artifact: string;
  criteria: string;
}
export interface LocalRun {
  attemptIds: string[];
  stages: { role: string; status: string; source: string }[];
  analysis?: string;
}
export interface LocalEvidence {
  attempts: LocalAttempt[];
  events: { text: string; sha256: string; run: LocalRun | null } | null;
}

function requireThat(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
function object(value: unknown, keys: string[], label: string): Record<string, unknown> {
  requireThat(value !== null && typeof value === "object" && !Array.isArray(value), `${label}: objeto requerido.`);
  requireThat(Object.keys(value).sort().join(",") === [...keys].sort().join(","), `${label}: campos o versión no admitidos.`);
  return value as Record<string, unknown>;
}
function string(value: unknown, label: string, limit = MAX_FILE_BYTES): string {
  requireThat(typeof value === "string" && new TextEncoder().encode(value).length <= limit, `${label}: texto ausente o demasiado grande.`);
  return value;
}
function id(value: unknown): asserts value is string {
  requireThat(typeof value === "string" && /^[a-zA-Z0-9_-]{1,128}$/.test(value), "Identificador inválido.");
}
function verdict(value: unknown): asserts value is LocalVerdict {
  requireThat(typeof value === "string" && ["PASS", "FAIL", "ERROR", "NOT_RUN"].includes(value), "Veredicto no admitido.");
}
function timestamp(value: unknown) {
  requireThat(typeof value === "number" && Number.isSafeInteger(value) && value >= 0, "Fecha fuera del rango entero seguro.");
}
function record(value: unknown): LocalRecord {
  const r = object(value, ["mission_id", "attempt_id", "previous_attempt_id", "criteria_hash", "artifact_hash", "report_hash", "verdict", "timestamp_unix_ms", "evidence_format_version"], "Registro");
  id(r.attempt_id); id(r.mission_id);
  if (r.previous_attempt_id !== null) id(r.previous_attempt_id);
  requireThat(r.evidence_format_version === 1, "Este visor requiere paquetes v1; no reconstruye registros históricos.");
  verdict(r.verdict); timestamp(r.timestamp_unix_ms);
  for (const key of ["criteria_hash", "artifact_hash", "report_hash"]) {
    requireThat(typeof r[key] === "string" && /^[0-9a-f]{64}$/.test(r[key]), `Hash inválido: ${key}.`);
  }
  return r as unknown as LocalRecord;
}
function report(value: unknown): LocalReport {
  const r = object(value, ["schema_version", "mission_id", "verdict", "checks", "criteria_hash", "artifact_hash", "report_hash", "timestamp_unix_ms", "isolation"], "Informe");
  requireThat(r.schema_version === 2, "Versión de informe no admitida.");
  id(r.mission_id); verdict(r.verdict); timestamp(r.timestamp_unix_ms);
  string(r.isolation, "Aislamiento");
  requireThat(Array.isArray(r.checks) && r.checks.length <= 1000, "Lista de pruebas inválida.");
  for (const item of r.checks) {
    const check = object(item, ["name", "status", "detail"], "Prueba");
    string(check.name, "Nombre"); string(check.detail, "Detalle"); verdict(check.status);
  }
  return r as unknown as LocalReport;
}
function canonical(value: unknown, depth = 0): string {
  requireThat(depth < 20, "JSON demasiado anidado.");
  if (Array.isArray(value)) return `[${value.map((v) => canonical(v, depth + 1)).join(",")}]`;
  if (typeof value === "object" && value !== null) {
    return `{${Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([k, v]) => `${JSON.stringify(k)}:${canonical(v, depth + 1)}`).join(",")}}`;
  }
  return JSON.stringify(value);
}
export async function sha256Text(text: string): Promise<string> {
  if (!globalThis.crypto?.subtle) throw new Error("La verificación SHA-256 requiere HTTPS o localhost.");
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(bytes), (b) => b.toString(16).padStart(2, "0")).join("");
}
async function checkHash(text: string, expected: unknown, label: string) {
  requireThat(typeof expected === "string" && await sha256Text(text) === expected, `SHA-256 no coincide: ${label}.`);
}

// Only one explicit mission session is interpreted. Unknown lines remain raw
// text; absence of a line never implies that an agent ran or completed.
export function readLocalRun(text: string, attempts: LocalAttempt[]): LocalRun | null {
  const lines = text.split(/\r?\n/);
  const starts = lines.filter((line) => line.startsWith("MISSION counter-v1; agents="));
  if (starts.length !== 1) return null;
  const session = lines.slice(lines.indexOf(starts[0]));
  requireThat(session.filter((line) => line.startsWith("MISSION ") && !line.startsWith("MISSION END ")).length === 1, "El log mezcla varias misiones.");
  const attemptIds: string[] = [];
  for (const line of session.filter((line) => line.startsWith("VERDICT "))) {
    const match = /^VERDICT (PASS|FAIL|ERROR|NOT_RUN) attempt_id=([\w-]+) previous_attempt_id=(None|Some\("([\w-]+)"\))$/.exec(line);
    requireThat(match, "Evento VERDICT no reconocido.");
    const attempt = attempts.find((a) => a.record.attempt_id === match[2]);
    requireThat(attempt && attempt.record.verdict === match[1] && attempt.record.previous_attempt_id === (match[4] ?? null), "El log contradice el historial o referencia un intento ausente.");
    requireThat(!attemptIds.includes(match[2]) && (!attemptIds.length || attempt.record.previous_attempt_id === attemptIds.at(-1)), "La secuencia del log no coincide con la cadena.");
    attemptIds.push(match[2]);
  }
  if (attemptIds.length === 0) return null;
  const source = (role: string) => session.find((line) => line.startsWith(`${role} START agents=`))?.split(" agents=")[1] ?? "Sin origen de agente registrado";
  const has = (line: string) => session.includes(line);
  const started = (role: string) => session.some((line) => line.startsWith(`${role} START agents=`));
  const analysisLine = session.find((line) => line.startsWith("REVIEWER analysis="));
  const analysis: unknown = analysisLine ? JSON.parse(analysisLine.slice("REVIEWER analysis=".length)) : undefined;
  requireThat(analysis === undefined || typeof analysis === "string", "Análisis del Reviewer inválido.");
  const repaired = has("REPAIR completado; segunda evaluación PASS") && started("REPAIR") && attemptIds.length === 2 && attempts.find((a) => a.record.attempt_id === attemptIds[1])?.record.verdict === "PASS";
  return { attemptIds, analysis, stages: [
    { role: "Builder", status: has("BUILDER OMITIDO (--demo-seed)") ? "Omitido · candidato proporcionado" : started("BUILDER") ? "Inicio registrado" : "Sin evento registrado", source: has("BUILDER OMITIDO (--demo-seed)") ? "DEMO CONTROLADA" : source("BUILDER") },
    { role: "Evaluator", status: `${attemptIds.length} veredictos vinculados`, source: "Log local y paquetes consistentes" },
    { role: "Reviewer", status: analysis ? "Análisis registrado" : started("REVIEWER") ? "Inicio registrado" : "Sin evento registrado", source: source("REVIEWER") },
    { role: "Repair", status: repaired ? "Completado según log" : started("REPAIR") ? "Inicio registrado" : has("REPAIR no fue necesario (primer intento PASS)") ? "No fue necesario según log" : "Sin evento registrado", source: source("REPAIR") },
  ] };
}

export async function parseLocalEvidence(text: string): Promise<LocalEvidence> {
  string(text, "Archivo JSON", MAX_ARCHIVE_BYTES);
  const archive = object(JSON.parse(text), ["format", "history", "packages", "events"], "Exportación");
  requireThat(archive.format === LOCAL_FORMAT, "Formato local no admitido.");
  const history = string(archive.history, "Historial", 2 * MAX_FILE_BYTES).split("\n").filter((line) => line.trim());
  requireThat(history.length > 0 && history.length <= MAX_ATTEMPTS, `Se requieren entre 1 y ${MAX_ATTEMPTS} intentos.`);
  const records = history.map((line) => record(JSON.parse(line)));
  requireThat(Array.isArray(archive.packages) && archive.packages.length === records.length, "Faltan paquetes o hay paquetes adicionales.");
  const previous = new Map<string, string>();
  const seen = new Set<string>();
  const attempts: LocalAttempt[] = [];
  for (const [i, r] of records.entries()) {
    requireThat(!seen.has(r.attempt_id), "Intento duplicado.");
    requireThat(r.previous_attempt_id === (previous.get(r.mission_id) ?? null), "Cadena de predecesores incompleta o alterada.");
    seen.add(r.attempt_id); previous.set(r.mission_id, r.attempt_id);
    const p = object(archive.packages[i], ["attempt_id", "manifest", "report", "payload", "artifact", "criteria"], "Paquete");
    requireThat(p.attempt_id === r.attempt_id, "Orden o identificador de paquete incorrecto.");
    const manifest = object(JSON.parse(string(p.manifest, "Manifiesto")), ["record", "report_file_hash"], "Manifiesto");
    requireThat(canonical(record(manifest.record)) === canonical(r), "Manifiesto e historial no coinciden.");
    const artifact = string(p.artifact, "Artefacto");
    const criteria = string(p.criteria, "Criterios");
    const payload = string(p.payload, "Payload");
    const reportText = string(p.report, "Informe");
    await Promise.all([
      checkHash(artifact, r.artifact_hash, "artifact.rs"), checkHash(criteria, r.criteria_hash, "criteria.txt"),
      checkHash(payload, r.report_hash, "report-payload.json"), checkHash(reportText, manifest.report_file_hash, "report.json"),
    ]);
    const parsed = report(JSON.parse(reportText));
    for (const key of ["mission_id", "verdict", "timestamp_unix_ms", "criteria_hash", "artifact_hash", "report_hash"] as const) {
      requireThat(parsed[key] === r[key], `Informe e historial no coinciden: ${key}.`);
    }
    const { report_hash: ownHash, ...hashPayload } = parsed;
    requireThat(ownHash === r.report_hash && canonical(JSON.parse(payload)) === canonical(hashPayload), "El informe no coincide con su payload.");
    attempts.push({ record: r, report: parsed, artifact, criteria });
  }
  let events: LocalEvidence["events"] = null;
  if (archive.events !== null) {
    const e = object(archive.events, ["text", "sha256"], "Eventos");
    const eventText = string(e.text, "Log local");
    const hash = string(e.sha256, "Hash del log", 64);
    await checkHash(eventText, hash, "orchestrate.log (copia exportada)");
    events = { text: eventText, sha256: hash, run: readLocalRun(eventText, attempts) };
  }
  return { attempts, events };
}

export function compareAttempts(left: LocalAttempt, right: LocalAttempt) {
  return (["artifact_hash", "report_hash", "criteria_hash"] as const).map((key) => ({ key, left: left.record[key], right: right.record[key], equal: left.record[key] === right.record[key] }));
}
