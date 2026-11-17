import { CircleAlert, CircleCheck, Download, Info, RefreshCw, RotateCcw } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";
import { checkForUpdates, installUpdate, useUpdater, type AvailableUpdate } from "@/lib/updater";

function size(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / 1024 / 1024).toLocaleString("en-US", { maximumFractionDigits: 1 })} MB`;
}
function formatDate(value?: string) {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.toLocaleDateString("en-US", { day: "2-digit", month: "long", year: "numeric" });
}

function UpdateDetails({ update }: { update: AvailableUpdate }) {
  const published = formatDate(update.date);
  return <div className="flex flex-col gap-2">
    <div className="flex flex-wrap items-center gap-2"><Badge>Version {update.version}</Badge>{published ? <span className="text-sm text-muted-foreground">Published {published}</span> : null}</div>
    {update.notes
      ? <div className="max-h-48 overflow-y-auto whitespace-pre-wrap rounded-md border bg-muted/40 p-3 text-sm">{update.notes}</div>
      : <p className="text-sm text-muted-foreground">This version has no release notes.</p>}
  </div>;
}

export function UpdatesPanel() {
  const { state, currentVersion } = useUpdater();
  const checking = state.status === "checking";
  const working = checking || state.status === "downloading" || state.status === "installing" || state.status === "installed";

  if (state.status === "unsupported") {
    return <Card className="settings-panel"><CardHeader><CardTitle>Updates</CardTitle><CardDescription>New versions are checked and installed by the desktop app.</CardDescription></CardHeader><CardContent>
      <Alert><Info /><AlertTitle>Only available in the desktop app</AlertTitle><AlertDescription>This browser preview doesn't check for or install updates. Open the TrapLink app installed on your computer.</AlertDescription></Alert>
    </CardContent></Card>;
  }

  const progress = state.status === "downloading" && state.total ? Math.min(100, Math.round(state.downloaded / state.total * 100)) : null;

  return <Card className="settings-panel"><CardHeader><CardTitle>Updates</CardTitle><CardDescription>Updates are signed and only installed when you confirm.</CardDescription></CardHeader><CardContent className="flex flex-col gap-4">
    <div className="about-panel"><div><p>Installed version</p><Badge variant="outline">{currentVersion ? `v${currentVersion}` : "Checking…"}</Badge></div>
      <Button type="button" variant="outline" onClick={() => void checkForUpdates()} disabled={working}>{checking ? <Spinner data-icon="inline-start" /> : <RefreshCw data-icon="inline-start" />}{checking ? "Checking…" : "Check for updates"}</Button>
    </div>

    {state.status === "current" ? <Alert><CircleCheck /><AlertTitle>TrapLink is up to date</AlertTitle><AlertDescription>No newer version found as of {new Date(state.checkedAt).toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit" })}.</AlertDescription></Alert> : null}

    {state.status === "error" ? <Alert variant="destructive"><CircleAlert /><AlertTitle>Update failed</AlertTitle><AlertDescription>{state.message}</AlertDescription></Alert> : null}

    {"update" in state && state.update ? <><Separator />
      <UpdateDetails update={state.update} />
      {state.status === "downloading" ? <div className="flex flex-col gap-2" aria-live="polite">
        <Progress value={progress ?? undefined} aria-label="Download progress" />
        <p className="text-sm text-muted-foreground">{progress !== null && state.total ? `Downloading ${size(state.downloaded)} of ${size(state.total)} (${progress}%)` : `Downloading ${size(state.downloaded)}…`}</p>
      </div> : null}
      {state.status === "installing" ? <p className="flex items-center gap-2 text-sm text-muted-foreground" aria-live="polite"><Spinner />Verifying signature and installing…</p> : null}
      {state.status === "installed" ? <p className="flex items-center gap-2 text-sm text-muted-foreground" aria-live="polite"><Spinner />Update installed. Restarting TrapLink…</p> : null}
      {state.status === "available" || state.status === "error" ? <div className="flex flex-wrap items-center gap-3">
        <Button type="button" onClick={() => void installUpdate()}>{state.status === "error" ? <RotateCcw data-icon="inline-start" /> : <Download data-icon="inline-start" />}{state.status === "error" ? "Try again" : "Download and install"}</Button>
        <span className="text-sm text-muted-foreground">TrapLink will restart when done. On Windows, the installer closes the app during installation.</span>
      </div> : null}
    </> : null}
  </CardContent></Card>;
}
