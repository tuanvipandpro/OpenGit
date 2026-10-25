use super::*;
impl Backend {
    pub fn get_sync_action_outcome(
        &self,
        window: &str,
        request: GetActionOutcomeRequest,
    ) -> Result<SyncActionOperationResult, AppError> {
        self.window(window)?;
        validation::request_size(&request)?;
        if self
            .scheduler
            .action_owner(&request.operation_id)?
            .as_deref()
            != Some(window)
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        let operation = self.scheduler.get(&request.operation_id)?;
        if !operation
            .action_kind
            .is_some_and(crate::actions::sync::supported)
        {
            return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
        }
        let record = crate::actions::journal::read(&self.registry.database, &request.operation_id)?;
        Ok(SyncActionOperationResult {
            operation,
            result: record.sync_action.map(|p| p.result),
        })
    }
    pub async fn get_publication_evidence(
        &self,
        window: &str,
        request: GetPublicationEvidenceRequest,
    ) -> Result<PublicationEvidenceResult, AppError> {
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
            let candidates = self
                .actions
                .authority
                .resolved_targets(
                    window,
                    &request.scope,
                    epoch,
                    &ActionIntent::BranchSwitch {
                        branch_handle: request.source_handle.clone(),
                    },
                    &before.fingerprint,
                )
                .or_else(|_| {
                    self.actions.authority.resolved_targets(
                        window,
                        &request.scope,
                        epoch,
                        &ActionIntent::TagDelete {
                            tag_handle: request.source_handle.clone(),
                        },
                        &before.fingerprint,
                    )
                })?;
            let Some(crate::actions::journal::SafeTargetProof::Ref {
                name,
                expected_oid: Some(oid),
                ..
            }) = &candidates[0].proof
            else {
                return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
            };
            let mut observations = {
                let conn = self.registry.database.lock()?;
                crate::actions::publication::read(&conn, &request.scope.repo_id, name, oid)?
            };
            let q = crate::actions::catalog::ReadQuery::new(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                access.cancel.clone(),
            );
            let config = crate::actions::catalog::configuration(&q).await?;
            observations.retain(|p| p.config_digest == config.digest);
            self.actions
                .authority
                .checked(window, &request.scope, epoch, || {
                    self.action_registration(&request.scope, &access.location)
                })?;
            let after = crate::actions::preflight::capture(
                self.git()?,
                &access.location,
                access.lifetime.clone(),
                &access.cancel,
            )
            .await?;
            if before.fingerprint != after.fingerprint {
                return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
            }
            let result = PublicationEvidenceResult {
                scope: request.scope,
                observations,
            };
            crate::actions::catalog::bounded_response(&result)?;
            Ok(result)
        })
        .await
        .map_err(|_| error(ErrorCode::Timeout, Phase::Inspection))?
    }
}
