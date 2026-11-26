import { Fragment, useEffect, useRef, useState } from "react";
import { ArrowDownLeft, ArrowRight, ArrowUpRight, Check, Clipboard, ClipboardPaste, Copy, FileUp, Keyboard, Link2, Monitor, Plus, RefreshCw, SlidersHorizontal, Unplug } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
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
import { inviteBaseline, inviteState, type InviteBaseline } from "@/lib/pairing";
import { allowLocally, revokeForPeer, type PeerFeature } from "@/lib/permissions";
import { InputControl } from "@/components/InputControl";
import { FlowControl } from "@/components/FlowControl";

const transferStatusLabels: Record<string, string> = { complete: "Complete", completed: "Complete", failed: "Failed", transferring: "Transferring", pending: "Pending", sending: "Sending", receiving: "Receiving" };

export function Connections({ desktop, intent, onIntentDone }: { desktop: Desktop; intent?: Intent | null; onIntentDone?: () => void }) {
  const [pairing, setPairing] = useState(false);
  useEffect(() => { if (intent?.kind === "pair") { setPairing(true); onIntentDone?.(); } }, [intent, onIntentDone]);
  const [selectedPeerId, setSelectedPeerId] = useState<string | null>(null);
  const snapshot = desktop.snapshot;
  const peers = snapshot?.peers ?? [];
  const connected = peers.filter(peer => peer.connected);
  const selectedPeer = peers.find(peer => peer.id === selectedPeerId);
  const transfers = snapshot?.transfers ?? [];

  async function sendFile(peer: ConnectedPeer) {
    await desktop.run(`file-${peer.id}`, async () => {
      const path = await chooseFile();
      if (path) { await nativeCommand<Transfer>("send_file", { peerId: peer.id, path }); toast.success("File sent."); }
    });
  }

  return <>
    <PageHeading title="Connections" description="Pair your computers, choose how to switch, and control what they can share.">
      <Button disabled={!desktop.native || !desktop.capabilities.lanPairing || !!desktop.busy} onClick={() => setPairing(true)}><Plus data-icon="inline-start" />Connect computer</Button>
    </PageHeading>
    <div className="content-stack connections-page">
      <div className="connections-grid">
        <Card>
          <CardHeader><div className="card-heading-row"><div><CardTitle>Computers</CardTitle><CardDescription>{peers.length ? `${connected.length} of ${peers.length} paired ${peers.length === 1 ? "computer" : "computers"} connected.` : "Computers allowed to connect to this device."}</CardDescription></div><IconButton label="Refresh computers" disabled={!desktop.native || !!desktop.busy} onClick={() => void desktop.refresh()}><RefreshCw /></IconButton></div></CardHeader>
          <CardContent>
            {peers.length === 0 ? <EmptyState icon={Link2} title="Connect your first computer" description={desktop.native ? "Open TrapLink on both computers on the same network. Create an invite here, then paste it on the other computer." : "Open the TrapLink desktop app on both computers to pair them and share input."} /> : <ItemGroup>{peers.map((peer, index) => <Fragment key={peer.id}>{index ? <ItemSeparator /> : null}<Item size="sm" role="listitem" className="connection-peer-row px-0">
              <ItemMedia variant="icon"><Monitor /></ItemMedia>
              <ItemContent>
                <ItemTitle className="flex-wrap"><span>{peer.name}</span><StatusBadge active={peer.connected}>{peer.connected ? "Connected" : "Disconnected"}</StatusBadge></ItemTitle>
                <ItemDescription><PlatformName platform={peer.platform} /> · {peer.address}{!peer.trusted ? " · Waiting for authorization" : ""}</ItemDescription>
                {desktop.inputStatus.connectingTo === peer.id ? <Badge variant="secondary" className="peer-control-state"><Spinner />Switching to this computer</Badge>
                  : desktop.inputStatus.sendingTo === peer.id ? <Badge className="peer-control-state"><ArrowUpRight />Sending mouse and keyboard</Badge>
                  : desktop.inputStatus.receivingFrom === peer.id ? <Badge className="peer-control-state"><ArrowDownLeft />Receiving mouse and keyboard</Badge> : null}
              </ItemContent>
              <ItemActions className="connection-peer-actions">
                {peer.connected ? <>
                  <IconButton label={`Send clipboard to ${peer.name}`} hint={peer.permissions?.clipboard && snapshot?.settings.clipboardEnabled ? "Send clipboard" : "Open Manage to allow clipboard sharing"} disabled={!desktop.native || !snapshot?.settings.clipboardEnabled || !peer.permissions?.clipboard || !desktop.capabilities.clipboardSharing || !!desktop.busy} onClick={() => void desktop.run(`clipboard-${peer.id}`, () => nativeCommand("send_clipboard", { peerId: peer.id }), "Clipboard sent.")}><Clipboard /></IconButton>
                  <IconButton label={`Send file to ${peer.name}`} hint={peer.permissions?.files && snapshot?.settings.fileSharingEnabled ? "Send file" : "Open Manage to allow file sharing"} disabled={!desktop.native || !snapshot?.settings.fileSharingEnabled || !peer.permissions?.files || !desktop.capabilities.fileSharing || !!desktop.busy} onClick={() => void sendFile(peer)}><FileUp /></IconButton>
                </> : <Button size="sm" variant="outline" disabled={!desktop.native || !!desktop.busy} onClick={() => void desktop.run(`connect-${peer.id}`, () => nativeCommand("connect_peer", { id: peer.id }), "Computer connected.")}><BusyIcon busy={desktop.busy === `connect-${peer.id}`} />Reconnect</Button>}
                <Button size="sm" variant="outline" aria-label={`Manage ${peer.name}`} onClick={() => setSelectedPeerId(peer.id)}><SlidersHorizontal data-icon="inline-start" />Manage</Button>
              </ItemActions>
            </Item></Fragment>)}</ItemGroup>}
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>This computer</CardTitle><CardDescription>How it appears on the local network.</CardDescription></CardHeader>
          <CardContent>
            <div className="device-summary"><div className="item-icon"><Monitor /></div><div><strong>{snapshot?.deviceName ?? "Desktop app required"}</strong><p>{snapshot ? <PlatformName platform={snapshot.platform} /> : "Interface preview"}</p></div></div>
            <div className="local-status"><StatusBadge active={!!snapshot?.listening}>{desktop.native ? snapshot?.listening ? "Available on network" : "Network unavailable" : "Browser preview"}</StatusBadge></div>
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
    {selectedPeer ? <PeerDialog key={selectedPeer.id} desktop={desktop} peer={selectedPeer} onClose={() => setSelectedPeerId(null)} /> : null}
  </>;
}

function PairingDialog({ desktop, open, onOpenChange }: { desktop: Desktop; open: boolean; onOpenChange: (value: boolean) => void }) {
  const [invite, setInvite] = useState<PairingInvite | null>(null);
  const [code, setCode] = useState("");
  const [pairError, setPairError] = useState<string | null>(null);
  const [inviteError, setInviteError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const opener = useRef<HTMLElement | null>(null);
  const copyTimer = useRef<number | undefined>(undefined);
  const activeInvite = useRef<PairingInvite | null>(null);
  const openRef = useRef(open);
  const inviteGeneration = useRef(0);
  openRef.current = open;
  const busy = !desktop.native || !desktop.capabilities.lanPairing || !!desktop.busy;
  const [now, setNow] = useState(Date.now);
  const remaining = invite ? Math.max(0, Math.ceil((Date.parse(invite.expiresAt) - now) / 1000)) : 0;
  const baseline = useRef<InviteBaseline>(inviteBaseline([]));
  // Judge expiry with the current time, not the once-per-second tick, so a snapshot taken just after expiry is not read as a live invite.
  const state = invite ? inviteState(invite, desktop.snapshot?.pairingCode, desktop.snapshot?.peers ?? [], baseline.current, Math.max(now, Date.now())) : null;
  const inviteAvailable = state?.kind === "available";
  const pairedPeer = state?.kind === "paired" ? state.peer : null;
  const { refresh } = desktop;
  useEffect(() => {
    if (!invite || !open) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [invite, open]);
  useEffect(() => {
    // Check sooner than the app-wide poll so the inviting computer sees the pairing promptly.
    if (!inviteAvailable || !open) return;
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(); }, 2000);
    return () => window.clearInterval(timer);
  }, [inviteAvailable, open, refresh]);
  useEffect(() => {
    if (!pairedPeer || !openRef.current) return;
    // The core already consumed the invite during the handshake, so there is nothing to cancel.
    activeInvite.current = null;
    toast.success(`${pairedPeer.name} paired.`);
    close(false, false);
  }, [pairedPeer?.id]); // Runs once per paired computer; close only uses refs and stable setters here.
  useEffect(() => () => {
    openRef.current = false;
    window.clearTimeout(copyTimer.current);
    if (activeInvite.current) void nativeCommand("cancel_pairing_invite").catch(error => toast.error(messageFrom(error)));
  }, []);

  function close(next: boolean, cancelInvite = true) {
    openRef.current = next;
    if (!next) {
      inviteGeneration.current += 1;
      if (invite && cancelInvite) void nativeCommand("cancel_pairing_invite").catch(error => toast.error(messageFrom(error)));
      activeInvite.current = null;
      window.clearTimeout(copyTimer.current);
      setInvite(null); setCode(""); setCopied(false); setPairError(null); setInviteError(null);
    }
    onOpenChange(next);
  }

  async function createInvite() {
    const generation = inviteGeneration.current;
    setCopied(false);
    setInviteError(null);
    const known = inviteBaseline(desktop.snapshot?.peers);
    const result = await desktop.run("invite", async () => {
      try { return await nativeCommand<PairingInvite>("create_pairing_invite"); }
      catch (error) { if (openRef.current && generation === inviteGeneration.current) setInviteError(messageFrom(error)); throw error; }
    });
    if (result && openRef.current && generation === inviteGeneration.current) { baseline.current = known; activeInvite.current = result; setInvite(result); setNow(Date.now()); }
    else if (result) await nativeCommand("cancel_pairing_invite").catch(error => toast.error(messageFrom(error)));
  }
  async function pair(event: React.FormEvent) {
    event.preventDefault();
    const generation = inviteGeneration.current;
    setPairError(null);
    const result = await desktop.run("pair", async () => {
      try { return await nativeCommand<ConnectedPeer>("pair_with_invite", { invitation: code.trim() }); }
      catch (error) { if (openRef.current && generation === inviteGeneration.current) setPairError(messageFrom(error)); throw error; }
    }, "Computer paired and connected.");
    if (result && openRef.current && generation === inviteGeneration.current) close(false);
  }
  async function copy(text: string) {
    const generation = inviteGeneration.current;
    try {
      await navigator.clipboard.writeText(text);
      if (!openRef.current || generation !== inviteGeneration.current) return;
      window.clearTimeout(copyTimer.current);
      setCopied(true);
      copyTimer.current = window.setTimeout(() => setCopied(false), 2000);
    }
    catch (err) { toast.error(messageFrom(err)); }
  }
  const commandKey = /mac/i.test(navigator.platform || navigator.userAgent) ? "⌘" : "Ctrl";
  async function paste() {
    const generation = inviteGeneration.current;
    try {
      const text = await navigator.clipboard.readText();
      if (!openRef.current || generation !== inviteGeneration.current) return;
      setCode(text.trim().slice(0, 2048));
      setPairError(null);
    } catch { toast.error(`Couldn't read the clipboard. Paste with ${commandKey} + V.`); }
  }

  return <Dialog open={open} onOpenChange={close}>
    <DialogContent className="pairing-dialog"
      onOpenAutoFocus={() => { opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; }}
      onCloseAutoFocus={event => { event.preventDefault(); if (opener.current?.isConnected) opener.current.focus(); else document.getElementById("main-content")?.focus(); }}>
      <DialogHeader>
        <div className="dialog-symbol"><Link2 aria-hidden="true" /></div>
        <DialogTitle>Connect a computer</DialogTitle>
        <DialogDescription>Open TrapLink on both computers on the same network.</DialogDescription>
      </DialogHeader>
      <Tabs defaultValue="invite">
        <TabsList className="full-tabs" aria-label="Pairing method">
          <TabsTrigger value="invite">Create invite</TabsTrigger>
          <TabsTrigger value="connect">Use an invite</TabsTrigger>
        </TabsList>
        <TabsContent value="invite">
          {inviteError ? <Alert variant="destructive" className="mt-4"><AlertTitle>Couldn't create the invite</AlertTitle><AlertDescription>{inviteError}</AlertDescription></Alert> : null}
          {invite ? <div className="invite-content">
            <div className="invite-heading">
              <StatusBadge active={inviteAvailable}>{inviteAvailable ? "Invite available" : remaining === 0 ? "Invite expired" : "Invite closed"}</StatusBadge>
              {inviteAvailable ? <span className="tabular-nums">Valid for {Math.floor(remaining / 60)}:{String(remaining % 60).padStart(2, "0")}</span> : null}
            </div>
            {inviteAvailable ? <><FieldGroup><Field>
              <FieldLabel htmlFor="invite-code">Connection invite</FieldLabel>
              <Textarea id="invite-code" readOnly value={invite.invitation} className="mono break-all resize-none" rows={4} spellCheck={false} aria-describedby="invite-help" onFocus={event => event.currentTarget.select()} />
              <FieldDescription id="invite-help">Paste this invite into “Use an invite” on the computer you want to authorize.</FieldDescription>
            </Field></FieldGroup>
            <Button className="w-full" variant="outline" disabled={busy} onClick={() => void copy(invite.invitation)}>
              {copied ? <Check data-icon="inline-start" /> : <Copy data-icon="inline-start" />}{copied ? "Copied" : "Copy invite"}
            </Button></> : <p className="input-control-note">Create another invite to pair a computer.</p>}
            <Button variant="link" disabled={busy} onClick={() => void createInvite()}><BusyIcon busy={desktop.busy === "invite"} />Create another invite</Button>
          </div> : <div className="invite-intro">
            <p>Create a single-use invite to paste into TrapLink on the other computer. The invite expires in five minutes.</p>
            <Button disabled={busy} onClick={() => void createInvite()}><BusyIcon busy={desktop.busy === "invite"} />Create invite<ArrowRight data-icon="inline-end" /></Button>
          </div>}
        </TabsContent>
        <TabsContent value="connect">
          <form className="pair-form" aria-busy={desktop.busy === "pair"} onSubmit={event => void pair(event)}>
            <FieldGroup><Field data-disabled={busy} data-invalid={!!pairError}>
              <div className="flex items-center justify-between gap-2">
                <FieldLabel htmlFor="pair-code">Connection invite</FieldLabel>
                <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={() => void paste()}><ClipboardPaste data-icon="inline-start" />Paste</Button>
              </div>
              <Textarea id="pair-code" value={code} onChange={event => { setCode(event.target.value); setPairError(null); }}
                onKeyDown={event => { if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); if (!busy && code.trim()) event.currentTarget.form?.requestSubmit(); } }}
                placeholder="Paste the invite that starts with traplink:v1:" required autoComplete="off" autoCapitalize="none" spellCheck={false} disabled={busy} maxLength={2048} rows={4} className="mono break-all resize-none"
                aria-describedby={pairError ? "pair-help pair-error" : "pair-help"} aria-invalid={!!pairError} />
              <FieldDescription id="pair-help">Use the invite from the other computer. Press {commandKey} + Enter to connect.</FieldDescription>
              {pairError ? <FieldError id="pair-error">{pairError}</FieldError> : null}
            </Field></FieldGroup>
            <Button type="submit" disabled={busy || !code.trim()}><BusyIcon busy={desktop.busy === "pair"} />Connect computer<ArrowRight data-icon="inline-end" /></Button>
          </form>
        </TabsContent>
      </Tabs>
      <SecurityNote>Pairing authorizes the local connection. Sharing starts off for every new computer.</SecurityNote>
    </DialogContent>
  </Dialog>;
}

const permissionCopy: Record<PeerFeature, { label: string; on: string; off: string }> = {
  clipboard: { label: "Clipboard", on: "Clipboard allowed with", off: "Clipboard turned off for" },
  files: { label: "Files", on: "Files allowed with", off: "Files turned off for" },
  input: { label: "Mouse and keyboard", on: "Mouse and keyboard allowed for", off: "Mouse and keyboard turned off for" },
};
const globalSwitch = { clipboard: "clipboardEnabled", files: "fileSharingEnabled", input: "inputEnabled" } as const;

function PeerDialog({ desktop, peer, onClose }: { desktop: Desktop; peer: ConnectedPeer; onClose: () => void }) {
  const opener = useRef<HTMLElement | null>(null);
  const flowTab = useRef<HTMLButtonElement>(null);
  const [forgetting, setForgetting] = useState(false);
  const [saving, setSaving] = useState<PeerFeature | null>(null);
  const settings = desktop.snapshot?.settings;
  const { receivingFrom, sendingTo, connectingTo, flowSession } = desktop.inputStatus;
  const inputDirection = receivingFrom === peer.id ? "Receiving input" : sendingTo === peer.id ? "Sending input" : null;
  const returnSession = inputDirection && !connectingTo ? flowSession : null;
  const refresh = () => desktop.refresh(true);
  const supported: Record<PeerFeature, boolean> = { clipboard: desktop.capabilities.clipboardSharing, files: desktop.capabilities.fileSharing, input: desktop.capabilities.inputSharing };
  const flowAvailable = supported.input && desktop.capabilities.flowEdgeSwitching !== false;
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
  const descriptions: Record<PeerFeature, string> = {
    clipboard: `Allow this pair to send and receive copied text, images, and files with Send clipboard.${flowAvailable ? " For automatic transfers, turn on Share clipboard on switch in Flow." : ""}`,
    files: "Send and receive files.",
    input: `${peer.name} can control this computer.`,
  };
  return <><Dialog open onOpenChange={value => { if (!value) onClose(); }}><DialogContent className="peer-dialog"
    onOpenAutoFocus={() => { opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; }}
    onCloseAutoFocus={event => { event.preventDefault(); if (opener.current?.isConnected) opener.current.focus(); else document.getElementById("main-content")?.focus(); }}>
    <DialogHeader className="peer-dialog-header">
      <div className="dialog-symbol"><Monitor aria-hidden="true" /></div>
      <div className="peer-dialog-identity"><DialogTitle>{peer.name}</DialogTitle><DialogDescription><PlatformName platform={peer.platform} /> · {peer.address}</DialogDescription></div>
      <StatusBadge active={peer.connected}>{inputDirection ?? (peer.connected ? "Connected" : "Disconnected")}</StatusBadge>
    </DialogHeader>
    <Tabs defaultValue="flow" activationMode="automatic" className="peer-tabs">
      <TabsList className="full-tabs" aria-label="Computer settings"><TabsTrigger value="flow" ref={flowTab}>Flow</TabsTrigger><TabsTrigger value="permissions">Permissions</TabsTrigger><TabsTrigger value="details">Details</TabsTrigger></TabsList>
      <TabsContent value="flow" className="peer-tab-content">
        <FlowControl peer={peer} peers={desktop.snapshot?.peers ?? []} available={flowAvailable} clipboardSupported={supported.clipboard} clipboardAllowed={allowed("clipboard")} startOnLogin={!!settings?.startOnLogin} onPermissionsChanged={refresh} />
        <Separator />
        <Disclosure label="Manual control" icon={Keyboard}>
          <InputControl peer={peer} enabled={desktop.native && desktop.capabilities.inputSharing} onPermissionsChanged={refresh} />
        </Disclosure>
      </TabsContent>
      <TabsContent value="permissions" className="peer-tab-content">
        <section className="peer-section">
          <h3>What this computer allows</h3>
          <p className="input-control-note">Choose what to share with {peer.name}. The other computer must allow sharing too.</p>
          <FieldGroup className="permission-fields">
            {(["clipboard", "files", "input"] as const).map(key => <Field key={key} orientation="horizontal" data-disabled={!desktop.native || !supported[key]}>
              <FieldContent>
                <FieldLabel htmlFor={`peer-${key}`}>{permissionCopy[key].label}</FieldLabel>
                <FieldDescription id={`peer-${key}-help`}>{supported[key] ? descriptions[key] : "Not supported on this computer."}</FieldDescription>
                {key === "clipboard" && supported.clipboard && flowAvailable ? <Button type="button" variant="link" size="sm" className="self-start px-0" onClick={() => flowTab.current?.focus()}>Open Flow settings<ArrowRight data-icon="inline-end" /></Button> : null}
              </FieldContent>
              {saving === key ? <Spinner aria-label={`Saving ${permissionCopy[key].label.toLowerCase()} permission`} /> : null}
              <Switch id={`peer-${key}`} aria-describedby={`peer-${key}-help`} checked={allowed(key)} disabled={!desktop.native || !supported[key] || saving !== null} onCheckedChange={value => void setPermission(key, value)} />
            </Field>)}
          </FieldGroup>
        </section>
      </TabsContent>
      <TabsContent value="details" className="peer-tab-content">
        <dl className="details-list"><div><dt>Platform</dt><dd><PlatformName platform={peer.platform} /></dd></div><div><dt>Network address</dt><dd className="mono">{peer.address}</dd></div><div><dt>Authorization</dt><dd>{peer.trusted ? "Paired with this computer" : "Waiting for authorization"}</dd></div></dl>
        <div className="fingerprint"><span>Computer identity</span><code>{peer.fingerprint}</code></div>
        <SecurityNote>Forgetting this computer removes its authorization. Pair again before reconnecting.</SecurityNote>
        <Button variant="outline" className="self-start" disabled={!!desktop.busy} onClick={() => setForgetting(true)}>Forget computer</Button>
      </TabsContent>
    </Tabs>
    <DialogFooter className="peer-dialog-footer">
      {peer.connected ? <Button variant="outline" disabled={!!desktop.busy} onClick={() => void desktop.run(`disconnect-${peer.id}`, () => nativeCommand("disconnect_peer", { id: peer.id }), "Computer disconnected.")}><Unplug data-icon="inline-start" />Disconnect</Button>
        : <Button variant="outline" disabled={!desktop.native || !!desktop.busy} onClick={() => void desktop.run(`connect-${peer.id}`, () => nativeCommand("connect_peer", { id: peer.id }), "Computer connected.")}><BusyIcon busy={desktop.busy === `connect-${peer.id}`} />Reconnect</Button>}
      {returnSession ? <Button disabled={desktop.busy === "release-flow"} onClick={() => void desktop.returnControl(peer.id, returnSession)}><BusyIcon busy={desktop.busy === "release-flow"} />Return control</Button> : null}
      <Button variant={returnSession ? "outline" : "default"} onClick={onClose}>Done</Button>
    </DialogFooter>
  </DialogContent></Dialog><ConfirmDialog open={forgetting} onOpenChange={setForgetting} title="Forget this computer?" description={`“${peer.name}” will be disconnected and lose its authorization on this installation. You'll need to pair again to connect.`} action="Forget computer" destructive busy={!!desktop.busy} onConfirm={() => void forgetPeer()} /></>;
}
