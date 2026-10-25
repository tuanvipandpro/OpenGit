export function CommitForm({ message, disabled, stagedCount, onChange }: { message: string; disabled: boolean; stagedCount: number; onChange(value: string): void }) {
  return <div className="commit-form"><p>Commit uses the real staged index. Git hooks and configured signing run normally.</p><p>{stagedCount} staged file changes in the current context.</p><label>Commit message<textarea aria-label="Commit message" rows={5} maxLength={8192} value={message} disabled={disabled} onChange={e => onChange(e.target.value)} /></label></div>;
}
