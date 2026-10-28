import { useCallback, useEffect, useRef, useState } from "react";
import { describeQueryError, PAGE_SIZE, type QueryFailure } from "../lib/registry";

export interface ConnectionState {
  status: "checking" | "connected" | "error";
  canisterId?: string;
  error?: QueryFailure;
}

interface PageState<T> {
  items: T[];
  cursor: string | null;
  hasMore: boolean;
  loading: boolean;
  error?: QueryFailure;
}

export function useRegistryPage<T>(
  fetchPage: (cursor: string | null) => Promise<{ items: T[]; canisterId: string }>,
  itemKey: (item: T) => string,
  onConnection: (state: ConnectionState) => void,
  blocked: boolean,
) {
  const [page, setPage] = useState<PageState<T>>({ items: [], cursor: null, hasMore: false, loading: true });
  const requestNumber = useRef(0);
  const pending = useRef(false);

  const load = useCallback((cursor: string | null, append: boolean) => {
    if (blocked || pending.current) return;
    pending.current = true;
    const currentRequest = ++requestNumber.current;
    void Promise.resolve().then(() => {
      if (currentRequest !== requestNumber.current) return null;
      return fetchPage(cursor);
    }).then((response) => {
      if (!response || currentRequest !== requestNumber.current) return;
      setPage((previous) => ({
        items: Array.from(new Map((append ? [...previous.items, ...response.items] : response.items).map((item) => [itemKey(item), item])).values()),
        cursor: response.items.length ? itemKey(response.items[response.items.length - 1]) : cursor,
        hasMore: response.items.length === PAGE_SIZE,
        loading: false,
      }));
      onConnection({ status: "connected", canisterId: response.canisterId });
    }).catch((error: unknown) => {
      if (currentRequest !== requestNumber.current) return;
      const failure = describeQueryError(error);
      setPage((previous) => ({ ...previous, loading: false, error: failure }));
      onConnection({ status: "error", error: failure });
    }).finally(() => {
      if (currentRequest === requestNumber.current) pending.current = false;
    });
  }, [blocked, fetchPage, itemKey, onConnection]);

  useEffect(() => {
    void load(null, false);
    return () => {
      // Ignore late responses after selection changes, unmounts or StrictMode cleanup.
      requestNumber.current += 1;
      pending.current = false;
    };
  }, [load]);

  function startRequest(cursor: string | null, append: boolean) {
    if (blocked || pending.current) return;
    setPage((previous) => ({ ...previous, loading: true, error: undefined }));
    onConnection({ status: "checking" });
    void load(cursor, append);
  }

  return { ...page, refresh: () => startRequest(null, false), loadMore: () => startRequest(page.cursor, true) };
}
