use super::*;

const MINIMAL: &str = r#"{"schema":"dfmcp.furniture-request/1","world_folder":"region1","site":2,"slots":[{"name":"bed","kind":"bed","target":[15,15,2]}]}"#;
const MINIMAL_CANONICAL: &[u8] = br#"{"excluded_items":[],"schema":"dfmcp.furniture-request/1","site":2,"slots":[{"after":[],"kind":"bed","material":null,"max_distance":65532,"name":"bed","subtype":null,"target":[15,15,2]}],"world_folder":"region1"}"#;

fn minimal() -> Result<FurnitureRequest> {
    FurnitureRequest::decode(MINIMAL.as_bytes())
}

fn rejected(raw: &str) {
    assert!(FurnitureRequest::decode(raw.as_bytes()).is_err(), "{raw}");
}

#[test]
fn minimal_canonical_bytes_and_domain_digest_match_independent_python() -> Result<()> {
    let value = minimal()?;
    // scripts/furniture_allocation.py Request.decode + furniture_plan.canonical
    // and Python hashlib generated these fixed expected bytes and digests.
    assert_eq!(value.canonical_bytes(), MINIMAL_CANONICAL);
    assert_eq!(
        value.digest().to_string(),
        "4c528a3b7faf5eaf30ab583e2f5136d5f3b38e0a6f800d25283276af838ddead"
    );
    assert_eq!(value.folder(), "region1");
    assert_eq!(value.site(), 2);
    assert_eq!(value.request().slots[0].max_distance, 65_532);
    assert_eq!(value.request().slots[0].material, None);
    assert_eq!(value.request().slots[0].subtype, None);
    assert!(value.request().excluded_items.is_empty());
    assert_eq!(FurnitureRequest::decode(value.canonical_bytes())?, value);
    assert_eq!(
        FurnitureRequest::new(
            value.folder().to_owned(),
            value.site(),
            value.request().clone()
        )?,
        value
    );
    assert_ne!(value.digest(), Digest32::of_bytes(value.canonical_bytes()));
    Ok(())
}

#[test]
fn constrained_unicode_request_matches_independent_python_canonical_and_digest() -> Result<()> {
    let raw = br#"{"slots":[{"target":[20,15,2],"kind":"table","name":"z","after":["b","a"],"material":[2147483647,2147483647],"subtype":2147483647,"max_distance":0},{"target":[18,15,2],"kind":"chair","name":"b","after":["a"],"material":[419,-1],"subtype":-1,"max_distance":100},{"target":[15,15,2],"kind":"bed","name":"a"}],"site":2147483647,"excluded_items":[2147483646,12,0],"world_folder":"r\u00e9gion\ud83c\udff0/\b\f\n\r\t\u0001\u007f\"\\","schema":"dfmcp.furniture-request/1"}"#;
    let expected = br#"{"excluded_items":[0,12,2147483646],"schema":"dfmcp.furniture-request/1","site":2147483647,"slots":[{"after":[],"kind":"bed","material":null,"max_distance":65532,"name":"a","subtype":null,"target":[15,15,2]},{"after":["a"],"kind":"chair","material":[419,-1],"max_distance":100,"name":"b","subtype":-1,"target":[18,15,2]},{"after":["a","b"],"kind":"table","material":[2147483647,2147483647],"max_distance":0,"name":"z","subtype":2147483647,"target":[20,15,2]}],"world_folder":"r\u00e9gion\ud83c\udff0/\b\f\n\r\t\u0001\u007f\"\\"}"#;
    let value = FurnitureRequest::decode(raw)?;
    assert_eq!(value.canonical_bytes(), expected);
    assert_eq!(
        value.digest().to_string(),
        "4c757ebbfa3828a0efd3fb09aa40b981ae47ce39edc4b45fd30bccbdd4dd0d09"
    );
    assert_eq!(
        value
            .request()
            .slots
            .iter()
            .map(|slot| slot.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "z"]
    );
    assert_eq!(value.request().slots[2].after, ["a", "b"]);
    assert_eq!(FurnitureRequest::decode(expected)?, value);
    Ok(())
}

#[test]
fn raw_utf8_and_escaped_unicode_keys_have_identical_normal_form() -> Result<()> {
    let raw = MINIMAL.replace("region1", "régi城🏰");
    let escaped = MINIMAL
        .replace("region1", r"r\u00e9gi\u57ce\ud83c\udff0")
        .replace("world_folder", r"world_\u0066older")
        .replace("schema", r"sch\u0065ma")
        .replace("furniture-request/1", r"furniture-request\/1")
        .replace("\"bed\"", r#""\u0062ed""#);
    let value = FurnitureRequest::decode(raw.as_bytes())?;
    assert_eq!(FurnitureRequest::decode(escaped.as_bytes())?, value);
    assert_eq!(value.folder(), "régi城🏰");
    assert!(value.canonical_bytes().is_ascii());
    for (raw_folder, escaped_folder) in [
        ("\u{80}", r"\u0080"),
        ("\u{ffff}", r"\uffff"),
        ("\u{10000}", r"\ud800\udc00"),
        ("\u{10ffff}", r"\udbff\udfff"),
    ] {
        assert_eq!(
            FurnitureRequest::decode(MINIMAL.replace("region1", raw_folder).as_bytes())?,
            FurnitureRequest::decode(MINIMAL.replace("region1", escaped_folder).as_bytes())?
        );
    }
    Ok(())
}

#[test]
fn duplicate_escaped_keys_and_unknown_fields_are_rejected_at_both_levels() {
    for replacement in [
        r#""site":2,"site":2"#,
        r#""site":2,"si\u0074e":2"#,
        r#""site":2,"extra":0"#,
        r#""site":2,"excluded_items":[],"excluded_items":[]"#,
    ] {
        rejected(&MINIMAL.replace("\"site\":2", replacement));
    }
    for replacement in [
        r#""name":"bed","na\u006de":"bed""#,
        r#""name":"bed","extra":0"#,
        r#""name":"bed","material":null,"material":null"#,
        r#""name":"bed","subtype":null,"subtype":-1"#,
        r#""name":"bed","after":[],"after":[]"#,
        r#""name":"bed","max_distance":0,"max_distance":1"#,
        r#""name":"bed","kind":"bed""#,
        r#""name":"bed","target":[15,15,2]"#,
    ] {
        rejected(&MINIMAL.replace("\"name\":\"bed\"", replacement));
    }
}

#[test]
fn integers_reject_type_syntax_and_range_substitution() -> Result<()> {
    for number in [
        "null",
        "true",
        "false",
        "\"2\"",
        "2.0",
        "2e0",
        "2E+0",
        "02",
        "+2",
        "- 0",
        "-1",
        "2147483648",
        "4294967296",
        "999999999999999999999999",
    ] {
        rejected(&MINIMAL.replace("\"site\":2", &format!("\"site\":{number}")));
    }
    let zero = FurnitureRequest::decode(MINIMAL.replace("\"site\":2", "\"site\":-0").as_bytes())?;
    assert_eq!(zero.site(), 0);
    let canonical = std::str::from_utf8(MINIMAL_CANONICAL).map_err(|_| invalid("fixture UTF-8"))?;
    for (old, new) in [
        ("\"material\":null", "\"material\":[-1,0]"),
        ("\"material\":null", "\"material\":[0,-2]"),
        ("\"material\":null", "\"material\":[0,2147483648]"),
        ("\"material\":null", "\"material\":[0,1.0]"),
        ("\"material\":null", "\"material\":[]"),
        ("\"material\":null", "\"material\":[0]"),
        ("\"material\":null", "\"material\":[0,1,2]"),
        ("\"material\":null", "\"material\":\"0,1\""),
        ("\"subtype\":null", "\"subtype\":-2"),
        ("\"subtype\":null", "\"subtype\":2147483648"),
        ("\"subtype\":null", "\"subtype\":false"),
        ("\"max_distance\":65532", "\"max_distance\":65533"),
        ("\"max_distance\":65532", "\"max_distance\":-1"),
        ("\"excluded_items\":[]", "\"excluded_items\":[2147483647]"),
        ("\"excluded_items\":[]", "\"excluded_items\":[-1]"),
        ("\"excluded_items\":[]", "\"excluded_items\":[true]"),
    ] {
        rejected(&canonical.replace(old, new));
    }
    Ok(())
}

#[test]
fn only_material_and_subtype_allow_explicit_null() -> Result<()> {
    let canonical = std::str::from_utf8(MINIMAL_CANONICAL).map_err(|_| invalid("fixture UTF-8"))?;
    assert_eq!(FurnitureRequest::decode(MINIMAL_CANONICAL)?, minimal()?);
    for field in [
        "\"excluded_items\":[]",
        "\"schema\":\"dfmcp.furniture-request/1\"",
        "\"site\":2",
        "\"world_folder\":\"region1\"",
        "\"after\":[]",
        "\"kind\":\"bed\"",
        "\"name\":\"bed\"",
        "\"max_distance\":65532",
        "\"target\":[15,15,2]",
    ] {
        let (key, _) = field
            .split_once(':')
            .ok_or_else(|| invalid("fixture field"))?;
        rejected(&canonical.replace(field, &format!("{key}:null")));
    }
    rejected(&MINIMAL.replace(
        r#"[{"name":"bed","kind":"bed","target":[15,15,2]}]"#,
        "null",
    ));
    Ok(())
}

#[test]
fn missing_fields_wrong_containers_and_trailing_data_are_rejected() {
    for field in [
        "\"schema\":\"dfmcp.furniture-request/1\",",
        "\"world_folder\":\"region1\",",
        "\"site\":2,",
        "\"name\":\"bed\",",
        "\"kind\":\"bed\",",
        ",\"target\":[15,15,2]",
    ] {
        rejected(&MINIMAL.replace(field, ""));
    }
    for raw in ["{}", "[]", "null", "true", "0", "", " ", "\"text\""] {
        rejected(raw);
    }
    for replacement in [
        "{}",
        "[]",
        "[15,15]",
        "[15,15,2,0]",
        "[15,15,2,]",
        "\"15,15,2\"",
    ] {
        rejected(&MINIMAL.replace("[15,15,2]", replacement));
    }
    for extra in [",", " null", "{}", "\0"] {
        rejected(&format!("{MINIMAL}{extra}"));
    }
    for end in 0..MINIMAL.len() {
        assert!(FurnitureRequest::decode(&MINIMAL.as_bytes()[..end]).is_err());
    }
}

#[test]
fn normalization_rejects_duplicate_geometry_names_exclusions_and_invalid_dags() -> Result<()> {
    let initial = minimal()?.request().clone();
    let mut cases = Vec::new();
    for name in ["", "bad name", "é", "bad/name"] {
        let mut value = initial.clone();
        value.slots[0].name = name.to_owned();
        cases.push(value);
    }
    for target in [
        [0, 15, 2],
        [15, 0, 2],
        [32767, 15, 2],
        [15, 32767, 2],
        [15, 15, 32768],
    ] {
        let mut value = initial.clone();
        value.slots[0].target = target;
        cases.push(value);
    }
    let mut long_name = initial.clone();
    long_name.slots[0].name = "x".repeat(49);
    cases.push(long_name);
    let mut no_slots = initial.clone();
    no_slots.slots.clear();
    cases.push(no_slots);
    for dependencies in [vec!["bed".to_owned()], vec!["missing".to_owned()]] {
        let mut value = initial.clone();
        value.slots[0].after = dependencies;
        cases.push(value);
    }
    let mut two = initial.clone();
    let mut second = two.slots[0].clone();
    second.name = "chair".to_owned();
    second.target[0] += 1;
    two.slots.push(second);
    let mut duplicate_name = two.clone();
    duplicate_name.slots[1].name = "bed".to_owned();
    cases.push(duplicate_name);
    let mut duplicate_target = two.clone();
    duplicate_target.slots[1].target = duplicate_target.slots[0].target;
    cases.push(duplicate_target);
    let mut duplicate_dependency = two.clone();
    duplicate_dependency.slots[1].after = vec!["bed".to_owned(), "bed".to_owned()];
    cases.push(duplicate_dependency);
    let mut cycle = two;
    cycle.slots[0].after = vec!["chair".to_owned()];
    cycle.slots[1].after = vec!["bed".to_owned()];
    cases.push(cycle);
    let mut duplicate_exclusion = initial;
    duplicate_exclusion.excluded_items = vec![1, 1];
    cases.push(duplicate_exclusion);
    for request in cases {
        assert!(FurnitureRequest::new("region1".to_owned(), 2, request).is_err());
    }
    rejected(&MINIMAL.replace("\"kind\":\"bed\"", "\"kind\":\"door\""));
    Ok(())
}

#[test]
fn exact_integer_name_and_dependency_boundaries_are_preserved() -> Result<()> {
    let mut request = minimal()?.request().clone();
    request.slots[0].name = "a".repeat(48);
    request.slots[0].target = [1, 1, 0];
    request.slots[0].material = Some((0, -1));
    request.slots[0].subtype = Some(-1);
    request.slots[0].max_distance = 0;
    let low = FurnitureRequest::new("region1".to_owned(), 0, request.clone())?;
    assert_eq!(FurnitureRequest::decode(low.canonical_bytes())?, low);
    request.slots[0].target = [32_766, 32_766, 32_767];
    request.slots[0].material = Some((i32::MAX, i32::MAX));
    request.slots[0].subtype = Some(i32::MAX);
    request.slots[0].max_distance = 65_532;
    request.excluded_items = vec![2_147_483_646, 0];
    let high = FurnitureRequest::new("region1".to_owned(), i32::MAX as u32, request)?;
    assert_eq!(FurnitureRequest::decode(high.canonical_bytes())?, high);
    let prototype = minimal()?.request().slots[0].clone();
    let mut slots: Vec<_> = (0..32)
        .map(|index| {
            let mut slot = prototype.clone();
            slot.name = format!("s{index:02}");
            slot.target[0] += index;
            slot
        })
        .collect();
    slots[31].after = slots[..31]
        .iter()
        .map(|slot| slot.name.clone())
        .rev()
        .collect();
    let maximum = FurnitureRequest::new(
        "region1".to_owned(),
        2,
        Request {
            slots: slots.clone(),
            excluded_items: vec![],
        },
    )?;
    assert_eq!(maximum.request().slots[31].after.len(), 31);
    assert_eq!(
        FurnitureRequest::decode(maximum.canonical_bytes())?,
        maximum
    );
    slots[31].after.push("extra".to_owned());
    assert!(
        FurnitureRequest::new(
            "region1".to_owned(),
            2,
            Request {
                slots,
                excluded_items: vec![]
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn unicode_controls_surrogates_and_utf8_are_validated_before_publication() {
    for folder in [
        r"\u0000",
        r"\ud800",
        r"\udc00",
        r"\ud800x",
        r"\ud800\u0000",
        r"\udc00\ud800",
        r"\ud800\ud800",
        r"\uGGGG",
        r"\u123",
        r"\q",
        "bad\nfolder",
        "",
    ] {
        rejected(&MINIMAL.replace("region1", folder));
    }
    let mut raw = MINIMAL.as_bytes().to_vec();
    raw[60] = 0xff;
    assert!(FurnitureRequest::decode(&raw).is_err());
    for prefix in [
        &[0xef, 0xbb, 0xbf][..],
        &[0xc0, 0x80][..],
        &[0xed, 0xa0, 0x80][..],
    ] {
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(MINIMAL.as_bytes());
        assert!(FurnitureRequest::decode(&bytes).is_err());
    }
}

#[test]
fn folder_bound_counts_utf8_bytes_and_site_bound_applies_to_typed_constructor() -> Result<()> {
    let request = minimal()?.request().clone();
    for folder in ["x".repeat(512), "é".repeat(256), "🏰".repeat(128)] {
        let value = FurnitureRequest::new(folder.clone(), i32::MAX as u32, request.clone())?;
        assert_eq!(value.folder().len(), 512);
        assert_eq!(FurnitureRequest::decode(value.canonical_bytes())?, value);
    }
    for folder in [
        "x".repeat(513),
        "é".repeat(257),
        "🏰".repeat(129),
        "".to_owned(),
        "x\0".to_owned(),
    ] {
        assert!(FurnitureRequest::new(folder, 2, request.clone()).is_err());
    }
    assert!(FurnitureRequest::new("region1".to_owned(), i32::MAX as u32 + 1, request).is_err());
    Ok(())
}

#[test]
fn input_depth_and_byte_bounds_are_checked_independently_of_normalized_size() -> Result<()> {
    let mut padded = MINIMAL.as_bytes().to_vec();
    padded.resize(MAX_REQUEST_BYTES, b' ');
    assert_eq!(FurnitureRequest::decode(&padded)?, minimal()?);
    padded.push(b' ');
    assert!(FurnitureRequest::decode(&padded).is_err());
    assert!(check_depth(b"[[[[[[[[]]]]]]]]").is_ok());
    assert!(check_depth(b"[[[[[[[[[]]]]]]]]]").is_err());
    assert!(check_depth(br#"{"quoted":"[[[[[[[[[[\\\""}"#).is_ok());
    rejected(&MINIMAL.replace("[15,15,2]", "[[[[[[[[[0]]]]]]]]]"));
    Ok(())
}

#[test]
fn slot_collection_and_normalized_output_bounds_prevent_partial_requests() -> Result<()> {
    let initial = minimal()?;
    let mut request = initial.request().clone();
    request.slots = (0..32)
        .rev()
        .map(|index| {
            let mut slot = request.slots[0].clone();
            slot.name = format!("slot-{index:02}");
            slot.target[0] += index;
            slot
        })
        .collect();
    let maximum = FurnitureRequest::new("region1".to_owned(), 2, request.clone())?;
    assert_eq!(maximum.request().slots.len(), 32);
    assert_eq!(
        FurnitureRequest::decode(maximum.canonical_bytes())?,
        maximum
    );
    let mut extra = request.slots[0].clone();
    extra.name = "extra".to_owned();
    extra.target[0] = 100;
    request.slots.push(extra);
    assert!(FurnitureRequest::new("region1".to_owned(), 2, request).is_err());
    let mut exclusions = initial.request().clone();
    exclusions.excluded_items = (0..3300).rev().collect();
    let accepted = FurnitureRequest::new("region1".to_owned(), 2, exclusions.clone())?;
    assert_eq!(accepted.request().excluded_items.len(), 3300);
    assert!(accepted.canonical_bytes().len() <= MAX_REQUEST_BYTES);
    exclusions.excluded_items = (0..4096).collect();
    assert!(FurnitureRequest::new("region1".to_owned(), 2, exclusions.clone()).is_err());
    exclusions.excluded_items.push(4096);
    assert!(FurnitureRequest::new("region1".to_owned(), 2, exclusions).is_err());
    // Compact input omits defaults; expanded normalized output must still fit.
    let excluded = (0..3250)
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let source = MINIMAL
        .replace(
            "\"site\":2",
            &format!("\"site\":2,\"excluded_items\":[{excluded}]"),
        )
        .replace("region1", &"é".repeat(256));
    assert!(source.len() <= MAX_REQUEST_BYTES);
    assert!(FurnitureRequest::decode(source.as_bytes()).is_err());
    Ok(())
}
