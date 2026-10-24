use crate::domain::contracts::GitOperation;
use std::path::Path;
pub fn detect(git_dir: &Path) -> Vec<GitOperation> {
    let mut result = Vec::new();
    for (names, kind) in [
        (vec!["MERGE_HEAD"], GitOperation::Merge),
        (vec!["rebase-merge", "rebase-apply"], GitOperation::Rebase),
        (vec!["CHERRY_PICK_HEAD"], GitOperation::CherryPick),
        (vec!["REVERT_HEAD"], GitOperation::Revert),
        (vec!["BISECT_LOG"], GitOperation::Bisect),
        (vec!["sequencer"], GitOperation::Sequencer),
    ] {
        if names.iter().any(|name| git_dir.join(name).exists()) {
            result.push(kind);
        }
    }
    result
}
