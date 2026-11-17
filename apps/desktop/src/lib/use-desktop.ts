import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { messageFrom, nativeAvailable, nativeCommand, noCapabilities, type Capabilities, type DesktopSnapshot } from "./bridge";

type InputStatus = {
  sendingTo: string | null;
  receivingFrom: string | null;
  connectingTo: string | null;
  flowSession: string | null;
};

export function useDesktop() {
  const [snapshot, setSnapshot] = useState<DesktopSnapshot | null>(null);
  const [capabilities, setCapabilities] = useState<Capabilities>(noCapabilities);
  const [loading, setLoading] = useState(nativeAvailable);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [inputStatus, setInputStatus] = useState<InputStatus>({ sendingTo: null, receivingFrom: null, connectingTo: null, flowSession: null });
  const inputRevision = useRef(0);
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
        const revision = inputRevision.current;
        const [next, input] = await Promise.all([
          nativeCommand<DesktopSnapshot>("get_snapshot"),
          nativeCommand<InputStatus>("get_input_status").catch(() => null),
        ]);
        if (mounted.current) {
          setSnapshot(next);
          if (input && revision === inputRevision.current) setInputStatus(input);
          setError(null);
        }
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
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<InputStatus>("input-status-changed", ({ payload }) => {
      if (disposed) return;
      inputRevision.current++;
      setInputStatus(payload);
    }).then(stop => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => { /* The periodic snapshot also refreshes input status. */ });
    void refresh();
    void nativeCommand<Capabilities>("get_capabilities").then(value => {
      if (mounted.current) setCapabilities(value);
    }).catch(err => { if (mounted.current) setError(messageFrom(err)); });
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 5000);
    const onFocus = () => { void refresh(); };
    window.addEventListener("focus", onFocus);
    return () => { disposed = true; unlisten?.(); mounted.current = false; window.clearInterval(timer); window.removeEventListener("focus", onFocus); };
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

  const returnControl = useCallback((peerId: string, session: string) =>
    run("release-flow", () => nativeCommand("release_flow_control", { peerId, session }),
      "Flow control released.", { concurrent: true }), [run]);

  return { snapshot, capabilities, inputStatus, loading, error, busy, refresh, run, returnControl, native: nativeAvailable };
}

export type Desktop = ReturnType<typeof useDesktop>;
