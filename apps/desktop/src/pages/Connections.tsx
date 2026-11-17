import { Fragment, useEffect, useRef, useState } from "react";
import { ArrowDownLeft, ArrowRight, ArrowUpRight, Check, Clipboard, ClipboardPaste, Copy, FileUp, Keyboard, Link2, Monitor, Plus, RefreshCw, SlidersHorizontal, Unplug } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Textarea } from "@/components/ui/textarea";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemSeparator, ItemTitle } from "@/components/ui/item";
import { BusyIcon, ConfirmDialog, Disclosure, EmptyState, formatBytes, formattedDate, IconButton, PageHeading, PlatformName, relativeDate, SecurityNote, StatusBadge } from "@/components/common";
import type { Intent } from "@/App";
import { chooseFile, messageFrom, nativeCommand, type ConnectedPeer, type PairingInvite, type Transfer } from "@/lib/bridge";
import type { Desktop } from "@/lib/use-desktop";
import { allowLocally, revokeForPeer, type PeerFeature } from "@/lib/permissions";
import { InputControl } from "@/components/InputControl";
import { FlowControl } from "@/components/FlowControl";

const transferStatusLabels: Record<string, string> = { complete: "Complete", completed: "Complete", failed: "Failed", transferring: "Transferring", pending: "Pending", sending: "Sending", receiving: "Receiving" };

export function Connections({ desktop, intent, onIntentDone }: { desktop: Desktop; intent?: Intent | null; onIntentDone?: () => void }) {
  const [pairing, setPairing] = useState(false);
  useEffect(() => { if (intent?.kind === "pair") { setPairing(true); onIntentDone?.(); } }, [intent, onIntentDone]);
  const [selectedPeer, setSelectedPeer] = useState<ConnectedPeer | null>(null);
  const snapshot = desktop.snapshot;
  const peers = snapshot?.peers ?? [];
  const connected = peers.filter(peer => peer.connected);
  const transfers = snapshot?.transfers ?? [];

  async function sendFile(peer: ConnectedPeer) {
    await desktop.run(`file-${peer.id}`, async () => {
      const path = await chooseFile();
      if (path) { await nativeCommand<Transfer>("send_file", { peerId: peer.id, path }); toast.success("File sent."); }
    });
  }

  return <>
    <PageHeading title="Connections" description="Manage computers connected on your local network.">
      <Button disabled={!desktop.native || !desktop.capabilities.lanPairing || !!desktop.busy} onClick={() => setPairing(true)}><Plus data-icon="inline-start" />Connect computer</Button>
    </PageHeading>
    <div className="content-stack">
      <div className="connections-grid">
        <Card>
          <CardHeader><div className="card-heading-row"><div><CardTitle>Computers</CardTitle><CardDescription>{peers.length ? `${connected.length} of ${peers.length} paired ${peers.length === 1 ? "computer" : "computers"} connected.` : "Computers allowed to connect to this device."}</CardDescription></div><IconButton label="Refresh computers" disabled={!desktop.native || !!desktop.busy} onClick={() => void desktop.refresh()}><RefreshCw /></IconButton></div></CardHeader>
          <CardContent>
            {peers.length === 0 ? <EmptyState icon={Link2} title="No paired computers" description="Open TrapLink on both computers on the same network. Use Connect computer to create or enter an invite." /> : <ItemGroup>{peers.map((peer, index) => <Fragment key={peer.id}>{index ? <ItemSeparator /> : null}<Item size="sm" className="px-0">
              <ItemMedia variant="icon"><Monitor /></ItemMedia>
              <ItemContent><ItemTitle className="flex-wrap">{peer.name}<StatusBadge active={peer.connected}>{peer.connected ? "Connected" : "Disconnected"}</StatusBadge></ItemTitle><ItemDescription><PlatformName platform={peer.platform} /> · {peer.address}{!peer.trusted ? " · Waiting for authorization" : ""}</ItemDescription></ItemContent>
              <ItemActions>{peer.connected ? <><IconButton label={`Send clipboard to ${peer.name}`} hint={peer.permissions?.clipboard && snapshot?.settings.clipboardEnabled ? "Send clipboard" : "Allow clipboard in Permissions and details"} disabled={!snapshot?.settings.clipboardEnabled || !peer.permissions?.clipboard || !desktop.capabilities.clipboardSharing || !!desktop.busy} onClick={() => void desktop.run(`clipboard-${peer.id}`, () => nativeCommand("send_clipboard", { peerId: peer.id }), "Clipboard sent.")}><Clipboard /></IconButton><IconButton label={`Send file to ${peer.name}`} hint={peer.permissions?.files && snapshot?.settings.fileSharingEnabled ? "Send file" : "Allow files in Permissions and details"} disabled={!snapshot?.settings.fileSharingEnabled || !peer.permissions?.files || !desktop.capabilities.fileSharing || !!desktop.busy} onClick={() => void sendFile(peer)}><FileUp /></IconButton></> : <Button size="sm" variant="outline" disabled={!!desktop.busy} onClick={() => void desktop.run(`connect-${peer.id}`, () => nativeCommand("connect_peer", { id: peer.id }), "Computer connected.")}><BusyIcon busy={desktop.busy === `connect-${peer.id}`} />Connect</Button>}<IconButton label={`Permissions for ${peer.name}`} hint="Permissions and details" onClick={() => setSelectedPeer(peer)}><SlidersHorizontal /></IconButton></ItemActions>
            </Item></Fragment>)}</ItemGroup>}
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>This computer</CardTitle><CardDescription>How it appears on the local network.</CardDescription></CardHeader>
          <CardContent>
            <div className="device-summary"><div className="item-icon"><Monitor /></div><div><strong>{snapshot?.deviceName ?? "Desktop app required"}</strong><p>{snapshot ? <PlatformName platform={snapshot.platform} /> : "Interface preview"}</p></div></div>
            <div className="local-status"><StatusBadge active={!!snapshot?.listening}>{desktop.native ? snapshot?.listening ? "Available on network" : "Network unavailable" : "Browser preview: native features require the desktop app"}</StatusBadge></div>
            {snapshot?.localAddresses.length ? <dl className="details-list local-details"><div><dt>Local address</dt><dd className="local-address-list">{snapshot.localAddresses.map(address => <code key={address}>{address}</code>)}</dd></div><div><dt>Port</dt><dd className="mono">{snapshot.settings.port}</dd></div></dl> : null}
          </CardContent>
          <CardFooter><SecurityNote>Sharing depends on each computer's permissions.</SecurityNote></CardFooter>
        </Card>
      </div>
      <Card>
        <CardHeader><CardTitle>Recent transfers</CardTitle><CardDescription>Files sent and received on this computer.</CardDescription></CardHeader>
        <CardContent>{transfers.length ? <ItemGroup>{transfers.slice(0, 8).map((transfer, index) => <Fragment key={transfer.id}>{index ? <ItemSeparator /> : null}<Item size="sm" className="px-0"><ItemMedia variant="icon">{transfer.direction === "incoming" ? <ArrowDownLeft /> : <ArrowUpRight />}</ItemMedia><ItemContent><ItemTitle className="break-all">{transfer.fileName}</ItemTitle><ItemDescription>{transfer.direction === "incoming" ? "From" : "To"} {transfer.peerName} · {formatBytes(transfer.bytes)} · <time dateTime={transfer.createdAt} title={formattedDate(transfer.createdAt)}>{relativeDate(transfer.createdAt)}</time></ItemDescription></ItemContent><ItemActions><Badge variant={transfer.status === "failed" ? "destructive" : "outline"}>{transferStatusLabels[transfer.status] ?? transfer.status}</Badge></ItemActions></Item></Fragment>)}</ItemGroup> : <EmptyState icon={FileUp} title="No transfers" description="Files you send and receive will appear here." />}</CardContent>
      </Card>
    </div>
    <PairingDialog desktop={desktop} open={pairing} onOpenChange={setPairing} />
    {selectedPeer ? <PeerDialog key={selectedPeer.id} desktop={desktop} peer={snapshot?.peers.find(peer => peer.id === selectedPeer.id) ?? selectedPeer} onClose={() => setSelectedPeer(null)} /> : null}
  </>;
}

function PairingDialog({ desktop, open, onOpenChange }: { desktop: Desktop; open: boolean; onOpenChange: (value: boolean) => void }) {
  const [invite, setInvite] = useState<PairingInvite | null>(null);
  const [code, setCode] = useState("");
  const [copied, setCopied] = useState(false);
  const openRef = useRef(open);
  const inviteGeneration = useRef(0);
  openRef.current = open;
  const busy = !!desktop.busy;
  const [now, setNow] = useState(Date.now);
  const remaining = invite ? Math.max(0, Math.ceil((Date.parse(invite.expiresAt) - now) / 1000)) : 0;
  useEffect(() => {
    if (!invite || !open) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [invite, open]);

  function close(next: boolean) {
    openRef.current = next;
    if (!next) {
      inviteGeneration.current += 1;
      if (invite) void nativeCommand("cancel_pairing_invite").catch(error => toast.error(messageFrom(error)));
      setInvite(null); setCode(""); setCopied(false);
    }
    onOpenChange(next);
  }

  async function createInvite() {
    const generation = inviteGeneration.current;
    setCopied(false);
    const result = await desktop.run("invite", () => nativeCommand<PairingInvite>("create_pairing_invite"));
    if (result && openRef.current && generation === inviteGeneration.current) { setInvite(result); setNow(Date.now()); }
    else if (result) await nativeCommand("cancel_pairing_invite").catch(error => toast.error(messageFrom(error)));
  }
  async function pair(event: React.FormEvent) {
    event.preventDefault();
    const result = await desktop.run("pair", () => nativeCommand<ConnectedPeer>("pair_with_invite", { invitation: code.trim() }), "Computer paired and connected.");
    if (result) close(false);
  }
  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); setCopied(true); window.setTimeout(() => setCopied(false), 2000); }
    catch (err) { toast.error(messageFrom(err)); }
  }

  return <Dialog open={open} onOpenChange={close}><DialogContent className="pairing-dialog"><DialogHeader><div className="dialog-symbol"><Link2 /></div><DialogTitle>Connect a computer</DialogTitle><DialogDescription>Both need to be on the same network with TrapLink open.</DialogDescription></DialogHeader><Tabs defaultValue="invite"><TabsList className="full-tabs"><TabsTrigger value="invite">Create invite</TabsTrigger><TabsTrigger value="connect">Use an invite</TabsTrigger></TabsList><TabsContent value="invite">{invite ? <div className="invite-content"><div className="invite-heading"><StatusBadge active={remaining > 0}>{remaining > 0 ? "Invite available" : "Invite expired"}</StatusBadge><span>{remaining > 0 ? `Valid for ${Math.floor(remaining / 60)}:${String(remaining % 60).padStart(2, "0")}` : "Create a new invite"}</span></div><FieldGroup><Field><FieldLabel htmlFor="invite-code">Connection invite</FieldLabel><Textarea id="invite-code" readOnly value={invite.invitation} className="mono break-all resize-none" rows={4} spellCheck={false} aria-describedby="invite-help" onFocus={event => event.currentTarget.select()} /><FieldDescription id="invite-help">Includes the address and key. Paste it into “Use an invite” only on the computer you want to authorize.</FieldDescription></Field></FieldGroup><Button className="w-full" variant="outline" disabled={remaining === 0} onClick={() => void copy(invite.invitation)}>{copied ? <Check data-icon="inline-start" /> : <Copy data-icon="inline-start" />}{copied ? "Copied" : "Copy invite"}</Button><Button variant="link" disabled={busy} onClick={() => void createInvite()}>Create another invite</Button></div> : <div className="invite-intro"><h3>Create a connection invite</h3><p>Create an invite and paste it into “Use an invite” on the other computer.</p><Button disabled={busy} onClick={() => void createInvite()}><BusyIcon busy={desktop.busy === "invite"} />Create invite<ArrowRight data-icon="inline-end" /></Button></div>}</TabsContent><TabsContent value="connect"><form className="pair-form" onSubmit={event => void pair(event)}><FieldGroup><Field data-disabled={busy}><div className="flex items-center justify-between gap-2"><FieldLabel htmlFor="pair-code">Connection invite</FieldLabel><Button type="button" variant="ghost" size="sm" disabled={busy} onClick={() => void navigator.clipboard.readText().then(text => setCode(text.trim().slice(0, 2048))).catch(() => toast.error("Couldn't read the clipboard. Paste with Ctrl + V."))}><ClipboardPaste data-icon="inline-start" />Paste</Button></div><Textarea id="pair-code" onKeyDown={event => { if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); } }} value={code} onChange={event => setCode(event.target.value)} placeholder="Paste the invite that starts with traplink:v1:" required autoComplete="off" autoCapitalize="none" spellCheck={false} disabled={busy} maxLength={2048} rows={4} className="mono break-all resize-none" aria-describedby="pair-help" /><FieldDescription id="pair-help">One invite, no IP or port needed. It is valid for five minutes and works only once. Press Ctrl + Enter to connect.</FieldDescription></Field></FieldGroup><Button type="submit" disabled={busy || !code.trim()}><BusyIcon busy={desktop.busy === "pair"} />Connect computer<ArrowRight data-icon="inline-end" /></Button></form></TabsContent></Tabs><SecurityNote>The connection stays on your local network. Sharing starts off for every new computer.</SecurityNote></DialogContent></Dialog>;
}

const permissionCopy: Record<PeerFeature, { label: string; on: string; off: string }> = {
  clipboard: { label: "Clipboard", on: "Clipboard allowed with", off: "Clipboard turned off for" },
  files: { label: "Files", on: "Files allowed with", off: "Files turned off for" },
  input: { label: "Mouse and keyboard", on: "Mouse and keyboard allowed for", off: "Mouse and keyboard turned off for" },
};
const globalSwitch = { clipboard: "clipboardEnabled", files: "fileSharingEnabled", input: "inputEnabled" } as const;

function PeerDialog({ desktop, peer, onClose }: { desktop: Desktop; peer: ConnectedPeer; onClose: () => void }) {
  const [forgetting, setForgetting] = useState(false);
  const [saving, setSaving] = useState<PeerFeature | null>(null);
  const settings = desktop.snapshot?.settings;
  const refresh = () => desktop.refresh(true);
  const supported: Record<PeerFeature, boolean> = { clipboard: desktop.capabilities.clipboardSharing, files: desktop.capabilities.fileSharing, input: desktop.capabilities.inputSharing };
  const allowed = (key: PeerFeature) => !!peer.permissions?.[key] && !!settings?.[globalSwitch[key]];
  async function setPermission(key: PeerFeature, value: boolean) {
    setSaving(key);
    try {
      if (value) await allowLocally(peer.id, [key]); else await revokeForPeer(peer, key);
      await refresh();
      toast.success(`${value ? permissionCopy[key].on : permissionCopy[key].off} ${peer.name}.`);
    } catch (error) { toast.error(messageFrom(error)); }
    finally { setSaving(null); }
  }
  async function forgetPeer() {
    const done = await desktop.run(`forget-${peer.id}`, async () => { await nativeCommand("forget_peer", { id: peer.id }); return true; }, "Computer removed from authorized connections.");
    if (done) onClose();
  }
  const descriptions: Record<PeerFeature, string> = { clipboard: "Send and receive copied text.", files: "Send and receive files.", input: `${peer.name} can control this computer.` };
  return <><Dialog open onOpenChange={value => { if (!value) onClose(); }}><DialogContent className="peer-dialog">
    <DialogHeader><div className="dialog-symbol"><Monitor /></div><DialogTitle>{peer.name}</DialogTitle><DialogDescription><PlatformName platform={peer.platform} />, {peer.address}</DialogDescription></DialogHeader>
    <StatusBadge active={peer.connected}>{peer.connected ? "Connected" : "Disconnected"}</StatusBadge>
    <FlowControl peer={peer} available={desktop.capabilities.inputSharing && desktop.capabilities.flowEdgeSwitching !== false} startOnLogin={!!settings?.startOnLogin} onPermissionsChanged={refresh} />
    <Separator />
    <section className="peer-section">
      <h3>Permissions</h3>
      <FieldGroup className="permission-fields">
        {(["clipboard", "files", "input"] as const).map(key => <Field key={key} orientation="horizontal" data-disabled={!desktop.native || !supported[key]}><FieldContent><FieldLabel htmlFor={`peer-${key}`}>{permissionCopy[key].label}</FieldLabel><FieldDescription>{descriptions[key]}</FieldDescription></FieldContent>{saving === key ? <Spinner /> : null}<Switch id={`peer-${key}`} checked={allowed(key)} disabled={!desktop.native || !supported[key] || saving !== null} onCheckedChange={value => void setPermission(key, value)} /></Field>)}
      </FieldGroup>
    </section>
    <Separator />
    <Disclosure label="Manual control" icon={Keyboard}>
      <InputControl peer={peer} enabled={desktop.native && desktop.capabilities.inputSharing} onPermissionsChanged={refresh} />
    </Disclosure>
    <div className="fingerprint"><span>Computer identity</span><code>{peer.fingerprint}</code></div>
    <DialogFooter><Button variant="ghost" disabled={!!desktop.busy} onClick={() => setForgetting(true)}>Forget computer</Button><Button variant="outline" onClick={onClose}>Close</Button>{peer.connected ? <Button variant="destructive" disabled={!!desktop.busy} onClick={() => void desktop.run(`disconnect-${peer.id}`, () => nativeCommand("disconnect_peer", { id: peer.id }), "Computer disconnected.")}><Unplug data-icon="inline-start" />Disconnect</Button> : null}</DialogFooter>
  </DialogContent></Dialog><ConfirmDialog open={forgetting} onOpenChange={setForgetting} title="Forget this computer?" description={`“${peer.name}” will be disconnected and lose its authorization on this installation. You'll need to pair again to connect.`} action="Forget computer" destructive busy={!!desktop.busy} onConfirm={() => void forgetPeer()} /></>;
}
