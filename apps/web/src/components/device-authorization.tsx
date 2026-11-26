"use client";

import { useRef, useState, type FormEvent } from "react";
import Link from "next/link";
import { REGEXP_ONLY_DIGITS_AND_CHARS } from "input-otp";
import { CircleCheck, CircleX, Laptop, TriangleAlert } from "lucide-react";
import { authClient } from "@/lib/auth-client";
import { deviceError } from "@/lib/device-errors";
import { AuthHeader } from "@/components/auth-header";
import { SessionLoading, SessionUnavailable } from "@/components/connected";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { InputOTP, InputOTPGroup, InputOTPSeparator, InputOTPSlot } from "@/components/ui/input-otp";
import { Separator } from "@/components/ui/separator";
import { Spinner } from "@/components/ui/spinner";

const CODE_LENGTH = 8;

function normalizeCode(value: string) { return value.replace(/[^a-z0-9]/gi, "").toUpperCase(); }

export function DeviceAuthorization({ initialCode }: { initialCode: string }) {
  const { data: session, isPending, isRefetching, error: sessionError, refetch } = authClient.useSession();
  const [code, setCode] = useState(() => normalizeCode(initialCode).slice(0, CODE_LENGTH));
  const [verification, setVerification] = useState<{ code: string; sessionId: string } | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState<"verify" | "approve" | "deny" | "account" | null>(null);
  const [error, setError] = useState("");
  const [done, setDone] = useState<"approved" | "denied" | null>(null);
  const inFlight = useRef(false);
  const errorNotice = useRef<HTMLDivElement>(null);
  const codeInput = useRef<HTMLInputElement>(null);
  const confirmationInput = useRef<HTMLButtonElement>(null);
  const normalizedCode = normalizeCode(code);
  const verified = verification !== null && verification.code === normalizedCode && verification.sessionId === session?.session.id;
  const callback = `/device${normalizedCode ? `?user_code=${encodeURIComponent(normalizedCode)}` : ""}`;
  const signInURL = `/sign-in?callback=${encodeURIComponent(callback)}`;
  const signUpURL = `/sign-up?callback=${encodeURIComponent(callback)}`;
  const codeInvalid = !!error && !verified;

  function fail(message: string) {
    setError(message);
    requestAnimationFrame(() => errorNotice.current?.focus());
  }

  async function verifyCode(candidate: string) {
    if (inFlight.current || !session) return;
    if (verification && verification.code === candidate && verification.sessionId === session.session.id) return;
    setError("");
    if (candidate.length !== CODE_LENGTH) {
      setError("Enter the eight characters of the code shown in the app.");
      requestAnimationFrame(() => codeInput.current?.focus());
      return;
    }
    inFlight.current = true;
    setBusy("verify");
    setVerification(null);
    setConfirmed(false);
    try {
      const result = await authClient.device({ query: { user_code: candidate } });
      if (result.error) throw new Error(deviceError(result.error, "Invalid code. Check the app and try again."));
      if (!result.data || result.data.client_id !== "traplink-desktop") throw new Error("Invalid code or linked to another session. Generate a new code in the app.");
      if (result.data.status !== "pending") throw new Error("This code has already been used. Generate a new one in the app.");
      setVerification({ code: candidate, sessionId: session.session.id });
      requestAnimationFrame(() => confirmationInput.current?.focus());
    } catch (cause) {
      fail(cause instanceof Error ? cause.message : "The service is unavailable. Try again shortly.");
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  }

  function verify(event: FormEvent) {
    event.preventDefault();
    void verifyCode(normalizedCode);
  }

  async function decide(approve: boolean) {
    if (inFlight.current || !verified || (approve && !confirmed)) return;
    inFlight.current = true;
    setBusy(approve ? "approve" : "deny");
    setError("");
    try {
      const result = approve
        ? await authClient.device.approve({ userCode: verification!.code })
        : await authClient.device.deny({ userCode: verification!.code });
      if (result.error) throw new Error(deviceError(result.error, "The request failed. Check the code and try again."));
      setDone(approve ? "approved" : "denied");
    } catch (cause) {
      setVerification(null);
      setConfirmed(false);
      fail(cause instanceof Error ? cause.message : "The service is unavailable. Try again shortly.");
    } finally {
      inFlight.current = false;
      setBusy(null);
    }
  }

  async function switchAccount() {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy("account");
    setError("");
    try {
      const result = await authClient.signOut();
      if (result.error) throw new Error();
      window.location.assign(signInURL);
      return;
    } catch {
      fail("Switching accounts failed. Try again.");
    }
    inFlight.current = false;
    setBusy(null);
  }

  function reset() {
    setDone(null);
    setCode("");
    setVerification(null);
    setConfirmed(false);
    setError("");
    requestAnimationFrame(() => codeInput.current?.focus());
  }

  if (isPending) return <SessionLoading description="Please wait to authorize the app." />;
  if (sessionError && sessionError.status !== 401) {
    return <SessionUnavailable detail="Try again. Your code is still on this page." retrying={isRefetching} onRetry={() => void refetch()} />;
  }
  if (!session) {
    return (
      <>
        <AuthHeader title="Authorize app" description="Sign in to connect TrapLink on your computer." />
        {normalizedCode.length === CODE_LENGTH ? (
          <div className="flex items-center justify-between gap-4 rounded-lg border bg-muted/40 px-4 py-3">
            <p className="text-sm text-muted-foreground">Code received</p>
            <Badge variant="outline" className="font-mono tracking-widest">{`${normalizedCode.slice(0, 4)}-${normalizedCode.slice(4)}`}</Badge>
          </div>
        ) : null}
        <Button asChild autoFocus><Link href={signInURL}>Sign in to my account</Link></Button>
        <p className="text-center text-sm text-muted-foreground">
          Don't have an account? <Link className="inline-link" href={signUpURL}>Create account</Link>
        </p>
        <p className="text-sm leading-relaxed text-muted-foreground">The computer is only authorized after you confirm.</p>
      </>
    );
  }
  if (done) {
    return (
      <>
        <div role="status" aria-live="polite" className="flex flex-col gap-6">
          <AuthHeader
            title={done === "approved" ? "App authorized" : "Request denied"}
            description={done === "approved" ? "Return to TrapLink on your computer to finish connecting. You can close this tab." : "No access was granted. To connect the computer, generate a new code in the app."}
          />
          <Alert>
            {done === "approved" ? <CircleCheck aria-hidden="true" /> : <CircleX aria-hidden="true" />}
            <AlertDescription>
              {done === "approved" ? "The computer received its own credential, which can be revoked in the app." : "The denied code can no longer be used."}
            </AlertDescription>
          </Alert>
        </div>
        <Button className="w-full" variant="outline" onClick={reset}>Check another code</Button>
      </>
    );
  }

  return (
    <>
      <AuthHeader title="Authorize app" description="Check the code shown in TrapLink on your computer." />
      <div className="flex items-center justify-between gap-4 rounded-lg border bg-muted/40 px-4 py-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <p className="text-xs text-muted-foreground">Signed-in account</p>
          <p className="account-email">{session.user.email}</p>
        </div>
        {!verified ? (
          <Button variant="link" size="sm" className="shrink-0 px-0" onClick={() => void switchAccount()} disabled={busy !== null}>
            {busy === "account" ? <Spinner data-icon="inline-start" aria-hidden="true" /> : null}
            Use another account
          </Button>
        ) : null}
      </div>
      {error ? (
        <Alert ref={errorNotice} tabIndex={-1} variant="destructive" id="device-error">
          <TriangleAlert aria-hidden="true" />
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
      <form onSubmit={verify} noValidate aria-busy={busy !== null}>
        <FieldGroup>
          <Field data-invalid={codeInvalid} data-disabled={busy !== null}>
            <FieldLabel htmlFor="device-code">App code</FieldLabel>
            <InputOTP
              ref={codeInput}
              id="device-code"
              name="device-code"
              maxLength={CODE_LENGTH}
              pattern={REGEXP_ONLY_DIGITS_AND_CHARS}
              pasteTransformer={pasted => normalizeCode(pasted).slice(0, CODE_LENGTH)}
              value={code}
              onChange={value => { setCode(value.toUpperCase()); setVerification(null); setConfirmed(false); setError(""); }}
              onComplete={(value: string) => void verifyCode(normalizeCode(value))}
              autoFocus={normalizedCode.length !== CODE_LENGTH}
              autoComplete="one-time-code"
              autoCapitalize="characters"
              spellCheck={false}
              inputMode="text"
              disabled={busy !== null}
              aria-invalid={codeInvalid}
              aria-describedby={codeInvalid ? "device-code-help device-error" : "device-code-help"}
              containerClassName="justify-center sm:justify-start"
            >
              <InputOTPGroup>
                {[0, 1, 2, 3].map(index => <InputOTPSlot key={index} index={index} aria-invalid={codeInvalid} className="size-9 font-mono text-base uppercase min-[400px]:size-10" />)}
              </InputOTPGroup>
              <InputOTPSeparator />
              <InputOTPGroup>
                {[4, 5, 6, 7].map(index => <InputOTPSlot key={index} index={index} aria-invalid={codeInvalid} className="size-9 font-mono text-base uppercase min-[400px]:size-10" />)}
              </InputOTPGroup>
            </InputOTP>
            <FieldDescription id="device-code-help">Type or paste the eight characters, with or without the hyphen. Checking starts once the code is complete.</FieldDescription>
          </Field>
          {!verified ? (
            <Button type="submit" disabled={busy !== null} autoFocus={normalizedCode.length === CODE_LENGTH}>
              {busy === "verify" ? <Spinner data-icon="inline-start" aria-hidden="true" /> : null}
              {busy === "verify" ? "Checking code…" : "Check code"}
            </Button>
          ) : null}
        </FieldGroup>
      </form>
      {verified ? (
        <div className="flex flex-col gap-6">
          <Separator />
          <Alert role="status">
            <Laptop aria-hidden="true" />
            <AlertTitle>Valid code</AlertTitle>
            <AlertDescription>TrapLink is requesting to link this computer and access your backups. Remote control requires separate pairing in the app.</AlertDescription>
          </Alert>
          <Field orientation="horizontal" data-disabled={busy !== null}>
            <Checkbox ref={confirmationInput} id="confirm-device" checked={confirmed} onCheckedChange={value => setConfirmed(value === true)} disabled={busy !== null} />
            <FieldContent>
              <FieldLabel htmlFor="confirm-device">The code matches the one shown on my computer, and I started this connection.</FieldLabel>
            </FieldContent>
          </Field>
          <div className="flex flex-col-reverse gap-3 sm:flex-row">
            <Button variant="outline" onClick={() => void decide(false)} disabled={busy !== null}>
              {busy === "deny" ? <Spinner data-icon="inline-start" aria-hidden="true" /> : null}
              {busy === "deny" ? "Denying…" : "Deny"}
            </Button>
            <Button className="sm:flex-1" onClick={() => void decide(true)} disabled={!confirmed || busy !== null}>
              {busy === "approve" ? <Spinner data-icon="inline-start" aria-hidden="true" /> : null}
              {busy === "approve" ? "Authorizing…" : "Authorize app"}
            </Button>
          </div>
        </div>
      ) : null}
    </>
  );
}
