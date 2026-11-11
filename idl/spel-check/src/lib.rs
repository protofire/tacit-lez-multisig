#[allow(dead_code)]
mod idl; // fetched by run.sh (gitignored)

#[cfg(test)]
mod tests {
    use super::idl::{IdlType, SpelIdl};

    #[test]
    fn msig_idl_deserialises_with_spel_types() {
        let s = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../msig.idl.json")).unwrap();
        let idl: SpelIdl = serde_json::from_str(&s).unwrap();
        let names: Vec<_> = idl.instructions.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["create", "execute", "queue", "execute_queued", "spend"]);
        assert_eq!(idl.errors.len(), 22);
        assert!(idl.types.iter().any(|t| t.name == "Instruction" && t.kind == "enum"));
        // round trip keeps the typed fields
        let again: SpelIdl = serde_json::from_str(&idl.to_json_pretty().unwrap()).unwrap();
        assert!(matches!(again.instructions[0].args[2].type_, IdlType::Option { .. }));
    }
}
