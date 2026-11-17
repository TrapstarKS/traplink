import { cn } from "@/lib/utils";

export function Brand({ className }: { className?: string }) {
  return <span className={cn("brand", className)}>
    {}
    <svg viewBox="0 0 36 36" fill="none" aria-hidden="true"><path d="M9 4H24L32 12V27H17L9 19V4Z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round"/><path d="M4 9H19L27 17V32H12L4 24V9Z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round"/></svg>
    <span>TrapLink</span>
  </span>;
}
