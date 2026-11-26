import { lazy, Suspense, useCallback, useEffect, useState } from "react";
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
import { Connections } from "@/pages/Connections";

const Account = lazy(() => import("@/pages/Account").then(module => ({ default: module.Account }), failedPage));
const Backups = lazy(() => import("@/pages/Backups").then(module => ({ default: module.Backups }), failedPage));
const Devices = lazy(() => import("@/pages/Devices").then(module => ({ default: module.Devices }), failedPage));
const Preferences = lazy(() => import("@/pages/Preferences").then(module => ({ default: module.Preferences }), failedPage));
const Profiles = lazy(() => import("@/pages/Profiles").then(module => ({ default: module.Profiles }), failedPage));

function failedPage() {
  return { default: () => <EmptyState icon={Info} title="Couldn't open this page" description="Reload the app to open this page again.">
    <Button variant="outline" onClick={() => window.location.reload()}><RefreshCw data-icon="inline-start" />Reload app</Button>
  </EmptyState> };
}

function PageLoading({ label }: { label: string }) {
  return <div className="loading-layout" role="status" aria-busy="true" aria-label={label}><Skeleton className="h-8 w-40" /><Skeleton className="h-4 w-1/2" /><Skeleton className="mt-4 h-56 w-full" /></div>;
}

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
  // Once opened, keep the account login poll and unsaved preference fields mounted.
  const [openedPanels, setOpenedPanels] = useState(() => ({ account: page === "account", preferences: page === "preferences" }));
  const [help, setHelp] = useState(false);
  const [palette, setPalette] = useState(false);
  const [intent, setIntent] = useState<Intent | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(storedSidebar);
  const [previewTheme, setPreviewTheme] = useState<Theme>("system");
  const { sendingTo, receivingFrom, connectingTo, flowSession } = desktop.inputStatus;
  const controlPeer = receivingFrom ?? sendingTo ?? connectingTo;
  const controlName = desktop.snapshot?.peers.find(peer => peer.id === controlPeer)?.name ?? "the paired computer";
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

  const navigate = useCallback((next: Page) => {
    setPage(next);
    setIntent(null);
    if (next === "account" || next === "preferences") setOpenedPanels(opened => opened[next] ? opened : { ...opened, [next]: true });
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
    if (settings && settings.theme !== next) void desktop.run("theme", () => nativeCommand("update_settings", { settings: { ...settings, theme: next }, base: settings }));
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
            {desktop.native && desktop.capabilities.inputSharing && (desktop.snapshot?.settings.inputEnabled || desktop.snapshot?.settings.remapEnabled || controlPeer) ? <Button variant={controlPeer ? "destructive" : "outline"} size="sm" onClick={() => void desktop.run("emergency-stop", () => nativeCommand("emergency_stop"), "Input sharing and button remapping turned off.", { concurrent: true })}>Emergency stop</Button> : null}
          </div>
        </header>
        <main id="main-content" className="main-content" tabIndex={-1}>
          {!desktop.native ? <NativeNotice /> : null}
          {controlPeer ? <Alert role="status" className="receiving-input">
            <ShieldCheck aria-hidden="true" />
            <AlertTitle>{receivingFrom ? `${controlName} is controlling this computer` : sendingTo ? `Controlling ${controlName}` : `Switching to ${controlName}…`}</AlertTitle>
            <AlertDescription className="flex flex-wrap items-center justify-between gap-2">
              <span>Press {emergencyKeys.join(" + ")} for an emergency stop.</span>
              {flowSession && !connectingTo ? <Button size="sm" variant="outline" disabled={desktop.busy === "release-flow"} onClick={() => void desktop.returnControl(controlPeer, flowSession)}>Return control</Button> : null}
            </AlertDescription>
          </Alert> : null}
          {desktop.snapshot?.warnings?.length ? <Alert className="connection-error"><Info /><AlertTitle>Some features need attention</AlertTitle><AlertDescription><ul className="list-disc pl-4">{desktop.snapshot.warnings.map((warning, index) => <li key={index}>{messageFrom(warning)}</li>)}</ul></AlertDescription></Alert> : null}
          {desktop.error ? <Alert variant="destructive" className="connection-error"><Info /><AlertTitle>Couldn't refresh the app</AlertTitle><AlertDescription>{desktop.error}<Button size="sm" variant="outline" onClick={() => void desktop.refresh()}>Try again</Button></AlertDescription></Alert> : null}
          <div key={page} className="page-enter">
            <Suspense fallback={<PageLoading label={`Loading ${current?.label ?? "page"}`} />}>
              {desktop.loading ? <PageLoading label="Loading app" />
                : !ready ? <EmptyState icon={Info} title="Couldn't load your data" description="Try refreshing the app to access your connections, devices and settings."><Button variant="outline" onClick={() => void desktop.refresh()}><RefreshCw data-icon="inline-start" />Try again</Button></EmptyState>
                : page === "connections" ? <Connections desktop={desktop} intent={intent} onIntentDone={clearIntent} />
                : page === "devices" ? <Devices desktop={desktop} onProfiles={() => navigate("profiles")} />
                : page === "profiles" ? <Profiles desktop={desktop} intent={intent} onIntentDone={clearIntent} />
                : page === "backups" ? <Backups desktop={desktop} onAccount={() => navigate("account")} intent={intent} onIntentDone={clearIntent} />
                : null}
            </Suspense>
          </div>
          {openedPanels.account && <div hidden={page !== "account" || !ready}>
            <Suspense fallback={<PageLoading label="Loading account" />}><Account desktop={desktop} onPreferences={() => navigate("preferences")} /></Suspense>
          </div>}
          {openedPanels.preferences && <div hidden={page !== "preferences" || !ready}>
            <Suspense fallback={<PageLoading label="Loading preferences" />}><Preferences desktop={desktop} /></Suspense>
          </div>}
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
  const shortcuts = [
    ["Search or run", [mod, "K"]],
    ["Go to a page", [mod, "1–4"]],
    ["Preferences", [mod, ","]],
    ["Collapse sidebar", [mod, "B"]],
    ["Emergency stop", emergencyKeys],
  ] as const;
  return <Dialog open={open} onOpenChange={onOpenChange}>
    <DialogContent className="help-dialog">
      <DialogHeader>
        <div className="dialog-symbol"><BookOpen aria-hidden="true" /></div>
        <DialogTitle>Connect two computers</DialogTitle>
        <DialogDescription>Open the TrapLink desktop app on both computers on the same local network.</DialogDescription>
      </DialogHeader>
      <ol className="help-steps">
        <li><span>1</span><div><h3>Create an invite</h3><p>In Connections, choose Connect computer, then Create invite.</p></div></li>
        <li><span>2</span><div><h3>Pair the other computer</h3><p>Open Use an invite on the other computer and paste the full invite.</p></div></li>
        <li><span>3</span><div><h3>Choose the screen edge</h3><p>Under Manage, choose the side facing the other screen, then turn on Flow. Approve the request on the other computer.</p></div></li>
      </ol>
      <Alert role="note"><ShieldCheck aria-hidden="true" /><AlertTitle>Take control back</AlertTitle>
        <AlertDescription>Push back through the edge you entered, or choose Return control during a Flow session. Emergency stop turns off input sharing and button remapping.</AlertDescription>
      </Alert>
      <div className="help-shortcuts"><h3>Shortcuts</h3><dl>{shortcuts.map(([label, keys]) =>
        <div key={label}><dt>{label}</dt><dd><KbdGroup>{keys.map(key => <Kbd key={key}>{key}</Kbd>)}</KbdGroup></dd></div>
      )}</dl></div>
    </DialogContent>
  </Dialog>;
}
