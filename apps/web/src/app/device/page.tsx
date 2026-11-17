import type { Metadata } from "next";
import { AuthShell } from "@/components/auth-shell";
import { DeviceAuthorization } from "@/components/device-authorization";

export const metadata: Metadata = { title: "Authorize app", description: "Check the code shown in TrapLink and authorize your computer." };

export default async function Device({ searchParams }: { searchParams: Promise<{ user_code?: string }> }) {
  const params = await searchParams;
  return <AuthShell><DeviceAuthorization initialCode={params.user_code ?? ""} /></AuthShell>;
}
