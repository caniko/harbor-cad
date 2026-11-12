use harbor_cad::{design, design_screen};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, process::Command};

fn request() -> Value {
    json!({
        "schema_version":1, "provenance":"synthetic scalar reference, not product measurements",
        "terms":[{"metric":"wall","weight":0.4},{"metric":"seal","weight":0.3},
            {"metric":"edge","weight":0.2},{"metric":"clearance","weight":0.1}],
        "constraints":[{"metric":"wall","op":"ge","limit":1.6,"unit":"mm"},
            {"metric":"seal","op":"ge","limit":2.5,"unit":"mm"},
            {"metric":"edge","op":"ge","limit":3.0,"unit":"mm"},
            {"metric":"clearance","op":"ge","limit":10.0,"unit":"mm"}],
        "gate_names":["manufacturing_ok"],
        "candidates":[
            {"name":"P","metrics":{"wall":2.0,"seal":3.0,"edge":4.2,"clearance":14.0},"gates":{"manufacturing_ok":true}},
            {"name":"Q","metrics":{"wall":1.76,"seal":3.0,"edge":3.6,"clearance":14.0},"gates":{"manufacturing_ok":true}},
            {"name":"R","metrics":{"wall":2.2,"seal":2.75,"edge":3.3,"clearance":11.0},"gates":{"manufacturing_ok":true}},
            {"name":"S","metrics":{"wall":2.4,"seal":3.5,"edge":4.8,"clearance":16.0},"gates":{"manufacturing_ok":false}},
            {"name":"T","metrics":{"wall":2.2,"seal":null,"edge":4.5,"clearance":15.0},"gates":{"manufacturing_ok":true}},
            {"name":"U","metrics":{"wall":2.2,"seal":3.2,"edge":4.2,"clearance":null},"gates":{"manufacturing_ok":true}}],
        "ledger":{"axes":{"wall":[1.76,2.4],"seal":[2.75,3.5],"edge":[3.3,4.8],"clearance":[11.0,16.0]},
            "baseline":{"wall":2.0,"seal":3.0,"edge":4.2,"clearance":14.0},"search":[],
            "fidelity_reruns":3,"repeats":0,"max_evaluations":24,
            "max_wall_seconds":900,"max_artifact_bytes":536870912,"estimated_bytes_per_evaluation":null}
    })
}

#[test]
fn screen_reuses_engine_math_and_counts_distinct_corners_without_qualifying_inputs() {
    let input = serde_json::from_value(request()).unwrap();
    let report = design_screen::screen(&input).unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["nominal_winners"], json!(["P", "Q"]));
    for name in ["P", "Q"] {
        assert!((value["candidates"][name]["objective"].as_f64().unwrap() + 0.04).abs() < 1e-12);
    }
    assert_eq!(
        value["candidates"]["P"]["bottlenecks"],
        json!(["clearance"])
    );
    assert_eq!(
        value["candidates"]["Q"]["bottlenecks"],
        json!(["clearance", "edge", "wall"])
    );
    for (name, standing) in [("S", "infeasible"), ("T", "unknown"), ("U", "unknown")] {
        assert_eq!(value["candidates"][name]["standing"], standing);
        assert!(value["candidates"][name]["objective"].is_null());
    }
    assert_eq!(value["ledger"]["corners"], 16);
    assert_eq!(value["ledger"]["additional_baseline"], 1);
    assert_eq!(value["ledger"]["total"], 20);
    assert_eq!(value["ledger"]["budget_spare"], 4);
    assert_eq!(value["ledger"]["hard_cap_headroom"], 8);
    assert!(value["ledger"]["estimated_artifact_bytes"].is_null());
    assert_eq!(value["qualified"], false);
    assert_eq!(value["envelope_validated"], false);
    let markdown = design_screen::markdown(&report);
    assert!(markdown.contains("P, Q"));
    assert!(markdown.contains("20"));
    assert!(!markdown.contains("next worst"));
    assert!(markdown.contains("gate manufacturing_ok false"));
    assert!(markdown.contains("metric seal unknown"));
    assert!(markdown.contains("metric clearance unknown"));

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("screen.json");
    fs::write(&file, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_harbor-cad"))
        .args(["design", "screen"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output, value);
    let output = Command::new(env!("CARGO_BIN_EXE_harbor-cad"))
        .args(["design", "screen"])
        .arg(&file)
        .arg("--markdown")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), markdown);
}

#[test]
fn screen_refuses_false_ledgers_and_ambiguous_or_unbounded_inputs() {
    for change in [
        ("/ledger/max_evaluations", json!(19)),
        ("/ledger/max_evaluations", json!(33)),
        ("/ledger/fidelity_reruns", json!(4)),
        ("/ledger/repeats", json!(u32::MAX)),
        ("/ledger/baseline/wall", json!(0.0)),
        ("/ledger/axes/wall", json!([2.4, 1.76])),
        ("/terms/0/weight", json!(-1.0)),
        ("/terms/1/metric", json!("wall")),
        ("/candidates/1/name", json!("P")),
        ("/candidates/0/metrics/undeclared", json!(1.0)),
    ] {
        let mut input = request();
        if change.0.ends_with("/undeclared") {
            input["candidates"][0]["metrics"]["undeclared"] = change.1;
        } else {
            *input.pointer_mut(change.0).unwrap() = change.1;
        }
        let input = serde_json::from_value(input).unwrap();
        assert!(
            design_screen::screen(&input).is_err(),
            "accepted {}",
            change.0
        );
    }
    let mut input = request();
    input["ledger"]["baseline"] = json!({"wall":1.76,"seal":2.75,"edge":3.3,"clearance":11.0});
    let input = serde_json::from_value(input).unwrap();
    let report = serde_json::to_value(design_screen::screen(&input).unwrap()).unwrap();
    assert_eq!(report["ledger"]["additional_baseline"], 0);
    assert_eq!(report["ledger"]["total"], 19);
    let mut input = request();
    input["ledger"]["search"] = json!([input["ledger"]["baseline"].clone()]);
    let input = serde_json::from_value(input).unwrap();
    assert!(design_screen::screen(&input).is_err());
    let mut input = request();
    input["ledger"]["estimated_bytes_per_evaluation"] = json!(30_000_000);
    let input = serde_json::from_value(input).unwrap();
    let report = serde_json::to_value(design_screen::screen(&input).unwrap()).unwrap();
    assert_eq!(report["ledger"]["estimated_artifact_bytes"], 720_000_000);
    assert_eq!(report["ledger"]["estimate_within_artifact_ceiling"], false);
    let duplicate = serde_json::to_string(&request()).unwrap().replace(
        "\"manufacturing_ok\":true",
        "\"manufacturing_ok\":false,\"manufacturing_ok\":true",
    );
    assert!(serde_json::from_str::<design_screen::Request>(&duplicate).is_err());
}

#[test]
fn unselected_overflow_is_unknown_in_the_shared_engine_too() {
    let mut envelope = design::example().unwrap();
    envelope.objective.terms.truncate(1);
    envelope.objective.kind = design::ObjectiveKind::MaximizeMinMargin;
    envelope.objective.terms[0].metric = "height".into();
    envelope.constraints = vec![
        design::Constraint {
            metric: "height".into(),
            op: design::ConstraintOp::Ge,
            limit: 1.,
            unit: "mm".into(),
            safety_factor: None,
        },
        design::Constraint {
            metric: "extra".into(),
            op: design::ConstraintOp::Ge,
            limit: 1e-308,
            unit: "mm".into(),
            safety_factor: None,
        },
    ];
    let assessment = design::assess(
        &envelope,
        &BTreeMap::from([("height".into(), Some(2.)), ("extra".into(), Some(1e308))]),
    );
    assert_eq!(assessment.standing, design::Standing::Unknown);
    assert_eq!(assessment.constraints[1].satisfied, None);
    envelope.constraints.truncate(1);
    envelope.objective.kind = design::ObjectiveKind::Minimize;
    envelope.objective.terms[0].weight = f64::MAX;
    let assessment = design::assess(&envelope, &BTreeMap::from([("height".into(), Some(2.))]));
    assert_eq!(assessment.standing, design::Standing::Unknown);
    assert_eq!(assessment.objective, None);
}
