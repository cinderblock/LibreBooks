// React wiring for the API: the client, the current company, and a small
// in-app bus that re-publishes server change events to components that care
// about one record (an open form noticing someone else saved it).

import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { ApiClient } from './client';
import { invalidateForEvent } from './keys';
import type { ChangeEvent, CompanyId } from './types';

interface ApiContextValue {
  client: ApiClient;
  bus: EventTarget;
}

const ApiContext = createContext<ApiContextValue | null>(null);

export function ApiProvider({ client, children }: { client: ApiClient; children: ReactNode }) {
  const value = useMemo(() => ({ client, bus: new EventTarget() }), [client]);
  return <ApiContext.Provider value={value}>{children}</ApiContext.Provider>;
}

function useApiContext(): ApiContextValue {
  const v = useContext(ApiContext);
  if (!v) throw new Error('useApi outside <ApiProvider>');
  return v;
}

export function useApi(): ApiClient {
  return useApiContext().client;
}

const CompanyContext = createContext<CompanyId | null>(null);

export function CompanyProvider({ companyId, children }: { companyId: CompanyId; children: ReactNode }) {
  return <CompanyContext.Provider value={companyId}>{children}</CompanyContext.Provider>;
}

export function useCompanyId(): CompanyId {
  const co = useContext(CompanyContext);
  if (!co) throw new Error('useCompanyId outside a company route');
  return co;
}

export type LiveStatus = 'connecting' | 'open' | 'error';

/**
 * Subscribe to the company's change stream for as long as the shell is mounted:
 * every event invalidates the queries it affects and is re-published on the bus.
 * Returns the connection status for the "live" indicator.
 */
export function useCompanyEvents(co: CompanyId): LiveStatus {
  const { client, bus } = useApiContext();
  const qc = useQueryClient();
  const [status, setStatus] = useState<LiveStatus>('connecting');
  useEffect(() => {
    const sub = client.subscribe(co, {
      onChange(e) {
        invalidateForEvent(qc, co, e);
        bus.dispatchEvent(new CustomEvent<ChangeEvent>('change', { detail: e }));
      },
      onStatus: setStatus,
    });
    return () => sub.close();
  }, [client, bus, qc, co]);
  return status;
}

/** Call `onChange` whenever the server reports a change to this record. */
export function useRecordChanges(kind: string, id: string | null | undefined, onChange: (e: ChangeEvent) => void) {
  const { bus } = useApiContext();
  const cb = useRef(onChange);
  cb.current = onChange;
  useEffect(() => {
    if (!id) return;
    const handler = (ev: Event) => {
      const e = (ev as CustomEvent<ChangeEvent>).detail;
      if (e.kind === kind && e.id === id) cb.current(e);
    };
    bus.addEventListener('change', handler);
    return () => bus.removeEventListener('change', handler);
  }, [bus, kind, id]);
}
