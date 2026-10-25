import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, it, vi } from 'vitest';
import { RefActionsMenu } from './RefActionsMenu';
import type { RefMenuTarget } from './RefActionsMenu';

afterEach(() => vi.restoreAllMocks());
const ref = { name: 'refs/heads/main', displayName: 'main', commitOid: 'oid', oid: 'oid', kind: 'local' as const, hidden: false, symbolicTarget: null };
function setup(filter = { hiddenRefs: [] as string[], soloRefs: [] as string[] }) {
  const trigger = document.createElement('button'); document.body.append(trigger);
  const target: RefMenuTarget = { ref, trigger, context: 'context' }, onClose = vi.fn(), onFilter = vi.fn(), onView = vi.fn();
  const view = render(<RefActionsMenu target={target} filter={filter} onClose={onClose} onFilter={onFilter} onView={onView} />);
  return { ...view, target, onClose, onFilter, onView, dispose: () => { view.unmount(); trigger.remove(); } };
}
it('AC4 focuses first item, wraps arrows, supports Home/End, Enter/Space, Escape and Tab', async () => {
  const test = setup(), user = userEvent.setup();
  try {
    expect(screen.getByRole('menuitem', { name: 'View commit' })).toHaveFocus();
    await user.keyboard('{ArrowUp}'); expect(screen.getByRole('menuitem', { name: 'Add to Solo' })).toHaveFocus();
    await user.keyboard('{ArrowDown}{End}'); expect(screen.getByRole('menuitem', { name: 'Add to Solo' })).toHaveFocus();
    await user.keyboard('{Home}{Enter}'); expect(test.onView).toHaveBeenCalledOnce();
    await user.keyboard('{ArrowDown} '); expect(test.onFilter).toHaveBeenCalledWith({ kind: 'hide', names: [ref.name] });
    await user.keyboard('{Escape}'); expect(test.onClose).toHaveBeenCalledOnce();
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Tab' }); expect(test.onClose).toHaveBeenCalledTimes(2);
  } finally { test.dispose(); }
});
it('AC2 offers Show and Remove from Solo using the qualified identity', () => {
  const test = setup({ hiddenRefs: [ref.name], soloRefs: [ref.name] });
  try {
    fireEvent.click(screen.getByRole('menuitem', { name: 'Show' })); expect(test.onFilter).toHaveBeenCalledWith({ kind: 'show', names: [ref.name] });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Remove from Solo' })); expect(test.onFilter).toHaveBeenCalledWith({ kind: 'unsolo', names: [ref.name] });
  } finally { test.dispose(); }
});
it('AC4 clamps a portal menu to eight pixels from the viewport edges and limits height', () => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    return (this.getAttribute('role') === 'menu' ? { width: 180, height: 120 } : { left: window.innerWidth - 2, bottom: window.innerHeight - 2 }) as unknown as DOMRect;
  });
  const test = setup();
  try {
    expect(screen.getByRole('menu')).toHaveStyle({ left: `${window.innerWidth - 188}px`, top: `${window.innerHeight - 128}px` });
    expect(screen.getByRole('menu').parentElement).toBe(document.body);
  } finally { test.dispose(); }
});
