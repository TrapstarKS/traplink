import { useCallback, useEffect, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { BookOpen, CircleHelp, CircleUserRound, FileSliders, FolderArchive, Info, Laptop, Link2, Moon, Plus, RefreshCw, Search, Settings2, ShieldCheck, Sun, Usb } from "lucide-react";
import { Toaster, toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Breadcrumb, BreadcrumbItem, BreadcrumbList, BreadcrumbPage, BreadcrumbSeparator } from "@/components/ui/breadcrumb";
import { Button } from "@/components/ui/button";
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator, CommandShortcut } from "@/components/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { Separator } from "@/components/ui/separator";
import { Sidebar, SidebarContent, SidebarFooter, SidebarGroup, SidebarGroupContent, SidebarGroupLabel, SidebarHeader, SidebarInset, SidebarMenu, SidebarMenuButton, SidebarMenuItem, SidebarProvider, SidebarRail, SidebarTrigger, useSidebar } from "@/components/ui/sidebar";
import { Skeleton } from "@/components/ui/skeleton";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Brand, EmptyState, NativeNotice, StatusBadge } from "@/components/common";
import { useDesktop } from "@/lib/use-desktop";
import { InputConsentPrompt } from "@/components/InputConsentPrompt";
import { messageFrom, nativeCommand, type Settings } from "@/lib/bridge";
import { checkOnStartup, requestUpdatesFocus } from "@/lib/updater";
import { Account } from "@/pages/Account";
import { Backups } from "@/pages/Backups";
import { Connections } from "@/pages/Connections";
import { Devices } from "@/pages/Devices";
import { Preferences } from "@/pages/Preferences";
import { Profiles } from "@/pages/Profiles";

export type Page = "connections" | "devices" | "profiles" | "backups" | "preferences" | "account";
export type Intent = { kind: "pair" | "new-profile" | "new-backup"; nonce: number };
type Theme = Settings["theme"];

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent);
const mod = isMac ? "⌘" : "Ctrl";
const emergencyKeys = isMac ? ["Control", "Option", "Esc"] : ["Ctrl", "Alt", "Esc"];
const pages: Array<{ id: Page; label: string; icon: LucideIcon; shortcut?: string; keywords: string[] }> = [
  { id: "connections", label: "Connections", icon: Link2, shortcut: "1", keywords: ["connection", "computer", "pairing", "network"] },
  { id: "devices", label: "Devices", icon: Usb, shortcut: "2", keywords: ["device", "mouse", "keyboard", "hardware"] },
  { id: "profiles", label: "Profiles", icon: FileSliders, shortcut: "3", keywords: ["profile", "mapping", "buttons"] },
  { id: "backups", label: "Backups", icon: FolderArchive, shortcut: "4", keywords: ["backup", "copy", "restore"] },
  { id: "preferences", label: "Preferences", icon: Settings2, shortcut: ",", keywords: ["settings", "options", "configuration"] },
  { id: "account", label: "My account", icon: CircleUserRound, keywords: ["account", "login", "sign in"] },
];
const mainPages = pages.slice(0, 4);
const themes: Array<{ id: Theme; label: string; icon: LucideIcon }> = [
  { id: "light", label: "Light", icon: Sun },
  { id: "dark", label: "Dark", icon: Moon },
  { id: "system", label: "System", icon: Laptop },
];

function storedPage(): Page {
  try {
    const value = localStorage.getItem("traplink.page");
    return pages.some(page => page.id === value) ? value as Page : "connections";
  } catch { return "connections"; }
}
function storedSidebar() {
  try { return localStorage.getItem("traplink.sidebar") !== "collapsed"; } catch { return true; }
}

export default function App() {
  const desktop = useDesktop();
  const [page, setPage] = useState<Page>(storedPage);
  const [help, setHelp] = useState(false);
  const [palette, setPalette] = useState(false);
  const [intent, setIntent] = useState<Intent | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(storedSidebar);
  const [previewTheme, setPreviewTheme] = useState<Theme>("system");
  const seenInput = useRef<string | null>(null);
  const theme = desktop.native ? desktop.snapshot?.settings.theme ?? "dark" : previewTheme;
  const current = pages.find(item => item.id === page);

  useEffect(() => {
    const system = window.matchMedia("(prefers-color-scheme: dark)");
    const updateTheme = () => {
      const dark = theme === "dark" || theme === "system" && system.matches;
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.style.colorScheme = dark ? "dark" : "light";
    };
    updateTheme(); system.addEventListener("change", updateTheme);
    return () => system.removeEventListener("change", updateTheme);
  }, [theme]);

  useEffect(() => {
    const incoming = desktop.inputStatus.receivingFrom;
    if (incoming && incoming !== seenInput.current) document.getElementById("main-content")?.focus({ preventScroll: true });
    seenInput.current = incoming;
  }, [desktop.inputStatus.receivingFrom]);

  const navigate = useCallback((next: Page) => {
    setPage(next);
    try { localStorage.setItem("traplink.page", next); } catch {  }
    document.getElementById("main-content")?.scrollTo({ top: 0 });
    window.scrollTo({ top: 0, behavior: "instant" });
  }, []);
  function changeSidebar(open: boolean) {
    setSidebarOpen(open);
    try { localStorage.setItem("traplink.sidebar", open ? "expanded" : "collapsed"); } catch {  }
  }
  function request(kind: Intent["kind"], target: Page) { navigate(target); setIntent({ kind, nonce: Date.now() }); }
  const clearIntent = useCallback(() => setIntent(null), []);
  function changeTheme(next: Theme) {
    if (!desktop.native) { setPreviewTheme(next); return; }
    const settings = desktop.snapshot?.settings;
    if (settings && settings.theme !== next) void desktop.run("theme", () => nativeCommand("update_settings", { settings: { ...settings, theme: next } }));
  }

  useEffect(() => { document.documentElement.dataset.hydrated = "1"; }, []);
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void checkOnStartup().then(update => {
        if (update) toast("Update available", { description: `TrapLink ${update.version} is ready to download.`, duration: 15000, action: { label: "Open Preferences", onClick: () => { requestUpdatesFocus(); navigate("preferences"); } } });
      });
    }, 3000);
    return () => window.clearTimeout(timer);
  }, [navigate]);
  useEffect(() => { document.getElementById("main-content")?.focus({ preventScroll: true }); }, [page]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if ((document.activeElement as HTMLElement | null)?.getAttribute("role") === "application") return;
      const modifier = isMac ? event.metaKey : event.ctrlKey;
      if (event.key === "F1") { event.preventDefault(); setHelp(true); return; }
      if (!modifier || event.altKey || event.shiftKey) return;
      const key = event.key.toLowerCase();
      if (key === "k") { event.preventDefault(); setPalette(open => !open); return; }
      const target = pages.find(item => item.shortcut === key);
      if (target) { event.preventDefault(); setPalette(false); navigate(target.id); }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate]);

  const ready = !desktop.loading && !(desktop.native && !desktop.snapshot);

  return <TooltipProvider delayDuration={300}>
    <SidebarProvider open={sidebarOpen} onOpenChange={changeSidebar}>
      <a href="#main-content" className="skip-link">Skip to content</a>
      <AppSidebar desktop={desktop} page={page} onNavigate={navigate} onHelp={() => setHelp(true)} />
      <SidebarInset className="min-w-0">
        <header className="app-header">
          <div className="flex min-w-0 items-center gap-2">
            <SidebarTrigger className="-ml-1" />
            <Separator orientation="vertical" className="mr-1 h-4" />
            <Breadcrumb><BreadcrumbList>
              <BreadcrumbItem className="hidden sm:inline-flex">TrapLink</BreadcrumbItem>
              <BreadcrumbSeparator className="hidden sm:inline-flex" />
              <BreadcrumbItem><BreadcrumbPage>{current?.label}</BreadcrumbPage></BreadcrumbItem>
            </BreadcrumbList></Breadcrumb>
          </div>
          <div className="flex min-w-0 items-center justify-end gap-2">
            <Button variant="outline" size="sm" className="search-trigger" onClick={() => setPalette(true)} aria-keyshortcuts={isMac ? "Meta+K" : "Control+K"}>
              <Search data-icon="inline-start" /><span className="hidden md:inline">Search or run…</span><span className="md:hidden">Search</span>
              <KbdGroup className="hidden md:inline-flex"><Kbd>{mod}</Kbd><Kbd>K</Kbd></KbdGroup>
            </Button>
            <span className="hidden lg:inline-flex"><StatusBadge active={desktop.native && !!desktop.snapshot?.listening}>{desktop.native ? desktop.snapshot ? desktop.snapshot.listening ? "Available on network" : "Network unavailable" : "Loading…" : "Browser preview"}</StatusBadge></span>
            {desktop.native && desktop.capabilities.inputSharing ? <Button variant={desktop.inputStatus.receivingFrom ? "destructive" : "outline"} size="sm" onClick={() => void desktop.run("emergency-stop", () => nativeCommand("emergency_stop"), "Input control stopped.", { concurrent: true })}>Stop input</Button> : null}
          </div>
        </header>
        <main id="main-content" className="main-content" tabIndex={-1}>
          {!desktop.native ? <NativeNotice /> : null}
          {desktop.inputStatus.receivingFrom ? <Alert className="receiving-input"><ShieldCheck /><AlertTitle>Receiving input control</AlertTitle><AlertDescription>Use “Stop input” or {emergencyKeys.join(" + ")} to stop receiving.</AlertDescription></Alert> : null}
          {desktop.snapshot?.warnings?.length ? <Alert className="connection-error"><Info /><AlertTitle>Some features need attention</AlertTitle><AlertDescription><ul className="list-disc pl-4">{desktop.snapshot.warnings.map((warning, index) => <li key={index}>{messageFrom(warning)}</li>)}</ul></AlertDescription></Alert> : null}
          {desktop.error ? <Alert variant="destructive" className="connection-error"><Info /><AlertTitle>Couldn't refresh the app</AlertTitle><AlertDescription>{desktop.error}<Button size="sm" variant="outline" onClick={() => void desktop.refresh()}>Try again</Button></AlertDescription></Alert> : null}
          <div key={page} className="page-enter">
            {desktop.loading ? <div className="loading-layout" aria-busy="true" aria-label="Loading app"><Skeleton className="h-8 w-40" /><Skeleton className="h-4 w-1/2" /><Skeleton className="mt-4 h-56 w-full" /><Skeleton className="h-40 w-full" /></div>
              : !ready ? <EmptyState icon={Info} title="Couldn't load your data" description="Try refreshing the app to access your connections, devices and settings."><Button variant="outline" onClick={() => void desktop.refresh()}><RefreshCw data-icon="inline-start" />Try again</Button></EmptyState>
              : page === "connections" ? <Connections desktop={desktop} intent={intent} onIntentDone={clearIntent} />
              : page === "devices" ? <Devices desktop={desktop} onProfiles={() => navigate("profiles")} />
              : page === "profiles" ? <Profiles desktop={desktop} intent={intent} onIntentDone={clearIntent} />
              : page === "backups" ? <Backups desktop={desktop} onAccount={() => navigate("account")} intent={intent} onIntentDone={clearIntent} />
              : null}
          </div>
          {}
          <div hidden={page !== "account" || !ready}><Account desktop={desktop} onPreferences={() => navigate("preferences")} /></div>
          <div hidden={page !== "preferences" || !ready}><Preferences desktop={desktop} /></div>
        </main>
      </SidebarInset>
    </SidebarProvider>
    <CommandMenu open={palette} onOpenChange={setPalette} desktop={desktop} theme={theme} onNavigate={navigate} onRequest={request} onTheme={changeTheme} onHelp={() => setHelp(true)} />
    <HelpDialog open={help} onOpenChange={setHelp} />
    {desktop.native ? <InputConsentPrompt onAnswered={() => void desktop.refresh(true)} /> : null}
    <Toaster position="bottom-right" theme={theme} richColors closeButton />
  </TooltipProvider>;
}

function AppSidebar({ desktop, page, onNavigate, onHelp }: { desktop: ReturnType<typeof useDesktop>; page: Page; onNavigate: (page: Page) => void; onHelp: () => void }) {
  const { isMobile, setOpenMobile } = useSidebar();
  const account = desktop.snapshot?.account;
  const initial = account ? (account.name || account.email).slice(0, 1).toLocaleUpperCase("en-US") : null;
  const peers = desktop.snapshot?.peers ?? [];
  const connected = peers.filter(peer => peer.connected).length;
  function go(next: Page) { onNavigate(next); if (isMobile) setOpenMobile(false); }
  const shortcut = (key?: string) => key ? `${mod}+${key}` : undefined;

  return <Sidebar collapsible="icon" variant="sidebar">
    <SidebarHeader>
      <SidebarMenu><SidebarMenuItem>
        <SidebarMenuButton size="lg" className="pointer-events-none" tabIndex={-1} asChild><div><Brand /></div></SidebarMenuButton>
      </SidebarMenuItem></SidebarMenu>
    </SidebarHeader>
    <SidebarContent>
      <SidebarGroup>
        <SidebarGroupLabel>General</SidebarGroupLabel>
        <SidebarGroupContent>
          <SidebarMenu>
            {mainPages.map(item => <SidebarMenuItem key={item.id}>
              <SidebarMenuButton isActive={page === item.id} aria-current={page === item.id ? "page" : undefined} tooltip={`${item.label} · ${shortcut(item.shortcut)}`} onClick={() => go(item.id)}>
                <item.icon /><span>{item.label}</span>
                {item.id === "connections" && connected ? <span className="sidebar-count" aria-label={`${connected} connected`}>{connected}</span> : null}
              </SidebarMenuButton>
            </SidebarMenuItem>)}
          </SidebarMenu>
        </SidebarGroupContent>
      </SidebarGroup>
      <SidebarGroup className="mt-auto">
        <SidebarGroupContent>
          <SidebarMenu>
            <SidebarMenuItem><SidebarMenuButton isActive={page === "preferences"} aria-current={page === "preferences" ? "page" : undefined} tooltip={`Preferences · ${mod}+,`} onClick={() => go("preferences")}><Settings2 /><span>Preferences</span></SidebarMenuButton></SidebarMenuItem>
            <SidebarMenuItem><SidebarMenuButton tooltip="Help · F1" onClick={() => { onHelp(); if (isMobile) setOpenMobile(false); }}><CircleHelp /><span>Help</span></SidebarMenuButton></SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroupContent>
      </SidebarGroup>
    </SidebarContent>
    <SidebarFooter>
      <SidebarMenu><SidebarMenuItem>
        <SidebarMenuButton size="lg" isActive={page === "account"} aria-current={page === "account" ? "page" : undefined} tooltip="My account" onClick={() => go("account")} aria-label="Open my account">
          <Avatar className="size-8 rounded-lg"><AvatarFallback className="rounded-lg">{initial ?? <CircleUserRound className="size-4" />}</AvatarFallback></Avatar>
          <span className="grid min-w-0 flex-1 text-left leading-tight"><span className="truncate text-sm font-medium">{account?.name || "My account"}</span><span className="truncate text-xs text-muted-foreground">{account?.email || "Connect account"}</span></span>
        </SidebarMenuButton>
      </SidebarMenuItem></SidebarMenu>
    </SidebarFooter>
    <SidebarRail />
  </Sidebar>;
}

function CommandMenu({ open, onOpenChange, desktop, theme, onNavigate, onRequest, onTheme, onHelp }: { open: boolean; onOpenChange: (open: boolean) => void; desktop: ReturnType<typeof useDesktop>; theme: Theme; onNavigate: (page: Page) => void; onRequest: (kind: Intent["kind"], page: Page) => void; onTheme: (theme: Theme) => void; onHelp: () => void }) {
  const canAct = desktop.native && !!desktop.snapshot && !desktop.busy;
  function run(action: () => void) { onOpenChange(false); action(); }
  return <CommandDialog open={open} onOpenChange={onOpenChange}>
    <CommandInput placeholder="Search pages or actions…" />
    <CommandList>
      <CommandEmpty>No results.</CommandEmpty>
      <CommandGroup heading="Go to">
        {pages.map(item => <CommandItem key={item.id} value={`go ${item.label}`} keywords={item.keywords} onSelect={() => run(() => onNavigate(item.id))}><item.icon />{item.label}{item.shortcut ? <CommandShortcut>{mod}+{item.shortcut}</CommandShortcut> : null}</CommandItem>)}
      </CommandGroup>
      <CommandSeparator />
      <CommandGroup heading="Actions">
        <CommandItem value="connect computer pair invitation" disabled={!canAct || !desktop.capabilities.lanPairing} onSelect={() => run(() => onRequest("pair", "connections"))}><Plus />Connect computer</CommandItem>
        <CommandItem value="create profile" disabled={!canAct} onSelect={() => run(() => onRequest("new-profile", "profiles"))}><Plus />Create profile</CommandItem>
        <CommandItem value="create backup" disabled={!canAct} onSelect={() => run(() => onRequest("new-backup", "backups"))}><Plus />Create backup</CommandItem>
        <CommandItem value="refresh data reload" disabled={!desktop.native} onSelect={() => run(() => void desktop.refresh())}><RefreshCw />Refresh data</CommandItem>
        <CommandItem value="help how to connect" onSelect={() => run(onHelp)}><CircleHelp />How to connect two computers<CommandShortcut>F1</CommandShortcut></CommandItem>
      </CommandGroup>
      <CommandSeparator />
      <CommandGroup heading="Appearance">
        {themes.map(item => <CommandItem key={item.id} value={`theme ${item.label}`} disabled={desktop.native && !canAct} onSelect={() => run(() => onTheme(item.id))}><item.icon />{item.label} theme{theme === item.id ? <CommandShortcut>Current</CommandShortcut> : null}</CommandItem>)}
      </CommandGroup>
    </CommandList>
  </CommandDialog>;
}

function HelpDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (value: boolean) => void }) {
  return <Dialog open={open} onOpenChange={onOpenChange}><DialogContent className="help-dialog"><DialogHeader><div className="dialog-symbol"><BookOpen /></div><DialogTitle>Connect two computers</DialogTitle><DialogDescription>Start with both computers on the same local network.</DialogDescription></DialogHeader><ol className="help-steps"><li><span>1</span><div><h3>Open TrapLink on both computers</h3><p>Use the desktop app for Windows or macOS. The browser preview can't access your hardware.</p></div></li><li><span>2</span><div><h3>Create an invitation in Connections</h3><p>On the other computer, choose “Use an invitation” and paste the full invitation.</p></div></li><li><span>3</span><div><h3>Choose what to share</h3><p>Turn on Files or Clipboard in Preferences and allow receiving on the paired computer.</p></div></li></ol><Alert><ShieldCheck /><AlertTitle>You decide when to share</AlertTitle><AlertDescription>Sending files and text is manual. Mouse and keyboard control requires permission and an explicit session, ended with Escape.</AlertDescription></Alert><div className="help-shortcuts"><h3>Shortcuts</h3><dl><div><dt>Search or run</dt><dd><KbdGroup><Kbd>{mod}</Kbd><Kbd>K</Kbd></KbdGroup></dd></div><div><dt>Go to a page</dt><dd><KbdGroup><Kbd>{mod}</Kbd><Kbd>1–4</Kbd></KbdGroup></dd></div><div><dt>Preferences</dt><dd><KbdGroup><Kbd>{mod}</Kbd><Kbd>,</Kbd></KbdGroup></dd></div><div><dt>Collapse sidebar</dt><dd><KbdGroup><Kbd>{mod}</Kbd><Kbd>B</Kbd></KbdGroup></dd></div><div><dt>Stop receiving</dt><dd><KbdGroup>{emergencyKeys.map(key => <Kbd key={key}>{key}</Kbd>)}</KbdGroup></dd></div></dl></div></DialogContent></Dialog>;
}
