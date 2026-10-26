export function CommitForm({ message, amend, canAmend, disabled, stagedCount, onChange, onAmendChange }: { message: string; amend: boolean; canAmend: boolean; disabled: boolean; stagedCount: number; onChange(value: string): void; onAmendChange(value: boolean): void }) {
  return <div className="commit-form"><p>Commit uses the real staged index. Git hooks and configured signing run normally.</p><p>{stagedCount} staged file changes in the current context.</p>
    <label><input type="checkbox" aria-label="Amend last commit" checked={amend} disabled={disabled || !canAmend} onChange={event => onAmendChange(event.target.checked)} />Amend last commit</label>
    {amend ? <p>Replace the current HEAD commit using this message and the current staged index. With no staged changes, amend changes the message only.</p> : !stagedCount && <p>Stage changes or explicitly select Amend last commit for a message-only amendment.</p>}
    {!canAmend && <p>Amend requires an existing HEAD commit.</p>}
    <label>Commit message<textarea aria-label="Commit message" rows={5} maxLength={8192} value={message} disabled={disabled} onChange={e => onChange(e.target.value)} /></label></div>;
}
