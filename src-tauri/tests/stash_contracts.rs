use opengit_lib::domain::contracts::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../fixtures/contracts/stash-wire.json")).unwrap()
}
fn exact<T: DeserializeOwned + Serialize>(value: &Value) {
    let parsed: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), *value);
}

#[test]
fn stash_wire_preserves_counters_and_matches_all_four_typed_commands() {
    let wire = fixture();
    exact::<GetRepositoryStashesRequest>(&wire["catalogRequest"]);
    exact::<RepositoryStashes>(&wire["catalog"]);
    exact::<ResolveStashTargetRequest>(&wire["resolveRequest"]);
    exact::<ResolvedStashTarget>(&wire["resolved"]);
    exact::<GetStashActionOutcomeRequest>(&wire["outcomeRequest"]);
    exact::<ReconcileStashActionRequest>(&wire["reconcileRequest"]);
    exact::<StashActionOperationResult>(&wire["result"]);
    assert_eq!(wire["catalog"]["entries"][0]["ordinal"], "9007199254740993");
}

#[test]
fn stash_wire_rejects_extra_path_authority_missing_scope_and_unknown_retention() {
    let wire = fixture();
    let mut request = wire["resolveRequest"].clone();
    request["path"] = json!("C:/synthetic/not-authority");
    assert!(serde_json::from_value::<ResolveStashTargetRequest>(request).is_err());
    let mut request = wire["reconcileRequest"].clone();
    request.as_object_mut().unwrap().remove("scope");
    assert!(serde_json::from_value::<ReconcileStashActionRequest>(request).is_err());
    let mut outcome = wire["result"].clone();
    outcome["result"]["stashRetention"] = json!("maybe");
    assert!(serde_json::from_value::<StashActionOperationResult>(outcome).is_err());
}
