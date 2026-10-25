export type RefreshReason = 'manual' | 'fallback' | 'focus' | 'operation' | 'watch' | 'resume';
export interface RefreshInteractionLease {
  ready: Promise<boolean>;
  closed: Promise<void>;
  isCurrent(): boolean;
  release(): void;
  cancel(): void;
}
export interface RefreshOptions {
  scope(): string | null;
  run(): Promise<void>;
  feedback(scope: string, visible: boolean): void;
  visible?(): boolean;
}
// One coordinator and at most one selection-owned interaction per workspace.
// Timers/watchers keep running; only passive reads defer while a form is open.
export function createRefreshCoordinator(options: RefreshOptions) {
  const visible = options.visible ?? (() => document.visibilityState !== 'hidden');
  let running: Promise<void> | null = null, pending = false, pendingUrgent = false, dirty = false, dirtyUrgent = false;
  let manual = false, activeScope: string | null = null;
  let interaction: { scope: string; deferred: boolean; ready(value: boolean): void; close(): void } | null = null;
  const invalidateInteraction = () => { const old = interaction; interaction = null; old?.ready(false); old?.close(); };
  const currentInteraction = () => {
    if (interaction && interaction.scope !== options.scope()) invalidateInteraction();
    return interaction;
  };
  const request = (reason: RefreshReason): Promise<void> => {
    if (!options.scope()) { currentInteraction(); return Promise.resolve(); }
    const held = currentInteraction(), urgent = reason === 'manual' || reason === 'operation';
    if (held && !urgent) { held.deferred = true; return running ?? Promise.resolve(); }
    if (!visible() && reason !== 'manual') { dirty = true; dirtyUrgent ||= urgent; return running ?? Promise.resolve(); }
    if (reason === 'manual') { manual = true; options.feedback(options.scope()!, true); }
    if (running) { pending = true; pendingUrgent ||= urgent; return running; }
    running = (async () => {
      do {
        pending = false; pendingUrgent = false; dirty = false; dirtyUrgent = false;
        const scope = options.scope();
        if (!scope) break;
        activeScope = scope;
        if (manual) options.feedback(scope, true);
        manual = false;
        const slow = setTimeout(() => { if (options.scope() === scope && visible()) options.feedback(scope, true); }, 500);
        try { await options.run(); }
        finally { clearTimeout(slow); options.feedback(scope, false); }
        const owner = currentInteraction();
        if (pending && owner && !pendingUrgent) { owner.deferred = true; pending = false; }
        if (pending && !visible() && !manual) { dirty = true; dirtyUrgent ||= pendingUrgent; pending = false; }
      } while (pending);
    })().finally(() => { running = null; activeScope = null; currentInteraction()?.ready(true); });
    return running;
  };
  return {
    request, invalidateInteraction,
    acquireInteraction(): RefreshInteractionLease | null {
      if (currentInteraction() || !options.scope()) return null;
      let close!: () => void, resolveReady!: (ready: boolean) => void;
      const closed = new Promise<void>(resolve => { close = resolve; });
      const ready = new Promise<boolean>(resolve => { resolveReady = resolve; });
      const owner = { scope: options.scope()!, deferred: pending && !pendingUrgent, ready: resolveReady, close };
      if (owner.deferred) pending = false;
      interaction = owner;
      const isCurrent = () => interaction === owner && options.scope() === owner.scope;
      if (!running) resolveReady(true);
      return { closed, ready, isCurrent,
        cancel: () => { if (interaction === owner) invalidateInteraction(); },
        release: () => {
          if (interaction !== owner) return;
          const flush = isCurrent() && owner.deferred;
          interaction = null; resolveReady(false); close();
          if (flush) void request('resume');
        },
      };
    },
    start() {
      let debounce: ReturnType<typeof setTimeout> | undefined;
      const focus = () => { clearTimeout(debounce); debounce = setTimeout(() => { void request('focus'); }, 150); };
      const visibility = () => {
        clearTimeout(debounce);
        if (!visible()) dirty = true;
        else if (dirty) { const reason = dirtyUrgent ? 'operation' : 'resume'; dirty = false; dirtyUrgent = false; void request(reason); }
      };
      const timer = setInterval(() => { void request('fallback'); }, 30_000);
      window.addEventListener('focus', focus);
      document.addEventListener('visibilitychange', visibility);
      if (running && activeScope !== options.scope()) { pending = true; pendingUrgent = true; }
      return () => {
        invalidateInteraction(); pending = false; pendingUrgent = false;
        clearInterval(timer); clearTimeout(debounce);
        window.removeEventListener('focus', focus);
        document.removeEventListener('visibilitychange', visibility);
      };
    },
  };
}
