import Link from "next/link";
import { KeyRound, LaptopMinimal, LockKeyhole, ShieldCheck, type LucideIcon } from "lucide-react";
import { Brand } from "@/components/brand";
import { Separator } from "@/components/ui/separator";

const highlights: { icon: LucideIcon; title: string; description: string }[] = [
  { icon: LaptopMinimal, title: "Local use without an account", description: "The app works on your computer even without signing in." },
  { icon: ShieldCheck, title: "Confirmed pairing", description: "Computers only connect after an invite and your confirmation, with authenticated encryption." },
  { icon: KeyRound, title: "Per-computer credentials", description: "Each authorized computer gets its own revocable credential." },
  { icon: LockKeyhole, title: "Encrypted backups", description: "The server only stores the encrypted envelope. The passphrase stays with you." },
];

function AuthAside() {
  return (
    <aside aria-labelledby="aside-title" className="relative hidden flex-col justify-center gap-12 overflow-hidden border-l bg-muted/50 p-10 lg:flex xl:p-14">
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 bg-[radial-gradient(hsl(var(--border))_1px,transparent_1px)] [background-size:22px_22px] [mask-image:radial-gradient(ellipse_at_top_right,black_10%,transparent_65%)]" />
      <div className="relative flex max-w-md flex-col gap-4">
        <p className="text-sm font-medium text-muted-foreground">App for Windows and macOS</p>
        <h2 id="aside-title" className="font-display text-3xl font-semibold leading-tight tracking-tight text-balance xl:text-4xl">
          Connect your computers and stay in control of your setup.
        </h2>
        <p className="text-sm leading-relaxed text-muted-foreground text-pretty">
          This website is only for signing in and authorizing the app. Devices, connections and backups are managed in TrapLink for desktop.
        </p>
      </div>
      <ul className="relative grid max-w-xl gap-6 xl:grid-cols-2">
        {highlights.map(({ icon: Icon, title, description }) => (
          <li key={title} className="flex gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-md border bg-background text-foreground shadow-sm">
              <Icon className="size-4" aria-hidden="true" />
            </span>
            <div className="flex flex-col gap-1">
              <p className="text-sm font-medium">{title}</p>
              <p className="text-sm leading-relaxed text-muted-foreground">{description}</p>
            </div>
          </li>
        ))}
      </ul>
    </aside>
  );
}

export function AuthShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid min-h-svh lg:grid-cols-2">
      <div className="flex flex-col gap-8 p-6 md:p-10">
        <div className="flex justify-center md:justify-start">
          <Link href="/" aria-label="TrapLink: home" className="rounded-md">
            <Brand />
          </Link>
        </div>
        <main id="conteudo" className="flex flex-1 items-start justify-center md:items-center">
          <div className="flex w-full max-w-sm flex-col gap-6">{children}</div>
        </main>
        <footer className="flex flex-col items-center gap-3 text-center text-xs text-muted-foreground md:flex-row md:justify-between md:text-left">
          <p>TrapLink works locally without an account.</p>
          <nav aria-label="Footer links" className="flex items-center gap-3">
            <Link href="/privacy" className="rounded-sm underline-offset-4 transition-colors hover:text-foreground hover:underline">Privacy</Link>
            <Separator orientation="vertical" className="h-3" />
            <Link href="/device" className="rounded-sm underline-offset-4 transition-colors hover:text-foreground hover:underline">Authorize app</Link>
          </nav>
        </footer>
      </div>
      <AuthAside />
    </div>
  );
}
