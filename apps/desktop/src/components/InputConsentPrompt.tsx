import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import { BusyIcon } from "@/components/common";
import { messageFrom, nativeAvailable, nativeCommand } from "@/lib/bridge";

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);

export interface InputConsentRequest { peerId: string; peerName: string; request: string; flow: boolean }

export function InputConsentPrompt({ onAnswered }: { onAnswered: () => void }) {
  const [requests, setRequests] = useState<InputConsentRequest[]>([]);
  const [answering, setAnswering] = useState<"allow" | "deny" | null>(null);
  const current = requests[0];

  const refresh = useCallback(async () => {
    try { setRequests(await nativeCommand<InputConsentRequest[]>("get_input_consent_requests")); } catch { setRequests([]); }
  }, []);

  useEffect(() => {
    if (!nativeAvailable) return;
    void refresh();
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen("input-consent-request", () => void refresh()).then(stop => { if (cancelled) stop(); else unlisten = stop; });
    return () => { cancelled = true; unlisten?.(); };
  }, [refresh]);

  useEffect(() => {
    if (!current) return;
    const timer = window.setInterval(() => void refresh(), 2000);
    return () => window.clearInterval(timer);
  }, [current, refresh]);

  async function answer(allow: boolean) {
    if (!current) return;
    setAnswering(allow ? "allow" : "deny");
    try {
      await nativeCommand("respond_input_consent", { peerId: current.peerId, request: current.request, allow });
      if (allow) toast.success(`Mouse and keyboard allowed for ${current.peerName}.`);
      onAnswered();
    } catch (error) { toast.error(messageFrom(error)); }
    finally { setAnswering(null); void refresh(); }
  }

  return <AlertDialog open={!!current}>
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>Allow control of this computer?</AlertDialogTitle>
        <AlertDialogDescription>{current ? `${current.peerName} wants to control this computer with mouse and keyboard${current.flow ? " (Flow)" : ""}. Allow?` : ""} {isMac ? "Control + Option + Esc" : "Ctrl + Alt + Esc"} ends control at any time.</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel disabled={!!answering} onClick={event => { event.preventDefault(); void answer(false); }}><BusyIcon busy={answering === "deny"} />Deny</AlertDialogCancel>
        <AlertDialogAction disabled={!!answering} onClick={event => { event.preventDefault(); void answer(true); }}><BusyIcon busy={answering === "allow"} />Allow</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>;
}
