import type { OperationSnapshot, Phase } from '../../contracts';
const phases: Record<Phase, string> = { validation: 'Validating request', discovery: 'Finding Git', inspection: 'Inspecting repository', queue: 'Waiting to start', authentication: 'Waiting for authentication', transfer: 'Transferring repository', checkout: 'Checking out files', verification: 'Verifying repository', publish: 'Publishing destination', registration: 'Adding to workspace', recovery: 'Checking interrupted operation' };
export function CloneProgress({ operation }: { operation: OperationSnapshot }) {
  const progress = operation.progress;
  return <div className="clone-progress"><span>{operation.state === 'Succeeded' ? 'Repository added to workspace' : phases[operation.phase]}</span>{progress && <><span>{progress.completed}{progress.total !== null ? ' / ' + progress.total : ''} {progress.unit}</span><progress aria-label="Clone progress" value={progress.total === null ? undefined : progress.completed} max={progress.total ?? 1} /></>}</div>;
}
