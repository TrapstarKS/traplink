import { useId, useState, type ComponentProps, type ReactNode } from "react";
import type { LucideIcon } from "lucide-react";
import { ArrowUpRight, ChevronDown, Monitor, ShieldCheck } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { Platform } from "@/lib/bridge";
import { cn } from "@/lib/utils";

export function Brand({ compact = false }: { compact?: boolean }) {
  return <div className="brand"><span className="brand-mark"><svg viewBox="0 0 36 36" aria-hidden="true"><path d="M9 4H24L32 12V27H17L9 19V4Z"/><path d="M4 9H19L27 17V32H12L4 24V9Z"/></svg></span>{!compact ? <span className="brand-name">TrapLink</span> : null}</div>;
}

export function PageHeading({ title, description, children }: { title: string; description: string; children?: ReactNode }) {
  return <header className="page-heading"><div><h1>{title}</h1><p className="page-description">{description}</p></div>{children ? <div className="page-actions">{children}</div> : null}</header>;
}

export function SectionHeading({ title, description, children }: { title: string; description?: string; children?: ReactNode }) {
  return <div className="section-heading"><div><h2>{title}</h2>{description ? <p>{description}</p> : null}</div>{children}</div>;
}

export function EmptyState({ icon: Icon, title, description, children }: { icon: LucideIcon; title: string; description: string; children?: ReactNode }) {
  return <Empty><EmptyHeader><EmptyMedia variant="icon"><Icon /></EmptyMedia><EmptyTitle>{title}</EmptyTitle><EmptyDescription>{description}</EmptyDescription></EmptyHeader>{children ? <EmptyContent>{children}</EmptyContent> : null}</Empty>;
}

export function NativeNotice({ compact = false }: { compact?: boolean }) {
  return <Alert className="native-notice"><Monitor /><AlertTitle>{compact ? "Available in the desktop app" : "You're in the browser preview"}</AlertTitle><AlertDescription>Devices, connections and backups require TrapLink for Windows or macOS. No computer is connected in this preview.</AlertDescription></Alert>;
}

export function SecurityNote({ children }: { children: ReactNode }) {
  return <div className="security-note"><ShieldCheck aria-hidden="true" /><p>{children}</p></div>;
}

export function BusyIcon({ busy }: { busy: boolean }) { return busy ? <Spinner data-icon="inline-start" /> : null; }

export function StatusBadge({ active, children }: { active: boolean; children: ReactNode }) {
  return <Badge variant="outline" data-active={active} className="status-badge"><span aria-hidden="true" />{children}</Badge>;
}

export function IconButton({ label, hint, children, className, ...props }: Omit<ComponentProps<typeof Button>, "aria-label"> & { label: string; hint?: string }) {
  return <Tooltip>
    <TooltipTrigger asChild><span className={cn("inline-flex", className)} tabIndex={props.disabled ? 0 : undefined}><Button variant="ghost" size="icon" aria-label={label} {...props}>{children}</Button></span></TooltipTrigger>
    <TooltipContent>{hint ?? label}</TooltipContent>
  </Tooltip>;
}

export function ConfirmDialog({ open, onOpenChange, title, description, action, destructive = false, busy = false, onConfirm }: { open: boolean; onOpenChange: (open: boolean) => void; title: string; description: string; action: string; destructive?: boolean; busy?: boolean; onConfirm: () => void }) {
  return <AlertDialog open={open} onOpenChange={onOpenChange}><AlertDialogContent><AlertDialogHeader><AlertDialogTitle>{title}</AlertDialogTitle><AlertDialogDescription>{description}</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel>Cancel</AlertDialogCancel><AlertDialogAction className={cn(destructive && buttonVariants({ variant: "destructive" }))} disabled={busy} onClick={event => { event.preventDefault(); onConfirm(); }}><BusyIcon busy={busy} />{action}</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>;
}

export function PlatformName({ platform }: { platform?: Platform }) {
  return <>{platform === "windows" ? "Windows" : platform === "macos" ? "macOS" : platform === "linux" ? "Linux" : "This computer"}</>;
}

const dateFormat = new Intl.DateTimeFormat("en-US", { day: "2-digit", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
const relativeFormat = new Intl.RelativeTimeFormat("en-US", { numeric: "auto" });

export function formattedDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : dateFormat.format(date);
}

export function relativeDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return value;
  const seconds = Math.round((date.valueOf() - Date.now()) / 1000);
  const abs = Math.abs(seconds);
  if (abs < 45) return "just now";
  if (abs < 3600) return relativeFormat.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return relativeFormat.format(Math.round(seconds / 3600), "hour");
  if (abs < 86400 * 7) return relativeFormat.format(Math.round(seconds / 86400), "day");
  return dateFormat.format(date);
}

export function formatBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

export function TextLink({ children, onClick }: { children: ReactNode; onClick: () => void }) {
  return <Button variant="link" onClick={onClick}>{children}<ArrowUpRight data-icon="inline-end" /></Button>;
}

export function Disclosure({ label, icon: Icon, children, defaultOpen = false }: { label: string; icon?: LucideIcon; children: ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const id = useId();
  return <div className="disclosure">
    <Button type="button" variant="ghost" size="sm" className="disclosure-trigger" aria-expanded={open} aria-controls={id} onClick={() => setOpen(value => !value)}>{Icon ? <Icon data-icon="inline-start" /> : null}{label}<ChevronDown data-icon="inline-end" data-open={open} className="disclosure-chevron" /></Button>
    {open ? <div id={id} className="disclosure-content">{children}</div> : null}
  </div>;
}
