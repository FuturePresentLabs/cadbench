//! Independent, deliberately small checks over an exported ASCII STEP file.
//!
//! The backend's JSON report is useful telemetry, but it is not an oracle:
//! accepting its claim about its own output would let a stale or flattering
//! report pass. These facts are therefore read from the exchange artifact.

use std::path::Path;

/// Facts established directly from an ISO-10303-21 clear-text artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepArtifactFacts {
    pub cylindrical_surfaces: u32,
}

/// Why an artifact cannot be used as STEP evidence.
#[derive(Debug, thiserror::Error)]
pub enum StepOracleError {
    #[error("reading STEP artifact {path}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

/// Inspects an ASCII STEP file without consulting backend-produced metadata.
///
/// This is intentionally not a general STEP parser. It establishes only the
/// fact the rubric consumes: whether the artifact is a real Part 21 exchange
/// file and how many cylindrical surface entities it contains.
pub fn inspect(path: &Path) -> Result<StepArtifactFacts, StepOracleError> {
    let text = std::fs::read_to_string(path).map_err(|source| StepOracleError::Read {
        path: path.display().to_string(),
        source,
    })?;
    inspect_text(&text)
}

fn inspect_text(text: &str) -> Result<StepArtifactFacts, StepOracleError> {
    let upper = text.to_ascii_uppercase();
    if !upper.trim_start().starts_with("ISO-10303-21;") {
        return Err(StepOracleError::Invalid(
            "STEP artifact lacks the ISO-10303-21 header".to_owned(),
        ));
    }
    if !upper.contains("DATA;") || !upper.trim_end().ends_with("END-ISO-10303-21;") {
        return Err(StepOracleError::Invalid(
            "STEP artifact has no complete DATA section/trailer".to_owned(),
        ));
    }

    Ok(StepArtifactFacts {
        cylindrical_surfaces: entity_count(&upper, "CYLINDRICAL_SURFACE")?,
    })
}

fn entity_count(text: &str, entity: &str) -> Result<u32, StepOracleError> {
    let mut count = 0_u32;
    for statement in text.split(';') {
        let Some((_, value)) = statement.split_once('=') else {
            continue;
        };
        if value.trim_start().starts_with(entity)
            && value.trim_start()[entity.len()..]
                .trim_start()
                .starts_with('(')
        {
            count = count
                .checked_add(1)
                .ok_or_else(|| StepOracleError::Invalid(format!("too many {entity} entities")))?;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "ISO-10303-21;\nHEADER;ENDSEC;\nDATA;\n#1=CYLINDRICAL_SURFACE('',#2,3.5);\n#2=PLANE('',#3);\nENDSEC;\nEND-ISO-10303-21;";

    #[test]
    fn counts_entities_from_the_artifact_not_prose() {
        let with_comment = VALID.replace(
            "ENDSEC;\nEND-ISO-10303-21;",
            "/* CYLINDRICAL_SURFACE in prose is not an entity */\nENDSEC;\nEND-ISO-10303-21;",
        );
        let facts = inspect_text(&with_comment).unwrap();
        assert_eq!(facts.cylindrical_surfaces, 1);
    }

    #[test]
    fn faceted_mutation_has_no_cylindrical_surfaces() {
        let faceted = VALID.replace("CYLINDRICAL_SURFACE('',#2,3.5)", "PLANE('',#2)");
        assert_eq!(inspect_text(&faceted).unwrap().cylindrical_surfaces, 0);
    }

    #[test]
    fn report_shaped_json_cannot_masquerade_as_step() {
        let error =
            inspect_text(r#"{"writer":"occt-brep","surfaces":{"cylinder":99}}"#).unwrap_err();
        assert!(error.to_string().contains("ISO-10303-21"));
    }

    #[test]
    fn truncated_artifact_is_rejected() {
        let error =
            inspect_text("ISO-10303-21;\nDATA;\n#1=CYLINDRICAL_SURFACE('',#2,3.5);").unwrap_err();
        assert!(error.to_string().contains("trailer"));
    }
}
