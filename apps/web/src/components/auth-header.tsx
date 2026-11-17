export function AuthHeader({ title, description, children }: { title: string; description?: React.ReactNode; children?: React.ReactNode }) {
  return (
    <header className="flex flex-col gap-2">
      {children}
      <h1 className="font-display text-2xl font-semibold tracking-tight text-balance">{title}</h1>
      {description ? <p className="text-sm leading-relaxed text-muted-foreground text-pretty">{description}</p> : null}
    </header>
  );
}
