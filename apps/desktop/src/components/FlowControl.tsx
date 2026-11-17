import { useCallback, useEffect, useState } from "react";
import { MonitorSmartphone, RefreshCw, Power, SlidersHorizontal } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Kbd } from "@/components/ui/kbd";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Disclosure } from "@/components/common";
import { MacPermissionAlert } from "@/components/MacPermissionAlert";
import { messageFrom, nativeAvailable, nativeCommand, type ConnectedPeer } from "@/lib/bridge";
import { allowLocally, askPeerForInput, macPermissionPanes, type MacPane } from "@/lib/permissions";

export type FlowEdge = "left" | "right" | "top" | "bottom";
export type FlowMode = "edge" | "ctrl_edge";
export interface FlowPeerStatus { edge: FlowEdge; mode: FlowMode; delayMs: number; blockWhileButton: boolean; clipboard: boolean }
export type FlowLinkState = "connected" | "reconnecting" | "waiting" | "gave_up" | "remote_closed" | "offline";
export interface FlowLinkStatus { state: FlowLinkState; attempt: number; maxAttempts: number; nextRetryMs?: number | null; rttMs?: number | null; lastError?: string | null }
export interface FlowCaptureStatus { installed: boolean; reinstalls: number; lastReinstallAgoMs?: number | null; reason?: "removed" | "repair" | null }
export interface FlowError { code: string; peer?: string | null; detail: string }
export interface FlowStatus { supported: boolean; reason?: string | null; peers: Record<string, FlowPeerStatus>; connecting?: string | null; forwarding?: string | null; lastError?: FlowError | null; links?: Record<string, FlowLinkStatus>; capture?: FlowCaptureStatus | null }

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);

const errorText: Record<string, { title: string; body: string }> = {
  connection_lost: { title: "Connection lost", body: "Control returned to this computer. TrapLink is reconnecting automatically." },
  reconnect_failed: { title: "Couldn't reconnect", body: "Make sure the other computer is on, has TrapLink open and is on the same network. Then use Repair connection." },
  remote_closed: { title: "The other computer closed the connection", body: "Use Repair connection when you want to use Flow again." },
  restore_failed: { title: "Flow couldn't turn back on", body: "Your Flow choice is saved. Fix the cause below and Flow turns back on by itself." },
  hook_removed: { title: "Capture reinstalled", body: "The system turned off mouse and keyboard capture. It was turned back on and control returned to this computer." },
  hook_failed: { title: "Capture unavailable", body: "Use Repair connection or restart TrapLink." },
  permission_changed: { title: "Permission changed", body: "Mouse and keyboard were turned off. Turn Flow off and on again to allow them." },
  peer_denied: { title: "Other computer hasn't allowed access", body: "Turn Flow off and on again to ask the other computer for permission." },
  peer_busy: { title: "Other computer is busy", body: "The other computer is already in another control session. End that session and try again." },
  peer_accessibility: { title: "Accessibility not allowed on the Mac", body: "On the other computer, turn on TrapLink in System Settings > Privacy & Security > Accessibility." },
  timeout: { title: "No response", body: "The other computer didn't confirm the switch in time. Control stays on this computer." },
  start_failed: { title: "Couldn't switch", body: "The other computer refused to start control." },
};
const RECENT_REINSTALL_MS = 15000;

function statusLine(peerName: string, connected: boolean, active: boolean, controlling: boolean, switching: boolean, link?: FlowLinkStatus, capture?: FlowCaptureStatus | null): { label: string; variant: "default" | "secondary" | "destructive" | "outline"; busy?: boolean } {
  if (!active) return { label: "Off", variant: "outline" };
  if (link?.state === "gave_up") return { label: "No connection", variant: "destructive" };
  if (link?.state === "remote_closed") return { label: "Disconnected by the other computer", variant: "outline" };
  const attempt = link ? `(attempt ${Math.max(link.attempt, 1)} of ${link.maxAttempts})` : "";
  if (link?.state === "reconnecting") return { label: `Reconnecting… ${attempt}`, variant: "secondary", busy: true };
  if (link?.state === "waiting") return { label: `Reconnecting in ${Math.max(1, Math.ceil((link.nextRetryMs ?? 0) / 1000))} s ${attempt}`, variant: "secondary" };
  if (link ? link.state !== "connected" : !connected) return { label: "Other computer disconnected", variant: "outline" };
  if (controlling) return { label: `Controlling ${peerName}`, variant: "default" };
  if (switching) return { label: "Switching…", variant: "secondary", busy: true };
  if (capture && !capture.installed) return { label: "Capture stopped", variant: "destructive" };
  if (capture?.lastReinstallAgoMs != null && capture.lastReinstallAgoMs < RECENT_REINSTALL_MS) return { label: "Capture reinstalled", variant: "secondary" };
  return { label: "Ready", variant: "secondary" };
}

const defaults: FlowPeerStatus = { edge: "right", mode: "edge", delayMs: 250, blockWhileButton: true, clipboard: false };
const edges: { value: FlowEdge; label: string }[] = [{ value: "left", label: "Left" }, { value: "right", label: "Right" }, { value: "top", label: "Above" }, { value: "bottom", label: "Below" }];
const delays = [0, 150, 250, 500, 1000];

export function FlowControl({ peer, available, startOnLogin, onPermissionsChanged }: { peer: ConnectedPeer; available: boolean; startOnLogin: boolean; onPermissionsChanged: () => Promise<void> }) {
  const [status, setStatus] = useState<FlowStatus | null>(null);
  const [draft, setDraft] = useState<FlowPeerStatus>(defaults);
  const [saving, setSaving] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const [macPanes, setMacPanes] = useState<MacPane[] | null>(null);
  const [repairing, setRepairing] = useState(false);
  const active = status?.peers[peer.id];

  const refresh = useCallback(async () => {
    if (!nativeAvailable) return;
    try { setStatus(await nativeCommand<FlowStatus>("get_flow_status")); } catch { setStatus(null); }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(timer);
  }, [refresh]);
  useEffect(() => { if (active) setDraft(active); }, [active?.edge, active?.mode, active?.delayMs, active?.blockWhileButton, active?.clipboard]);

  function fail(error: unknown) {
    const text = messageFrom(error);
    const panes = macPermissionPanes(text);
    if (panes) setMacPanes(panes); else toast.error(text);
    void refresh();
  }
  const send = (enabled: boolean, next: FlowPeerStatus) => nativeCommand<FlowStatus>("set_flow", { peerId: peer.id, enabled, edge: next.edge, mode: next.mode, delayMs: next.delayMs, blockWhileButton: next.blockWhileButton, clipboard: next.clipboard });

  async function turnOn() {
    setSaving(true);
    setMacPanes(null);
    try {
      await allowLocally(peer.id, draft.clipboard ? ["input", "clipboard"] : ["input"]);
      await onPermissionsChanged();
      setWaiting(true);
      let answer: "granted" | "outdated";
      try { answer = await askPeerForInput(peer.id, true); } finally { setWaiting(false); }
      setStatus(await send(true, draft));
      if (answer === "outdated") toast.info(`Flow is on. ${peer.name} runs an older TrapLink: confirm there that mouse and keyboard are allowed for this computer.`);
      else toast.success(`Flow is on with ${peer.name}.`);
    } catch (error) { fail(error); }
    finally { setSaving(false); }
  }
  async function turnOff() {
    setSaving(true);
    setMacPanes(null);
    try { setStatus(await send(false, draft)); toast.success("Flow is off."); }
    catch (error) { fail(error); }
    finally { setSaving(false); }
  }
  async function change(patch: Partial<FlowPeerStatus>) {
    const next = { ...draft, ...patch };
    setDraft(next);
    if (!active) return;
    setSaving(true);
    try {
      if (patch.clipboard) { await allowLocally(peer.id, ["clipboard"]); await onPermissionsChanged(); }
      setStatus(await send(true, next));
    } catch (error) { fail(error); }
    finally { setSaving(false); }
  }
  async function repair() {
    setRepairing(true);
    try {
      const next = await nativeCommand<FlowStatus>("repair_flow", { peerId: peer.id });
      setStatus(next);
      if (next.links?.[peer.id]?.state === "connected") toast.success("Connection restored.");
      else toast.error("Still not connected. TrapLink keeps trying.");
    } catch (error) { toast.error(messageFrom(error)); void refresh(); }
    finally { setRepairing(false); }
  }

  const heading = <div className="input-control-heading"><MonitorSmartphone /><div><h3>Flow</h3><p>Move the pointer past the chosen edge to control {peer.name}.</p></div></div>;
  if (!nativeAvailable) return <section className="input-control">{heading}<Alert><MonitorSmartphone /><AlertTitle>Only available in the desktop app</AlertTitle><AlertDescription>Flow doesn't work in the browser preview.</AlertDescription></Alert></section>;
  if (!available || (status && !status.supported)) return <section className="input-control">{heading}<Alert><MonitorSmartphone /><AlertTitle>Not supported</AlertTitle><AlertDescription>{status?.reason ?? "Flow isn't supported on this platform yet."}</AlertDescription></Alert></section>;

  const controlling = status?.forwarding === peer.id;
  const connecting = status?.connecting === peer.id;
  const link = status?.links?.[peer.id];
  const line = waiting ? { label: `Waiting for permission on ${peer.name}…`, variant: "secondary" as const, busy: true } : statusLine(peer.name, peer.connected, !!active, controlling, connecting, link, status?.capture);
  const error = status?.lastError && (!status.lastError.peer || status.lastError.peer === peer.id) ? status.lastError : null;
  const explained = error ? errorText[error.code] ?? { title: "Flow error", body: error.detail } : null;
  const latency = link?.state === "connected" && link.rttMs != null ? link.rttMs : null;
  const shortcut = isMac ? <><Kbd>Control</Kbd> + <Kbd>Option</Kbd> + <Kbd>Esc</Kbd></> : <><Kbd>Ctrl</Kbd> + <Kbd>Alt</Kbd> + <Kbd>Esc</Kbd></>;

  return <section className="input-control">
    <div className="flow-heading">{heading}<Switch id={`flow-${peer.id}`} aria-label={`Turn on Flow with ${peer.name}`} checked={!!active} disabled={saving || (!active && !peer.connected)} onCheckedChange={value => void (value ? turnOn() : turnOff())} /></div>
    <div className="flex flex-wrap items-center gap-2">
      <Badge variant={line.variant} className="gap-1">{line.busy ? <Spinner data-icon="inline-start" /> : null}{line.label}</Badge>
      {latency != null ? <span className="text-xs text-muted-foreground">Latency: {latency} ms</span> : null}
      {saving && !waiting ? <Spinner /> : null}
      {active ? <Button type="button" variant="outline" size="sm" className="ml-auto" disabled={repairing || saving} onClick={() => void repair()}>{repairing ? <Spinner data-icon="inline-start" /> : <RefreshCw data-icon="inline-start" />}Repair connection</Button> : null}
    </div>
    {!peer.connected && !active ? <p className="input-control-note">Connect {peer.name} to turn on Flow.</p> : null}
    {macPanes ? <MacPermissionAlert panes={macPanes} /> : null}
    {error && explained ? <Alert variant={error.code === "hook_removed" ? "default" : "destructive"}><AlertTitle>{explained.title}</AlertTitle><AlertDescription className="flex flex-col gap-1"><span>{explained.body}</span>{error.detail && error.detail !== explained.body ? <span className="text-xs opacity-80">Details: {error.detail}</span> : null}</AlertDescription></Alert> : null}
    {active && !startOnLogin ? <Alert><Power /><AlertTitle>Tip</AlertTitle><AlertDescription>Turn on “Start with computer” in Preferences to keep Flow running with the window closed.</AlertDescription></Alert> : null}
    <Field><FieldLabel>Edge of this screen</FieldLabel><ToggleGroup type="single" variant="outline" className="justify-start flex-wrap" value={draft.edge} disabled={saving} onValueChange={value => { if (value) void change({ edge: value as FlowEdge }); }}>{edges.map(edge => <ToggleGroupItem key={edge.value} value={edge.value}>{edge.label}</ToggleGroupItem>)}</ToggleGroup></Field>
    <Disclosure label="More options" icon={SlidersHorizontal}>
      <FieldGroup>
        <Field><FieldLabel>Switch mode</FieldLabel><ToggleGroup type="single" variant="outline" className="justify-start flex-wrap" value={draft.mode} disabled={saving} onValueChange={value => { if (value) void change({ mode: value as FlowMode }); }}><ToggleGroupItem value="edge">On reaching the edge</ToggleGroupItem><ToggleGroupItem value="ctrl_edge">Holding {isMac ? "Control" : "Ctrl"} + edge</ToggleGroupItem></ToggleGroup></Field>
        <Field orientation="horizontal"><FieldContent><FieldLabel htmlFor={`flow-delay-${peer.id}`}>Edge delay</FieldLabel><FieldDescription>Prevents accidental switches.</FieldDescription></FieldContent><Select value={String(draft.delayMs)} disabled={saving || draft.mode === "ctrl_edge"} onValueChange={value => void change({ delayMs: Number(value) })}><SelectTrigger id={`flow-delay-${peer.id}`} className="w-32"><SelectValue /></SelectTrigger><SelectContent>{delays.map(delay => <SelectItem key={delay} value={String(delay)}>{delay === 0 ? "None" : `${delay} ms`}</SelectItem>)}</SelectContent></Select></Field>
        <Field orientation="horizontal"><FieldContent><FieldLabel htmlFor={`flow-button-${peer.id}`}>Don't switch while dragging</FieldLabel><FieldDescription>Blocks switching while a mouse button is held.</FieldDescription></FieldContent><Switch id={`flow-button-${peer.id}`} checked={draft.blockWhileButton} disabled={saving} onCheckedChange={value => void change({ blockWhileButton: value })} /></Field>
        <Field orientation="horizontal"><FieldContent><FieldLabel htmlFor={`flow-clipboard-${peer.id}`}>Copy and paste between computers</FieldLabel><FieldDescription>Sends copied text on each switch (up to 24 KiB).</FieldDescription></FieldContent><Switch id={`flow-clipboard-${peer.id}`} checked={draft.clipboard} disabled={saving} onCheckedChange={value => void change({ clipboard: value })} /></Field>
      </FieldGroup>
      {isMac ? <p className="input-control-note">On Mac, ⌘ Command reaches the other computer as Ctrl, and Control as the Windows key.</p> : null}
    </Disclosure>
    <p className="input-control-note">Stays on, even after restart, until you turn it off. {shortcut} returns control to this computer.</p>
  </section>;
}
