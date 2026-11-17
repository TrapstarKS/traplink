import { Fragment, useEffect, useState } from "react";
import { CloudBackup } from "@/components/CloudBackup";
import { toast } from "sonner";
import { Archive, ArrowDownToLine, ArrowUpFromLine, Plus, RotateCcw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemSeparator, ItemTitle } from "@/components/ui/item";
import { BusyIcon, ConfirmDialog, EmptyState, formattedDate, IconButton, PageHeading, relativeDate, SecurityNote } from "@/components/common";
import type { Intent } from "@/App";
import { chooseBackupDestination, chooseFile, nativeCommand, type Backup } from "@/lib/bridge";
import type { Desktop } from "@/lib/use-desktop";

type BackupAction = { backup: Backup; kind: "restore" | "delete" };

export function Backups({ desktop, onAccount, intent, onIntentDone }: { desktop: Desktop; onAccount: () => void; intent?: Intent | null; onIntentDone?: () => void }) {
  const [creating, setCreating] = useState(false);
  useEffect(() => { if (intent?.kind === "new-backup") { setCreating(true); onIntentDone?.(); } }, [intent, onIntentDone]);
  const [name, setName] = useState("");
  const [action, setAction] = useState<BackupAction | null>(null);
  const [importing, setImporting] = useState(false);
  const backups = desktop.snapshot?.backups ?? [];

  async function create(event: React.FormEvent) {
    event.preventDefault();
    const result = await desktop.run("create-backup", () => nativeCommand<Backup>("create_backup", { name: name.trim() }), "Backup created.");
    if (result) { setCreating(false); setName(""); }
  }
  async function executeAction() {
    if (!action) return;
    const result = await desktop.run(`${action.kind}-backup`, async () => {
      await nativeCommand(action.kind === "restore" ? "restore_backup" : "delete_backup", { id: action.backup.id });
      return true;
    }, action.kind === "restore" ? "Backup restored." : "Backup deleted.");
    if (result) setAction(null);
  }
  async function exportBackup(backup: Backup) {
    await desktop.run(`export-${backup.id}`, async () => {
      const path = await chooseBackupDestination(backup.name);
      if (path) { await nativeCommand("export_backup_file", { id: backup.id, path }); return true; }
      return false;
    }).then(result => { if (result) toast.success("Backup exported."); });
  }
  async function importBackup() {
    const result = await desktop.run("import-backup", async () => {
      const path = await chooseFile();
      if (path) { await nativeCommand("import_backup_file", { path }); return true; }
      return false;
    });
    if (result) { setImporting(false); toast.success("Backup imported and restored."); }
  }

  return <>
    <PageHeading title="Backups" description="Save your profiles, computer name and appearance."><Button variant="outline" disabled={!desktop.native || !!desktop.busy} onClick={() => setImporting(true)}><ArrowUpFromLine data-icon="inline-start" />Import file</Button><Button disabled={!desktop.native || !!desktop.busy} onClick={() => setCreating(true)}><Plus data-icon="inline-start" />Create backup</Button></PageHeading>
    <div className="backup-layout">
      <Card><CardHeader><CardTitle>Local backups</CardTitle><CardDescription>{backups.length ? `${backups.length} cop${backups.length === 1 ? "y" : "ies"} on this computer.` : "Create a copy before changing or restoring a configuration."}</CardDescription></CardHeader>
        <CardContent>{backups.length ? <ItemGroup>{backups.map((backup, index) => <Fragment key={backup.id}>{index ? <ItemSeparator /> : null}<Item size="sm" className="px-0"><ItemMedia variant="icon"><Archive /></ItemMedia><ItemContent><ItemTitle className="break-all">{backup.name}</ItemTitle><ItemDescription><time dateTime={backup.createdAt} title={formattedDate(backup.createdAt)}>{relativeDate(backup.createdAt)}</time> · {backup.profileCount} profile{backup.profileCount === 1 ? "" : "s"}</ItemDescription></ItemContent><ItemActions><IconButton label={`Export ${backup.name}`} hint="Export to file" disabled={!!desktop.busy} onClick={() => void exportBackup(backup)}><ArrowDownToLine /></IconButton><Button variant="outline" size="sm" disabled={!!desktop.busy} onClick={() => setAction({ backup, kind: "restore" })}><RotateCcw data-icon="inline-start" />Restore</Button><IconButton label={`Delete ${backup.name}`} hint="Delete backup" disabled={!!desktop.busy} onClick={() => setAction({ backup, kind: "delete" })}><Trash2 /></IconButton></ItemActions></Item></Fragment>)}</ItemGroup> : <EmptyState icon={Archive} title="No saved backups" description="Use Create backup to save the current configuration on this computer." />}</CardContent>
      </Card>
      <CloudBackup desktop={desktop} onAccount={onAccount} />
    </div>
    <Dialog open={creating} onOpenChange={setCreating}><DialogContent><DialogHeader><div className="dialog-symbol"><Archive /></div><DialogTitle>Create backup</DialogTitle><DialogDescription>Save the name, appearance and {desktop.snapshot?.profiles.length ?? 0} profiles of this computer.</DialogDescription></DialogHeader><form onSubmit={event => void create(event)}><FieldGroup><Field><FieldLabel htmlFor="backup-name">Backup name</FieldLabel><Input id="backup-name" value={name} onChange={event => setName(event.target.value)} placeholder="e.g. My work setup" maxLength={80} required autoFocus /><FieldDescription>Pick a name that helps you recognize this configuration.</FieldDescription></Field></FieldGroup><DialogFooter className="mt-6"><Button type="button" variant="outline" onClick={() => setCreating(false)}>Cancel</Button><Button type="submit" disabled={!name.trim() || !!desktop.busy}><BusyIcon busy={desktop.busy === "create-backup"} />Create backup</Button></DialogFooter></form></DialogContent></Dialog>
    <ConfirmDialog open={!!action} onOpenChange={open => { if (!open) setAction(null); }} title={action?.kind === "restore" ? "Restore this backup?" : "Delete this backup?"} description={action?.kind === "restore" ? `Your profiles, computer name and appearance will be replaced by the copy “${action.backup.name}”. Sharing will be turned off. Create a current backup if you want to keep this data.` : `The copy “${action?.backup.name ?? ""}” will be removed from this computer. Files already exported will be kept.`} action={action?.kind === "restore" ? "Restore backup" : "Delete backup"} destructive busy={!!desktop.busy} onConfirm={() => void executeAction()} />
    <Dialog open={importing} onOpenChange={setImporting}><DialogContent><DialogHeader><div className="dialog-symbol"><ArrowUpFromLine /></div><DialogTitle>Import and restore a backup</DialogTitle><DialogDescription>Choose a TrapLink backup file. It will replace the profiles, computer name and appearance. Sharing will be turned off.</DialogDescription></DialogHeader><SecurityNote>Choose a file you recognize. Restoring keeps this computer's connections, credentials, server and downloads folder.</SecurityNote><DialogFooter><Button variant="outline" onClick={() => setImporting(false)}>Cancel</Button><Button disabled={!!desktop.busy} onClick={() => void importBackup()}><BusyIcon busy={desktop.busy === "import-backup"} />Choose file</Button></DialogFooter></DialogContent></Dialog>
  </>;
}
