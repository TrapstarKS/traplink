import { useEffect, useRef, useState } from "react";
import { ArrowRight, BatteryCharging, Info, Keyboard, Mouse, RefreshCw, ScanLine, Search, Usb } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Switch } from "@/components/ui/switch";
import { BusyIcon, EmptyState, PageHeading, StatusBadge } from "@/components/common";
import { getBatteryStatus, messageFrom, nativeCommand, type BatteryLevel, type BatteryStatus, type Device, type KeyboardSettings } from "@/lib/bridge";
import type { Desktop } from "@/lib/use-desktop";

function DeviceIcon({ device }: { device: Device }) {
  return inputKinds(device).includes("mouse") ? <Mouse /> : <Keyboard />;
}

function inputKinds(device: Device): Array<"keyboard" | "mouse"> {
  const reported = (device as Device & { inputKinds?: string[] }).inputKinds ?? device.capabilities;
  return reported.filter((kind): kind is "keyboard" | "mouse" => kind === "keyboard" || kind === "mouse");
}

function inputKindLabel(device: Device): string {
  const kinds = inputKinds(device);
  return kinds.includes("keyboard") && kinds.includes("mouse") ? "Keyboard and mouse" : kinds.includes("mouse") ? "Mouse" : "Keyboard";
}

function displayedName(device: Device): string {
  if (device.name && device.name !== "Connected input device" && device.name !== "HID device") return device.name;
  const kinds = inputKinds(device);
  return kinds.includes("keyboard") && kinds.includes("mouse") ? "HID input device" : kinds.includes("mouse") ? "Mouse HID" : "HID keyboard";
}

export function Devices({ desktop, onProfiles }: { desktop: Desktop; onProfiles: () => void }) {
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Device | null>(null);
  const devices = (desktop.snapshot?.devices ?? []).filter(device => inputKinds(device).length > 0);
  const filtered = devices.filter(device => `${displayedName(device)} ${inputKindLabel(device)} ${device.manufacturer ?? ""}`.toLocaleLowerCase("en-US").includes(search.toLocaleLowerCase("en-US")));
  return <>
    <PageHeading title="Devices" description="Keyboards and mice detected on this computer."><Button variant="outline" disabled={!desktop.native || desktop.loading || !!desktop.busy} onClick={() => void desktop.refresh()}><RefreshCw data-icon="inline-start" />Refresh devices</Button></PageHeading>
    <div className="content-stack">
      <div className="list-toolbar"><p>{devices.length} device{devices.length === 1 ? "" : "s"}, with interfaces of the same peripheral grouped</p><Field className="search-field"><FieldLabel htmlFor="device-search"><Search aria-hidden="true" />Search devices</FieldLabel><Input id="device-search" placeholder="Name or manufacturer" value={search} onChange={event => setSearch(event.target.value)} /></Field></div>
      {filtered.length ? <div className="hardware-grid">{filtered.map(device => <Card className="hardware-card" key={device.id}>
        <CardHeader className="hardware-card-header"><div className="hardware-card-meta"><span className="item-icon"><DeviceIcon device={device} /></span><Badge variant="outline">{device.transport || "HID"}</Badge></div><CardTitle>{displayedName(device)}</CardTitle><CardDescription>{inputKindLabel(device)}{device.manufacturer ? `, ${device.manufacturer}` : ""}</CardDescription></CardHeader>
        <CardFooter className="hardware-card-footer"><span className="hardware-id">{device.isLogitech ? "Logitech" : `${device.vendorId.toString(16).padStart(4, "0")}:${device.productId.toString(16).padStart(4, "0")}`}</span><Button variant="outline" size="sm" aria-label={`View details for ${displayedName(device)}`} onClick={() => setSelected(device)}>Details<ArrowRight data-icon="inline-end" /></Button></CardFooter>
      </Card>)}</div> : <Card><CardContent className="pt-6"><EmptyState icon={desktop.native ? Usb : ScanLine} title={search ? "No devices found" : desktop.native ? "No keyboard or mouse detected" : "Devices are available in the desktop app"} description={search ? "Try another name or manufacturer." : desktop.native ? "Connect a keyboard or mouse over USB or Bluetooth and refresh the list. A receiver may show the generic name reported by the system." : "The browser cannot access system mice and keyboards. Native features require the desktop app."} /></CardContent></Card>}
      <Card><CardHeader><div className="card-heading-row"><div><CardTitle>Device settings</CardTitle><CardDescription>Features available for the detected hardware.</CardDescription></div><StatusBadge active={desktop.native && desktop.capabilities.hidInventory}>{desktop.native && desktop.capabilities.hidInventory ? "System read" : "Desktop app required"}</StatusBadge></div></CardHeader><CardContent><p className="hardware-summary">On supported Logitech keyboards, open the details to check and change Fn behavior. Applying profiles, DPI, lighting and other buttons is not available yet.</p></CardContent><CardFooter><Button variant="outline" onClick={onProfiles}>View profiles<ArrowRight data-icon="inline-end" /></Button></CardFooter></Card>
    </div>
    {selected ? <DeviceDialog key={selected.id} device={selected} desktop={desktop} onClose={() => setSelected(null)} onProfiles={() => { setSelected(null); onProfiles(); }} /> : null}
  </>;
}

function DeviceDialog({ device, desktop, onClose, onProfiles }: { device: Device; desktop: Desktop; onClose: () => void; onProfiles: () => void }) {
  const [keyboard, setKeyboard] = useState<KeyboardSettings | null>(null);
  const [busy, setBusy] = useState<"read" | "write" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const requestActive = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  async function readKeyboard() {
    if (requestActive.current) return;
    requestActive.current = true;
    setBusy("read"); setError(null);
    try {
      const settings = await nativeCommand<KeyboardSettings>("get_keyboard_settings", { deviceId: device.id });
      if (alive.current) setKeyboard(settings);
    } catch (cause) { if (alive.current) setError(messageFrom(cause)); }
    finally { requestActive.current = false; if (alive.current) setBusy(null); }
  }

  async function setFnInversion(enabled: boolean) {
    if (requestActive.current) return;
    requestActive.current = true;
    setBusy("write"); setError(null);
    try {
      const settings = await nativeCommand<KeyboardSettings>("set_fn_inversion", { deviceId: device.id, enabled });
      if (alive.current) {
        setKeyboard(settings);
        if (settings.supported && settings.fnInversion === enabled) toast.success("Fn key behavior updated.");
        else setError(settings.reason || "The device did not confirm the new setting. Check again before retrying.");
      }
    } catch (cause) {
      if (alive.current) {
        setKeyboard(null);
        setError(messageFrom(cause));
      }
    } finally { requestActive.current = false; if (alive.current) setBusy(null); }
  }

  return <Dialog open onOpenChange={value => { if (!value) onClose(); }}><DialogContent>
    <DialogHeader><div className="dialog-symbol"><DeviceIcon device={device} /></div><DialogTitle>{displayedName(device)}</DialogTitle><DialogDescription>Information provided by the operating system.</DialogDescription></DialogHeader>
    <dl className="details-list"><div><dt>Manufacturer</dt><dd>{device.manufacturer || "Not reported"}</dd></div><div><dt>Connection</dt><dd>{device.transport || "HID"}</dd></div><div><dt>Vendor ID</dt><dd className="mono">0x{device.vendorId.toString(16).padStart(4, "0")}</dd></div><div><dt>Product ID</dt><dd className="mono">0x{device.productId.toString(16).padStart(4, "0")}</dd></div>{device.serialNumber ? <div><dt>Serial number</dt><dd className="mono">{device.serialNumber}</dd></div> : null}<div><dt>Identification</dt><dd>{device.isLogitech ? "Logitech" : "HID"}</dd></div><div><dt>Device ID</dt><dd className="mono">{device.id}</dd></div></dl>
    {device.capabilities.length ? <div className="capabilities-list">{device.capabilities.filter(capability => capability !== "keyboard" && capability !== "mouse").map(capability => <Badge variant="secondary" key={capability}>{capability === "hidpp-settings-candidate" ? "Settings candidate" : capability === "read-only-inventory" ? "System read" : capability.replace("hid-usage-", "HID usage ")}</Badge>)}</div> : null}
    {device.isLogitech && inputKinds(device).includes("keyboard") ? <section className="flex flex-col gap-4" aria-labelledby="keyboard-config-heading">
      <div className="flex items-center justify-between gap-3"><h3 id="keyboard-config-heading" className="text-sm font-medium">Function keys</h3><Badge variant="outline">Logitech</Badge></div>
      {!keyboard ? <><p className="text-xs leading-relaxed text-muted-foreground">Check the device to see whether it can invert Fn behavior. Checking does not change your settings.</p><Button variant="outline" disabled={!desktop.native || !!busy} onClick={() => void readKeyboard()}><BusyIcon busy={busy === "read"} />{busy === "read" ? "Checking device…" : "Check settings"}</Button></> : keyboard.supported && typeof keyboard.fnInversion === "boolean" ? <>
        <FieldGroup><Field orientation="horizontal" data-disabled={!!busy}><FieldContent><FieldLabel htmlFor="fn-inversion">Invert Fn keys</FieldLabel><FieldDescription>On: media actions by default, hold Fn for F1 to F12. Off: F1 to F12 by default. The change is sent to this keyboard.</FieldDescription></FieldContent><Switch id="fn-inversion" checked={keyboard.fnInversion} disabled={!!busy} onCheckedChange={enabled => void setFnInversion(enabled)} /></Field></FieldGroup>
        <div className="flex items-center justify-between gap-3"><span className="text-xs text-muted-foreground">{busy === "write" ? "Confirming on device…" : "State read from device"}</span><Button variant="ghost" size="sm" disabled={!!busy} onClick={() => void readKeyboard()}><RefreshCw data-icon="inline-start" />Refresh</Button></div>
      </> : <Alert><Info /><AlertTitle>Setting unavailable</AlertTitle><AlertDescription>{keyboard.reason ? messageFrom(keyboard.reason) : "This device did not report support for inverting Fn keys."}<Button variant="link" size="sm" className="mt-2 p-0" disabled={!!busy} onClick={() => void readKeyboard()}><BusyIcon busy={busy === "read"} />Check again</Button></AlertDescription></Alert>}
      {error ? <Alert variant="destructive"><AlertTitle>Could not complete</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}
    </section> : null}
    {device.isLogitech ? <BatterySection device={device} native={desktop.native} /> : null}
    <DialogFooter><Button variant="outline" onClick={onClose}>Close</Button><Button onClick={onProfiles}>View profiles<ArrowRight data-icon="inline-end" /></Button></DialogFooter>
  </DialogContent></Dialog>;
}

const batteryLevelLabel: Record<BatteryLevel, string> = { critical: "Critical", low: "Low", good: "Good", full: "Full" };

function BatterySection({ device, native }: { device: Device; native: boolean }) {
  const [battery, setBattery] = useState<BatteryStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  async function readBattery() {
    if (busy) return;
    setBusy(true); setError(null);
    try {
      const status = await getBatteryStatus(device.id);
      if (alive.current) setBattery(status);
    } catch (cause) { if (alive.current) { setBattery(null); setError(messageFrom(cause)); } }
    finally { if (alive.current) setBusy(false); }
  }

  return <section className="flex flex-col gap-3" aria-labelledby="battery-heading">
    <div className="flex items-center justify-between gap-3"><h3 id="battery-heading" className="text-sm font-medium">Battery</h3>{battery?.supported && battery.charging ? <Badge variant="secondary"><BatteryCharging data-icon="inline-start" />Charging</Badge> : null}</div>
    {battery?.supported ? <div className="flex flex-col gap-2">
      {typeof battery.percent === "number" ? <><div className="flex items-center justify-between gap-3 text-sm"><span>{battery.percent}%</span>{battery.level ? <span className="text-xs text-muted-foreground">{batteryLevelLabel[battery.level]}</span> : null}</div><Progress value={battery.percent} aria-label={`Battery at ${battery.percent}%`} /></> : battery.level ? <p className="text-sm">Level: {batteryLevelLabel[battery.level]}</p> : null}
      <p className="text-xs text-muted-foreground">{battery.externalPower ? "Connected to power." : "On battery."} Value read from the device.</p>
    </div> : battery ? <Alert><Info /><AlertTitle>Battery reading unavailable</AlertTitle><AlertDescription>{battery.reason ? messageFrom(battery.reason) : "This device did not report its battery level."}</AlertDescription></Alert>
      : <p className="text-xs leading-relaxed text-muted-foreground">{native ? "Read the battery level directly from the device. Reading does not change any setting." : "Battery reading is only available in the TrapLink desktop app."}</p>}
    <Button variant="outline" disabled={!native || busy} onClick={() => void readBattery()}><BusyIcon busy={busy} />{busy ? "Reading battery…" : battery ? "Read again" : "Read battery"}</Button>
    {error ? <Alert variant="destructive"><AlertTitle>Could not read the battery</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}
  </section>;
}
