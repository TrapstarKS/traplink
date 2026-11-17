import type { Metadata } from "next";
import { AuthShell } from "@/components/auth-shell";
import { Connected } from "@/components/connected";

export const metadata: Metadata = { title: "Account connected", description: "Your TrapLink account is ready to use in the app." };

export default function ConnectedPage() {
  return <AuthShell><Connected /></AuthShell>;
}
