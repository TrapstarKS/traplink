"use client";

import { useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import Link from "next/link";
import { ArrowRight, CircleCheck, Eye, EyeOff, TriangleAlert } from "lucide-react";
import { authClient } from "@/lib/auth-client";
import { AuthHeader } from "@/components/auth-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Checkbox } from "@/components/ui/checkbox";
import { Spinner } from "@/components/ui/spinner";

type Mode = "sign-in" | "sign-up";
type FieldName = "name" | "email" | "password" | "privacy";
type FieldErrors = Partial<Record<FieldName, string>>;
const titles: Record<Mode, string> = { "sign-in": "Sign in to your account", "sign-up": "Create an account" };
const descriptions: Record<Mode, string> = { "sign-in": "Use your email and password to continue to TrapLink.", "sign-up": "Fill in your details to set up your TrapLink account." };
const fieldOrder: FieldName[] = ["name", "email", "password", "privacy"];

export function safeReturnTo(value?: string) {
  if (!value || !value.startsWith("/") || value.startsWith("//") || value.includes("\\") || /[\u0000-\u001f]/.test(value)) return "/connected";
  try {
    const target = new URL(value, "https://traplink.invalid");
    return target.pathname === "/device" ? `${target.pathname}${target.search}` : "/connected";
  } catch {
    return "/connected";
  }
}

function validateName(value: string) {
  return value.length < 2 || value.length > 80 ? "Enter a name between 2 and 80 characters." : undefined;
}

function validateEmail(input: HTMLInputElement | null, value: string) {
  return !value || !input?.validity.valid ? "Enter a valid email, like you@example.com." : undefined;
}

function validatePassword(mode: Mode, value: string) {
  if (mode === "sign-in") return !value || value.length > 128 ? "Enter your password." : undefined;
  return value.length < 12 || value.length > 128 ? "Use a password between 12 and 128 characters." : undefined;
}

function describedBy(...ids: (string | false | undefined)[]) {
  return ids.filter(Boolean).join(" ") || undefined;
}

export function AuthForm({ mode, callback, initialError }: { mode: Mode; callback?: string; initialError?: string }) {
  const [busy, setBusy] = useState(false);
  const [visible, setVisible] = useState(false);
  const [capsLock, setCapsLock] = useState(false);
  const [error, setError] = useState(initialError ? "Your previous session expired. Sign in again." : "");
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});
  const [success, setSuccess] = useState(false);
  const [accepted, setAccepted] = useState(false);
  const inFlight = useRef(false);
  const errorNotice = useRef<HTMLDivElement>(null);
  const returnTo = safeReturnTo(callback);
  const suffix = callback ? `?callback=${encodeURIComponent(returnTo)}` : "";
  const continuingDevice = returnTo.startsWith("/device");

  function setFieldError(name: FieldName, message: string | undefined) {
    setFieldErrors(current => current[name] === message ? current : { ...current, [name]: message });
  }

  function clearField(name: FieldName) {
    setFieldError(name, undefined);
    setError("");
  }

  function trackCapsLock(event: KeyboardEvent<HTMLInputElement>) {
    if (typeof event.getModifierState === "function") setCapsLock(event.getModifierState("CapsLock"));
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (inFlight.current) return;
    const form = event.currentTarget;
    const values = new FormData(form);
    const name = String(values.get("name") ?? "").trim();
    const email = String(values.get("email") ?? "").trim();
    const password = String(values.get("password") ?? "");
    const invalid: FieldErrors = {};
    if (mode === "sign-up") invalid.name = validateName(name);
    invalid.email = validateEmail(form.elements.namedItem("email") as HTMLInputElement | null, email);
    invalid.password = validatePassword(mode, password);
    if (mode === "sign-up" && !accepted) invalid.privacy = "Confirm that you have read the privacy information.";
    setFieldErrors(invalid);
    setError("");
    const firstInvalid = fieldOrder.find(field => invalid[field]);
    if (firstInvalid) {
      requestAnimationFrame(() => document.getElementById(firstInvalid)?.focus());
      return;
    }

    inFlight.current = true;
    setBusy(true);
    try {
      const result = mode === "sign-in"
        ? await authClient.signIn.email({ email, password, rememberMe: true, callbackURL: returnTo })
        : await authClient.signUp.email({ email, password, name, callbackURL: returnTo });
      if (result.error) {
        if (result.error.status === 429) throw new Error("Too many attempts. Wait a few minutes and try again.");
        if (result.error.status >= 500 || result.error.status === 0) throw new Error("The service is unavailable. Try again shortly.");
        throw new Error(mode === "sign-in"
          ? "Check your email and password and try again."
          : "Check your details to create your account. If you already have an account, sign in instead.");
      }
      if (mode === "sign-in" || result.data?.token) {
        window.location.assign(returnTo);
        return;
      }
      setSuccess(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "The service is unavailable. Try again shortly.");
      requestAnimationFrame(() => errorNotice.current?.focus());
    }
    inFlight.current = false;
    setBusy(false);
  }

  if (success) {
    return (
      <>
        <AuthHeader title="Account created" description="Sign in with the email and password you just registered to continue to the app." />
        <Alert role="status">
          <CircleCheck aria-hidden="true" />
          <AlertDescription>Your account is ready.</AlertDescription>
        </Alert>
        <Button asChild>
          <Link href={`/sign-in${suffix}`}>
            Continue to sign in
            <ArrowRight data-icon="inline-end" aria-hidden="true" />
          </Link>
        </Button>
      </>
    );
  }

  return (
    <>
      <AuthHeader
        title={titles[mode]}
        description={continuingDevice ? "After signing in, you will return to authorize the app on your computer." : descriptions[mode]}
      />
      {error ? (
        <Alert ref={errorNotice} tabIndex={-1} variant="destructive" id="auth-error">
          <TriangleAlert aria-hidden="true" />
          <AlertTitle>{mode === "sign-in" ? "Could not sign in" : "Could not create the account"}</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}
      <form onSubmit={submit} noValidate aria-busy={busy} aria-describedby={error ? "auth-error" : undefined}>
        <FieldGroup>
          {mode === "sign-up" ? (
            <Field data-invalid={!!fieldErrors.name} data-disabled={busy}>
              <FieldLabel htmlFor="name">Name</FieldLabel>
              <Input
                id="name" name="name" placeholder="Your name" autoComplete="name" autoCapitalize="words" autoFocus
                minLength={2} maxLength={80} required disabled={busy}
                aria-invalid={!!fieldErrors.name} aria-describedby={describedBy(!!fieldErrors.name && "name-error")}
                onChange={() => clearField("name")}
                onBlur={event => { const value = event.currentTarget.value.trim(); if (value) setFieldError("name", validateName(value)); }}
              />
              {fieldErrors.name ? <FieldError id="name-error">{fieldErrors.name}</FieldError> : null}
            </Field>
          ) : null}
          <Field data-invalid={!!fieldErrors.email} data-disabled={busy}>
            <FieldLabel htmlFor="email">E-mail</FieldLabel>
            <Input
              id="email" name="email" type="email" inputMode="email" autoFocus={mode === "sign-in"}
              autoComplete={mode === "sign-in" ? "username" : "email"} autoCapitalize="none" autoCorrect="off" spellCheck={false}
              placeholder="you@example.com" required maxLength={254} disabled={busy}
              aria-invalid={!!fieldErrors.email} aria-describedby={describedBy(!!fieldErrors.email && "email-error")}
              onChange={() => clearField("email")}
              onBlur={event => { const value = event.currentTarget.value.trim(); if (value) setFieldError("email", validateEmail(event.currentTarget, value)); }}
            />
            {fieldErrors.email ? <FieldError id="email-error">{fieldErrors.email}</FieldError> : null}
          </Field>
          <Field data-invalid={!!fieldErrors.password} data-disabled={busy}>
            <FieldLabel htmlFor="password">Password</FieldLabel>
            <InputGroup data-disabled={busy}>
              <InputGroupInput
                id="password" name="password" type={visible ? "text" : "password"}
                autoComplete={mode === "sign-in" ? "current-password" : "new-password"}
                autoCapitalize="none" autoCorrect="off" spellCheck={false}
                placeholder={mode === "sign-in" ? "Your password" : "At least 12 characters"}
                minLength={mode === "sign-in" ? 1 : 12} maxLength={128} required disabled={busy}
                aria-invalid={!!fieldErrors.password}
                aria-describedby={describedBy(mode === "sign-up" && "password-help", capsLock && "password-caps", !!fieldErrors.password && "password-error")}
                onChange={() => clearField("password")}
                onKeyDown={trackCapsLock}
                onKeyUp={trackCapsLock}
                onBlur={event => { setCapsLock(false); if (mode === "sign-up" && event.currentTarget.value) setFieldError("password", validatePassword(mode, event.currentTarget.value)); }}
              />
              <InputGroupAddon align="inline-end">
                <InputGroupButton type="button" size="icon-xs" aria-label={visible ? "Hide password" : "Show password"} aria-controls="password" aria-pressed={visible} disabled={busy} onClick={() => setVisible(!visible)}>
                  {visible ? <EyeOff aria-hidden="true" /> : <Eye aria-hidden="true" />}
                </InputGroupButton>
              </InputGroupAddon>
            </InputGroup>
            <div aria-live="polite" className="contents">
              {capsLock ? (
                <FieldDescription id="password-caps" className="flex items-center gap-1.5">
                  <TriangleAlert className="size-3.5 shrink-0" aria-hidden="true" />
                  Caps Lock is on.
                </FieldDescription>
              ) : null}
            </div>
            {mode === "sign-up" ? <FieldDescription id="password-help">Use 12 to 128 characters. TrapLink does not offer password recovery, so keep it somewhere safe.</FieldDescription> : null}
            {fieldErrors.password ? <FieldError id="password-error">{fieldErrors.password}</FieldError> : null}
          </Field>
          {mode === "sign-up" ? (
            <Field orientation="horizontal" data-invalid={!!fieldErrors.privacy} data-disabled={busy}>
              <Checkbox
                id="privacy" checked={accepted} disabled={busy}
                onCheckedChange={value => { setAccepted(value === true); clearField("privacy"); }}
                aria-invalid={!!fieldErrors.privacy} aria-describedby={describedBy(!!fieldErrors.privacy && "privacy-error")}
              />
              <FieldContent>
                <FieldLabel htmlFor="privacy">
                  <span>I have read the <Link href="/privacy" className="inline-link" target="_blank" rel="noopener noreferrer">privacy information<span className="sr-only"> (opens in a new tab)</span></Link>.</span>
                </FieldLabel>
                {fieldErrors.privacy ? <FieldError id="privacy-error">{fieldErrors.privacy}</FieldError> : null}
              </FieldContent>
            </Field>
          ) : null}
          <Field>
            <Button type="submit" className="w-full" disabled={busy}>
              {busy ? <Spinner data-icon="inline-start" aria-hidden="true" /> : null}
              {busy ? mode === "sign-in" ? "Signing in…" : "Creating account…" : mode === "sign-in" ? "Sign in" : "Create account"}
            </Button>
            <FieldDescription className="text-center">
              {mode === "sign-in" ? "Don't have an account?" : "Already have an account?"}{" "}
              <Link className="inline-link" href={`${mode === "sign-in" ? "/sign-up" : "/sign-in"}${suffix}`}>{mode === "sign-in" ? "Create account" : "Sign in"}</Link>
            </FieldDescription>
          </Field>
        </FieldGroup>
      </form>
    </>
  );
}
