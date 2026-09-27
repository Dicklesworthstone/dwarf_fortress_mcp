use super::*;

fn slot(name: &str, x: u32) -> Slot {
    Slot {
        name: name.to_owned(),
        kind: Kind::Bed,
        target: [x, 10, 2],
        after: Vec::new(),
        material: None,
        subtype: None,
        max_distance: MAX_DISTANCE,
    }
}
fn item(native_id: u32, x: u32) -> Candidate {
    Candidate {
        native_id,
        kind: Kind::Bed,
        position: [x, 10, 2],
        material_type: 1,
        material_index: -1,
        subtype: -1,
    }
}
fn request(slots: Vec<Slot>) -> Request {
    Request {
        slots,
        excluded_items: Vec::new(),
    }
}
fn idle() -> Result<()> {
    Ok(())
}
fn allocated_ids(value: &Allocation) -> Vec<u32> {
    value
        .assignments
        .iter()
        .map(|entry| entry.item_id)
        .collect()
}

#[derive(Debug, Default)]
struct Oracle {
    maximum: usize,
    optimum: Option<(u64, Vec<u32>)>,
}

// Exhaustive assignments and unmatched rows, independent of augmentation,
// pruning and Hungarian potentials. Only used with tiny test graphs.
fn brute(edges: &Edges) -> Oracle {
    fn visit(
        edges: &Edges,
        row: usize,
        used: &mut BTreeSet<u32>,
        chosen: &mut Vec<u32>,
        total: u64,
        result: &mut Oracle,
    ) {
        if row == edges.len() {
            result.maximum = result.maximum.max(used.len());
            if used.len() == edges.len() {
                let objective = (total, chosen.clone());
                if result
                    .optimum
                    .as_ref()
                    .is_none_or(|prior| &objective < prior)
                {
                    result.optimum = Some(objective);
                }
            }
            return;
        }
        visit(edges, row + 1, used, chosen, total, result);
        for (&identity, &distance) in &edges[row] {
            if used.insert(identity) {
                chosen.push(identity);
                visit(
                    edges,
                    row + 1,
                    used,
                    chosen,
                    total + u64::from(distance),
                    result,
                );
                chosen.pop();
                used.remove(&identity);
            }
        }
    }
    let mut result = Oracle::default();
    visit(
        edges,
        0,
        &mut BTreeSet::new(),
        &mut Vec::new(),
        0,
        &mut result,
    );
    result
}

fn assert_hall_witness(slots: &[Slot], edges: &Edges, shortage: &Shortage) {
    let rows: Vec<_> = slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| shortage.slots.contains(&slot.name))
        .map(|(index, _)| index)
        .collect();
    let neighbors: BTreeSet<_> = rows
        .iter()
        .flat_map(|&row| edges[row].keys().copied())
        .collect();
    assert_eq!(shortage.slots.len(), rows.len());
    assert_eq!(
        shortage.candidate_items,
        neighbors.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(
        shortage.missing as usize,
        rows.len() - shortage.candidate_items.len()
    );
    assert!(shortage.missing > 0);
}

#[test]
fn all_4096_three_by_four_graphs_match_independent_exhaustive_oracle() -> Result<()> {
    let slots: Vec<_> = (0..3).map(|row| slot(&row.to_string(), row + 1)).collect();
    for mask in 0u32..4096 {
        let edges: Edges = (0..3)
            .map(|row| {
                (0..4)
                    .filter(|&identity| mask & (1 << (row * 4 + identity)) != 0)
                    .map(|identity| (identity, (row * 7 + identity * 3 + mask) % 11))
                    .collect()
            })
            .collect();
        let expected = brute(&edges);
        let counts: Vec<_> = edges.iter().map(|row| row.len() as u32).collect();
        let (maximum, shortage) = maximum_matching(&edges, &slots, &counts, &mut idle)?;
        assert_eq!(maximum, expected.maximum, "graph={mask}");
        if let Some(optimum) = expected.optimum {
            assert!(shortage.is_none(), "graph={mask}");
            let chosen = minimum_cost(&edges, &mut idle)?;
            let total: u64 = chosen
                .iter()
                .enumerate()
                .map(|(row, id)| u64::from(edges[row][id]))
                .sum();
            assert_eq!((total, chosen), optimum, "graph={mask}");
        } else {
            assert_hall_witness(
                &slots,
                &edges,
                &shortage.expect("infeasible graph has a witness"),
            );
        }
    }
    Ok(())
}

#[test]
fn global_assignment_avoids_greedy_material_starvation() -> Result<()> {
    let mut constrained = slot("b", 11);
    constrained.material = Some((7, 8));
    let requested = request(vec![slot("a", 10), constrained]);
    let mut scarce = item(1, 10);
    scarce.material_type = 7;
    scarce.material_index = 8;
    let result = allocate(&requested, &[scarce, item(2, 20)], &mut idle)?;
    assert_eq!(allocated_ids(&result), [2, 1]);
    assert_eq!(result.total_distance, Some(11));
    assert_eq!(
        result.compatible_counts,
        [("a".to_owned(), 2), ("b".to_owned(), 1)]
    );
    assert_eq!(result.maximum_assignable, 2);
    Ok(())
}

#[test]
fn total_distance_precedes_exact_id_vector_in_lexical_slot_order() -> Result<()> {
    let mut dependent = slot("a", 10);
    dependent.after = vec!["z".to_owned()];
    let requested = request(vec![slot("z", 12), dependent]);
    let result = allocate(
        &requested,
        &[item(0, 30), item(MAX_ITEM_ID, 10), item(7, 12)],
        &mut idle,
    )?;
    assert_eq!(allocated_ids(&result), [MAX_ITEM_ID, 7]);
    assert_eq!(result.total_distance, Some(0));
    let equal = allocate(&requested, &[item(MAX_ITEM_ID, 11), item(0, 11)], &mut idle)?;
    assert_eq!(allocated_ids(&equal), [0, MAX_ITEM_ID]);
    assert_eq!(equal.total_distance, Some(2));
    let normalized = normalize(&requested, &mut idle)?;
    assert_eq!(normalized.slots[0].name, "a");
    assert_eq!(normalized.slots[0].after, ["z"]);
    Ok(())
}

#[test]
fn joint_shortage_can_exist_when_every_slot_has_compatible_supply() -> Result<()> {
    let mut only_second = slot("c", 100);
    only_second.material = Some((8, 9));
    let requested = request(vec![slot("a", 10), slot("b", 11), only_second]);
    let mut second = item(5, 100);
    second.material_type = 8;
    second.material_index = 9;
    let result = allocate(&requested, &[item(4, 10), second], &mut idle)?;
    assert_eq!(result.maximum_assignable, 2);
    assert!(result.assignments.is_empty());
    assert_eq!(result.total_distance, None);
    assert!(result.compatible_counts.iter().all(|(_, count)| *count > 0));
    assert_eq!(
        result.shortage,
        Some(Shortage {
            slots: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            candidate_items: vec![4, 5],
            missing: 1,
        })
    );
    Ok(())
}

#[test]
fn empty_inventory_has_complete_explicit_shortage() -> Result<()> {
    let result = allocate(&request(vec![slot("bed", 10)]), &[], &mut idle)?;
    assert_eq!(result.maximum_assignable, 0);
    assert_eq!(result.compatible_counts, [("bed".to_owned(), 0)]);
    assert!(result.assignments.is_empty());
    assert_eq!(result.total_distance, None);
    assert_eq!(
        result.shortage,
        Some(Shortage {
            slots: vec!["bed".to_owned()],
            candidate_items: vec![],
            missing: 1,
        })
    );
    Ok(())
}

#[test]
fn same_level_distance_material_subtype_kind_and_exclusions_are_hard_constraints() -> Result<()> {
    let mut target = slot("bed", 10);
    target.material = Some((1, -1));
    target.subtype = Some(-1);
    target.max_distance = 2;
    let mut requested = request(vec![target]);
    requested.excluded_items = vec![6];
    let mut candidates: Vec<_> = (0..8).map(|id| item(id, 10)).collect();
    candidates[0].position[2] = 3;
    candidates[1].position[0] = 13;
    candidates[2].material_type = 2;
    candidates[3].subtype = 5;
    candidates[4].kind = Kind::Chair;
    candidates[5].kind = Kind::Table;
    candidates[7].position[0] = 12;
    let result = allocate(&requested, &candidates, &mut idle)?;
    assert_eq!(result.compatible_counts, [("bed".to_owned(), 1)]);
    assert_eq!(allocated_ids(&result), [7]);
    assert_eq!(result.total_distance, Some(2));
    Ok(())
}

// A fixed, platform-independent PRNG avoids adding a dependency to generate
// varied full graphs. Eligibility below is recomputed without distance().
struct Random(u32);
impl Random {
    fn next(&mut self, bound: u32) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) % bound
    }
}

#[test]
fn varied_inventories_match_the_full_unpruned_permutation_oracle() -> Result<()> {
    let mut random = Random(11_914);
    for trial in 0..200 {
        let n = 1 + random.next(4);
        let m = random.next(8);
        let slots: Vec<_> = (0..n)
            .map(|row| {
                let mut value = slot(&row.to_string(), row + 5);
                value.material = if random.next(2) == 0 {
                    Some((random.next(2) as i32, -1))
                } else {
                    None
                };
                value.max_distance = 1 + random.next(15);
                value
            })
            .collect();
        let mut requested = request(slots);
        if m > 1 && trial % 3 == 0 {
            requested.excluded_items.push(1);
        }
        let candidates: Vec<_> = (0..m)
            .map(|id| {
                let mut value = item(id, random.next(25));
                value.material_type = random.next(2) as i32;
                value
            })
            .collect();
        let edges: Edges = requested
            .slots
            .iter()
            .map(|slot| {
                candidates
                    .iter()
                    .filter_map(|candidate| {
                        let compatible_material = slot.material.is_none_or(|material| {
                            material == (candidate.material_type, candidate.material_index)
                        });
                        let distance = (i64::from(slot.target[0])
                            - i64::from(candidate.position[0]))
                        .unsigned_abs() as u32;
                        (compatible_material
                            && distance <= slot.max_distance
                            && !requested.excluded_items.contains(&candidate.native_id))
                        .then_some((candidate.native_id, distance))
                    })
                    .collect()
            })
            .collect();
        let expected = brute(&edges);
        let result = allocate(&requested, &candidates, &mut idle)?;
        assert_eq!(result.maximum_assignable, expected.maximum, "trial={trial}");
        if let Some((total, chosen)) = expected.optimum {
            assert_eq!(result.total_distance, Some(total), "trial={trial}");
            assert_eq!(allocated_ids(&result), chosen, "trial={trial}");
        } else {
            assert_hall_witness(
                &requested.slots,
                &edges,
                &result.shortage.expect("infeasible graph has a witness"),
            );
        }
    }
    Ok(())
}

#[test]
fn full_65536_item_32_slot_inventory_is_exact_within_shared_work_ceiling() -> Result<()> {
    let requested = request(
        (0..32)
            .map(|row| slot(&format!("{row:02}"), row + 1))
            .collect(),
    );
    let mut candidates: Vec<_> = (0..65_536).map(|id| item(id, 50)).collect();
    let mut work = 0u64;
    let result = allocate(&requested, &candidates, &mut || {
        work += 1;
        if work > crate::operations_analysis::MAX_ANALYSIS_WORK {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "test work ceiling exceeded",
            ));
        }
        Ok(())
    })?;
    assert_eq!(allocated_ids(&result), (0..32).collect::<Vec<_>>());
    assert_eq!(
        result.total_distance,
        Some((0..32).map(|row| 49u64 - row).sum())
    );
    assert!(
        result
            .compatible_counts
            .iter()
            .all(|(_, count)| *count == 65_536)
    );
    assert!(work > 65_536 * 32);
    candidates.push(item(65_536, 50));
    assert_eq!(
        allocate(&requested, &candidates, &mut idle)
            .expect_err("oversized roster")
            .code,
        ErrorCode::InvalidRequest
    );
    Ok(())
}

#[test]
fn maximum_1024_reduced_union_is_solved_without_dense_full_inventory() -> Result<()> {
    let requested = request(
        (0..32)
            .map(|row| {
                let mut value = slot(&format!("{row:02}"), 1 + row * 100);
                value.max_distance = 0;
                value
            })
            .collect(),
    );
    let candidates: Vec<_> = (0..32)
        .flat_map(|row| (0..32).map(move |column| item(row * 32 + column, 1 + row * 100)))
        .collect();
    let result = allocate(&requested, &candidates, &mut idle)?;
    assert_eq!(
        allocated_ids(&result),
        (0..32).map(|row| row * 32).collect::<Vec<_>>()
    );
    assert_eq!(result.total_distance, Some(0));
    Ok(())
}

#[test]
fn maximum_coordinates_ids_and_32_lexical_dimensions_remain_exact() -> Result<()> {
    let requested = request(
        (0..32)
            .map(|row| {
                let mut value = slot(&format!("{row:02}"), row + 1);
                value.target = [row + 1, 1, 32_767];
                value
            })
            .collect(),
    );
    let candidates: Vec<_> = (0..32)
        .map(|offset| {
            let mut value = item(MAX_ITEM_ID - offset, 32_767);
            value.position = [32_767, 32_767, 32_767];
            value
        })
        .collect();
    let result = allocate(&requested, &candidates, &mut idle)?;
    assert_eq!(
        allocated_ids(&result),
        (MAX_ITEM_ID - 31..=MAX_ITEM_ID).collect::<Vec<_>>()
    );
    assert_eq!(
        result.total_distance,
        Some((0..32).map(|row| u64::from(MAX_DISTANCE - row)).sum())
    );
    Ok(())
}

#[test]
fn permutations_of_slots_candidates_dependencies_and_exclusions_are_identical() -> Result<()> {
    let mut final_slot = slot("c", 15);
    final_slot.after = vec!["b".to_owned(), "a".to_owned()];
    let slots = [final_slot, slot("b", 11), slot("a", 10)];
    let candidates = [item(9, 11), item(1, 14), item(4, 10)];
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let expected = allocate(&request(slots.to_vec()), &candidates, &mut idle)?;
    for slot_order in permutations {
        for candidate_order in permutations {
            let mut requested = request(slot_order.map(|index| slots[index].clone()).to_vec());
            requested.excluded_items = vec![42, 41];
            for slot in &mut requested.slots {
                slot.after.reverse();
            }
            let actual = allocate(
                &requested,
                &candidate_order.map(|index| candidates[index]),
                &mut idle,
            )?;
            assert_eq!(actual, expected);
            let normalized = normalize(&requested, &mut idle)?;
            assert_eq!(normalized.excluded_items, [41, 42]);
            assert_eq!(normalized.slots[2].after, ["a", "b"]);
        }
    }
    Ok(())
}

#[test]
fn request_validation_rejects_invalid_bounds_aliases_and_dependency_graphs() {
    let valid = request(vec![slot("bed", 10)]);
    let mut bad = vec![request(vec![]), request(vec![slot("bed", 10); 33])];
    let mut duplicate = request(vec![slot("a", 10), slot("a", 11)]);
    bad.push(duplicate.clone());
    duplicate.slots[1].name = "b".to_owned();
    duplicate.slots[1].target = duplicate.slots[0].target;
    bad.push(duplicate);
    for name in ["", "a b", "nonasciié", "bad/slash", &"a".repeat(49)] {
        let mut changed = valid.clone();
        changed.slots[0].name = name.to_owned();
        bad.push(changed);
    }
    for target in [
        [0, 1, 0],
        [1, 0, 0],
        [32_767, 1, 0],
        [1, 32_767, 0],
        [1, 1, 32_768],
    ] {
        let mut changed = valid.clone();
        changed.slots[0].target = target;
        bad.push(changed);
    }
    for material in [(-1, -1), (1, -2)] {
        let mut changed = valid.clone();
        changed.slots[0].material = Some(material);
        bad.push(changed);
    }
    let mut changed = valid.clone();
    changed.slots[0].subtype = Some(-2);
    bad.push(changed);
    let mut changed = valid.clone();
    changed.slots[0].max_distance = MAX_DISTANCE + 1;
    bad.push(changed);
    for dependencies in [
        vec!["bed".to_owned()],
        vec!["missing".to_owned()],
        vec!["a".to_owned(); 32],
    ] {
        let mut changed = valid.clone();
        changed.slots[0].after = dependencies;
        bad.push(changed);
    }
    let mut dependent = request(vec![slot("a", 10), slot("b", 11)]);
    dependent.slots[0].after = vec!["b".to_owned(), "b".to_owned()];
    bad.push(dependent.clone());
    dependent.slots[0].after.pop();
    dependent.slots[1].after.push("a".to_owned());
    bad.push(dependent);
    for excluded_items in [vec![1, 1], vec![MAX_ITEM_ID + 1], vec![0; MAX_EXCLUDED + 1]] {
        let mut changed = valid.clone();
        changed.excluded_items = excluded_items;
        bad.push(changed);
    }
    for requested in bad {
        assert_eq!(
            allocate(&requested, &[], &mut idle)
                .expect_err("invalid request")
                .code,
            ErrorCode::InvalidRequest,
            "{requested:?}"
        );
    }
}

#[test]
fn candidates_are_validated_even_when_excluded_or_kind_incompatible() {
    let mut requested = request(vec![slot("bed", 10)]);
    requested.excluded_items = vec![0];
    let mut irrelevant = item(0, 10);
    irrelevant.kind = Kind::Chair;
    assert_eq!(
        allocate(&requested, &[irrelevant, irrelevant], &mut idle)
            .expect_err("duplicate excluded item")
            .code,
        ErrorCode::InvalidRequest
    );
    let mut invalid = vec![];
    let mut changed = irrelevant;
    changed.native_id = MAX_ITEM_ID + 1;
    invalid.push(changed);
    let mut changed = irrelevant;
    changed.position = [0, 0, 32_768];
    invalid.push(changed);
    let mut changed = irrelevant;
    changed.material_type = -1;
    invalid.push(changed);
    let mut changed = irrelevant;
    changed.material_index = -2;
    invalid.push(changed);
    let mut changed = irrelevant;
    changed.subtype = -2;
    invalid.push(changed);
    for candidate in invalid {
        assert_eq!(
            allocate(&requested, &[candidate], &mut idle)
                .expect_err("malformed candidate")
                .code,
            ErrorCode::InvalidRequest
        );
    }
}

#[test]
fn work_and_cancellation_at_every_guard_boundary_never_return_partial_success() -> Result<()> {
    let requested = request(vec![slot("a", 10), slot("b", 11), slot("c", 12)]);
    let candidates = [item(0, 10), item(1, 11), item(2, 12), item(3, 13)];
    let mut total = 0usize;
    allocate(&requested, &candidates, &mut || {
        total += 1;
        Ok(())
    })?;
    for (code, message) in [
        (ErrorCode::BudgetExceeded, "work exhausted"),
        (ErrorCode::CancellationRequested, "cancelled"),
    ] {
        for limit in 0..total {
            let mut seen = 0usize;
            let actual = allocate(&requested, &candidates, &mut || {
                seen += 1;
                if seen > limit {
                    Err(DfmcpError::new(code, message))
                } else {
                    Ok(())
                }
            });
            let error = actual.expect_err("every injected interruption propagates");
            assert_eq!(error.code, code, "boundary={limit}");
            assert_eq!(error.message, message);
        }
    }
    Ok(())
}

#[test]
fn lexicographic_cost_checked_arithmetic_and_kind_keys() -> Result<()> {
    let high = LexCost::edge(MAX_DISTANCE, MAX_ITEM_ID, MAX_SLOTS - 1);
    assert_eq!(high.combine(high, true)?, LexCost::ZERO);
    let low = LexCost::edge(0, MAX_ITEM_ID, 0);
    assert!(low < high);
    let mut overflow = LexCost::ZERO;
    overflow.ids[0] = i64::MAX;
    assert_eq!(
        overflow
            .combine(low, false)
            .expect_err("checked ID overflow")
            .code,
        ErrorCode::InternalInvariantViolation
    );
    overflow.ids[0] = 0;
    overflow.distance = i64::MIN;
    assert_eq!(
        overflow
            .combine(high, true)
            .expect_err("checked distance overflow")
            .code,
        ErrorCode::InternalInvariantViolation
    );
    assert_eq!(
        [
            Kind::Bed.as_str(),
            Kind::Chair.as_str(),
            Kind::Table.as_str()
        ],
        ["bed", "chair", "table"]
    );
    assert_eq!(
        [
            Kind::Bed.native_key(),
            Kind::Chair.native_key(),
            Kind::Table.native_key()
        ],
        ["BED", "CHAIR", "TABLE"]
    );
    Ok(())
}
