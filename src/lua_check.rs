//! Sandboxed Lua composition for product-specific eval checks.
//!
//! Rust owns identity resolution and predicate truth. Lua can only compose
//! `check.require_role(role)` and `check.require_predicate(kind, roles)`.
//! It cannot inspect names, raw geometry, files, processes, or backend state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, StdLib, Table, VmState};
use serde::{Deserialize, Serialize};

pub const FACTS_SCHEMA: &str = "cadbench.eval-facts.v1";
pub const EXPECTED_SCHEMA: &str = "cadbench.lua-check-input.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyFacts {
    pub schema: String,
    pub parts: Vec<PartFact>,
    pub predicates: Vec<PredicateFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartFact {
    /// Stable machine identity. Generated display names are intentionally not
    /// part of this public contract.
    pub id: String,
    /// Controlled semantic role used by task checks.
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateFact {
    pub kind: String,
    /// Stable part IDs, in predicate-defined order.
    pub subjects: Vec<String>,
    pub pass: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedInput {
    schema: String,
    #[serde(default)]
    required_roles: Vec<String>,
    #[serde(default)]
    predicates: Vec<ExpectedPredicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedPredicate {
    kind: String,
    roles: Vec<String>,
}

/// Executes one check. An invalid contract is an error, not a failed score;
/// a valid assertion that the artifact does not meet is returned as `Ok(Err)`.
pub fn evaluate(
    script_path: &Path,
    expected_path: &Path,
    facts: &AssemblyFacts,
) -> Result<Result<String, String>, String> {
    validate_facts(facts)?;
    let expected_text = std::fs::read_to_string(expected_path)
        .map_err(|e| format!("reading {}: {e}", expected_path.display()))?;
    let expected: ExpectedInput = serde_json::from_str(&expected_text)
        .map_err(|e| format!("parsing {}: {e}", expected_path.display()))?;
    if expected.schema != EXPECTED_SCHEMA {
        return Err(format!(
            "{}: expected schema {EXPECTED_SCHEMA:?}, got {:?}",
            expected_path.display(),
            expected.schema
        ));
    }
    let script = std::fs::read_to_string(script_path)
        .map_err(|e| format!("reading {}: {e}", script_path.display()))?;

    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .map_err(|e| format!("creating Lua sandbox: {e}"))?;
    let math: Table = lua.globals().get("math").map_err(show_lua)?;
    math.set("random", mlua::Value::Nil).map_err(show_lua)?;
    math.set("randomseed", mlua::Value::Nil).map_err(show_lua)?;
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1_000),
        move |_, _| {
            if ticks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 100 {
                Err(LuaError::external(
                    "Lua check exceeded 100,000 instructions",
                ))
            } else {
                Ok(VmState::Continue)
            }
        },
    );
    let check = lua.create_table().map_err(show_lua)?;

    let roles = role_index(facts);
    let role_fn = lua
        .create_function(move |_, role: String| match roles.get(&role) {
            Some(ids) if ids.len() == 1 => Ok(()),
            Some(ids) => Err(LuaError::external(format!(
                "cadbench assertion: role {role:?} is ambiguous: {} parts ({})",
                ids.len(),
                ids.join(", ")
            ))),
            None => Err(LuaError::external(format!(
                "cadbench assertion: required role {role:?} is missing"
            ))),
        })
        .map_err(show_lua)?;
    check.set("require_role", role_fn).map_err(show_lua)?;

    let facts_for_predicates = facts.clone();
    let predicate_fn = lua
        .create_function(move |_, (kind, requested_roles): (String, Table)| {
            require_predicate(&facts_for_predicates, &kind, requested_roles)
                .map_err(LuaError::external)
        })
        .map_err(show_lua)?;
    check
        .set("require_predicate", predicate_fn)
        .map_err(show_lua)?;

    let expected_roles = expected.required_roles.clone();
    let facts_for_expected_roles = facts.clone();
    check
        .set(
            "require_expected_roles",
            lua.create_function(move |_, ()| {
                let roles = role_index(&facts_for_expected_roles);
                for role in &expected_roles {
                    match roles.get(role) {
                        Some(ids) if ids.len() == 1 => {}
                        Some(ids) => {
                            return Err(LuaError::external(format!(
                                "cadbench assertion: role {role:?} is ambiguous: {} parts",
                                ids.len()
                            )))
                        }
                        None => {
                            return Err(LuaError::external(format!(
                                "cadbench assertion: required role {role:?} is missing"
                            )))
                        }
                    }
                }
                Ok(())
            })
            .map_err(show_lua)?,
        )
        .map_err(show_lua)?;
    let expected_predicates = expected.predicates.clone();
    let facts_for_expected_predicates = facts.clone();
    check
        .set(
            "require_expected_predicates",
            lua.create_function(move |lua, ()| {
                for expected in &expected_predicates {
                    let table = lua.create_sequence_from(expected.roles.clone())?;
                    require_predicate(&facts_for_expected_predicates, &expected.kind, table)
                        .map_err(LuaError::external)?;
                }
                Ok(())
            })
            .map_err(show_lua)?,
        )
        .map_err(show_lua)?;
    lua.globals().set("check", check).map_err(show_lua)?;

    match lua
        .load(&script)
        .set_name(script_path.display().to_string())
        .exec()
    {
        Ok(()) => Ok(Ok("Lua check passed".to_owned())),
        Err(error) if error.to_string().contains("cadbench assertion:") => {
            Ok(Err(error.to_string()))
        }
        Err(error) => Err(format!("{}: {error}", script_path.display())),
    }
}

fn validate_facts(facts: &AssemblyFacts) -> Result<(), String> {
    if facts.schema != FACTS_SCHEMA {
        return Err(format!(
            "expected facts schema {FACTS_SCHEMA:?}, got {:?}",
            facts.schema
        ));
    }
    let mut ids = BTreeSet::new();
    for part in &facts.parts {
        if part.id.trim().is_empty() || part.role.trim().is_empty() {
            return Err("part IDs and roles must be non-empty".to_owned());
        }
        if !ids.insert(&part.id) {
            return Err(format!("duplicate stable part ID {:?}", part.id));
        }
    }
    for predicate in &facts.predicates {
        if predicate.kind.trim().is_empty() {
            return Err("predicate kind must be non-empty".to_owned());
        }
        for subject in &predicate.subjects {
            if !ids.contains(subject) {
                return Err(format!(
                    "predicate {:?} references unknown part ID {subject:?}",
                    predicate.kind
                ));
            }
        }
    }
    Ok(())
}

fn role_index(facts: &AssemblyFacts) -> BTreeMap<String, Vec<String>> {
    let mut roles = BTreeMap::<String, Vec<String>>::new();
    for part in &facts.parts {
        roles
            .entry(part.role.clone())
            .or_default()
            .push(part.id.clone());
    }
    roles
}

fn require_predicate(facts: &AssemblyFacts, kind: &str, requested: Table) -> Result<(), String> {
    let roles = role_index(facts);
    let requested: Vec<String> = requested
        .sequence_values::<String>()
        .collect::<mlua::Result<_>>()
        .map_err(|e| format!("cadbench assertion: invalid roles: {e}"))?;
    let mut subjects = Vec::with_capacity(requested.len());
    for role in &requested {
        match roles.get(role) {
            Some(ids) if ids.len() == 1 => subjects.push(ids[0].clone()),
            Some(ids) => {
                return Err(format!(
                    "cadbench assertion: role {role:?} is ambiguous: {} parts",
                    ids.len()
                ))
            }
            None => {
                return Err(format!(
                    "cadbench assertion: required role {role:?} is missing"
                ))
            }
        }
    }
    match facts
        .predicates
        .iter()
        .find(|fact| fact.kind == kind && fact.subjects == subjects)
    {
        Some(fact) if fact.pass => Ok(()),
        Some(fact) => Err(format!(
            "cadbench assertion: {kind} failed: {}",
            fact.detail
        )),
        None => Err(format!(
            "cadbench assertion: backend supplied no {kind:?} predicate for roles {requested:?}"
        )),
    }
}

fn show_lua(error: LuaError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> AssemblyFacts {
        AssemblyFacts {
            schema: FACTS_SCHEMA.into(),
            parts: vec![
                PartFact {
                    id: "part-01".into(),
                    role: "enclosure_base".into(),
                },
                PartFact {
                    id: "part-02".into(),
                    role: "removable_lid".into(),
                },
            ],
            predicates: vec![PredicateFact {
                kind: "no_clash".into(),
                subjects: vec!["part-01".into(), "part-02".into()],
                pass: true,
                detail: "minimum separation 0.3 mm".into(),
            }],
        }
    }

    #[test]
    fn predicates_resolve_roles_to_stable_ids_not_names() {
        let lua = Lua::new();
        let roles = lua
            .create_sequence_from(["enclosure_base", "removable_lid"])
            .unwrap();
        require_predicate(&facts(), "no_clash", roles).unwrap();
    }

    #[test]
    fn ambiguous_roles_fail_loudly() {
        let mut got = facts();
        got.parts.push(PartFact {
            id: "part-03".into(),
            role: "removable_lid".into(),
        });
        let lua = Lua::new();
        let roles = lua
            .create_sequence_from(["enclosure_base", "removable_lid"])
            .unwrap();
        let error = require_predicate(&got, "no_clash", roles).unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");
    }

    #[test]
    fn dangling_ids_make_the_fact_contract_invalid() {
        let mut got = facts();
        got.predicates[0].subjects[1] = "not-a-part".into();
        let error = validate_facts(&got).unwrap_err();
        assert!(error.contains("unknown part ID"), "{error}");
    }

    #[test]
    fn public_fixture_and_sandboxed_script_run_end_to_end() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut got = facts();
        got.parts.extend([
            PartFact {
                id: "part-03".into(),
                role: "seal".into(),
            },
            PartFact {
                id: "part-04".into(),
                role: "protected_component".into(),
            },
        ]);
        got.predicates = vec![
            PredicateFact {
                kind: "no_clash".into(),
                subjects: vec!["part-04".into(), "part-01".into()],
                pass: true,
                detail: "clear".into(),
            },
            PredicateFact {
                kind: "insertion_clear".into(),
                subjects: vec!["part-04".into(), "part-01".into()],
                pass: true,
                detail: "clear".into(),
            },
            PredicateFact {
                kind: "seal_continuous".into(),
                subjects: vec!["part-03".into(), "part-01".into(), "part-02".into()],
                pass: true,
                detail: "continuous".into(),
            },
        ];
        let result = evaluate(
            &root.join("tasks/planned/checks/assembly-v1.lua"),
            &root.join("tasks/planned/checks/sensor-cover-v1.json"),
            &got,
        )
        .expect("valid contract");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn a_false_rust_predicate_is_an_eval_failure() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut got = facts();
        got.parts.extend([
            PartFact {
                id: "part-03".into(),
                role: "seal".into(),
            },
            PartFact {
                id: "part-04".into(),
                role: "protected_component".into(),
            },
        ]);
        got.predicates = vec![PredicateFact {
            kind: "no_clash".into(),
            subjects: vec!["part-04".into(), "part-01".into()],
            pass: false,
            detail: "overlap 1.2 mm^3".into(),
        }];
        let result = evaluate(
            &root.join("tasks/planned/checks/assembly-v1.lua"),
            &root.join("tasks/planned/checks/sensor-cover-v1.json"),
            &got,
        )
        .expect("valid contract")
        .expect_err("false predicate fails");
        assert!(result.contains("overlap 1.2"), "{result}");
    }
}
