import { useRef, useState, type FormEvent } from "react";
import { ArrowRight, CloudDownload, CloudUpload, KeyRound } from "lucide-react";
import { Spinner } from "@/components/ui/spinner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { nativeCommand, utf8Length, type Backup } from "@/lib/bridge";
import type { Desktop } from "@/lib/use-desktop";

export function CloudBackup({ desktop, onAccount }: { desktop: Desktop; onAccount: () => void }) {
  const [action, setAction] = useState<"upload" | "download" | null>(null);
  const [backupId, setBackupId] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [working, setWorking] = useState(false);
  const submitting = useRef(false);
  const backups = desktop.snapshot?.backups ?? [];
  const connected = !!desktop.snapshot?.account;
  const supported = !!desktop.capabilities.cloudBackups;
  const passphraseValid = Array.from(passphrase).length >= 12 && utf8Length(passphrase) <= 1024 && !!passphrase.trim();
  function close() { if (!working) { setAction(null); setPassphrase(""); setConfirmation(""); } }
  async function submit(event: FormEvent) {
    event.preventDefault(); if (submitting.current || !action || !passphraseValid || (action === "upload" && (!backupId || passphrase !== confirmation))) return;
    submitting.current = true;
    setWorking(true);
    try {
      const result = await desktop.run("cloud-backup", async () => {
        if (action === "upload") return nativeCommand<{ updatedAt: string; bytes: number }>("upload_cloud_backup", { id: backupId, passphrase });
        return nativeCommand<Backup>("download_cloud_backup", { passphrase });
      }, action === "upload" ? "Encrypted copy saved to your account." : "Copy recovered and saved locally. You can restore it anytime.");
      if (result) setAction(null);
    } finally { setPassphrase(""); setConfirmation(""); setWorking(false); submitting.current = false; }
  }
  return <Card>
    <CardHeader><CardTitle>Account backup</CardTitle><CardDescription>An encrypted copy, protected by a passphrase only you know.</CardDescription></CardHeader><CardContent className="cloud-backup-content">
    <div className="cloud-backup-actions"><Button variant="outline" disabled={!connected || !supported || !backups.length || !!desktop.busy} onClick={() => {setBackupId(backups[0]?.id ?? "");setAction("upload");}}><CloudUpload data-icon="inline-start"/>Save to account</Button><Button variant="outline" disabled={!connected || !supported || !!desktop.busy} onClick={() => setAction("download")}><CloudDownload data-icon="inline-start"/>Recover from account</Button></div>
    {!connected ? <div className="cloud-backup-note"><p>Connect your account to use online backup. Your local backups remain available.</p><Button variant="link" onClick={onAccount}>Open my account<ArrowRight data-icon="inline-end" /></Button></div> : !supported ? <p className="cloud-backup-note">Online backup isn't available in this version yet.</p> : <p className="cloud-backup-note">One copy per account. Saving a new one replaces the previous one. Local backups are kept.</p>}
    </CardContent><Dialog open={action !== null} onOpenChange={open => { if(!open) close(); }}><DialogContent><DialogHeader><DialogTitle>{action === "upload" ? "Save encrypted copy" : "Recover encrypted copy"}</DialogTitle><DialogDescription>{action === "upload" ? "Choose the local backup and a passphrase. Keep this passphrase to recover on another computer." : "Use the same passphrase you chose when saving. The copy will be added to your local backups without changing your current profiles."}</DialogDescription></DialogHeader><form onSubmit={submit}><FieldGroup>{action === "upload" && <Field><FieldLabel htmlFor="cloud-backup-id">Local backup</FieldLabel><Select value={backupId} onValueChange={setBackupId} disabled={working}><SelectTrigger id="cloud-backup-id"><SelectValue placeholder="Choose a backup"/></SelectTrigger><SelectContent><SelectGroup>{backups.map(backup => <SelectItem key={backup.id} value={backup.id}>{backup.name}</SelectItem>)}</SelectGroup></SelectContent></Select></Field>}<Field data-invalid={!!passphrase && !passphraseValid}><FieldLabel htmlFor="backup-passphrase">Passphrase</FieldLabel><Input id="backup-passphrase" aria-invalid={!!passphrase && !passphraseValid} type="password" autoComplete="off" maxLength={1024} value={passphrase} onChange={event => setPassphrase(event.target.value)} required disabled={working}/><FieldDescription>Use a unique passphrase with at least 12 characters and up to 1024 bytes. Keep it to recover the backup.</FieldDescription></Field>{action === "upload" && <Field data-invalid={!!confirmation && confirmation !== passphrase}><FieldLabel htmlFor="backup-passphrase-confirm">Confirm passphrase</FieldLabel><Input id="backup-passphrase-confirm" aria-invalid={!!confirmation && confirmation !== passphrase} type="password" autoComplete="off" maxLength={1024} value={confirmation} onChange={event => setConfirmation(event.target.value)} required disabled={working}/>{confirmation && confirmation !== passphrase ? <FieldDescription>Passphrases must match.</FieldDescription> : null}</Field>}<Alert><KeyRound/><AlertDescription>{action === "upload" ? "This replaces the previous online copy. Use a long passphrase and keep it somewhere safe." : "Recovering adds a local copy. Your current profiles are kept until you choose to restore."}</AlertDescription></Alert></FieldGroup><DialogFooter className="mt-6"><Button type="button" variant="outline" onClick={close} disabled={working}>Cancel</Button><Button type="submit" disabled={working || !passphraseValid || (action === "upload" && (!backupId || confirmation !== passphrase))}>{working && <Spinner data-icon="inline-start"/>}{action === "upload" ? "Encrypt and save" : "Recover copy"}</Button></DialogFooter></form></DialogContent></Dialog>
  </Card>;
}
