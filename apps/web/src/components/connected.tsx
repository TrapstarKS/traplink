"use client";

import { useRef, useState } from "react";
import Link from "next/link";
import { CircleCheck, LogOut, RotateCw, TriangleAlert } from "lucide-react";
import { authClient } from "@/lib/auth-client";
import { AuthHeader } from "@/components/auth-header";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

export function SessionLoading({ description }: { description: string }) {
  return (
    <div aria-busy="true" className="flex flex-col gap-6">
      <AuthHeader title="Checking your account…" description={description} />
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner aria-label="Checking your account" />
        <span aria-hidden="true">This only takes a moment.</span>
      </div>
    </div>
  );
}

export function SessionUnavailable({ detail, retrying, onRetry }: { detail: string; retrying: boolean; onRetry: () => void }) {
  return (
    <>
      <AuthHeader title="Could not load" description="The account service is currently unavailable." />
      <Alert variant="destructive">
        <TriangleAlert aria-hidden="true" />
        <AlertTitle>Could not check your session</AlertTitle>
        <AlertDescription>{detail}</AlertDescription>
      </Alert>
      <Button onClick={onRetry} disabled={retrying}>
        {retrying ? <Spinner data-icon="inline-start" aria-hidden="true" /> : <RotateCw data-icon="inline-start" aria-hidden="true" />}
        {retrying ? "Retrying…" : "Try again"}
      </Button>
    </>
  );
}

export function Connected() {
  const { data: session, isPending, isRefetching, error: sessionError, refetch } = authClient.useSession();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const errorNotice = useRef<HTMLDivElement>(null);

  async function signOut() {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError("");
    try {
      const result = await authClient.signOut();
      if (result.error) throw new Error();
      window.location.assign("/sign-in");
      return;
    } catch {
      setError("Signing out failed. Try again.");
      requestAnimationFrame(() => errorNotice.current?.focus());
    }
    inFlight.current = false;
    setBusy(false);
  }

  if (isPending) return <SessionLoading description="Please wait to continue to the app." />;
  if (sessionError && sessionError.status !== 401) {
    return <SessionUnavailable detail="Try loading your session again." retrying={isRefetching} onRetry={() => void refetch()} />;
  }
  if (!session) {
    return (
      <>
        <AuthHeader title="Sign in to your account" description="Sign in to connect the app on your computer." />
        <Button asChild className="w-full"><Link href="/sign-in">Sign in</Link></Button>
        <p className="text-center text-sm text-muted-foreground">
          Don't have an account? <Link className="inline-link" href="/sign-up">Create account</Link>
        </p>
      </>
    );
  }

  return (
    <>
      <AuthHeader title="Account connected" description="Continue in TrapLink on your computer. Devices, connections and backups are managed in the app." />
      <Alert role="status">
        <CircleCheck aria-hidden="true" />
        <AlertTitle>Signed in as</AlertTitle>
        <AlertDescription className="break-all">{session.user.email}</AlertDescription>
      </Alert>
      {error ? (
        <Alert ref={errorNotice} tabIndex={-1} variant="destructive">
          <TriangleAlert aria-hidden="true" />
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
      <div className="flex flex-col gap-2">
        <Button asChild><Link href="/device">I have a code from the app</Link></Button>
        <Button variant="ghost" onClick={() => void signOut()} disabled={busy}>
          {busy ? <Spinner data-icon="inline-start" aria-hidden="true" /> : <LogOut data-icon="inline-start" aria-hidden="true" />}
          {busy ? "Signing out…" : "Sign out"}
        </Button>
      </div>
    </>
  );
}
