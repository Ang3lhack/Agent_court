import type { RegistryError } from "../backend/api/backend";

export const PAGE_SIZE = 20;
const QUERY_TIMEOUT_MS = 15_000;

export interface QueryFailure {
  title: string;
  message: string;
  detail: string;
  clockError: boolean;
}

export function unwrapResult<T>(result: { __kind__: "Ok"; Ok: T } | { __kind__: "Err"; Err: RegistryError }): T {
  if (result.__kind__ === "Err") {
    throw new Error(`El canister devolvió un error: ${JSON.stringify(result.Err)}`);
  }
  return result.Ok;
}

export function withQueryTimeout<T>(request: Promise<T>, timeoutMs = QUERY_TIMEOUT_MS): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`El canister no respondió antes del límite de ${timeoutMs} ms.`)), timeoutMs);
    request.then(
      (result) => { clearTimeout(timer); resolve(result); },
      (error: unknown) => { clearTimeout(timer); reject(error); },
    );
  });
}

function errorDetails(error: unknown, seen = new Set<unknown>()): string {
  if (seen.has(error)) return "[Referencia circular]";
  seen.add(error);
  if (error instanceof Error) {
    const nested = error instanceof AggregateError ? [...error.errors] : [];
    if (error.cause !== undefined) nested.push(error.cause);
    return [error.stack ?? error.message, ...nested.map((cause) => `Causa: ${errorDetails(cause, seen)}`)].join("\n");
  }
  if (typeof error === "object" && error !== null) {
    try {
      return JSON.stringify(error, (_, value: unknown) => typeof value === "bigint" ? value.toString() : value, 2);
    } catch {
      return String(error);
    }
  }
  return String(error);
}

export function describeQueryError(error: unknown): QueryFailure {
  const detail = errorDetails(error);
  if (/ingress_expiry|invalid request expiry|clock skew|certificate.*(too old|future|time)|time.*(out of sync|drift)/i.test(detail)) {
    return { title: "Consulta detenida por un desfase de reloj", message: "ICP rechazó la consulta por una diferencia de tiempo. Las consultas quedan detenidas; revisa el reloj local y el de la réplica antes de recargar la página.", detail, clockError: true };
  }
  if (/cookie|PUBLIC_CANISTER_ID|canister.*environment|root key|canister id/i.test(detail)) {
    return { title: "No se pudo identificar el backend", message: "Abre la aplicación desde el servidor Vite configurado o desde el frontend desplegado. No se encontró una configuración válida del canister.", detail, clockError: false };
  }
  if (/fetch|network|ECONNREFUSED|ENOTFOUND|failed to connect|load failed|timeout|no respondió|HTTP.*50[234]/i.test(detail)) {
    return { title: "No se pudo contactar con el canister", message: "Comprueba que la red local de ICP esté disponible y que el backend esté desplegado. Puedes volver a intentar la consulta con Actualizar.", detail, clockError: false };
  }
  return { title: "La consulta no se pudo completar", message: "El backend devolvió un error o una respuesta no disponible. Revisa los detalles técnicos y vuelve a intentar la consulta cuando se resuelva el problema.", detail, clockError: false };
}

export function formatTimestamp(value: bigint, unit: "ns" | "ms"): { label: string; iso?: string } {
  const milliseconds = unit === "ns" ? value / 1_000_000n : value;
  const maxDateMilliseconds = 8_640_000_000_000_000n;
  if (milliseconds < -maxDateMilliseconds || milliseconds > maxDateMilliseconds) {
    return { label: "Fecha fuera del rango admitido" };
  }
  const date = new Date(Number(milliseconds));
  return { label: new Intl.DateTimeFormat("es", { dateStyle: "medium", timeStyle: "short" }).format(date), iso: date.toISOString() };
}
