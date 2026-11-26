import { useCallback, useEffect, useRef, useState } from "react";
import { MousePointer2, Square } from "lucide-react";
import { Kbd } from "@/components/ui/kbd";
import { Spinner } from "@/components/ui/spinner";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/common";
import { messageFrom, nativeCommand, type ConnectedPeer, type DesktopSnapshot } from "@/lib/bridge";
import { allowLocally, askPeerForInput } from "@/lib/permissions";
import { needsInputGrantDisclosure } from "@/lib/permission-rules";

type InputEvent = { type: "key"; key: string; down: boolean } | { type: "move"; x: number; y: number } | { type: "button"; button: number; down: boolean } | { type: "scroll"; x: number; y: number };

export function InputControl({ peer, enabled, onPermissionsChanged }: { peer: ConnectedPeer; enabled: boolean; onPermissionsChanged: () => Promise<void> }) {
  const [active, setActive] = useState(false);
  const [starting, setStarting] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const [confirmingGrant, setConfirmingGrant] = useState(false);
  const pane = useRef<HTMLDivElement>(null);
  const activeRef = useRef(false);
  const animationFrame = useRef<number | null>(null);
  const pendingMove = useRef<InputEvent | null>(null);
  const commandQueue = useRef<Promise<unknown>>(Promise.resolve());
  const generation = useRef(0);
  const mounted = useRef(true);

  const stop = useCallback(() => {
    const wasActive = activeRef.current;
    activeRef.current = false;
    generation.current += 1;
    setActive(false);
    if (animationFrame.current !== null) { cancelAnimationFrame(animationFrame.current); animationFrame.current = null; }
    pendingMove.current = null;
    if (wasActive) void nativeCommand("set_peer_control", { id: peer.id, enabled: false }).catch(() => undefined);
  }, [peer.id]);

  const send = useCallback((event: InputEvent) => {
    if (!activeRef.current) return;
    const session = generation.current;
    commandQueue.current = commandQueue.current.then(() => {
      if (activeRef.current && generation.current === session) return nativeCommand("send_input", { peerId: peer.id, event });
    }).catch(error => { if (activeRef.current) { stop(); toast.error(messageFrom(error)); } });
  }, [peer.id, stop]);

  // Control uses one pair permission, so starting it also lets the peer control this computer.
  async function requestStart() {
    let disclose: boolean;
    try {
      const snapshot = await nativeCommand<DesktopSnapshot>("get_snapshot");
      disclose = needsInputGrantDisclosure(snapshot.peers.find(item => item.id === peer.id), snapshot.settings);
    } catch (error) { if (mounted.current) toast.error(messageFrom(error)); return; }
    if (!mounted.current) return;
    if (disclose) setConfirmingGrant(true); else void start();
  }
  async function start() {
    const request = ++generation.current;
    setStarting(true);
    try {
      setWaiting(true);
      let answer: "granted" | "outdated";
      try { answer = await askPeerForInput(peer.id, false); } finally { if (mounted.current) setWaiting(false); }
      if (!mounted.current || generation.current !== request) return;
      await allowLocally(peer.id, ["input"]);
      await onPermissionsChanged();
      if (!mounted.current || generation.current !== request) return;
      if (answer === "outdated") toast.info(`${peer.name} runs an older TrapLink. If control is refused, allow mouse and keyboard there.`);
      await nativeCommand("set_peer_control", { id: peer.id, enabled: true });
      if (!mounted.current || generation.current !== request) {
        void nativeCommand("set_peer_control", { id: peer.id, enabled: false }).catch(() => undefined);
        return;
      }
      activeRef.current = true;
      setActive(true);
    } catch (error) { if (mounted.current && generation.current === request) toast.error(messageFrom(error)); }
    finally { if (mounted.current) setStarting(false); }
  }

  useEffect(() => {
    if (!active) return;
    const node = pane.current;
    node?.focus();
    const keepalive = window.setInterval(() => {
      void nativeCommand("input_keepalive", { peerId: peer.id }).catch(() => stop());
    }, 1000);
    const wheel = (event: WheelEvent) => {
      if (!activeRef.current) return;
      event.preventDefault();
      send({ type: "scroll", x: Math.max(-10, Math.min(10, Math.round(event.deltaX / 45))), y: Math.max(-10, Math.min(10, Math.round(event.deltaY / 45))) });
    };
    node?.addEventListener("wheel", wheel, { passive: false });
    window.addEventListener("blur", stop);
    const visibility = () => { if (document.hidden) stop(); };
    document.addEventListener("visibilitychange", visibility);
    return () => { window.clearInterval(keepalive); node?.removeEventListener("wheel", wheel); window.removeEventListener("blur", stop); document.removeEventListener("visibilitychange", visibility); };
  }, [active, peer.id, send, stop]);

  useEffect(() => { if (!enabled || !peer.connected) stop(); }, [enabled, peer.connected, stop]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current += 1;
      const wasActive = activeRef.current;
      activeRef.current = false;
      if (animationFrame.current !== null) cancelAnimationFrame(animationFrame.current);
      if (wasActive) void nativeCommand("set_peer_control", { id: peer.id, enabled: false }).catch(() => undefined);
    };
  }, [peer.id]);

  return <section className="input-control">{active ? <><div ref={pane} tabIndex={0} role="application" aria-label={`Controlling ${peer.name}. Press Escape to stop.`} className="input-control-pane" onBlur={stop} onContextMenu={event => event.preventDefault()} onKeyDown={event => { event.preventDefault(); event.stopPropagation(); if (event.code === "Escape") { stop(); return; } if (!event.repeat) send({ type: "key", key: event.code, down: true }); }} onKeyUp={event => { event.preventDefault(); event.stopPropagation(); if (event.code !== "Escape") send({ type: "key", key: event.code, down: false }); }} onPointerMove={event => { const rect = event.currentTarget.getBoundingClientRect(); pendingMove.current = { type: "move", x: Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)), y: Math.max(0, Math.min(1, (event.clientY - rect.top) / rect.height)) }; if (animationFrame.current === null) animationFrame.current = requestAnimationFrame(() => { animationFrame.current = null; if (pendingMove.current) send(pendingMove.current); pendingMove.current = null; }); }} onPointerDown={event => { event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId); event.currentTarget.focus(); if (event.button <= 2) send({ type: "button", button: event.button, down: true }); }} onPointerUp={event => { if (event.button <= 2) send({ type: "button", button: event.button, down: false }); if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); }} onPointerCancel={stop}><MousePointer2 /><strong>Controlling {peer.name}</strong><span>Move the mouse, click or type in this area</span><span className="input-control-exit"><Kbd>Esc</Kbd> to stop</span></div><Button variant="destructive" onClick={stop}><Square data-icon="inline-start" />Stop control</Button></> : <><Button variant="outline" disabled={!enabled || !peer.connected || starting} onClick={() => void requestStart()}>{starting ? <Spinner data-icon="inline-start" /> : <MousePointer2 data-icon="inline-start" />}{waiting ? `Waiting for permission on ${peer.name}…` : "Start control"}</Button><p className="input-control-note">Control {peer.name} from an area of this window. Esc stops. Starting also lets {peer.name} control this computer.</p><ConfirmDialog open={confirmingGrant} onOpenChange={setConfirmingGrant} title={`Let ${peer.name} control this computer too?`} description={`Starting control also lets ${peer.name} control this computer without asking. You can turn this off in Permissions for ${peer.name}.`} action="Start control" onConfirm={() => { setConfirmingGrant(false); void start(); }} /></>}</section>;
}
