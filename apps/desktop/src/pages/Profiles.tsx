import { useCallback, useEffect, useState } from "react";
import { AppWindow, ArrowRight, FileSliders, Keyboard, MousePointerClick, Pencil, Plus, RefreshCw, Search, ShieldAlert, Trash2, X } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { BusyIcon, ConfirmDialog, EmptyState, formattedDate, IconButton, PageHeading, relativeDate } from "@/components/common";
import type { Intent } from "@/App";
import { messageFrom, nativeCommand, utf8Length, type Profile, type RemapStatus, type RunningApp } from "@/lib/bridge";
import { formatTarget, isModifierKey, isSupportedKey, keyLabel, modifiers, mouseSources, parseTarget, sourceLabel, targetLabel, type Modifier, type Target } from "@/lib/keys";
import type { Desktop } from "@/lib/use-desktop";

function useRemapStatus(native: boolean) {
  const [status, setStatus] = useState<RemapStatus | null>(null);
  const load = useCallback(async () => {
    if (!native) return;
    try { setStatus(await nativeCommand<RemapStatus>("get_remap_status")); } catch {  }
  }, [native]);
  useEffect(() => {
    if (!native) return;
    void load();
    const timer = window.setInterval(() => void load(), 1500);
    return () => window.clearInterval(timer);
  }, [native, load]);
  return { status, setStatus, load };
}

export function Profiles({ desktop, intent, onIntentDone }: { desktop: Desktop; intent?: Intent | null; onIntentDone?: () => void }) {
  const [editor, setEditor] = useState<Profile | "new" | null>(null);
  useEffect(() => { if (intent?.kind === "new-profile") { setEditor("new"); onIntentDone?.(); } }, [intent, onIntentDone]);
  const [removing, setRemoving] = useState<Profile | null>(null);
  const [query, setQuery] = useState("");
  const remap = useRemapStatus(desktop.native);
  const profiles = desktop.snapshot?.profiles ?? [];
  const filtered = profiles.filter(profile => `${profile.name} ${profile.application ?? ""}`.toLocaleLowerCase("en-US").includes(query.toLocaleLowerCase("en-US")));
  async function removeProfile() {
    if (!removing) return;
    const result = await desktop.run(`delete-profile-${removing.id}`, async () => { await nativeCommand("delete_profile", { id: removing.id }); return true; }, "Profile deleted.");
    if (result) { setRemoving(null); void remap.load(); }
  }
  return <>
    <PageHeading title="Profiles" description="Remap keys and mouse buttons for all apps or for a specific app."><Button disabled={!desktop.native || !!desktop.busy} onClick={() => setEditor("new")}><Plus data-icon="inline-start" />Create profile</Button></PageHeading>
    <div className="content-stack">
      <RemapPanel desktop={desktop} status={remap.status} onStatus={remap.setStatus} />
      <div className="list-toolbar"><p>{profiles.length} saved profile{profiles.length === 1 ? "" : "s"}</p><Field className="search-field"><FieldLabel htmlFor="profile-search"><Search aria-hidden="true" />Search profiles</FieldLabel><Input id="profile-search" placeholder="Profile or app" value={query} onChange={event => setQuery(event.target.value)} /></Field></div>
      {filtered.length ? <div className="profiles-grid">{filtered.map(profile => {
        const state = remap.status?.profiles.find(item => item.id === profile.id);
        const active = !!remap.status?.running && remap.status.activeProfiles.some(item => item.id === profile.id);
        return <Card className="profile-card" key={profile.id}>
          <CardHeader className="profile-card-header"><div><CardTitle>{profile.name}</CardTitle><CardDescription className="mt-2">{profile.application || "All apps"}</CardDescription></div><IconButton label={`Delete profile ${profile.name}`} hint="Delete profile" onClick={() => setRemoving(profile)}><Trash2 /></IconButton></CardHeader>
          <CardContent className="profile-meta">
            <Badge variant="secondary">{profile.mappings.length} mapping{profile.mappings.length === 1 ? "" : "s"}</Badge>
            {active ? <Badge>Active now</Badge> : null}
            {state?.reason === "device_filter" ? <Badge variant="outline">Not applied: specific device</Badge> : null}
            {state?.ignored.length ? <Badge variant="outline">{state.ignored.length} unrecognized</Badge> : null}
          </CardContent>
          <CardFooter className="profile-card-footer"><span><time dateTime={profile.updatedAt} title={formattedDate(profile.updatedAt)}>Updated {relativeDate(profile.updatedAt)}</time></span><Button variant="outline" size="sm" onClick={() => setEditor(profile)} aria-label={`Edit profile ${profile.name}`}><Pencil data-icon="inline-start" />Edit</Button></CardFooter>
        </Card>;
      })}</div> : <Card><CardContent className="pt-6"><EmptyState icon={FileSliders} title={query ? "No profiles found" : "No saved profiles"} description={query ? "Search for another name or app." : "Use Create profile to choose an app and the keys you want to swap."} /></CardContent></Card>}
    </div>
    {editor ? <ProfileEditor key={editor === "new" ? "new" : editor.id} desktop={desktop} profile={editor === "new" ? undefined : editor} ignored={editor === "new" ? [] : remap.status?.profiles.find(item => item.id === editor.id)?.ignored ?? []} onSaved={() => void remap.load()} onClose={() => setEditor(null)} /> : null}
    <ConfirmDialog open={!!removing} onOpenChange={open => { if (!open) setRemoving(null); }} title="Delete this profile?" description={`“${removing?.name ?? ""}” will be removed from this computer. Existing backups will not be changed.`} action="Delete profile" destructive busy={!!desktop.busy} onConfirm={() => void removeProfile()} />
  </>;
}

function RemapPanel({ desktop, status, onStatus }: { desktop: Desktop; status: RemapStatus | null; onStatus: (status: RemapStatus) => void }) {
  const supported = desktop.native && (status?.supported ?? false);
  async function toggle(enabled: boolean) {
    const result = await desktop.run("remap-toggle", () => nativeCommand<RemapStatus>("set_remap_enabled", { enabled }), enabled ? "Remapping turned on." : "Remapping turned off.");
    if (result) onStatus(result);
  }
  return <Card>
    <CardHeader className="profile-card-header">
      <div><CardTitle className="flex items-center gap-2"><Keyboard className="size-4" aria-hidden="true" />Remapping</CardTitle><CardDescription className="mt-2">Applies saved profiles to keys and the middle, back and forward mouse buttons. Off by default.</CardDescription></div>
      <Switch id="remap-enabled" aria-label="Remapping" checked={!!status?.enabled} disabled={!supported || !!desktop.busy} onCheckedChange={value => void toggle(value)} />
    </CardHeader>
    <CardContent className="remap-panel">
      {!desktop.native ? <Alert><AppWindow /><AlertTitle>Available in the desktop app</AlertTitle><AlertDescription>Remapping requires TrapLink for Windows. Nothing is remapped in this browser preview.</AlertDescription></Alert>
        : status && !status.supported ? <Alert><AppWindow /><AlertTitle>Not supported on this platform</AlertTitle><AlertDescription>{status.reason ?? "Remapping is not supported on this platform yet."}</AlertDescription></Alert>
        : status?.enabled && !status.running ? <Alert variant="destructive"><ShieldAlert /><AlertTitle>Remapping stopped</AlertTitle><AlertDescription>Keyboard and mouse capture is not active. Turn it off and on again to retry.</AlertDescription></Alert>
        : status?.running ? <div className="remap-active">
          <p>{status.paused ? "TrapLink is in the foreground. Remapping is paused here so you can capture the original keys." : <>Foreground app: <strong>{status.activeApp ?? "unknown"}</strong></>}</p>
          {!status.paused ? <div className="profile-meta">{status.activeProfiles.length ? status.activeProfiles.map(item => <Badge key={item.id}>{item.name}</Badge>) : <span className="text-muted-foreground text-xs">No profile for this app.</span>}</div> : null}
          {!status.hookInstalled ? <p className="text-destructive text-xs">Windows removed the capture. TrapLink will try to reinstall it.</p> : null}
        </div> : null}
      <ul className="remap-notes">
        <li><Kbd>Ctrl</Kbd> + <Kbd>Alt</Kbd> + <Kbd>Esc</Kbd> turns remapping off immediately and releases any key it is holding.</li>
        <li>Games with anti-cheat may ignore or flag software-generated input. Use at your own risk and check the game rules.</li>
        <li>Special Logitech buttons, such as the gesture button and thumb wheel, are not supported yet. TrapLink does not change device settings.</li>
        <li>Windows running as administrator are not remapped unless TrapLink also runs as administrator.</li>
        <li>During Flow, keys are sent to the other computer without remapping.</li>
      </ul>
    </CardContent>
  </Card>;
}

type Row = { from: string; kind: Target["kind"]; key: string; modifiers: Modifier[]; raw: string };
function toRow(from: string, to: string): Row {
  const target = parseTarget(to);
  return { from, kind: target.kind, key: target.kind === "key" || target.kind === "shortcut" ? target.key : "", modifiers: target.kind === "shortcut" ? target.modifiers : [], raw: target.kind === "custom" ? target.raw : "" };
}
function rowTarget(row: Row): string {
  switch (row.kind) {
    case "disabled": return formatTarget({ kind: "disabled" });
    case "custom": return row.raw;
    case "key": return row.key;
    case "shortcut": return formatTarget({ kind: "shortcut", modifiers: row.modifiers, key: row.key });
  }
}
function rowInvalid(row: Row): boolean {
  const to = rowTarget(row);
  return !row.from.trim() || !to.trim() || utf8Length(row.from) > 80 || utf8Length(to) > 80 || (row.kind === "shortcut" && (!row.key || !row.modifiers.length));
}

function ProfileEditor({ desktop, profile, ignored, onSaved, onClose }: { desktop: Desktop; profile?: Profile; ignored: number[]; onSaved: () => void; onClose: () => void }) {
  const [name, setName] = useState(profile?.name ?? "");
  const [scope, setScope] = useState<"all" | "app">(profile?.application ? "app" : "all");
  const [application, setApplication] = useState(profile?.application ?? "");
  const [deviceId, setDeviceId] = useState(profile?.deviceId ?? "any");
  const [rows, setRows] = useState<Row[]>(() => (profile?.mappings ?? []).map(mapping => toRow(mapping.from, mapping.to)));
  const [touched, setTouched] = useState(false);
  const [picking, setPicking] = useState(false);
  const devices = desktop.snapshot?.devices ?? [];
  const appMissing = scope === "app" && !application.trim();
  const invalid = !name.trim() || appMissing || rows.some(rowInvalid);

  function updateRow(index: number, patch: Partial<Row>) { setRows(current => current.map((row, rowIndex) => rowIndex === index ? { ...row, ...patch } : row)); }
  async function save(event: React.FormEvent) {
    event.preventDefault(); setTouched(true); if (invalid || desktop.busy) return;
    const values = { name: name.trim(), application: scope === "app" ? application.trim() : undefined, deviceId: deviceId === "any" ? undefined : deviceId, mappings: rows.map(row => ({ from: row.from.trim(), to: rowTarget(row).trim() })) };
    const result = await desktop.run("save-profile", () => profile ? nativeCommand<Profile>("update_profile", { profile: { ...profile, ...values } }) : nativeCommand<Profile>("create_profile", values), "Profile saved.");
    if (result) { onSaved(); onClose(); }
  }
  const sources = rows.map(row => row.from.trim().toLowerCase());

  return <Dialog open onOpenChange={value => { if (!value) onClose(); }}><DialogContent className="profile-dialog"><DialogHeader><div className="dialog-symbol"><FileSliders /></div><DialogTitle>{profile ? "Edit profile" : "Create a profile"}</DialogTitle><DialogDescription>Choose where the profile applies and which keys or buttons it swaps.</DialogDescription></DialogHeader>
    <form onSubmit={event => void save(event)} className="profile-form">
      <FieldGroup>
        <Field data-invalid={touched && !name.trim()}><FieldLabel htmlFor="profile-name">Profile name</FieldLabel><Input id="profile-name" value={name} onChange={event => setName(event.target.value)} placeholder="e.g. Roblox" maxLength={80} required autoFocus aria-invalid={touched && !name.trim()} /></Field>
        <Field><FieldLabel htmlFor="profile-scope">Apply to</FieldLabel><Select value={scope} onValueChange={value => setScope(value as "all" | "app")}><SelectTrigger id="profile-scope"><SelectValue /></SelectTrigger><SelectContent><SelectGroup><SelectItem value="all">All apps</SelectItem><SelectItem value="app">A specific app</SelectItem></SelectGroup></SelectContent></Select><FieldDescription>App profiles add to the all-apps profiles.</FieldDescription></Field>
        {scope === "app" ? <Field data-invalid={touched && appMissing}>
          <FieldLabel htmlFor="profile-application">App executable</FieldLabel>
          <div className="app-picker-row"><Input id="profile-application" value={application} onChange={event => setApplication(event.target.value)} placeholder="e.g. RobloxPlayerBeta.exe" maxLength={255} aria-invalid={touched && appMissing} /><Button type="button" variant="outline" disabled={!desktop.native} onClick={() => setPicking(value => !value)}><AppWindow data-icon="inline-start" />Choose an open app</Button></div>
          <FieldDescription>Matches only the file name, case-insensitive. The folder is ignored, so updates that install to another folder are still recognized.</FieldDescription>
          {picking ? <RunningApps onPick={exe => { setApplication(exe); setPicking(false); }} /> : null}
        </Field> : null}
        <Field><FieldLabel htmlFor="profile-device">Device</FieldLabel><Select value={deviceId} onValueChange={setDeviceId}><SelectTrigger id="profile-device"><SelectValue /></SelectTrigger><SelectContent><SelectGroup><SelectItem value="any">Any device</SelectItem>{profile?.deviceId && !devices.some(device => device.id === profile.deviceId) ? <SelectItem value={profile.deviceId}>Device not connected</SelectItem> : null}{devices.map(device => <SelectItem key={device.id} value={device.id}>{device.name}</SelectItem>)}</SelectGroup></SelectContent></Select>{deviceId !== "any" ? <FieldDescription>Windows does not report which keyboard produced each key. Device-specific profiles are saved but not applied.</FieldDescription> : null}</Field>
      </FieldGroup>
      <div className="mapping-editor">
        <div className="mapping-heading"><div><h3>Mappings</h3><p>Click a source and press the key or mouse button (middle, back or forward). Up to 128 mappings.</p></div><Button type="button" variant="outline" size="sm" disabled={rows.length >= 128} onClick={() => setRows(current => [...current, { from: "", kind: "key", key: "", modifiers: [], raw: "" }])}><Plus data-icon="inline-start" />Add</Button></div>
        {rows.length ? <div className="mapping-list">{rows.map((row, index) => {
          const duplicate = !!row.from.trim() && sources.indexOf(row.from.trim().toLowerCase()) !== index;
          const unrecognized = ignored.includes(index) && profile?.mappings[index]?.from === row.from && profile?.mappings[index]?.to === rowTarget(row);
          return <div className="mapping-item" key={index}>
            <FieldGroup className="mapping-row">
              <Field data-invalid={touched && !row.from.trim()}><FieldLabel className="sr-only" htmlFor={`mapping-from-${index}`}>Source {index + 1}</FieldLabel><KeyCapture id={`mapping-from-${index}`} label={row.from ? sourceLabel(row.from) : ""} allowMouse invalid={touched && !row.from.trim()} onCapture={code => updateRow(index, { from: code })} /></Field>
              <ArrowRight aria-hidden="true" />
              <Field><FieldLabel className="sr-only" htmlFor={`mapping-kind-${index}`}>Action type {index + 1}</FieldLabel><Select value={row.kind} onValueChange={value => updateRow(index, { kind: value as Row["kind"] })}><SelectTrigger id={`mapping-kind-${index}`}><SelectValue /></SelectTrigger><SelectContent><SelectGroup><SelectItem value="key">Key</SelectItem><SelectItem value="shortcut">Shortcut</SelectItem><SelectItem value="disabled">Disable</SelectItem>{row.kind === "custom" ? <SelectItem value="custom">Legacy text</SelectItem> : null}</SelectGroup></SelectContent></Select></Field>
              <Button type="button" size="icon" variant="ghost" aria-label={`Remove mapping ${index + 1}`} onClick={() => setRows(current => current.filter((_, rowIndex) => rowIndex !== index))}><X /></Button>
            </FieldGroup>
            {row.kind === "key" || row.kind === "shortcut" ? <div className="mapping-target">
              {row.kind === "shortcut" ? <ToggleGroup type="multiple" variant="outline" size="sm" value={row.modifiers} onValueChange={value => updateRow(index, { modifiers: modifiers.filter(modifier => value.includes(modifier)) })} aria-label={`Modifiers ${index + 1}`}>{modifiers.map(modifier => <ToggleGroupItem key={modifier} value={modifier}>{modifier}</ToggleGroupItem>)}</ToggleGroup> : null}
              <KeyCapture id={`mapping-to-${index}`} label={row.key ? keyLabel(row.key) : ""} placeholder={row.kind === "shortcut" ? "Press the shortcut key" : "Press the new key"} shortcut={row.kind === "shortcut"} invalid={touched && !row.key} onCapture={(code, held) => updateRow(index, row.kind === "shortcut" ? { key: code, modifiers: held.length ? held : row.modifiers } : { key: code })} />
            </div> : null}
            {row.kind === "custom" ? <p className="mapping-warning">Action “{row.raw}” is not recognized. Choose Key, Shortcut or Disable to apply it.</p> : null}
            {row.kind === "shortcut" && touched && !row.modifiers.length ? <p className="mapping-warning">Choose at least one modifier.</p> : null}
            {unrecognized && row.kind !== "custom" ? <p className="mapping-warning">Source “{row.from}” is not recognized and will be ignored. Capture the key again.</p> : null}
            {duplicate ? <p className="mapping-warning">This source already appears above. The last row wins.</p> : null}
            {row.from && row.kind !== "custom" && rowTarget(row) ? <p className="mapping-summary">{sourceLabel(row.from)} → {targetLabel(rowTarget(row))}</p> : null}
          </div>;
        })}</div> : <p className="mapping-empty">No mappings. You can also save just the profile.</p>}
      </div>
      <p className="form-disclaimer">The profile only applies when Remapping is on. <Kbd>Ctrl</Kbd> + <Kbd>Alt</Kbd> + <Kbd>Esc</Kbd> is never remapped.</p>
      <DialogFooter><Button type="button" variant="outline" onClick={onClose}>Cancel</Button><Button type="submit" disabled={!!desktop.busy || !name.trim()}><BusyIcon busy={desktop.busy === "save-profile"} />Save profile</Button></DialogFooter>
    </form></DialogContent></Dialog>;
}

const mouseButtons: Record<number, keyof typeof mouseSources> = { 1: "MouseMiddle", 3: "MouseBack", 4: "MouseForward" };

function KeyCapture({ id, label, placeholder = "Press a key", allowMouse = false, shortcut = false, invalid, onCapture }: { id: string; label: string; placeholder?: string; allowMouse?: boolean; shortcut?: boolean; invalid?: boolean; onCapture: (code: string, held: Modifier[]) => void }) {
  const [listening, setListening] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  useEffect(() => {
    if (!listening) return;
    const stop = (event: Event) => { event.preventDefault(); event.stopPropagation(); };
    const onKey = (event: KeyboardEvent) => {
      stop(event);
      if (event.repeat || (shortcut && isModifierKey(event.code))) return;
      if (!isSupportedKey(event.code)) { setHint("This key cannot be used for remapping yet."); return; }
      const held = ([event.ctrlKey && "Ctrl", event.shiftKey && "Shift", event.altKey && "Alt", event.metaKey && "Win"].filter(Boolean)) as Modifier[];
      setHint(null); setListening(false); onCapture(event.code, held);
    };
    const onMouseDown = (event: MouseEvent) => {
      const code = allowMouse ? mouseButtons[event.button] : undefined;
      if (event.button !== 0) stop(event);
      if (code) { setHint(null); setListening(false); onCapture(code, []); }
    };
    const blockAux = (event: MouseEvent) => { if (event.button !== 0) stop(event); };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", stop, true);
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("mouseup", blockAux, true);
    window.addEventListener("auxclick", blockAux, true);
    window.addEventListener("contextmenu", stop, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", stop, true);
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("mouseup", blockAux, true);
      window.removeEventListener("auxclick", blockAux, true);
      window.removeEventListener("contextmenu", stop, true);
    };
  }, [listening, allowMouse, shortcut, onCapture]);
  return <div className="key-capture">
    <Button id={id} type="button" variant={listening ? "secondary" : "outline"} className="key-capture-button" aria-invalid={invalid} aria-pressed={listening} onClick={() => { setHint(null); setListening(value => !value); }} onBlur={() => setListening(false)}>
      {allowMouse ? <MousePointerClick data-icon="inline-start" /> : <Keyboard data-icon="inline-start" />}
      <span className="truncate">{listening ? (allowMouse ? "Press a key or button…" : "Press a key…") : label || placeholder}</span>
      {listening ? <RefreshCw className="animate-spin" data-icon="inline-end" aria-hidden="true" /> : null}
    </Button>
    {hint ? <p className="mapping-warning" role="status">{hint}</p> : null}
  </div>;
}

function RunningApps({ onPick }: { onPick: (exe: string) => void }) {
  const [apps, setApps] = useState<RunningApp[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    setError(null);
    try { setApps(await nativeCommand<RunningApp[]>("list_running_apps")); } catch (err) { setError(messageFrom(err)); setApps([]); }
  }, []);
  useEffect(() => { void load(); }, [load]);
  return <div className="running-apps">
    <Command>
      <CommandInput placeholder="Search open apps" />
      <CommandList>
        <CommandEmpty>{apps === null ? "Loading…" : error ?? "No apps with an open window were found."}</CommandEmpty>
        <CommandGroup heading="Apps with an open window">{(apps ?? []).map(app => <CommandItem key={app.exe} value={`${app.exe} ${app.title}`} onSelect={() => onPick(app.exe)}><AppWindow aria-hidden="true" /><span className="running-app-exe">{app.exe}</span><span className="running-app-title">{app.title}</span></CommandItem>)}</CommandGroup>
      </CommandList>
    </Command>
    <Button type="button" variant="ghost" size="sm" onClick={() => void load()}><RefreshCw data-icon="inline-start" />Refresh list</Button>
  </div>;
}
