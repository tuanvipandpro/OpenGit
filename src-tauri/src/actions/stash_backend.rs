use super::*;
use crate::domain::stash::*;

impl Backend {
    pub async fn get_repository_stashes(
        &self,
        window: &str,
        request: GetRepositoryStashesRequest,
    ) -> Result<RepositoryStashes, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let access = self.action_read_scope(window, &request.scope).await?;
            let q = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                access.cancel.clone(),
            );
            let before = crate::actions::preflight::capture(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                &access.cancel,
            )
            .await?;
            let stack = crate::actions::stash::stack(&q).await?;
            let regular_index = match crate::actions::changes::index_supported(&q).await {
                Ok(()) => match crate::actions::stash::parse_index(
                    &crate::actions::changes::run(
                        &q,
                        &crate::git::runner::argv(&["ls-files", "--stage", "-z"]),
                    )
                    .await?,
                    stack.length,
                ) {
                    Ok(_) => true,
                    Err(e) if e.code == ErrorCode::UnsupportedAction => false,
                    Err(e) => return Err(e),
                },
                Err(e) if e.code == ErrorCode::UnsupportedAction => false,
                Err(e) => return Err(e),
            };
            let mut entries = crate::actions::stash::entries(&q, &stack).await?;
            if !regular_index {
                for entry in &mut entries {
                    entry.apply_available = false;
                }
            }
            let changes = crate::actions::changes::capture(&q).await?;
            let after = crate::actions::preflight::capture(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                &access.cancel,
            )
            .await?;
            let after_stack = crate::actions::stash::stack(&q).await?;
            if before.fingerprint != after.fingerprint || stack.digest != after_stack.digest {
                return Err(crate::actions::changes::changed());
            }
            let stamp = crate::actions::preflight::digest(
                format!("{}{}", before.fingerprint, stack.digest).as_bytes(),
            );
            let ticket =
                self.actions
                    .authority
                    .checked(window, &request.scope, access.epoch, || {
                        self.action_registration(&request.scope, &access.location)?;
                        self.actions.stash_catalogs.publish(
                            window,
                            request.scope.clone(),
                            access.epoch,
                            stamp,
                        )
                    })?;
            let value = RepositoryStashes {
                scope: request.scope,
                catalog_id: ticket.id,
                observed_at: chrono::Utc::now().to_rfc3339(),
                entries,
                save_available: regular_index
                    && !changes.unsupported
                    && !before.bare
                    && !before.busy
                    && !before.unmerged
                    && changes.status.head.oid.is_some(),
            };
            crate::actions::catalog::bounded_response(&value)?;
            Ok(value)
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn resolve_stash_target(
        &self,
        window: &str,
        request: ResolveStashTargetRequest,
    ) -> Result<ResolvedStashTarget, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            // Original catalog lookup and existing epoch precede every scope access.
            // No begin/adoption is permitted for a stale/forged capability.
            let ticket =
                self.actions
                    .stash_catalogs
                    .get(window, &request.scope, &request.catalog_id)?;
            let location = self
                .registry
                .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
            let access = self
                .action_read_scope_existing(window, &request.scope, location, ticket.epoch)
                .await?;
            let q = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                access.cancel.clone(),
            );
            let before = crate::actions::preflight::capture(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                &access.cancel,
            )
            .await?;
            let stack = crate::actions::stash::stack(&q).await?;
            let stamp = crate::actions::preflight::digest(
                format!("{}{}", before.fingerprint, stack.digest).as_bytes(),
            );
            if stamp != ticket.stamp {
                return Err(crate::actions::changes::changed());
            }
            let target = crate::actions::stash::target(&stack, &request.entry_id)?;
            crate::actions::stash::validate_target(&q, &target).await?;
            let after = crate::actions::preflight::capture(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                &access.cancel,
            )
            .await?;
            let after_stack = crate::actions::stash::stack(&q).await?;
            if before.fingerprint != after.fingerprint || stack.digest != after_stack.digest {
                return Err(crate::actions::changes::changed());
            }
            let handle = self.actions.authority.issue_until(
                window,
                &request.scope,
                ticket.epoch,
                target,
                before.fingerprint,
                Some(ticket.expires),
                || {
                    self.action_registration(&request.scope, &access.location)?;
                    self.actions
                        .stash_catalogs
                        .get(window, &request.scope, &request.catalog_id)
                        .map(|_| ())
                },
            )?;
            Ok(ResolvedStashTarget {
                scope: request.scope,
                catalog_id: request.catalog_id,
                entry_id: request.entry_id,
                stash_handle: handle,
            })
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub fn get_stash_action_outcome(
        &self,
        window: &str,
        request: GetStashActionOutcomeRequest,
    ) -> Result<StashActionOperationResult, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if self
            .scheduler
            .action_owner(&request.operation_id)?
            .as_deref()
            != Some(window)
        {
            return Err(crate::actions::changes::invalid());
        }
        let operation = self.scheduler.get(&request.operation_id)?;
        if !operation
            .action_kind
            .is_some_and(crate::actions::stash::supported)
        {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        let record = crate::actions::journal::read(&self.registry.database, &request.operation_id)?;
        let value = StashActionOperationResult {
            operation,
            result: record.stash_action.map(|proof| proof.result),
        };
        crate::actions::catalog::bounded_response(&value)?;
        Ok(value)
    }
    pub async fn reconcile_stash_action(
        &self,
        window: &str,
        request: ReconcileStashActionRequest,
    ) -> Result<StashActionOperationResult, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if self
            .scheduler
            .action_owner(&request.operation_id)?
            .as_deref()
            != Some(window)
        {
            return Err(crate::actions::changes::invalid());
        }
        let operation = self.scheduler.get(&request.operation_id)?;
        if !operation
            .action_kind
            .is_some_and(crate::actions::stash::supported)
            || !crate::operations::state::terminal(&operation.state)
        {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        let mut record =
            crate::actions::journal::read(&self.registry.database, &request.operation_id)?;
        if request.scope.repo_id != record.repo_id
            || request.scope.worktree_id != record.worktree_id
        {
            return Err(crate::actions::changes::invalid());
        }
        let original = record
            .stash_action
            .clone()
            .ok_or_else(crate::actions::changes::invalid)?;
        // Historical operation ownership does not authorize changing the
        // window's current read scope. Reject an old generation before begin().
        let epoch = self.actions.authority.read_epoch(window, &request.scope)?;
        let location = self
            .registry
            .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
        let access = self
            .action_read_scope_existing(window, &request.scope, location, epoch)
            .await?;
        if access.location.common_identity != record.common_identity
            || access.location.root_identity != record.root_identity
        {
            return Err(crate::actions::changes::changed());
        }
        let q = crate::actions::catalog::ReadQuery::new(
            self.git()?,
            &access.location,
            access.lifetime.clone(),
            access.cancel.clone(),
        );
        let before = crate::actions::preflight::capture(
            self.git()?,
            &access.location,
            access.lifetime.clone(),
            &access.cancel,
        )
        .await?;
        let stack = crate::actions::stash::stack(&q).await?;
        let changes = crate::actions::changes::capture(&q).await?;
        let recovery_ref_retention = match (&original.recovery_ref, &original.pinned_oid) {
            (Some(name), Some(value)) => {
                let owned = record.artifacts.iter().any(|artifact| {
                    artifact.root == crate::actions::journal::ArtifactRoot::Common
                        && artifact.kind == crate::actions::journal::ArtifactKind::RecoveryRef
                        && artifact.name == *name
                        && artifact.identity == value.as_bytes()
                });
                if !owned {
                    if original.result.recovery_ref_retention == StashRetention::Absent {
                        StashRetention::Absent
                    } else {
                        StashRetention::Unknown
                    }
                } else {
                    let symbolic = crate::actions::changes::output(
                        &q,
                        &crate::git::runner::argv(&["symbolic-ref", "--quiet", name]),
                    )
                    .await?;
                    if symbolic.success
                        || symbolic.exit_code != Some(1)
                        || !symbolic.stdout.is_empty()
                        || !symbolic.stderr.is_empty()
                    {
                        // A foreign symref can dereference to the expected OID.
                        // It is not the direct ref this operation created.
                        StashRetention::Unknown
                    } else {
                        let actual = crate::actions::changes::output(
                            &q,
                            &crate::git::runner::argv(&["rev-parse", "--verify", "--quiet", name]),
                        )
                        .await?;
                        if actual.success
                            && actual.stderr.is_empty()
                            && actual.stdout == format!("{value}\n").as_bytes()
                        {
                            StashRetention::Present
                        } else if !actual.success
                            && actual.exit_code == Some(1)
                            && actual.stdout.is_empty()
                            && actual.stderr.is_empty()
                        {
                            StashRetention::Absent
                        } else {
                            StashRetention::Unknown
                        }
                    }
                }
            }
            _ => StashRetention::Absent,
        };
        let after = crate::actions::preflight::capture(
            self.git()?,
            &access.location,
            access.lifetime.clone(),
            &access.cancel,
        )
        .await?;
        if before.fingerprint != after.fingerprint {
            return Err(crate::actions::changes::changed());
        }
        let proof = record.stash_action.as_mut().unwrap();
        let unmerged = changes
            .status
            .entries
            .iter()
            .any(|entry| entry.kind == StatusEntryKind::Unmerged);
        proof.result.resolution = if unmerged {
            StashResolution::Unmerged
        } else if original.phase == crate::actions::stash::StashPhase::Conflict {
            StashResolution::ResolvedExternally
        } else {
            StashResolution::Inspect
        };
        proof.result.stash_retention = if proof
            .result
            .stash_oid
            .as_ref()
            .is_some_and(|value| stack.oids.contains(value))
        {
            StashRetention::Present
        } else if proof.result.stash_oid.is_some() {
            StashRetention::Absent
        } else {
            StashRetention::Unknown
        };
        proof.result.recovery_ref_retention = recovery_ref_retention;
        // Reconcile writes evidence only. It never drops a stash, removes a pin,
        // retries native Apply, changes historical effects, or fabricates success.
        let mut database = self.registry.database.lock()?;
        let tx = database
            .transaction()
            .map_err(|_| crate::persistence::workspace::db_error())?;
        let current = crate::actions::journal::read_from(&tx, &request.operation_id)?;
        if serde_json::to_value(current.stash_action)
            .map_err(|_| crate::persistence::workspace::db_error())?
            != serde_json::to_value(Some(original))
                .map_err(|_| crate::persistence::workspace::db_error())?
        {
            return Err(crate::actions::changes::changed());
        }
        crate::actions::journal::write_to(&tx, &request.operation_id, &record)?;
        tx.commit()
            .map_err(|_| crate::persistence::workspace::db_error())?;
        Ok(StashActionOperationResult {
            operation,
            result: record.stash_action.map(|proof| proof.result),
        })
    }
}
