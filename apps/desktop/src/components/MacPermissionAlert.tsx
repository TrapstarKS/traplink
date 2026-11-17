import { LockKeyhole, Settings2 } from "lucide-react";
import { toast } from "sonner";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { messageFrom, nativeCommand } from "@/lib/bridge";
import type { MacPane } from "@/lib/permissions";

const paneLabels: Record<MacPane, string> = { accessibility: "Accessibility", input_monitoring: "Input Monitoring" };

export function MacPermissionAlert({ panes }: { panes: MacPane[] }) {
  async function open(pane: MacPane) {
    try { await nativeCommand("open_privacy_settings", { pane }); }
    catch (error) { toast.error(messageFrom(error)); }
  }
  return <Alert>
    <LockKeyhole />
    <AlertTitle>macOS permission missing</AlertTitle>
    <AlertDescription className="flex flex-col gap-2">
      <span>Turn on TrapLink in {panes.map(pane => paneLabels[pane]).join(" and ")}. Then quit and reopen TrapLink.</span>
      <span className="flex flex-wrap gap-2">{panes.map(pane => <Button key={pane} type="button" size="sm" variant="outline" onClick={() => void open(pane)}><Settings2 data-icon="inline-start" />Open Settings{panes.length > 1 ? ` (${paneLabels[pane]})` : ""}</Button>)}</span>
    </AlertDescription>
  </Alert>;
}
