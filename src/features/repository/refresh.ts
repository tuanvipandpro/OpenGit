export type RefreshReason = 'manual' | 'fallback' | 'focus' | 'operation' | 'watch' | 'resume';
export interface RefreshOptions {
  scope(): string | null;
  run(): Promise<void>;
  feedback(scope: string, visible: boolean): void;
  visible?(): boolean;
}
// One coordinator per workspace store. The pending bit also survives selection changes.
export function createRefreshCoordinator(options: RefreshOptions) {
  const visible = options.visible ?? (() => document.visibilityState !== 'hidden');
  let running: Promise<void> | null = null, pending = false, dirty = false;
  let manual = false, activeScope: string | null = null;
  const request = (reason: RefreshReason): Promise<void> => {
    if (!options.scope()) return Promise.resolve();
    if (!visible() && reason !== 'manual') { dirty = true; return running ?? Promise.resolve(); }
    if (reason === 'manual') { manual = true; options.feedback(options.scope()!, true); }
    if (running) { pending = true; return running; }
    running = (async () => {
      do {
        pending = false; dirty = false;
        const scope = options.scope();
        if (!scope) break;
        activeScope = scope;
        if (manual) options.feedback(scope, true);
        manual = false;
        const slow = setTimeout(() => { if (options.scope() === scope && visible()) options.feedback(scope, true); }, 500);
        try { await options.run(); }
        finally { clearTimeout(slow); options.feedback(scope, false); }
        if (pending && !visible() && !manual) { dirty = true; pending = false; }
      } while (pending);
    })().finally(() => { running = null; activeScope = null; });
    return running;
  };
  return {
    request,
    start() {
      let debounce: ReturnType<typeof setTimeout> | undefined;
      const focus = () => { clearTimeout(debounce); debounce = setTimeout(() => { void request('focus'); }, 150); };
      const visibility = () => {
        clearTimeout(debounce);
        if (!visible()) dirty = true;
        else if (dirty) { dirty = false; void request('resume'); }
      };
      const timer = setInterval(() => { void request('fallback'); }, 30_000);
      window.addEventListener('focus', focus);
      document.addEventListener('visibilitychange', visibility);
      // A new active scope during an existing request needs one rerun for that scope.
      if (running && activeScope !== options.scope()) pending = true;
      return () => {
        pending = false;
        clearInterval(timer); clearTimeout(debounce);
        window.removeEventListener('focus', focus);
        document.removeEventListener('visibilitychange', visibility);
      };
    },
  };
}
