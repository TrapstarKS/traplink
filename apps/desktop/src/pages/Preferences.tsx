import { useEffect, useState } from "react";
import { Clipboard, FolderDown, Globe2, Info, Keyboard, Laptop, LockKeyhole, Moon, Network, Save, Sun } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldLegend, FieldSet } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BusyIcon, PageHeading, SecurityNote, StatusBadge } from "@/components/common";
import { UpdatesPanel } from "@/components/UpdatesPanel";
import { toast } from "sonner";
import { chooseDirectory, messageFrom, nativeCommand, type Settings } from "@/lib/bridge";
import type { Desktop } from "@/lib/use-desktop";
import { useUpdater } from "@/lib/updater";

const defaultSettings: Settings = { deviceName: "", authServerUrl: "", clipboardEnabled: false, fileSharingEnabled: false, inputEnabled: false, startOnLogin: false, theme: "dark", downloadDirectory: "", port: 48372 };

export function Preferences({ desktop }: { desktop: Desktop }) {
  const initial = desktop.snapshot?.settings ?? defaultSettings;
  const [draft, setDraft] = useState<Settings>(() => ({ ...initial }));
  const [baseline, setBaseline] = useState(() => JSON.stringify(initial));
  const disabled = !desktop.native || !desktop.snapshot || !!desktop.busy;
  const draftJson = JSON.stringify(draft);
  const initialJson = JSON.stringify(initial);
  const dirty = draftJson !== baseline;
  const [tab, setTab] = useState("general");
  const { focusRequest } = useUpdater();
  useEffect(() => { if (focusRequest) setTab("updates"); }, [focusRequest]);
  const isMac = desktop.snapshot?.platform === "macos";
  async function openPane(pane: "accessibility" | "input_monitoring") {
    try { await nativeCommand("open_privacy_settings", { pane }); } catch (error) { toast.error(messageFrom(error)); }
  }
  const valid = !!draft.deviceName.trim() && draft.deviceName.trim().length <= 80 && Number.isInteger(draft.port) && draft.port >= 1024 && draft.port <= 65535 && !!draft.authServerUrl.trim() && !!draft.downloadDirectory.trim();

  useEffect(() => {
    if (!dirty || draftJson === initialJson) {
      setDraft(JSON.parse(initialJson) as Settings);
      setBaseline(initialJson);
    }
  }, [dirty, draftJson, initialJson]);

  function update<K extends keyof Settings>(key: K, value: Settings[K]) { setDraft(current => ({ ...current, [key]: value })); }
  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!valid || disabled) return;
    const settings = { ...draft, deviceName: draft.deviceName.trim(), authServerUrl: draft.authServerUrl.trim(), downloadDirectory: draft.downloadDirectory.trim() };
    const saved = await desktop.run("save-settings", () => nativeCommand("update_settings", { settings }), "Preferences saved.");
    if (saved) { setDraft(settings); setBaseline(JSON.stringify(settings)); }
  }
  async function selectDirectory() {
    const path = await desktop.run("choose-directory", chooseDirectory);
    if (path) update("downloadDirectory", path);
  }

  return <>
    <PageHeading title="Preferences" description="Configure this computer and sharing features." />
    <form onSubmit={event => void save(event)} className="preferences-form">
      <Tabs value={tab} onValueChange={setTab}>
        <TabsList className="preference-tabs"><TabsTrigger value="general">General</TabsTrigger><TabsTrigger value="sharing">Sharing</TabsTrigger><TabsTrigger value="permissions">Permissions</TabsTrigger><TabsTrigger value="updates">Updates</TabsTrigger></TabsList>
        <TabsContent value="general"><div className="settings-stack">
          <Card className="settings-panel"><CardHeader><CardTitle>This computer</CardTitle><CardDescription>App identity and appearance.</CardDescription></CardHeader><CardContent><FieldGroup>
            <Field data-disabled={disabled}><FieldLabel htmlFor="device-name">Computer name</FieldLabel><Input id="device-name" value={draft.deviceName} onChange={event => update("deviceName", event.target.value)} disabled={disabled} placeholder={desktop.native ? "My computer" : "Available in the desktop app"} maxLength={80} required /></Field>
            <Field data-disabled={disabled}><FieldLabel htmlFor="appearance">Appearance</FieldLabel><Select value={draft.theme} onValueChange={value => update("theme", value as Settings["theme"])} disabled={disabled}><SelectTrigger id="appearance"><SelectValue /></SelectTrigger><SelectContent><SelectGroup><SelectItem value="dark"><span className="select-with-icon"><Moon />Dark</span></SelectItem><SelectItem value="light"><span className="select-with-icon"><Sun />Light</span></SelectItem><SelectItem value="system"><span className="select-with-icon"><Laptop />Use system setting</span></SelectItem></SelectGroup></SelectContent></Select></Field>
            <Field orientation="horizontal" data-disabled={!desktop.capabilities.startOnLogin || disabled}><FieldContent><FieldLabel htmlFor="start-on-login">Start at login</FieldLabel><FieldDescription>Opens TrapLink in the system tray at sign-in without showing the window, so remapping works without opening the app. While on, closing the window keeps TrapLink in the tray; use Quit in the tray menu to exit.</FieldDescription></FieldContent><Switch id="start-on-login" checked={draft.startOnLogin} disabled={!desktop.capabilities.startOnLogin || disabled} onCheckedChange={value => update("startOnLogin", value)} /></Field>
          </FieldGroup></CardContent></Card>
          <Card className="settings-panel"><CardHeader><CardTitle>Account and network</CardTitle><CardDescription>Authentication service and local connection.</CardDescription></CardHeader><CardContent><FieldGroup>
            <Field data-disabled={disabled}><FieldLabel htmlFor="auth-server">Authentication server</FieldLabel><Input id="auth-server" type="url" value={draft.authServerUrl} onChange={event => update("authServerUrl", event.target.value)} placeholder="https://account.yourdomain.com" disabled={disabled} /><FieldDescription>Used to sign in and authorize the app.</FieldDescription></Field>
            <Field data-disabled={disabled}><FieldLabel htmlFor="network-port">Local network port</FieldLabel><Input id="network-port" type="number" min={1024} max={65535} value={desktop.native ? draft.port : ""} placeholder="Set in the desktop app" onChange={event => update("port", Number(event.target.value))} disabled={disabled} required /><FieldDescription>Computers connect directly over the local network.</FieldDescription></Field>
          </FieldGroup>{desktop.snapshot?.localAddresses.length ? <div className="local-addresses"><Network /><div><span>This computer's addresses</span>{desktop.snapshot.localAddresses.map(address => <code key={address}>{address}</code>)}</div></div> : null}</CardContent></Card>
        </div></TabsContent>
        <TabsContent value="sharing"><div className="settings-stack">
          <Card className="settings-panel"><CardHeader><CardTitle>Sharing</CardTitle><CardDescription>Turning on a feature for a computer in Connections also turns it on here.</CardDescription></CardHeader><CardContent><FieldGroup><FieldSet><FieldLegend className="sr-only">Sharing permissions</FieldLegend><FieldGroup>
            <Field orientation="horizontal" data-disabled={disabled || !desktop.capabilities.clipboardSharing}><span className="setting-icon"><Clipboard /></span><FieldContent><FieldLabel htmlFor="clipboard-enabled">Clipboard</FieldLabel><FieldDescription>Send and receive copied text, images, and files. Copied files also require file sharing.</FieldDescription></FieldContent><Switch id="clipboard-enabled" checked={draft.clipboardEnabled} onCheckedChange={value => update("clipboardEnabled", value)} disabled={disabled || !desktop.capabilities.clipboardSharing} /></Field><Separator />
            <Field orientation="horizontal" data-disabled={disabled || !desktop.capabilities.fileSharing}><span className="setting-icon"><FolderDown /></span><FieldContent><FieldLabel htmlFor="files-enabled">Files</FieldLabel><FieldDescription>Send and receive files up to 128 MB.</FieldDescription></FieldContent><Switch id="files-enabled" checked={draft.fileSharingEnabled} onCheckedChange={value => update("fileSharingEnabled", value)} disabled={disabled || !desktop.capabilities.fileSharing} /></Field><Separator />
            <Field orientation="horizontal" data-disabled={!desktop.capabilities.inputSharing || disabled}><span className="setting-icon"><Keyboard /></span><FieldContent><FieldLabel htmlFor="input-enabled">Mouse and keyboard</FieldLabel><FieldDescription>{`Accept control from allowed computers. Stays on until you turn it off; ${isMac ? "Control + Option + Esc" : "Ctrl + Alt + Esc"} stops it.`}</FieldDescription></FieldContent><Switch id="input-enabled" checked={draft.inputEnabled} onCheckedChange={value => update("inputEnabled", value)} disabled={!desktop.capabilities.inputSharing || disabled} /></Field>
          </FieldGroup></FieldSet><Separator />
            <Field data-disabled={disabled}><FieldLabel htmlFor="download-directory">Folder for received files</FieldLabel><div className="directory-field"><Input id="download-directory" value={draft.downloadDirectory} onChange={event => update("downloadDirectory", event.target.value)} disabled={disabled} placeholder="System Downloads folder" /><Button type="button" variant="outline" onClick={() => void selectDirectory()} disabled={disabled}>Choose folder</Button></div><FieldDescription>Files are saved to a TrapLink subfolder without replacing existing files.</FieldDescription></Field>
          </FieldGroup></CardContent></Card>
          <SecurityNote>Features start off, and each computer needs its own permission.</SecurityNote>
        </div></TabsContent>
        <TabsContent value="permissions"><div className="settings-stack">
          <Card className="settings-panel"><CardHeader><CardTitle>System permissions</CardTitle><CardDescription>Access reported by the operating system.</CardDescription></CardHeader><CardContent className="content-stack">
            <div className="system-permission-row"><span className="setting-icon"><Keyboard /></span><div><h3>Accessibility</h3><p>{desktop.snapshot?.platform === "macos" ? "Required to receive mouse and keyboard control." : "Input access depends on system permissions."}</p></div>{isMac ? <Button type="button" size="sm" variant="outline" onClick={() => void openPane("accessibility")}>Open Settings</Button> : null}<StatusBadge active={!!desktop.snapshot?.permissions.accessibility}>{desktop.snapshot?.permissions.accessibility ? "Authorized" : "Not verified"}</StatusBadge></div><Separator />
            <div className="system-permission-row"><span className="setting-icon"><LockKeyhole /></span><div><h3>Input Monitoring</h3><p>{isMac ? "Required for Flow to read the keyboard. After allowing it, quit and reopen TrapLink." : "Input is sent only from the focused control area or while Flow is on."}</p></div>{isMac ? <Button type="button" size="sm" variant="outline" onClick={() => void openPane("input_monitoring")}>Open Settings</Button> : null}<StatusBadge active={!!desktop.snapshot?.permissions.inputMonitoring}>{desktop.snapshot?.permissions.inputMonitoring ? "Authorized" : "Not verified"}</StatusBadge></div>
          </CardContent></Card>
          <Alert className="settings-panel"><Info /><AlertTitle>Feature availability</AlertTitle><AlertDescription>Controls unavailable in this version are disabled. Review the access granted to the app in system settings.</AlertDescription></Alert>
          <Card className="settings-panel"><CardHeader><CardTitle>About TrapLink</CardTitle></CardHeader><CardContent className="about-panel"><p>Desktop app</p><Badge variant="outline">{desktop.snapshot ? `v${desktop.snapshot.version}` : "Interface preview"}</Badge></CardContent></Card>
        </div></TabsContent>
        <TabsContent value="updates"><div className="settings-stack"><UpdatesPanel /></div></TabsContent>
      </Tabs>
      <div className="save-bar"><div>{desktop.native ? <><span className="save-state" data-dirty={dirty} />{dirty ? "Unsaved changes" : "Preferences saved"}</> : <><Info />Preferences are available in the desktop app.</>}</div><Button type="submit" disabled={disabled || !dirty || !valid}><BusyIcon busy={desktop.busy === "save-settings"} />{desktop.busy !== "save-settings" ? <Save data-icon="inline-start" /> : null}Save changes</Button></div>
    </form>
  </>;
}
