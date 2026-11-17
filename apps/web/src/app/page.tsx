import { AuthShell } from "@/components/auth-shell";
import { AuthForm } from "@/components/auth-form";

export default function Home() {
  return <AuthShell><AuthForm mode="sign-in" /></AuthShell>;
}
