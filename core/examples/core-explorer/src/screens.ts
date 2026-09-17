export const primaryScreens = ['data', 'jobs', 'overview', 'networking'] as const;
export const screens = [...primaryScreens, 'portals', 'poses', 'domains', 'settings', 'filters', 'record', 'preview', 'technical', 'upload', 'access'] as const;
export type Screen = typeof screens[number];
export function isScreen(value: string): value is Screen { return (screens as readonly string[]).includes(value); }

/** Navigation stores routes only: switching screens never changes SDK resources. */
export class ScreenHistory {
  current: Screen = 'access';
  private history: Screen[] = [];
  go(screen: Screen) {
    if (screen === this.current) return;
    if ((primaryScreens as readonly string[]).includes(screen)) this.history = [];
    else this.history.push(this.current);
    this.current = screen;
  }
  back(): Screen { return this.current = this.history.pop() ?? 'data'; }
  reset(screen: Screen) { this.history = []; this.current = screen; }
}

/** Restore controls without removing them from sequential keyboard navigation. */
export function focusScreenTarget(target: Pick<HTMLElement, 'tagName' | 'tabIndex' | 'focus'>): void {
  if (/^H[1-6]$/.test(target.tagName)) target.tabIndex = -1;
  target.focus({ preventScroll: true });
}

/** Only visible, nonterminal task details warrant bounded polling. */
export function jobsShouldPoll(visible: boolean, screen: string, tasks: readonly { status: string }[]): boolean {
  return visible && screen === 'detail' && tasks.some(task => ['queued', 'leased', 'running'].includes(task.status));
}

/** Group detail routes without changing their resource ownership. */
export function primaryScreen(screen: Screen): typeof primaryScreens[number] | undefined {
  if (['overview', 'portals', 'poses'].includes(screen)) return 'overview';
  if (['data', 'filters', 'record', 'preview', 'upload'].includes(screen)) return 'data';
  if (screen === 'jobs' || screen === 'networking') return screen;
  return undefined;
}
