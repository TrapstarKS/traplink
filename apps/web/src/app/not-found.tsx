import type { Metadata } from "next";
import Link from "next/link";
import { AuthHeader } from "@/components/auth-header";
import { AuthShell } from "@/components/auth-shell";
import { Button } from "@/components/ui/button";

export const metadata: Metadata = { title: "Page not found" };

export default function NotFound() {
  return (
    <AuthShell>
      <AuthHeader title="Page not found" description="This address is not part of TrapLink. Check the link or go back to sign in.">
        <p className="font-mono text-sm text-muted-foreground">Error 404</p>
      </AuthHeader>
      <Button asChild className="w-full"><Link href="/sign-in">Back to sign in</Link></Button>
    </AuthShell>
  );
}
