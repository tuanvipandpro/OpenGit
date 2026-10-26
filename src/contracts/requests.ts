import type { CloneSource, Id, Counter, Environment, Workspace, RepositorySnapshot, LocalCloneSource, CloneDestination, OperationSnapshot, OperationsSnapshot } from './models';
export type EmptyRequest = Record<string, never>;
import type { HistoryCommandMap } from './history';
import type { DailyCommandMap } from './daily';
import type { RefActionCommandMap } from './ref_actions';
import type { RemoteActionCommandMap } from './remote_actions';
import type { ChangeCommandMap } from './changes';
import type { SyncActionCommandMap } from './sync_actions';
import type { StashCommandMap } from './stash';
export interface CommandMap extends StashCommandMap, SyncActionCommandMap, ChangeCommandMap, RemoteActionCommandMap, HistoryCommandMap, DailyCommandMap, RefActionCommandMap {
  get_environment: { request: EmptyRequest; value: Environment };
  get_workspace: { request: EmptyRequest; value: Workspace };
  choose_repository: { request: EmptyRequest; value: Workspace | null };
  inspect_repository: { request: { repoId: Id; worktreeId: Id | null; generation: Counter }; value: RepositorySnapshot };
  select_repository: { request: { repoId: Id; worktreeId: Id | null }; value: Workspace };
  forget_repository: { request: { repoId: Id }; value: Workspace };
  choose_clone_source: { request: EmptyRequest; value: LocalCloneSource | null };
  choose_clone_destination: { request: EmptyRequest; value: CloneDestination | null };
  start_clone: { request: { source: CloneSource; destinationHandle: Id; directoryName: string }; value: OperationSnapshot };
  get_operations: { request: EmptyRequest; value: OperationsSnapshot };
  cancel_operation: { request: { operationId: Id }; value: OperationSnapshot };
  respond_auth_prompt: { request: { operationId: Id; promptHandle: Id; value: string }; value: null };
}
