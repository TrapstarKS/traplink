import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Monitor, MonitorSmartphone, RefreshCw, SlidersHorizontal } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldTitle } from "@/components/ui/field";
import { Kbd } from "@/components/ui/kbd";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
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
export type FlowLinkState = "connected" | "reconnecting" | "waiting" | "remote_closed" | "offline";
export interface FlowLinkStatus { state: FlowLinkState; attempt: number; nextRetryMs?: number | null; rttMs?: number | null; lastError?: string | null }
export interface FlowCaptureStatus { installed: boolean; reinstalls: number; lastReinstallAgoMs?: number | null; reason?: "removed" | "repair" | null }
export interface FlowError { code: string; peer?: string | null; detail: string }
interface FlowTiming { peer: string; elapsedMs: number }
export interface FlowStatus { supported: boolean; reason?: string | null; peers: Record<string, FlowPeerStatus>; saved?: Record<string, FlowPeerStatus>; connecting?: string | null; forwarding?: string | null; lastError?: FlowError | null; links?: Record<string, FlowLinkStatus>; capture?: FlowCaptureStatus | null; lastHandoff?: FlowTiming | null; lastReturn?: FlowTiming | null }

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);

const errorText: Record<string, { title: string; body: string }> = {
  connection_lost: { title: "Connection lost", body: "Control returned to this computer. Keep TrapLink open on both computers while it reconnects." },
  reconnect_failed: { title: "Couldn't reconnect", body: "Make sure the other computer is on, has TrapLink open and is on the same network. Then use Repair connection." },
  remote_closed: { title: "The other computer closed the connection", body: "Use Repair connection when you want to use Flow again." },
  restore_failed: { title: "Flow couldn't turn back on", body: "Your edge settings are saved. Check permissions on both computers, then turn Flow on." },
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

export function flowStatusLine(peerName: string, connected: boolean, active: boolean, controlling: boolean, switching: boolean, link?: FlowLinkStatus, capture?: FlowCaptureStatus | null): { label: string; variant: "default" | "secondary" | "destructive" | "outline"; busy?: boolean } {
  if (!active) return { label: "Off", variant: "outline" };
  if (link?.state === "remote_closed") return { label: "Disconnected by the other computer", variant: "outline" };
  if (link?.state === "reconnecting") return { label: `Reconnecting (${link.attempt})`, variant: "secondary", busy: true };
  if (link?.state === "waiting") return { label: `Retry ${link.attempt + 1} in ${Math.max(1, Math.ceil((link.nextRetryMs ?? 0) / 1000))} s`, variant: "secondary" };
  if (link ? link.state !== "connected" : !connected) return { label: "Other computer disconnected", variant: "outline" };
  if (controlling) return { label: `Controlling ${peerName}`, variant: "default" };
  if (switching) return { label: "Switching…", variant: "secondary", busy: true };
  if (capture && !capture.installed) return { label: "Capture stopped", variant: "destructive" };
  if (capture?.lastReinstallAgoMs != null && capture.lastReinstallAgoMs < RECENT_REINSTALL_MS) return { label: "Capture reinstalled", variant: "secondary" };
  return { label: "Ready", variant: "secondary" };
}

const defaults: FlowPeerStatus = { edge: "right", mode: "edge", delayMs: 250, blockWhileButton: true, clipboard: false };
const edges = [
  { value: "top", label: "Top", opposite: "bottom", icon: ArrowUp },
  { value: "left", label: "Left", opposite: "right", icon: ArrowLeft },
  { value: "right", label: "Right", opposite: "left", icon: ArrowRight },
  { value: "bottom", label: "Bottom", opposite: "top", icon: ArrowDown },
] as const;
const delays = [0, 150, 250, 500, 1000];

export function FlowControl({ peer, peers, available, clipboardSupported, clipboardAllowed, startOnLogin, onPermissionsChanged }: { peer: ConnectedPeer; peers: ConnectedPeer[]; available: boolean; clipboardSupported: boolean; clipboardAllowed: boolean; startOnLogin: boolean; onPermissionsChanged: () => Promise<void> }) {
  const [status, setStatus] = useState<FlowStatus | null>(null);
  const [draft, setDraft] = useState<FlowPeerStatus>(() => ({ ...defaults, clipboard: peer.flowClipboard ?? false }));
  const [saving, setSaving] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const [macPanes, setMacPanes] = useState<MacPane[] | null>(null);
  const [repairing, setRepairing] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const mounted = useRef(true);
  const operation = useRef(false);
  const reading = useRef(false);
  const revision = useRef(0);
  const active = status?.peers[peer.id];
  const saved = status?.saved?.[peer.id];
  const current = active ?? saved;

  const refresh = useCallback(async () => {
    if (!nativeAvailable || operation.current || reading.current) return;
    reading.current = true;
    const version = revision.current;
    try {
      const next = await nativeCommand<FlowStatus>("get_flow_status");
      if (mounted.current && version === revision.current) { setStatus(next); setReadError(null); }
    } catch (error) {
      if (mounted.current && version === revision.current) setReadError(messageFrom(error));
    } finally { reading.current = false; }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 1000);
    return () => { mounted.current = false; revision.current += 1; window.clearInterval(timer); };
  }, [refresh]);
  const armed = !!active;
  useEffect(() => { if (current) setDraft(current); }, [armed, current?.edge, current?.mode, current?.delayMs, current?.blockWhileButton, current?.clipboard]);

  function fail(error: unknown) {
    if (!mounted.current) return;
    const text = messageFrom(error);
    const panes = macPermissionPanes(text);
    if (panes) setMacPanes(panes); else { setActionError(text); toast.error(text); }
  }
  const send = (enabled: boolean, next: FlowPeerStatus) => nativeCommand<FlowStatus>("set_flow", { peerId: peer.id, enabled, edge: next.edge, mode: next.mode, delayMs: next.delayMs, blockWhileButton: next.blockWhileButton, clipboard: next.clipboard });

  function beginOperation() {
    if (operation.current) return false;
    operation.current = true;
    revision.current += 1;
    setActionError(null);
    setSaving(true);
    return true;
  }
  function finishOperation() {
    operation.current = false;
    if (mounted.current) { setSaving(false); setWaiting(false); setRepairing(false); void refresh(); }
  }

  async function turnOn() {
    if (!beginOperation()) return;
    setMacPanes(null);
    try {
      setWaiting(true);
      let answer: "granted" | "outdated";
      try { answer = await askPeerForInput(peer.id, true); } finally { if (mounted.current) setWaiting(false); }
      if (!mounted.current) return;
      await allowLocally(peer.id, draft.clipboard ? ["input", "clipboard"] : ["input"]);
      await onPermissionsChanged();
      if (!mounted.current) return;
      setStatus(await send(true, draft));
      if (answer === "outdated") toast.info(`Flow is on. Allow mouse and keyboard for this computer in the older TrapLink on ${peer.name}.`);
      else toast.success(`Flow is on with ${peer.name}.`);
    } catch (error) { fail(error); }
    finally { finishOperation(); }
  }
  async function turnOff() {
    if (!beginOperation()) return;
    setMacPanes(null);
    try {
      const next = await send(false, draft);
      setStatus(next);
      if (next.saved?.[peer.id]) setActionError("Flow is off, but the saved setup could not be removed.");
      else toast.success(active ? "Flow is off." : "Saved Flow setup removed.");
    }
    catch (error) { fail(error); }
    finally { finishOperation(); }
  }
  async function change(patch: Partial<FlowPeerStatus>) {
    if (operation.current) return;
    const next = { ...draft, ...patch };
    setDraft(next);
    setActionError(null);
    if (!active) {
      if (patch.clipboard === undefined || !beginOperation()) return;
      try {
        await nativeCommand("remember_flow_clipboard", { peerId: peer.id, enabled: patch.clipboard });
        await onPermissionsChanged();
      } catch (error) { setDraft(draft); fail(error); }
      finally { finishOperation(); }
      return;
    }
    if (!beginOperation()) return;
    try {
      if (patch.clipboard) { await allowLocally(peer.id, ["clipboard"]); await onPermissionsChanged(); }
      setStatus(await send(true, next));
    } catch (error) { setDraft(active); fail(error); }
    finally { finishOperation(); }
  }
  async function repair() {
    if (!beginOperation()) return;
    setRepairing(true);
    try {
      const next = await nativeCommand<FlowStatus>("repair_flow", { peerId: peer.id });
      setStatus(next);
      if (next.links?.[peer.id]?.state === "connected") toast.success("Connection restored.");
      else toast.info("Repair started. Keep TrapLink open on both computers.");
    } catch (error) { fail(error); }
    finally { finishOperation(); }
  }

  const heading = <div className="input-control-heading"><MonitorSmartphone aria-hidden="true" /><div><h3>Flow</h3><p>Use this screen’s edge to control {peer.name}.</p></div></div>;
  if (!nativeAvailable) return <section className="input-control">{heading}<Alert><MonitorSmartphone /><AlertTitle>Only available in the desktop app</AlertTitle><AlertDescription>Flow doesn't work in the browser preview.</AlertDescription></Alert></section>;
  if (!available || (status && !status.supported)) return <section className="input-control">{heading}<Alert><MonitorSmartphone /><AlertTitle>Not supported</AlertTitle><AlertDescription>{status?.reason ?? "Flow isn't supported on this platform yet."}</AlertDescription></Alert></section>;

  const controlling = status?.forwarding === peer.id;
  const connecting = status?.connecting === peer.id;
  const link = status?.links?.[peer.id];
  const line = readError ? { label: "Status unavailable", variant: "outline" as const }
    : !status ? { label: "Checking Flow…", variant: "outline" as const, busy: true }
    : waiting ? { label: "Waiting for permission…", variant: "secondary" as const, busy: true }
    : saved && !active ? { label: "Paused", variant: "outline" as const }
    : flowStatusLine(peer.name, peer.connected, !!active, controlling, connecting, link, status.capture);
  const error = status?.lastError && (!status.lastError.peer || status.lastError.peer === peer.id) ? status.lastError : null;
  const explained = error ? errorText[error.code] ?? { title: "Flow error", body: error.detail } : null;
  const latency = link?.state === "connected" && link.rttMs != null ? link.rttMs : null;
  const handoff = status?.lastHandoff?.peer === peer.id ? status.lastHandoff.elapsedMs : null;
  const lastReturn = status?.lastReturn?.peer === peer.id ? status.lastReturn.elapsedMs : null;
  const elapsed = (ms: number) => ms < 1 ? "<1 ms" : `${ms} ms`;
  const shortcut = isMac ? <><Kbd>Control</Kbd> + <Kbd>Option</Kbd> + <Kbd>Esc</Kbd></> : <><Kbd>Ctrl</Kbd> + <Kbd>Alt</Kbd> + <Kbd>Esc</Kbd></>;
  const connected = link ? link.state === "connected" : peer.connected;
  const controlsDisabled = saving || !status || !!readError || (!!active && !connected);
  const assigned = new Map<FlowEdge, string>();
  for (const [id, config] of Object.entries({ ...status?.saved, ...status?.peers })) {
    if (id !== peer.id) assigned.set(config.edge, peers.find(computer => computer.id === id)?.name ?? "another computer");
  }
  const edgeTaken = assigned.has(draft.edge);
  const selectedEdge = edges.find(edge => edge.value === draft.edge)!;
  const delayOptions = delays.includes(draft.delayMs) ? delays : [...delays, draft.delayMs].sort((a, b) => a - b);
  const modifier = isMac ? "Control" : "Ctrl";
  const route = draft.mode === "ctrl_edge" ? `Hold ${modifier} and push past the ${draft.edge} edge.`
    : draft.delayMs === 0 ? `Push past the ${draft.edge} edge to switch.`
    : `Keep pushing at the ${draft.edge} edge for ${draft.delayMs} ms to switch.`;
  const connectionNeedsHelp = link?.state === "remote_closed" || link?.state === "offline";
  const liveStatus = waiting ? `Waiting for permission on ${peer.name}.` : readError ? "Flow status could not be checked." : link?.state === "reconnecting" || link?.state === "waiting" ? `Reconnecting to ${peer.name}.` : `${peer.name}. Flow ${line.label}.`;
  const guidance = waiting ? `Accept the mouse and keyboard request on ${peer.name}.`
    : readError ? "Check the status again to change Flow settings."
    : !status ? "Reading this computer’s Flow settings."
    : controlling ? `Push past the ${selectedEdge.opposite} edge on ${peer.name} to return here.`
    : connecting ? `Waiting for ${peer.name} to acknowledge the switch. Your pointer stays here until then.`
    : link?.state === "reconnecting" || link?.state === "waiting" ? "Control returned here. Keep both computers on the same network with TrapLink open."
    : active && (connectionNeedsHelp || !connected) ? `Wake ${peer.name}, open TrapLink there, then repair the connection.`
    : active && status.capture && !status.capture.installed ? "Repair the connection to restart mouse and keyboard capture."
    : active ? route
    : edgeTaken ? assigned.size === edges.length ? "All edges are assigned. Open another computer’s Flow settings to free an edge." : "Choose an available edge before turning Flow on."
    : !peer.connected ? `Reconnect ${peer.name} to turn Flow on.`
    : saved ? "Your edge settings are saved. Turn Flow on to use them again."
    : "Choose an edge, then turn Flow on to request access on the other computer.";
  const canRepair = active && !controlling && !connecting && !readError && (connectionNeedsHelp || !connected || !!error || status?.capture?.installed === false);

  return <section className="input-control flow-control" aria-label={`Flow with ${peer.name}`}>
    <div className="flow-heading">{heading}<Switch id={`flow-${peer.id}`} aria-label={`Flow with ${peer.name}`} checked={!!active} disabled={saving || (!active && (!peer.connected || !status || !!readError || edgeTaken))} onCheckedChange={value => void (value ? turnOn() : turnOff())} /></div>
    <span role="status" aria-atomic="true" className="sr-only">{liveStatus}</span>
    <div className="flow-state-panel">
      <div className="flow-state-heading">
        <Badge variant={line.variant} className="gap-1">{line.busy ? <Spinner data-icon="inline-start" /> : null}{line.label}</Badge>
        {latency != null && !readError ? <span className="flow-latency">{latency} ms round trip</span> : null}
        {saving && !waiting && !repairing ? <Spinner aria-label="Saving Flow settings" /> : null}
      </div>
      <p>{guidance}</p>
      {!readError && (handoff != null || lastReturn != null) ? <dl className="flow-timings" aria-label="Measured switches">
        {handoff != null ? <div title="From the edge trigger until this computer begins forwarding input."><dt>Last switch to {peer.name}</dt><dd>{elapsed(handoff)}</dd></div> : null}
        {lastReturn != null ? <div title="From crossing the return edge until the other computer confirms release."><dt>Last return to {peer.name}</dt><dd>{elapsed(lastReturn)}</dd></div> : null}
      </dl> : null}
      {canRepair ? <Button type="button" variant="outline" size="sm" disabled={saving || link?.state === "reconnecting"} onClick={() => void repair()}>{repairing ? <Spinner data-icon="inline-start" /> : <RefreshCw data-icon="inline-start" />}Repair connection</Button> : null}
      {saved && !active && !readError ? <Button type="button" variant="outline" size="sm" disabled={saving} onClick={() => void turnOff()}>Remove saved setup</Button> : null}
    </div>
    {macPanes ? <MacPermissionAlert panes={macPanes} /> : null}
    {readError ? <Alert variant="destructive"><AlertTitle>Couldn't check Flow</AlertTitle><AlertDescription className="flex flex-col gap-3"><span>{readError}</span><Button variant="outline" size="sm" onClick={() => void refresh()}><RefreshCw data-icon="inline-start" />Check again</Button></AlertDescription></Alert> : null}
    {actionError ? <Alert variant="destructive"><AlertTitle>Couldn't apply this change</AlertTitle><AlertDescription>{actionError}</AlertDescription></Alert> : null}
    {error && explained && !actionError ? <Alert variant={error.code === "hook_removed" ? "default" : "destructive"}><AlertTitle>{explained.title}</AlertTitle><AlertDescription>{explained.body}</AlertDescription></Alert> : null}
    <Field orientation="horizontal" data-disabled={controlsDisabled || !clipboardSupported}>
      <FieldContent>
        <FieldLabel htmlFor={`flow-clipboard-${peer.id}`}>Share clipboard on switch</FieldLabel>
        <FieldDescription id={`flow-clipboard-help-${peer.id}`}>
          {clipboardSupported ? <>
            Transfer copied text, images, or files when you switch to {peer.name} or return here.{" "}
            {active && draft.clipboard && !clipboardAllowed ? "Allow Clipboard in Permissions on this computer to resume transfers."
              : `Allow Clipboard on ${peer.name}. Copied files also need Files permission on both computers.`}
            {!active ? " This choice applies when you turn Flow on." : null}
          </> : "Clipboard sharing isn't supported on this computer."}
        </FieldDescription>
      </FieldContent>
      <Switch id={`flow-clipboard-${peer.id}`} aria-describedby={`flow-clipboard-help-${peer.id}`} checked={draft.clipboard} disabled={controlsDisabled || !clipboardSupported} onCheckedChange={value => void change({ clipboard: value })} />
    </Field>
    <FieldGroup className="flow-settings">
      <Field data-disabled={controlsDisabled}>
        <FieldTitle id={`flow-edge-label-${peer.id}`}>Edge of this screen</FieldTitle>
        <FieldDescription id={`flow-edge-help-${peer.id}`}>Choose the side facing {peer.name}.</FieldDescription>
        <ToggleGroup type="single" variant="outline" role="radiogroup" className="flow-edge-map" aria-labelledby={`flow-edge-label-${peer.id}`} aria-describedby={`flow-edge-help-${peer.id}`} value={edgeTaken ? "" : draft.edge} disabled={controlsDisabled} onValueChange={value => { if (value) void change({ edge: value as FlowEdge }); }}>
          <div className="flow-local-screen" aria-hidden="true"><Monitor /><span>This computer</span></div>
          {edges.map(edge => {
            const owner = assigned.get(edge.value);
            const EdgeIcon = edge.icon;
            return <ToggleGroupItem key={edge.value} value={edge.value} data-edge={edge.value} className="flow-edge-option" disabled={controlsDisabled || !!owner} aria-label={`${edge.label} edge${owner ? `, in use by ${owner}` : ""}`} title={owner ? `In use by ${owner}` : undefined}><EdgeIcon aria-hidden="true" /><span>{edge.label}{owner ? <small>In use</small> : null}</span></ToggleGroupItem>;
          })}
        </ToggleGroup>
      </Field>
      <Field data-disabled={controlsDisabled} className="flow-mode-field">
        <FieldTitle id={`flow-mode-label-${peer.id}`}>Switch method</FieldTitle>
        <ToggleGroup type="single" variant="outline" role="radiogroup" className="flow-mode-options" aria-labelledby={`flow-mode-label-${peer.id}`} value={draft.mode} disabled={controlsDisabled} onValueChange={value => { if (value) void change({ mode: value as FlowMode }); }}>
          <ToggleGroupItem value="edge">Edge only</ToggleGroupItem>
          <ToggleGroupItem value="ctrl_edge">{modifier} + edge</ToggleGroupItem>
        </ToggleGroup>
      </Field>
      <Field className="flow-delay-field" data-disabled={controlsDisabled || draft.mode === "ctrl_edge"}>
        <FieldLabel htmlFor={`flow-delay-${peer.id}`}>Edge delay</FieldLabel>
        <Select value={String(draft.mode === "ctrl_edge" ? 0 : draft.delayMs)} disabled={controlsDisabled || draft.mode === "ctrl_edge"} onValueChange={value => void change({ delayMs: Number(value) })}>
          <SelectTrigger id={`flow-delay-${peer.id}`} className="flow-delay-select" aria-describedby={`flow-delay-help-${peer.id}`}><SelectValue /></SelectTrigger>
          <SelectContent><SelectGroup>{delayOptions.map(delay => <SelectItem key={delay} value={String(delay)}>{delay === 0 ? "No delay" : `${delay} ms`}</SelectItem>)}</SelectGroup></SelectContent>
        </Select>
        <FieldDescription id={`flow-delay-help-${peer.id}`}>{draft.mode === "ctrl_edge" ? `Holding ${modifier} switches immediately.` : "A short delay helps prevent accidental switches."}</FieldDescription>
      </Field>
    </FieldGroup>
    <Disclosure label="More Flow settings" icon={SlidersHorizontal}>
      <FieldGroup>
        <Field orientation="horizontal" data-disabled={controlsDisabled}><FieldContent><FieldLabel htmlFor={`flow-button-${peer.id}`}>Don't switch while dragging</FieldLabel><FieldDescription>Stay on this computer while a mouse button is held.</FieldDescription></FieldContent><Switch id={`flow-button-${peer.id}`} checked={draft.blockWhileButton} disabled={controlsDisabled} onCheckedChange={value => void change({ blockWhileButton: value })} /></Field>
      </FieldGroup>
      {isMac ? <p className="input-control-note">On Mac, ⌘ Command reaches the other computer as Ctrl, and Control as the Windows key.</p> : null}
      <p className="input-control-note">{active ? "Changes apply immediately. Flow keeps running when this window closes." : "Your choices apply when you turn Flow on."} {startOnLogin ? "TrapLink starts with this computer." : "Enable “Start with computer” in Preferences to resume after signing in."}</p>
    </Disclosure>
    <div className="flow-return-note"><span>Emergency stop</span><div>{shortcut}</div><p>Turns off mouse and keyboard sharing and button remapping.</p></div>
  </section>;
}
