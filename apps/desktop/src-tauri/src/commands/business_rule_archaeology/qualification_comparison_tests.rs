use super::*;
use std::{fs, path::PathBuf};

const CORPUS: &[u8] = include_bytes!("fixtures/expected.json.fixture");
const FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/fixtures/business-rule-archaeology/model-comparison-fixture-v1.json"
);
const POLICY: &[u8] = include_bytes!(
    "../../../../tests/fixtures/business-rule-archaeology/qualification-policy-v1.json"
);

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/business-rule-archaeology/model-comparison-report-v1.json")
}
async fn report() -> Value {
    evaluate(CORPUS, FIXTURE, POLICY).await.unwrap()
}
fn n(value: &Value, key: &str) -> u64 {
    value[key].as_u64().unwrap()
}

#[tokio::test]
async fn checked_report_is_exact_and_reproducible() {
    let actual_report = report().await;
    let bytes = encode(&actual_report).unwrap();
    if std::env::var_os("UPDATE_ARCHAEOLOGY_COMPARISON_REPORT").is_some() {
        fs::write(path(), &bytes).unwrap();
    }
    assert_eq!(
        bytes,
        fs::read(path()).expect("regenerate with UPDATE_ARCHAEOLOGY_COMPARISON_REPORT=1")
    );
    assert_eq!(actual_report, report().await);
}

#[tokio::test]
async fn reversed_cases_are_semantically_deterministic() {
    let first = report().await;
    let mut fixture: Value = serde_json::from_slice(FIXTURE).unwrap();
    fixture["cases"].as_array_mut().unwrap().reverse();
    let reversed_bytes = serde_json::to_vec_pretty(&fixture).unwrap();
    let reversed = evaluate(CORPUS, &reversed_bytes, POLICY).await.unwrap();
    assert_ne!(
        first["input_identities"]["synthesis_fixture"],
        reversed["input_identities"]["synthesis_fixture"]
    );
    for key in [
        "scope",
        "policy",
        "variants",
        "cases",
        "zero_model_catalog",
        "gates",
        "limitations",
    ] {
        assert_eq!(first[key], reversed[key], "semantic mismatch for {key}");
    }
}

#[tokio::test]
async fn scope_alias_history_and_quantifier_gap_reconcile() {
    let report = report().await;
    let scope = &report["scope"];
    assert_eq!(n(scope, "primary_current_cases"), 6);
    assert_eq!(n(scope, "generated_alias_cases"), 1);
    assert_eq!(n(scope, "historical_cases"), 1);
    assert_eq!(n(scope, "reconciled_rule_total"), 9);
    assert_eq!(report["cases"].as_array().unwrap().len(), 6);
    assert_eq!(scope["missing_clause_shapes"], json!(["quantifier"]));
    assert_eq!(report["full_qualification"], false);
}

#[tokio::test]
async fn mutation_and_gate_regression_fail_closed() {
    let mut fixture: Value = serde_json::from_slice(FIXTURE).unwrap();
    fixture["cases"][0]["action_fact_ids"] = json!(["fact:unknown"]);
    assert!(
        evaluate(CORPUS, &serde_json::to_vec(&fixture).unwrap(), POLICY)
            .await
            .is_err()
    );
    let failing = json!({"supported_clause_rate_millionths":970000,"unsupported_clause_rate_millionths":30000});
    let zero = json!({"exact_rerun_parity":true,"canonical_rule_rows":1,"manifest_rows":1,"fts_rows":1,"manifest_fts_exact_parity":true,"provider_calls":0,"synthesis_attempt_rows":0});
    assert_eq!(
        gates(&failing, &failing, &zero, 980000, 20000).unwrap()["comparison_gate_pass"],
        false
    );
}

#[tokio::test]
async fn corrections_calls_tokens_cost_and_privacy_are_bounded() {
    let report = report().await;
    let deterministic = &report["variants"][0];
    let synthesis = &report["variants"][1];
    assert_eq!(deterministic["variant"], "deterministic_template");
    for key in [
        "mock_provider_calls",
        "external_model_calls",
        "attempts",
        "input_tokens",
        "output_tokens",
    ] {
        assert_eq!(
            n(deterministic, key),
            0,
            "unexpected deterministic accounting for {key}"
        );
    }
    assert_eq!(synthesis["variant"], "mock_structured_synthesis");
    assert_eq!(n(synthesis, "mock_provider_calls"), 6);
    assert_eq!(n(synthesis, "external_model_calls"), 0);
    assert_eq!(n(synthesis, "attempts"), 6);
    assert_eq!(n(synthesis, "input_tokens"), 768);
    assert_eq!(n(synthesis, "output_tokens"), 384);
    assert_eq!(n(synthesis, "reported_cost_microusd"), 0);
    assert_eq!(n(synthesis, "estimated_cost_microusd"), 0);
    assert!(synthesis["pricing_identity"].is_null());
    for case in report["cases"].as_array().unwrap() {
        for key in ["deterministic_correction", "synthesis_correction"] {
            let d = &case[key];
            assert_eq!(
                n(d, "text_edit_distance"),
                n(d, "insertions") + n(d, "deletions") + n(d, "substitutions")
            );
        }
    }
    assert!(n(deterministic, "text_edit_distance") > 0 && n(synthesis, "text_edit_distance") > 0);
    let encoded = String::from_utf8(encode(&report).unwrap()).unwrap();
    for forbidden in [
        "/Users/",
        "C:\\\\",
        "REQUEST_JSON",
        "CLAIM-AMOUNT",
        "AMOUNT POSITIVE",
        "api_key",
        "credential",
        "raw_output",
    ] {
        assert!(!encoded.contains(forbidden), "{forbidden}");
    }
}

#[tokio::test]
async fn zero_model_catalog_has_manifest_fts_rerun_parity() {
    let report = report().await;
    let proof = &report["zero_model_catalog"];
    assert_eq!(proof["first_receipt"], proof["rerun_receipt"]);
    assert_eq!(proof["exact_rerun_parity"], true);
    for key in ["canonical_rule_rows", "manifest_rows", "fts_rows"] {
        assert_eq!(
            n(proof, key),
            1,
            "unexpected zero-model row count for {key}"
        );
    }
    assert_eq!(proof["manifest_fts_exact_parity"], true);
    assert_eq!(n(proof, "provider_calls"), 0);
    assert_eq!(n(proof, "synthesis_attempt_rows"), 0);
}

#[tokio::test]
async fn unknown_fields_are_rejected() {
    let mut fixture: Value = serde_json::from_slice(FIXTURE).unwrap();
    fixture["unexpected"] = json!(true);
    assert!(
        evaluate(CORPUS, &serde_json::to_vec(&fixture).unwrap(), POLICY)
            .await
            .is_err()
    );
    let mut report = report().await;
    report["unexpected"] = json!(true);
    assert!(validate_report(&report).is_err());
}
