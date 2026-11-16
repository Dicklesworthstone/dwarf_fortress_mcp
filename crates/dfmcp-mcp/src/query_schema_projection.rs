//! Preserve the complete validation grammar when schema annotations exceed
//! the response allowance. Call only with the embedded, trusted schema.
use serde_json::Value;

pub(super) fn without_descriptions(schema: &Value) -> Value {
    let mut result = schema.clone();
    strip_descriptions(&mut result);
    result
}

fn strip_descriptions(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    object.remove("description");
    // These objects map names to schemas. A property/definition literally named
    // `description` is a real constraint and must remain in the map.
    for keyword in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
        "dependencies",
    ] {
        if let Some(children) = object.get_mut(keyword).and_then(Value::as_object_mut) {
            for child in children.values_mut() {
                strip_descriptions(child);
            }
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(children) = object.get_mut(keyword).and_then(Value::as_array_mut) {
            for child in children {
                strip_descriptions(child);
            }
        }
    }
    for keyword in [
        "additionalProperties",
        "unevaluatedProperties",
        "additionalItems",
        "unevaluatedItems",
        "contains",
        "contentSchema",
        "propertyNames",
        "not",
        "if",
        "then",
        "else",
    ] {
        if let Some(child) = object.get_mut(keyword) {
            strip_descriptions(child);
        }
    }
    if let Some(items) = object.get_mut("items") {
        match items {
            Value::Array(children) => {
                for child in children {
                    strip_descriptions(child);
                }
            }
            child => strip_descriptions(child),
        }
    }
    // Never walk into const, enum, default, examples or arbitrary extension
    // data: a key called description there is user data, not an annotation.
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_named_properties_and_complete_validation_constraints() {
        let source = json!({"type":"object","description":"root help",
            "required":["description"],"additionalProperties":false,
            "properties":{"description":{"type":"string","description":"field help","minLength":1}},
            "$defs":{"description":{"type":"integer","minimum":1,"description":"definition help"}},
            "oneOf":[{"properties":{"kind":{"const":"history","description":"kind help"}}}]});
        let projected = without_descriptions(&source);
        assert_eq!(
            projected,
            json!({"type":"object","required":["description"],"additionalProperties":false,
            "properties":{"description":{"type":"string","minLength":1}},
            "$defs":{"description":{"type":"integer","minimum":1}},
            "oneOf":[{"properties":{"kind":{"const":"history"}}}]})
        );
        assert_eq!(source["description"], "root help");
    }

    #[test]
    fn literal_values_and_extension_data_are_not_schema_nodes() {
        let literal = json!({"description":"this is a required literal value"});
        let source = json!({"description":"schema help","const":literal,
            "enum":[literal],"default":literal,"examples":[literal],"x-data":literal});
        let mut expected = source.clone();
        expected
            .as_object_mut()
            .expect("fixture object")
            .remove("description");
        assert_eq!(without_descriptions(&source), expected);
    }

    #[test]
    fn boolean_schemas_dependencies_arrays_and_conditions_are_preserved() {
        let source = json!({"items":[true,{"description":"help","type":"string"}],
            "prefixItems":[false,{"description":"help","minimum":1}],
            "dependencies":{"description":["other"],"conditional":{"description":"help","required":["other"]}},
            "if":{"description":"help","type":"integer"},
            "then":{"description":"help","minimum":1},"else":false});
        assert_eq!(
            without_descriptions(&source),
            json!({"items":[true,{"type":"string"}],
            "prefixItems":[false,{"minimum":1}],
            "dependencies":{"description":["other"],"conditional":{"required":["other"]}},
            "if":{"type":"integer"},"then":{"minimum":1},"else":false})
        );
        assert_eq!(without_descriptions(&json!(true)), json!(true));
    }
}
