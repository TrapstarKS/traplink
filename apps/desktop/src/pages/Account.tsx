import { useEffect, useRef, useState } from "react";
import { ArrowRight, Check, Copy, ExternalLink, LogOut, ShieldCheck } from "lucide-react";
import { Spinner } from "@/components/ui/spinner";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { BusyIcon, ConfirmDialog, IconButton, PageHeading, SecurityNote } from "@/components/common";
import { messageFrom, nativeCommand, type LoginResult, type LoginStart } from "@/lib/bridge";
import type { Desktop } from "@/lib/use-desktop";

export function Account({ desktop, onPreferences }: { desktop: Desktop; onPreferences: () => void }) {
  const [login, setLogin] = useState<LoginStart | null>(null);
  const [status, setStatus] = useState<"pending" | "expired" | "denied">("pending");
  const [copied, setCopied] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const [canceling, setCanceling] = useState(false);
  const loginGeneration = useRef(0);
  const account = desktop.snapshot?.account;
  const serverConfigured = !!desktop.snapshot?.settings.authServerUrl;

  useEffect(() => {
    if (!login || status !== "pending") return;
    let disposed = false;
    let polling = false;
    const generation = loginGeneration.current;
    const timer = window.setInterval(async () => {
      if (polling) return;
      polling = true;
      try {
        const response = await nativeCommand<LoginResult>("poll_login");
        if (disposed || generation !== loginGeneration.current) return;
        if (response.status === "authorized") { setLogin(null); await desktop.refresh(); toast.success("Account connected."); }
        else if (response.status === "expired" || response.status === "denied") setStatus(response.status);
      } catch (error) {
        if (!disposed && generation === loginGeneration.current) { setStatus("expired"); toast.error(messageFrom(error)); }
      } finally { polling = false; }
    }, Math.max(3, login.interval) * 1000);
    const expiry = window.setTimeout(() => { if (!disposed && generation === loginGeneration.current) setStatus("expired"); }, login.expiresIn * 1000);
    return () => { disposed = true; window.clearInterval(timer); window.clearTimeout(expiry); };
  }, [login, status, desktop.refresh]);

  async function startLogin() {
    const generation = ++loginGeneration.current;
    const response = await desktop.run("login", () => nativeCommand<LoginStart>("start_login"));
    if (response && generation === loginGeneration.current) { setLogin(response); setStatus("pending"); }
  }
  async function cancelLogin() {
    loginGeneration.current += 1;
    setLogin(null); setCanceling(true);
    try { await nativeCommand("cancel_login"); }
    catch (error) { toast.error(messageFrom(error)); }
    finally { setCanceling(false); await desktop.refresh(true); }
  }
  async function copyCode() {
    if (!login) return;
    try { await navigator.clipboard.writeText(login.userCode); setCopied(true); window.setTimeout(() => setCopied(false), 2000); }
    catch (error) { toast.error(messageFrom(error)); }
  }
  async function logout() {
    const result = await desktop.run("logout", async () => { await nativeCommand("logout"); return true; }, "Account disconnected.");
    if (result) setLoggingOut(false);
  }

  return <>
    <PageHeading title="My account" description="Manage the account authorized in this app." />
    <div className="account-layout">
      <Card>
        <CardHeader><CardTitle>{account ? "Account connected" : login ? "Authorize app" : "Connect account"}</CardTitle><CardDescription>{account ? "This session belongs to this computer." : "Sign in from your browser to authorize TrapLink."}</CardDescription></CardHeader>
        <CardContent>{account ? <div className="account-connected"><div className="account-details"><Avatar className="size-11 rounded-lg"><AvatarFallback className="rounded-lg text-base font-medium">{(account.name || account.email).slice(0, 1).toLocaleUpperCase("en-US")}</AvatarFallback></Avatar><div><h2>{account.name || "My account"}</h2><p>{account.email}</p></div></div><Badge variant="secondary"><ShieldCheck />Authorized</Badge><Button variant="outline" onClick={() => setLoggingOut(true)} disabled={!!desktop.busy}><LogOut data-icon="inline-start" />Sign out</Button></div> : login ? <div className="account-login"><p>Enter this code on the authorization page.</p><div className="login-code"><strong>{login.userCode}</strong><IconButton label="Copy authorization code" hint={copied ? "Copied" : "Copy code"} onClick={() => void copyCode()}>{copied ? <Check /> : <Copy />}</IconButton></div>{status === "pending" ? <><Button disabled={!!desktop.busy} onClick={() => void desktop.run("open-login", () => nativeCommand("reopen_login"))}>Open authorization<ExternalLink data-icon="inline-end" /></Button><p className="login-pending"><Spinner />Waiting for authorization…</p></> : <><Alert variant="destructive"><AlertTitle>{status === "expired" ? "This code expired" : "Authorization denied"}</AlertTitle><AlertDescription>Create a new code to try again.</AlertDescription></Alert><Button onClick={() => void startLogin()} disabled={!!desktop.busy}>Create new code</Button></>}<Button variant="link" onClick={() => void cancelLogin()}>Cancel</Button></div> : <div className="account-intro"><p>Connect your account to save and restore an encrypted backup across computers.</p><Button disabled={!desktop.native || !serverConfigured || !!desktop.busy || canceling} onClick={() => void startLogin()}><BusyIcon busy={desktop.busy === "login"} />Sign in<ArrowRight data-icon="inline-end" /></Button>{!desktop.native ? <span className="account-hint">Open the desktop app to connect your account.</span> : !serverConfigured ? <Button variant="link" onClick={onPreferences}>Set up authentication server</Button> : <span className="account-hint">The authorization page will open in your browser.</span>}</div>}</CardContent>
      </Card>
      <Card><CardHeader><CardTitle>About this session</CardTitle><CardDescription>Your local data stays on this computer.</CardDescription></CardHeader><CardContent className="account-aside"><SecurityNote>Credentials are stored in the operating system's secure keychain.</SecurityNote><SecurityNote>Local connections require pairing and permissions, even with your account connected.</SecurityNote></CardContent></Card>
    </div>
    <ConfirmDialog open={loggingOut} onOpenChange={setLoggingOut} title="Sign out of your account?" description="This app's session will end. Your profiles and local backups stay on this computer." action="Sign out" destructive busy={!!desktop.busy} onConfirm={() => void logout()} />
  </>;
}
