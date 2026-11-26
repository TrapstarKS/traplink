type VisibilitySource = EventTarget & { readonly hidden: boolean };

/**
 * Calls `load` every `ms` while the document is visible, and once right away
 * when it becomes visible again. Returns a cleanup function.
 */
export function visiblePoll(load: () => void, ms: number, source: VisibilitySource = document): () => void {
  const timer = setInterval(() => { if (!source.hidden) load(); }, ms);
  const onVisibility = () => { if (!source.hidden) load(); };
  source.addEventListener("visibilitychange", onVisibility);
  return () => { clearInterval(timer); source.removeEventListener("visibilitychange", onVisibility); };
}
