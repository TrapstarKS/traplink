import type { Metadata } from "next";
import { AuthShell } from "@/components/auth-shell";
import { AuthForm } from "@/components/auth-form";

export const metadata: Metadata = { title: "Create account", description: "Create your TrapLink account to connect the app on your computer." };

export default async function SignUp({ searchParams }: { searchParams: Promise<{ callback?: string }> }) {
  const params = await searchParams;
  return <AuthShell><AuthForm mode="sign-up" callback={params.callback} /></AuthShell>;
}
