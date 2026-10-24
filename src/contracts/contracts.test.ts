import { describe, it, expect, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { call } from './client';
import type { ApiResult, RepositorySnapshot, CloneSource } from './models';
import type { CommandMap } from './requests';
import fixture from '../../fixtures/contracts/wire.json';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
describe('frozen IPC contract', () => {
  it('keeps nullable fields, opaque OIDs and counters through JSON transport', () => {
    const value = JSON.parse(JSON.stringify(fixture));
    expect(value).toEqual(fixture);
    expect(value.commands[1].result.value.revision).toBe('9007199254740993');
    expect(value.snapshots[1].head).toEqual({ kind: 'unborn', branch: 'main', oid: null });
    expect(value.snapshots[2].isDirty).toBeNull();
  });
  it('passes only the documented request envelope and returns service results', async () => {
    vi.mocked(invoke).mockResolvedValueOnce(fixture.commands[0].result);
    expect(await call('get_environment', {})).toEqual(fixture.commands[0].result);
    expect(invoke).toHaveBeenCalledWith('get_environment', { request: {} });
  });
  it('redacts rejected transport errors including credential canaries', async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error('CANARY_SECRET raw transport text'));
    const result = await call('get_environment', {});
    expect(result).toEqual({ ok: false, error: { code: 'Internal', message: 'The desktop service could not complete this request.', phase: 'validation', retry: 'none' } });
    expect(JSON.stringify(result)).not.toContain('CANARY_SECRET');
  });
});
// Compile-time examples exercise discriminated unions without weakening production types.
const local: CloneSource = { kind: 'local', sourceHandle: 'fixture' };
const nullable: ApiResult<RepositorySnapshot | null> = { ok: true, value: null };
void local; void nullable;

type WireSamples = { [C in keyof CommandMap]: { command: C; request: CommandMap[C]['request']; result: ApiResult<CommandMap[C]['value']> } }[keyof CommandMap][];
const typedSamples = [
  {
    "command": "get_environment",
    "request": {},
    "result": {
      "ok": true,
      "value": {
        "contractVersion": "1",
        "appVersion": "0.1.0",
        "platform": "windows",
        "git": {
          "available": false,
          "version": null
        },
        "credentialManager": {
          "available": false,
          "version": null
        },
        "openSsh": {
          "available": false,
          "version": null
        },
        "webView2": {
          "available": true,
          "version": null
        },
        "identity": {
          "nameConfigured": false,
          "emailConfigured": false
        }
      }
    }
  },
  {
    "command": "get_workspace",
    "request": {},
    "result": {
      "ok": true,
      "value": {
        "id": "workspace-fixture",
        "revision": "9007199254740993",
        "repositories": [
          {
            "id": "repo-fixture",
            "name": "Fixture",
            "commonGitDirDisplay": "C:\\fixtures\\repo.git",
            "isBare": false,
            "availability": "available",
            "worktrees": [
              {
                "id": "worktree-fixture",
                "rootDisplay": "C:\\fixtures\\repo",
                "isMain": true
              }
            ],
            "lastOpenedAt": "2026-10-24T00:00:00Z"
          }
        ],
        "activeSelection": {
          "repoId": "repo-fixture",
          "worktreeId": "worktree-fixture"
        }
      }
    }
  },
  {
    "command": "choose_repository",
    "request": {},
    "result": {
      "ok": true,
      "value": null
    }
  },
  {
    "command": "inspect_repository",
    "request": {
      "repoId": "repo-fixture",
      "worktreeId": "worktree-fixture",
      "generation": "2"
    },
    "result": {
      "ok": true,
      "value": {
        "repoId": "repo-fixture",
        "worktreeId": "worktree-fixture",
        "generation": "2",
        "observedAt": "2026-10-24T00:00:00Z",
        "isBare": false,
        "head": {
          "kind": "branch",
          "branch": "main",
          "oid": "opaque-object-id"
        },
        "status": [
          {
            "id": "entry-1",
            "kind": "rename",
            "pathDisplay": "new name.txt",
            "previousPathDisplay": "old name.txt",
            "indexStatus": "R",
            "worktreeStatus": ".",
            "submodule": false
          }
        ],
        "isDirty": true,
        "isShallow": false,
        "remotes": [
          {
            "name": "origin",
            "fetchUrlDisplay": null,
            "pushUrlDisplay": null
          }
        ],
        "identity": {
          "nameConfigured": false,
          "emailConfigured": false
        },
        "gitOperations": []
      }
    }
  },
  {
    "command": "select_repository",
    "request": {
      "repoId": "repo-fixture",
      "worktreeId": "worktree-fixture"
    },
    "result": {
      "ok": true,
      "value": {
        "id": "workspace-fixture",
        "revision": "9007199254740993",
        "repositories": [
          {
            "id": "repo-fixture",
            "name": "Fixture",
            "commonGitDirDisplay": "C:\\fixtures\\repo.git",
            "isBare": false,
            "availability": "available",
            "worktrees": [
              {
                "id": "worktree-fixture",
                "rootDisplay": "C:\\fixtures\\repo",
                "isMain": true
              }
            ],
            "lastOpenedAt": "2026-10-24T00:00:00Z"
          }
        ],
        "activeSelection": {
          "repoId": "repo-fixture",
          "worktreeId": "worktree-fixture"
        }
      }
    }
  },
  {
    "command": "forget_repository",
    "request": {
      "repoId": "repo-fixture"
    },
    "result": {
      "ok": true,
      "value": {
        "id": "workspace-fixture",
        "revision": "9007199254740993",
        "repositories": [
          {
            "id": "repo-fixture",
            "name": "Fixture",
            "commonGitDirDisplay": "C:\\fixtures\\repo.git",
            "isBare": false,
            "availability": "available",
            "worktrees": [
              {
                "id": "worktree-fixture",
                "rootDisplay": "C:\\fixtures\\repo",
                "isMain": true
              }
            ],
            "lastOpenedAt": "2026-10-24T00:00:00Z"
          }
        ],
        "activeSelection": {
          "repoId": "repo-fixture",
          "worktreeId": "worktree-fixture"
        }
      }
    }
  },
  {
    "command": "choose_clone_source",
    "request": {},
    "result": {
      "ok": true,
      "value": {
        "sourceHandle": "local-fixture",
        "display": "Local fixture"
      }
    }
  },
  {
    "command": "choose_clone_destination",
    "request": {},
    "result": {
      "ok": true,
      "value": {
        "destinationHandle": "destination-fixture",
        "parentDisplay": "C:\\fixtures"
      }
    }
  },
  {
    "command": "start_clone",
    "request": {
      "source": {
        "kind": "local",
        "sourceHandle": "local-fixture"
      },
      "destinationHandle": "destination-fixture",
      "directoryName": "new"
    },
    "result": {
      "ok": true,
      "value": {
        "id": "operation-fixture",
        "sequence": "3",
        "kind": "clone",
        "state": "Queued",
        "phase": "queue",
        "repoId": null,
        "worktreeId": null,
        "sourceDisplay": "Local repository",
        "destinationDisplay": "C:\\fixtures\\new",
        "progress": null,
        "canCancel": true,
        "authPrompt": null,
        "error": null,
        "createdAt": "2026-10-24T00:00:00Z",
        "updatedAt": "2026-10-24T00:00:00Z"
      }
    }
  },
  {
    "command": "get_operations",
    "request": {},
    "result": {
      "ok": true,
      "value": {
        "streamId": "stream-fixture",
        "sequence": "3",
        "operations": [
          {
            "id": "operation-fixture",
            "sequence": "3",
            "kind": "clone",
            "state": "Queued",
            "phase": "queue",
            "repoId": null,
            "worktreeId": null,
            "sourceDisplay": "Local repository",
            "destinationDisplay": "C:\\fixtures\\new",
            "progress": null,
            "canCancel": true,
            "authPrompt": null,
            "error": null,
            "createdAt": "2026-10-24T00:00:00Z",
            "updatedAt": "2026-10-24T00:00:00Z"
          }
        ]
      }
    }
  },
  {
    "command": "cancel_operation",
    "request": {
      "operationId": "operation-fixture"
    },
    "result": {
      "ok": true,
      "value": {
        "id": "operation-fixture",
        "sequence": "3",
        "kind": "clone",
        "state": "Cancelled",
        "phase": "queue",
        "repoId": null,
        "worktreeId": null,
        "sourceDisplay": "Local repository",
        "destinationDisplay": "C:\\fixtures\\new",
        "progress": null,
        "canCancel": false,
        "authPrompt": null,
        "error": null,
        "createdAt": "2026-10-24T00:00:00Z",
        "updatedAt": "2026-10-24T00:00:00Z"
      }
    }
  },
  {
    "command": "respond_auth_prompt",
    "request": {
      "operationId": "operation-fixture",
      "promptHandle": "prompt-fixture",
      "value": "yes"
    },
    "result": {
      "ok": true,
      "value": null
    }
  }
] satisfies WireSamples;
it('shares the same typed examples with the Rust JSON fixtures', () => { expect(fixture.commands).toEqual(typedSamples); });
