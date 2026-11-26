import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { Laptop, LogOut, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemSeparator, ItemTitle } from "@/components/ui/item";
import { Skeleton } from "@/components/ui/skeleton";
import { ConfirmDialog, EmptyState, formattedDate, IconButton, PlatformName, relativeDate } from "@/components/common";
import { listAccountDevices, messageFrom, revokeAccountDevice, type AccountDevice } from "@/lib/bridge";
import type { Desktop } from "@/lib/use-desktop";

/** The signed-in account's computers, with revoke for lost or unused ones. */
export function AccountDevices({ desktop, onSignOut }: { desktop: Desktop; onSignOut: () => void }) {
  const [devices, setDevices] = useState<AccountDevice[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [revoking, setRevoking] = useState<AccountDevice | null>(null);
  const request = useRef(0);
  const { refresh, run, busy } = desktop;

  const load = useCallback(async () => {
    const current = ++request.current;
    setLoading(true); setError(null);
    try {
      const list = await listAccountDevices();
      if (current === request.current) setDevices(list);
    } catch (err) {
      if (current === request.current) setError(messageFrom(err));
      // A rejected credential signs this computer out, so show that state.
      await refresh(true);
    } finally {
      if (current === request.current) setLoading(false);
    }
  }, [refresh]);
  useEffect(() => { void load(); return () => { request.current += 1; }; }, [load]);

  async function revoke() {
    if (!revoking) return;
    const target = revoking;
    const signedOut = await run(`revoke-${target.id}`, () => revokeAccountDevice(target.id));
    setRevoking(null);
    if (signedOut === undefined) {
      // A rejected credential signs this computer out; otherwise show the current list.
      await refresh(true);
      void load();
    } else if (signedOut) toast.success("This computer was signed out.");
    else { toast.success("Device revoked."); void load(); }
  }

  const now = Date.now();
  return <Card className="account-devices">
    <CardHeader className="account-devices-header">
      <div><CardTitle>Devices</CardTitle><CardDescription>Computers signed in to this account. Revoke one you lost or no longer use.</CardDescription></div>
      <IconButton label="Refresh devices" hint="Refresh" disabled={loading || !!busy} onClick={() => void load()}><RefreshCw /></IconButton>
    </CardHeader>
    <CardContent aria-busy={loading}>
      {error ? <Alert variant="destructive"><AlertTitle>Couldn't load devices</AlertTitle><AlertDescription><p>{error}</p><Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>Try again</Button></AlertDescription></Alert>
        : loading && !devices ? <div className="account-devices-loading" role="status" aria-label="Loading devices"><Skeleton className="h-12 w-full" /><Skeleton className="h-12 w-full" /></div>
        : devices?.length ? <ItemGroup>{devices.map((device, index) => {
          const revoked = !!device.revokedAt;
          const expired = !revoked && new Date(device.expiresAt).valueOf() <= now;
          return <Fragment key={device.id}>{index ? <ItemSeparator /> : null}<Item size="sm" className="px-0" data-inactive={revoked || expired}>
            <ItemMedia variant="icon"><Laptop /></ItemMedia>
            <ItemContent>
              <ItemTitle className="break-all">{device.name}{device.current ? <Badge variant="secondary">This computer</Badge> : null}{revoked ? <Badge variant="outline">Revoked</Badge> : expired ? <Badge variant="outline">Expired</Badge> : null}</ItemTitle>
              <ItemDescription>
                {device.platform === "unknown" ? "Unknown system" : <PlatformName platform={device.platform} />}{device.appVersion ? ` · TrapLink ${device.appVersion}` : ""}{" · "}
                {revoked && device.revokedAt ? <>Revoked <time dateTime={device.revokedAt} title={formattedDate(device.revokedAt)}>{relativeDate(device.revokedAt)}</time></>
                  : <>Last seen <time dateTime={device.lastSeenAt} title={formattedDate(device.lastSeenAt)}>{relativeDate(device.lastSeenAt)}</time>{" · "}{expired ? "Expired" : "Expires"} <time dateTime={device.expiresAt} title={formattedDate(device.expiresAt)}>{relativeDate(device.expiresAt)}</time></>}
              </ItemDescription>
            </ItemContent>
            <ItemActions>{device.current
              ? <Button variant="outline" size="sm" disabled={!!busy} onClick={onSignOut}><LogOut data-icon="inline-start" />Sign out</Button>
              : !revoked ? <Button variant="outline" size="sm" disabled={!!busy} onClick={() => setRevoking(device)} aria-label={`Revoke ${device.name}`}><Trash2 data-icon="inline-start" />Revoke</Button> : null}
            </ItemActions>
          </Item></Fragment>;
        })}</ItemGroup>
        : <EmptyState icon={Laptop} title="No devices" description="This account has no registered computers." />}
    </CardContent>
    <ConfirmDialog open={!!revoking} onOpenChange={open => { if (!open) setRevoking(null); }} title={`Revoke “${revoking?.name ?? ""}”?`} description="That computer is signed out and can no longer use this account's online backup. Signing in there again adds it back." action="Revoke" destructive busy={!!busy} onConfirm={() => void revoke()} />
  </Card>;
}
