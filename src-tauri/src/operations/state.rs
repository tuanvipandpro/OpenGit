use crate::domain::contracts::*;
pub fn terminal(state: &OperationState) -> bool {
    matches!(
        state,
        OperationState::Succeeded
            | OperationState::Failed
            | OperationState::Cancelled
            | OperationState::Interrupted
    )
}
pub fn allowed(from: &OperationState, to: &OperationState) -> bool {
    if terminal(from) {
        return false;
    }
    match to {
        OperationState::Queued => false,
        OperationState::Running => matches!(
            from,
            OperationState::Queued | OperationState::WaitingForUser
        ),
        OperationState::WaitingForUser => matches!(from, OperationState::Running),
        OperationState::Cancelling => !matches!(from, OperationState::Cancelling),
        _ => true,
    }
}
