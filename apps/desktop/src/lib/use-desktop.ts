import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { messageFrom, nativeAvailable, nativeCommand, noCapabilities, type Capabilities, type DesktopSnapshot } from "./bridge";

export function useDesktop() {
  const [snapshot, setSnapshot] = useState<DesktopSnapshot | null>(null);
  const [capabilities, setCapabilities] = useState<Capabilities>(noCapabilities);
  const [loading, setLoading] = useState(nativeAvailable);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [inputStatus, setInputStatus] = useState<{ sendingTo: string | null; receivingFrom: string | null }>({ sendingTo: null, receivingFrom: null });
  const inFlight = useRef<Promise<void> | null>(null);
  const actions = useRef(new Map<symbol, string>());
  const mounted = useRef(true);

  const refresh = useCallback(async (afterCurrent = false) => {
    if (!nativeAvailable) return;
    if (inFlight.current) {
      await inFlight.current;
      if (!afterCurrent) return;
      if (inFlight.current) return inFlight.current;
    }
    const request = (async () => {
      try {
        const [next, input] = await Promise.all([
          nativeCommand<DesktopSnapshot>("get_snapshot"),
          nativeCommand<{ sendingTo: string | null; receivingFrom: string | null }>("get_input_status").catch(() => null),
        ]);
        if (mounted.current) { setSnapshot(next); if (input) setInputStatus(input); setError(null); }
      } catch (err) {
        if (mounted.current) setError(messageFrom(err));
      } finally {
        if (mounted.current) setLoading(false);
      }
    })();
    inFlight.current = request;
    try { await request; } finally { if (inFlight.current === request) inFlight.current = null; }
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!nativeAvailable) return () => { mounted.current = false; };
    void refresh();
    void nativeCommand<Capabilities>("get_capabilities").then(value => {
      if (mounted.current) setCapabilities(value);
    }).catch(err => { if (mounted.current) setError(messageFrom(err)); });
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 5000);
    const onFocus = () => { void refresh(); };
    window.addEventListener("focus", onFocus);
    return () => { mounted.current = false; window.clearInterval(timer); window.removeEventListener("focus", onFocus); };
  }, [refresh]);

  const run = useCallback(async <T,>(name: string, action: () => Promise<T>, success?: string, options?: { concurrent?: boolean }): Promise<T | undefined> => {
    if (actions.current.size && !options?.concurrent) {
      toast.info("Wait for the current action to finish.");
      return undefined;
    }
    const token = Symbol(name);
    actions.current.set(token, name);
    setBusy(name);
    try {
      const value = await action();
      await refresh(true);
      if (success) toast.success(success);
      return value;
    } catch (err) {
      toast.error(messageFrom(err));
      return undefined;
    } finally {
      actions.current.delete(token);
      if (mounted.current) setBusy(actions.current.values().next().value ?? null);
    }
  }, [refresh]);

  return { snapshot, capabilities, inputStatus, loading, error, busy, refresh, run, native: nativeAvailable };
}

export type Desktop = ReturnType<typeof useDesktop>;
