use super::*;
impl Backend {
    pub async fn resolve_change_selection(
        &self,
        window: &str,
        request: ResolveChangeSelectionRequest,
    ) -> Result<ResolvedChangeSelection, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let epoch = self.actions.authority.read_epoch(window, &request.scope)?;
            let location = self
                .registry
                .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
            let access = self
                .action_read_scope_existing(window, &request.scope, location, epoch)
                .await?;
            let before = crate::actions::preflight::capture(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                &access.cancel,
            )
            .await?;
            let (target, expires) = self.actions.authority.change_grant(
                window,
                &request.scope,
                epoch,
                &request.change_handle,
                &before.fingerprint,
            )?;
            let (side, _) = crate::actions::changes::paths(&target)?;
            let intent = if side == ChangeSide::Unstaged {
                ActionIntent::Stage {
                    change_handles: vec![request.change_handle.clone()],
                }
            } else {
                ActionIntent::Unstage {
                    change_handles: vec![request.change_handle.clone()],
                }
            };
            crate::actions::changes::validate_targets(
                &access.location,
                &intent,
                std::slice::from_ref(&target),
            )?;
            let q = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                access.cancel.clone(),
            );
            let data = crate::actions::patch::capture(
                &q,
                &target,
                &request.change_handle,
                &before.fingerprint,
            )
            .await?;
            if data.diff_id != request.diff_id {
                return Err(crate::actions::changes::invalid());
            }
            let selected = data.select(&request.selection)?;
            let selection = crate::actions::patch::selection_target(
                &target,
                &request.change_handle,
                &data,
                &selected,
            )?;
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
            crate::actions::changes::validate_targets(
                &access.location,
                &intent,
                std::slice::from_ref(&target),
            )?;
            let handle = self.actions.authority.issue_until(
                window,
                &request.scope,
                epoch,
                selection,
                before.fingerprint,
                Some(expires),
                || self.action_registration(&request.scope, &access.location),
            )?;
            Ok(ResolvedChangeSelection {
                scope: request.scope,
                change_handle: request.change_handle,
                diff_id: request.diff_id,
                selection_handle: handle,
            })
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn resolve_change_targets(
        &self,
        window: &str,
        request: ResolveChangeTargetsRequest,
    ) -> Result<ResolvedChangeTargets, AppError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.resolve_change_targets_inner(window, request),
        )
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn get_change_diff(
        &self,
        window: &str,
        request: GetChangeDiffRequest,
    ) -> Result<ChangeDiff, AppError> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.get_change_diff_inner(window, request),
        )
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    pub async fn get_repository_changes(
        &self,
        window: &str,
        request: GetRepositoryChangesRequest,
    ) -> Result<RepositoryChanges, AppError> {
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
            let captured = crate::actions::changes::capture(&q).await?;
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
            let stamp = crate::actions::preflight::digest(
                format!("{}{}", before.fingerprint, captured.digest).as_bytes(),
            );
            let ticket =
                self.actions
                    .authority
                    .checked(window, &request.scope, access.epoch, || {
                        self.action_registration(&request.scope, &access.location)?;
                        self.actions.change_catalogs.publish(
                            window,
                            request.scope.clone(),
                            access.epoch,
                            stamp,
                        )
                    })?;
            let result = RepositoryChanges {
                scope: request.scope,
                catalog_id: ticket.id,
                observed_at: chrono::Utc::now().to_rfc3339(),
                head: captured.status.head,
                entries: captured.wire,
                unsupported: captured.unsupported || before.busy || before.unmerged,
            };
            crate::actions::catalog::bounded_response(&result)?;
            Ok(result)
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
    async fn resolve_change_targets_inner(
        &self,
        window: &str,
        request: ResolveChangeTargetsRequest,
    ) -> Result<ResolvedChangeTargets, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if request.targets.is_empty() || request.targets.len() > 128 {
            return Err(error(ErrorCode::OutputLimit, Phase::Validation));
        }
        let ticket =
            self.actions
                .change_catalogs
                .get(window, &request.scope, &request.catalog_id)?;
        let location = self
            .registry
            .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
        // Existing catalog epoch is authoritative. Never re-begin an invalid old capability.
        let access = self
            .action_read_scope_existing(window, &request.scope, location, ticket.epoch)
            .await?;
        let before = crate::actions::preflight::capture(
            self.git()?,
            &access.location,
            access.lifetime.clone(),
            &access.cancel,
        )
        .await?;
        let q = crate::actions::catalog::ReadQuery::new(
            self.git()?,
            &access.location,
            access.lifetime.clone(),
            access.cancel.clone(),
        );
        let captured = crate::actions::changes::capture(&q).await?;
        if crate::actions::preflight::digest(
            format!("{}{}", before.fingerprint, captured.digest).as_bytes(),
        ) != ticket.stamp
        {
            return Err(crate::actions::changes::changed());
        }
        let targets = request
            .targets
            .iter()
            .map(|s| crate::actions::changes::target(&captured, s))
            .collect::<Result<Vec<_>, _>>()?;
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
        let mut result = Vec::new();
        for (selection, target) in request.targets.iter().zip(targets) {
            let handle = self.actions.authority.issue_until(
                window,
                &request.scope,
                ticket.epoch,
                target,
                before.fingerprint.clone(),
                Some(ticket.expires),
                || {
                    self.action_registration(&request.scope, &access.location)?;
                    self.actions
                        .change_catalogs
                        .get(window, &request.scope, &request.catalog_id)
                        .map(|_| ())
                },
            )?;
            result.push(ChangeTarget {
                entry_id: selection.entry_id.clone(),
                side: selection.side,
                change_handle: handle,
            });
        }
        Ok(ResolvedChangeTargets {
            scope: request.scope,
            catalog_id: request.catalog_id,
            targets: result,
        })
    }
    async fn get_change_diff_inner(
        &self,
        window: &str,
        request: GetChangeDiffRequest,
    ) -> Result<ChangeDiff, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        let epoch = self.actions.authority.read_epoch(window, &request.scope)?;
        let location = self
            .registry
            .location(&request.scope.repo_id, request.scope.worktree_id.as_deref())?;
        let access = self
            .action_read_scope_existing(window, &request.scope, location, epoch)
            .await?;
        let before = crate::actions::preflight::capture(
            self.git()?,
            &access.location,
            access.lifetime.clone(),
            &access.cancel,
        )
        .await?;
        let targets = self.actions.authority.resolved_targets(
            window,
            &request.scope,
            epoch,
            &ActionIntent::Stage {
                change_handles: vec![request.change_handle.clone()],
            },
            &before.fingerprint,
        )?;
        let target = targets
            .first()
            .ok_or_else(crate::actions::changes::invalid)?;
        let (side, _) = crate::actions::changes::paths(target)?;
        let intent = if side == ChangeSide::Staged {
            ActionIntent::Unstage {
                change_handles: vec![request.change_handle.clone()],
            }
        } else {
            ActionIntent::Stage {
                change_handles: vec![request.change_handle.clone()],
            }
        };
        crate::actions::changes::validate_targets(&access.location, &intent, &targets)?;
        let q = crate::actions::catalog::ReadQuery::new(
            self.git()?,
            &access.location,
            access.lifetime.clone(),
            access.cancel.clone(),
        );
        let data =
            crate::actions::patch::capture(&q, target, &request.change_handle, &before.fingerprint)
                .await?;
        let content = if data.eligibility == PartialEligibility::TooLarge {
            "Diff exceeds the partial display limit. Whole-file actions remain available.".into()
        } else {
            crate::actions::changes::diff(&q, target).await?
        };
        let hunks = data.hunks()?;
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
        crate::actions::changes::validate_targets(&access.location, &intent, &targets)?;
        self.actions
            .authority
            .checked(window, &request.scope, epoch, || {
                self.action_registration(&request.scope, &access.location)
            })?;
        let result = ChangeDiff {
            scope: request.scope,
            change_handle: request.change_handle,
            diff_id: data.diff_id,
            side,
            content,
            partial_eligibility: data.eligibility,
            hunks,
        };
        crate::actions::catalog::bounded_response(&result)?;
        Ok(result)
    }
    pub fn get_change_action_outcome(
        &self,
        window: &str,
        request: GetActionOutcomeRequest,
    ) -> Result<ChangeActionOperationResult, AppError> {
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
            .is_some_and(crate::actions::changes::supported)
        {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        let record = crate::actions::journal::read(&self.registry.database, &request.operation_id)?;
        Ok(ChangeActionOperationResult {
            operation,
            result: record.change_action.map(|p| p.result),
        })
    }
}
