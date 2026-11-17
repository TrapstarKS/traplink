import type { Metadata } from "next";
import Link from "next/link";
import { ArrowLeft, Archive, KeyRound, MousePointer2, TriangleAlert, type LucideIcon } from "lucide-react";
import { Brand } from "@/components/brand";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";

export const metadata: Metadata = { title: "Privacy", description: "What data the TrapLink website stores when you sign in and authorize the app." };

const sections: { id: string; icon: LucideIcon; title: string; body: string }[] = [
  {
    id: "privacy-account",
    icon: KeyRound,
    title: "Account and computers",
    body: "When you create an account, the service stores your name, email, hashed password and the information needed to keep you signed in. When you authorize the app, it records the name, operating system, identifier and activity of the linked computer.",
  },
  {
    id: "privacy-backups",
    icon: Archive,
    title: "Content and backups",
    body: "Local settings stay on your computer. Online features must be turned on in the app. The server only receives the encrypted backup sent by the app. The backup passphrase stays with you.",
  },
  {
    id: "privacy-control",
    icon: MousePointer2,
    title: "Control",
    body: "Keyboard and mouse control requires separate pairing and sharing permissions, managed in the app.",
  },
];

export default function Privacy() {
  return (
    <div className="flex min-h-svh flex-col">
      <header className="border-b">
        <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-4 px-6 py-4">
          <Link href="/" aria-label="TrapLink: home" className="rounded-md"><Brand /></Link>
          <Button asChild variant="ghost" size="sm">
            <Link href="/sign-in"><ArrowLeft data-icon="inline-start" aria-hidden="true" />Back to sign in</Link>
          </Button>
        </div>
      </header>
      <main id="conteudo" className="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-10 px-6 py-12 md:py-16">
        <div className="flex flex-col gap-3">
          <p className="text-sm font-medium text-muted-foreground">Privacy</p>
          <h1 className="font-display text-3xl font-semibold tracking-tight text-balance md:text-4xl">Privacy at TrapLink</h1>
          <p className="max-w-2xl leading-relaxed text-muted-foreground text-pretty">
            The website signs you in and authorizes the app. TrapLink works locally without an account.
          </p>
        </div>
        <Separator />
        <div className="flex flex-col gap-10">
          {sections.map(({ id, icon: Icon, title, body }) => (
            <section key={id} aria-labelledby={id} className="grid items-start gap-3 md:grid-cols-[14rem_1fr] md:gap-8">
              <h2 id={id} className="flex items-center gap-2 font-medium">
                <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                {title}
              </h2>
              <p className="text-sm leading-relaxed text-muted-foreground">{body}</p>
            </section>
          ))}
        </div>
        <Alert role="note">
          <TriangleAlert aria-hidden="true" />
          <AlertTitle>Early deployment</AlertTitle>
          <AlertDescription>
            This is a development version. Before opening the service to the public, the operator must publish their contact details, the account deletion process and the applicable retention periods.
          </AlertDescription>
        </Alert>
      </main>
      <footer className="border-t">
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-2 px-6 py-6 text-xs text-muted-foreground sm:flex-row sm:justify-between">
          <p>TrapLink works locally without an account.</p>
          <Link href="/sign-up" className="rounded-sm underline-offset-4 transition-colors hover:text-foreground hover:underline">Create account</Link>
        </div>
      </footer>
    </div>
  );
}
