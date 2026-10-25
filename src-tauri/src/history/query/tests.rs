use super::*;
fn control() -> OrderControl {
    OrderControl {
        deadline: Instant::now() + Duration::from_secs(30),
        cancel: Cancellation::default(),
    }
}
fn ids(count: usize) -> Vec<String> {
    (1..=count).map(|i| format!("{i:064x}")).collect()
}
#[tokio::test]
async fn sh01_stable_induced_sort_uses_original_rank_and_only_selected_edges() {
    let original = ids(3);
    let index: HashMap<_, _> = original
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i as u32))
        .collect();
    let parents = vec![vec![], vec![0, 0], vec![]];
    assert_eq!(
        stable_induced_order(&original, &index, &parents, &control())
            .await
            .unwrap(),
        [
            original[1].clone(),
            original[0].clone(),
            original[2].clone()
        ]
    );
    let visible = vec![original[0].clone(), original[2].clone()];
    assert_eq!(
        stable_induced_order(&visible, &index, &parents, &control())
            .await
            .unwrap(),
        visible
    );
    let sorted = vec![
        original[1].clone(),
        original[0].clone(),
        original[2].clone(),
    ];
    assert_eq!(
        stable_induced_order(&sorted, &index, &parents, &control())
            .await
            .unwrap(),
        sorted
    );
}
#[tokio::test]
async fn sh01_cycle_unknown_duplicate_or_invalid_indices_fail_without_an_order() {
    let original = ids(2);
    let index: HashMap<_, _> = original
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i as u32))
        .collect();
    for parents in [vec![vec![1], vec![0]], vec![vec![2], vec![]]] {
        assert_eq!(
            stable_induced_order(&original, &index, &parents, &control())
                .await
                .unwrap_err()
                .code,
            ErrorCode::MalformedOutput
        );
    }
    assert_eq!(
        stable_induced_order(
            &[original[0].clone(), original[0].clone()],
            &index,
            &[vec![], vec![]],
            &control()
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::MalformedOutput
    );
    assert_eq!(
        stable_induced_order(&["f".repeat(64)], &index, &[vec![], vec![]], &control())
            .await
            .unwrap_err()
            .code,
        ErrorCode::MalformedOutput
    );
}
#[test]
fn sh01_metadata_preflight_raw_and_parent_bounds_use_checked_arithmetic() {
    let mut budget = ParentBudget::new(100_000, 64).unwrap();
    for _ in 0..300_000 {
        budget.parent().unwrap();
    }
    assert_eq!(budget.semantic, 26_000_000);
    assert_eq!(budget.parent().unwrap_err().code, ErrorCode::OutputLimit);
    assert_eq!(
        ParentBudget::new(100_001, 64).err().unwrap().code,
        ErrorCode::OutputLimit
    );
    for maximum in [ORDER_METADATA_BYTES, ORDER_PREFLIGHT_BYTES, ORDER_RAW_BYTES] {
        let mut count = 0;
        charge(&mut count, maximum, maximum).unwrap();
        assert_eq!(
            charge(&mut count, 1, maximum).unwrap_err().code,
            ErrorCode::OutputLimit
        );
        assert_eq!(count, maximum);
    }
    let mut overflow = usize::MAX;
    assert_eq!(
        charge(&mut overflow, 1, usize::MAX).unwrap_err().code,
        ErrorCode::OutputLimit
    );
}
#[test]
fn sh01_outside_parent_occurrences_are_charged_without_resolving_or_storing_nodes() {
    let children = ids(1);
    let index = HashMap::from([(children[0].as_str(), 0)]);
    let outside = "f".repeat(64);
    let mut edges = Vec::new();
    let mut budget = ParentBudget::new(1, 64).unwrap();
    budget.parents = ORDER_PARENT_OCCURRENCES - 1;
    record_parent(&outside, &index, &mut edges, &mut budget).unwrap();
    assert!(edges.is_empty());
    assert_eq!(budget.parents, ORDER_PARENT_OCCURRENCES);
    assert_eq!(
        record_parent(&outside, &index, &mut edges, &mut budget)
            .unwrap_err()
            .code,
        ErrorCode::OutputLimit
    );
    assert!(edges.is_empty());
    let mut budget = ParentBudget::new(1, 64).unwrap();
    assert_eq!(
        record_parent("--not-an-object", &index, &mut edges, &mut budget)
            .unwrap_err()
            .code,
        ErrorCode::MalformedOutput
    );
}
#[tokio::test]
async fn sh01_sort_observes_cancel_and_same_deadline_during_cooperative_work() {
    let original = ids(4096);
    let index: HashMap<_, _> = original
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i as u32))
        .collect();
    let parents = vec![vec![]; original.len()];
    let expired = OrderControl {
        deadline: Instant::now(),
        cancel: Default::default(),
    };
    assert_eq!(
        stable_induced_order(&original, &index, &parents, &expired)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
    let control = control();
    let cancel = control.cancel.clone();
    let canceller = async move {
        tokio::task::yield_now().await;
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(
        stable_induced_order(&original, &index, &parents, &control),
        canceller
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Interrupted);
}
