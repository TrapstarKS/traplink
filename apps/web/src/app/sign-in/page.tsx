import type { Metadata } from "next";
import { AuthShell } from "@/components/auth-shell";
import { AuthForm } from "@/components/auth-form";

export const metadata: Metadata = { title: "Sign in", description: "Sign in to your TrapLink account to authorize the app on your computer." };

export default async function SignIn({ searchParams }: { searchParams: Promise<{ callback?: string; error?: string }> }) {
  const params = await searchParams;
  return <AuthShell><AuthForm mode="sign-in" callback={params.callback} initialError={params.error} /></AuthShell>;
}
