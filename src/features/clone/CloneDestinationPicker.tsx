import type { CloneDestination } from '../../contracts';
import { Button } from '../../components/Button';
export function CloneDestinationPicker({ destination, busy, name, onChoose, onName }: { destination: CloneDestination | null; busy: boolean; name: string; onChoose: () => void; onName: (value: string) => void }) {
  return <fieldset disabled={busy}><legend>Clone destination</legend><div className="picker-row"><span>{destination?.parentDisplay ?? 'No parent folder selected'}</span><Button type="button" onClick={onChoose}>Choose destination folder</Button></div><label className="field-label">Folder name<input name="directory-name" value={name} maxLength={200} placeholder="my-repository" autoComplete="off" spellCheck={false} onChange={event => onName(event.target.value)} /></label><p className="field-hint">A new folder will be created inside this parent. Existing folders are never overwritten.</p></fieldset>;
}
