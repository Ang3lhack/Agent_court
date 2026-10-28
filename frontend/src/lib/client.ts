import { getCanisterEnv } from "@icp-sdk/core/agent/canister-env";
import { createActor, type backendInterface } from "../backend/api/backend";
import { PAGE_SIZE, unwrapResult, withQueryTimeout } from "./registry";

type ReadOnlyRegistry = Pick<backendInterface, "list_missions" | "list_attempts">;
let connection: { client: ReadOnlyRegistry; canisterId: string } | undefined;

function getConnection() {
  if (!connection) {
    // Resolve inside the request so missing cookies appear as a visible error.
    const env = getCanisterEnv<{ readonly "PUBLIC_CANISTER_ID:backend"?: string }>();
    const canisterId = env["PUBLIC_CANISTER_ID:backend"];
    if (!canisterId) throw new Error("Falta PUBLIC_CANISTER_ID:backend en la cookie de configuración.");
    connection = { canisterId, client: createActor(canisterId, { agentOptions: { rootKey: env.IC_ROOT_KEY } }) };
  }
  return connection;
}

export async function readMissionPage(cursor: string | null) {
  const { client, canisterId } = getConnection();
  return { canisterId, items: unwrapResult(await withQueryTimeout(client.list_missions(cursor, PAGE_SIZE))) };
}

export async function readAttemptPage(missionId: string, cursor: string | null) {
  const { client, canisterId } = getConnection();
  return { canisterId, items: unwrapResult(await withQueryTimeout(client.list_attempts(missionId, cursor, PAGE_SIZE))) };
}
