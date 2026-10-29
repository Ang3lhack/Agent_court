import { useCallback, useEffect, useRef, useState } from "react";
import type { EvaluationAttempt, Mission, Verdict } from "./backend/api/backend";
import { useRegistryPage, type ConnectionState } from "./hooks/useRegistryPage";
import { readAttemptPage, readMissionPage } from "./lib/client";
import { formatTimestamp, type QueryFailure } from "./lib/registry";
import LocalEvidencePanel from "./LocalEvidencePanel";
import "./App.css";

const missionKey = (mission: Mission) => mission.mission_id;
const attemptKey = (attempt: EvaluationAttempt) => attempt.attempt_id;
const verdictLabels: Record<Verdict, string> = {
  PASS: "Aprobado",
  FAIL: "Falló la evaluación",
  ERROR: "Error de evaluación",
  NOT_RUN: "Sin ejecutar",
};

function QueryError({ error }: { error: QueryFailure }) {
  return (
    <div className="notice notice-error" role="alert">
      <strong>{error.title}</strong>
      <p>{error.message}</p>
      <details>
        <summary>Detalles técnicos completos</summary>
        <pre>{error.detail}</pre>
      </details>
    </div>
  );
}

function Timestamp({ value, unit }: { value: bigint; unit: "ns" | "ms" }) {
  const date = formatTimestamp(value, unit);
  return date.iso ? <time dateTime={date.iso} title={date.iso}>{date.label}</time> : <span>{date.label}</span>;
}

function Hash({ label, value }: { label: string; value: string }) {
  return <div className="metadata-field"><dt>{label}</dt><dd><code className="hash">{value}</code></dd></div>;
}

function MissionDetail({ mission, blocked, onConnection }: {
  mission: Mission;
  blocked: boolean;
  onConnection: (state: ConnectionState) => void;
}) {
  const fetchPage = useCallback((cursor: string | null) => readAttemptPage(mission.mission_id, cursor), [mission.mission_id]);
  const attempts = useRegistryPage(fetchPage, attemptKey, onConnection, blocked);

  return (
    <>
      <a className="back-to-missions" href="#missions">← Volver a las misiones</a>
      <div className="section-heading">
        <div><p className="eyebrow">DETALLE DE MISIÓN</p><h2 id="mission-detail-title">{mission.name}</h2></div>
        <button className="button button-secondary" onClick={attempts.refresh} disabled={attempts.loading || blocked}>Actualizar intentos</button>
      </div>
      <code className="identifier">{mission.mission_id}</code>
      <p className="description">{mission.description || "Sin descripción."}</p>
      <dl className="metadata mission-metadata">
        <Hash label="Hash de criterios · SHA-256" value={mission.criteria_hash} />
        <div className="metadata-field"><dt>Creada</dt><dd><Timestamp value={mission.created_at} unit="ns" /></dd></div>
      </dl>
      <div className="subsection-heading"><h3>Intentos de evaluación</h3><span className="count">{attempts.items.length} cargados</span></div>
      {attempts.error && <QueryError error={attempts.error} />}
      {attempts.loading && !blocked && <p className="loading-message" role="status">Consultando intentos…</p>}
      {!attempts.loading && !attempts.error && attempts.items.length === 0 && (
        <div className="empty-state"><span className="empty-symbol" aria-hidden="true">↳</span><h3>Todavía no hay intentos</h3><p>Esta misión existe en el registro, pero aún no tiene evaluaciones registradas.</p></div>
      )}
      <ol className="attempt-list" aria-label="Intentos registrados" aria-busy={attempts.loading}>
        {attempts.items.map((attempt) => (
          <li className="attempt-card" key={attempt.attempt_id}>
            <div className="attempt-heading"><code className="identifier">{attempt.attempt_id}</code><span className={`verdict verdict-${attempt.verdict.toLowerCase().replaceAll("_", "-")}`} title={verdictLabels[attempt.verdict]}>{attempt.verdict}</span></div>
            <p className="attempt-date"><Timestamp value={attempt.timestamp_unix_ms} unit="ms" /> · {verdictLabels[attempt.verdict]}</p>
            <dl className="metadata">
              <Hash label="Hash del artefacto · SHA-256" value={attempt.artifact_hash} />
              <Hash label="Hash del informe · SHA-256" value={attempt.report_hash} />
              <div className="metadata-field"><dt>Intento anterior</dt><dd>{attempt.previous_attempt_id ? <code className="identifier">{attempt.previous_attempt_id}</code> : "Ninguno · primer intento"}</dd></div>
            </dl>
          </li>
        ))}
      </ol>
      {attempts.hasMore && <button className="button button-pagination" disabled={attempts.loading || blocked} onClick={attempts.loadMore}>Cargar más intentos</button>}
      {attempts.items.length > 0 && <p className="pagination-note">{attempts.hasMore ? "Se muestran los intentos cargados; puede haber más." : "Fin del listado de intentos consultado."}</p>}
    </>
  );
}

function RegistryView() {
  const [connection, setConnection] = useState<ConnectionState>({ status: "checking" });
  const [selectedMissionId, setSelectedMissionId] = useState<string | null>(null);
  const detailPanel = useRef<HTMLElement>(null);
  const onConnection = useCallback((state: ConnectionState) => {
    // A clock error stops further requests until the page is reloaded.
    setConnection((previous) => previous.error?.clockError ? previous : { ...state, canisterId: state.canisterId ?? previous.canisterId });
  }, []);
  const blocked = connection.error?.clockError === true;
  const missions = useRegistryPage(readMissionPage, missionKey, onConnection, blocked);
  const selectedMission = missions.items.find((mission) => mission.mission_id === selectedMissionId) ?? null;
  useEffect(() => {
    if (selectedMissionId) detailPanel.current?.focus();
  }, [selectedMissionId]);
  const connectionLabel = connection.status === "connected" ? "Conectado" : connection.status === "checking" ? "Comprobando conexión" : blocked ? "Consultas detenidas" : "Consulta fallida";

  return (
    <>
      <p className="notice local-notice">Metadatos consultados al canister. Esta vista no descarga informes ni eventos de agentes y no certifica el contenido de un archivo local.</p>
        <section className="registry-status" aria-label="Estado del registro">
          <div><span className="stat-label">Misiones cargadas</span><strong className="stat-number">{missions.items.length}</strong><span className="stat-note">{missions.hasMore ? "Puede haber más páginas" : "En el listado actual"}</span></div>
          <div><span className="stat-label">Última consulta a ICP</span><span className={`connection-status connection-${connection.status}`} role="status"><span className="status-dot" aria-hidden="true" />{connectionLabel}</span><span className="stat-note">{connection.status === "connected" ? "Respuesta recibida del canister" : "El estado se comprueba mediante una consulta"}</span></div>
          <div className="canister-stat"><span className="stat-label">Canister backend</span><code className="identifier">{connection.canisterId ?? "Pendiente de verificar"}</code><span className="stat-note">Fechas en tu zona horaria</span></div>
        </section>
        {blocked && connection.error && <QueryError error={connection.error} />}
        <div className="dashboard-grid">
          <section className="panel missions-panel" id="missions" tabIndex={-1} aria-labelledby="missions-title">
            <div className="section-heading"><div><p className="eyebrow">REGISTRO ICP</p><h2 id="missions-title">Misiones</h2></div><button className="button button-secondary" disabled={missions.loading || blocked} onClick={missions.refresh}>Actualizar misiones</button></div>
            {missions.error && !blocked && <QueryError error={missions.error} />}
            {missions.loading && !blocked && <p className="loading-message" role="status">Consultando misiones…</p>}
            {!missions.loading && !missions.error && missions.items.length === 0 && (
              <div className="empty-state"><span className="empty-symbol" aria-hidden="true">[ ]</span><h3>El registro está vacío</h3><p>Todavía no existen misiones registradas en este canister. Cuando se registre una misión, aparecerá aquí al actualizar.</p></div>
            )}
            <ul className="mission-list" aria-label="Misiones registradas" aria-busy={missions.loading}>
              {missions.items.map((mission) => (
                <li key={mission.mission_id} className={`mission-card${selectedMission?.mission_id === mission.mission_id ? " mission-selected" : ""}`}>
                  <button className="mission-select" onClick={() => { if (selectedMission?.mission_id !== mission.mission_id) { onConnection({ status: "checking" }); setSelectedMissionId(mission.mission_id); } }} aria-pressed={selectedMission?.mission_id === mission.mission_id} aria-controls="mission-detail"><span>{mission.name}</span><span aria-hidden="true">↗</span></button>
                  <code className="identifier">{mission.mission_id}</code>
                  <p className="description">{mission.description || "Sin descripción."}</p>
                  <dl className="metadata"><Hash label="Hash de criterios" value={mission.criteria_hash} /><div className="metadata-field"><dt>Creada</dt><dd><Timestamp value={mission.created_at} unit="ns" /></dd></div></dl>
                </li>
              ))}
            </ul>
            {missions.hasMore && <button className="button button-pagination" disabled={missions.loading || blocked} onClick={missions.loadMore}>Cargar más misiones</button>}
            {missions.items.length > 0 && <p className="pagination-note">{missions.hasMore ? "El contador corresponde a las misiones cargadas, no al total del registro." : "Fin del listado de misiones consultado."}</p>}
          </section>
          <section className="panel detail-panel" id="mission-detail" ref={detailPanel} tabIndex={-1} aria-labelledby="mission-detail-title">
            {selectedMission ? <MissionDetail key={selectedMission.mission_id} mission={selectedMission} blocked={blocked} onConnection={onConnection} /> : <div className="empty-state selection-state"><span className="empty-symbol" aria-hidden="true">⌘</span><p className="eyebrow">HISTORIAL DE EVALUACIÓN</p><h2 id="mission-detail-title">Selecciona una misión</h2><p>Consulta sus intentos, veredictos y hashes de evidencia en este panel.</p></div>}
          </section>
        </div>
    </>
  );
}

function App() {
  const [view, setView] = useState<"local" | "icp">("local");
  const [openedRegistry, setOpenedRegistry] = useState(false);
  return <div className="app-shell">
    <a className="skip-link" href="#evidence-view">Ir a la evidencia</a>
    <header className="app-header">
      <a className="brand" href="#" aria-label="Agent Court, inicio"><span className="brand-mark" aria-hidden="true">AC</span><span>Agent Court<span className="brand-caption">Codex Evidence Engine</span></span></a>
      <span className="read-only">Solo lectura</span>
    </header>
    <main>
      <section className="intro" aria-labelledby="page-title">
        <p className="eyebrow">EVIDENCIA PARA EL CÓDIGO GENERADO POR AGENTES</p>
        <h1 id="page-title">Agent Court<span className="title-dot">.</span></h1>
        <p className="intro-description">Compara resultados, inspecciona pruebas y sigue la cadena de evidencia. Cada dato conserva su origen: archivos locales o metadatos de Internet Computer.</p>
      </section>
      <nav className="source-switch" aria-label="Origen de los datos">
        <button className="button" aria-pressed={view === "local"} aria-controls="evidence-view" onClick={() => setView("local")}>Evidencia local</button>
        <button className="button" aria-pressed={view === "icp"} aria-controls="evidence-view" onClick={() => { setOpenedRegistry(true); setView("icp"); }}>Registro ICP</button>
      </nav>
      <div id="evidence-view">
        <div hidden={view !== "local"}><LocalEvidencePanel /></div>
        <div hidden={view !== "icp"}>{openedRegistry && <RegistryView />}</div>
      </div>
    </main>
    <footer className="app-footer"><span>Agent Court / Codex Evidence Engine</span><span>{view === "local" ? "Archivos locales · Sin publicación" : "Consultas al registro · Sin escritura"}</span></footer>
  </div>;
}

export default App;
