use harbor_cad::manufacturing::*;
use harbor_cad::materials::PhysicalInput;
use serde_json::{Value, json};

fn known(value: Value) -> Value {
    json!({"availability": "known", "value": value, "provenance": "synthetic test fixture", "synthetic": true})
}
fn range(low: f64, high: f64, unit: &str) -> Value {
    known(json!({"low": {"value": low, "unit": unit}, "high": {"value": high, "unit": unit}}))
}
fn money(low: f64, high: f64, basis: &str) -> Value {
    known(json!({"low": low, "high": high, "currency": "USD", "basis": basis, "year": 2026}))
}
fn missing() -> Value {
    json!({"availability": "missing", "reason": "not in fixture"})
}

fn material(id: &str, cost: Value) -> Value {
    json!({"id": id, "name": id,
        "density": range(1000., 2000., "kg/m3"),
        "thermal_conductivity": range(0.2, 0.3, "W/(m*K)"),
        "specific_heat": range(1000., 1200., "J/(kg*K)"),
        "youngs_modulus": range(1000., 2000., "MPa"),
        "poisson_ratio": range(0.3, 0.4, "1"),
        "yield_strength": range(40., 50., "MPa"),
        "tensile_strength": range(45., 55., "MPa"),
        "thermal_expansion": range(8e-5, 1e-4, "1/K"),
        "max_service_temperature": range(80., 100., "degC"),
        "min_service_temperature": range(-40., -30., "degC"),
        "emissivity": range(0.9, 0.95, "1"),
        "solar_absorptance": missing(),
        "compression_set": missing(),
        "hardness_shore_a": missing(),
        "uv_resistance": known(json!("fixture statement")),
        "material_cost": cost})
}

fn process(id: &str, materials: &[&str], extra: Value) -> Value {
    let mut p = json!({"id": id, "name": id, "compatible_materials": materials,
        "min_wall_thickness": range(1., 2., "mm"),
        "max_envelope": known(json!([{"value": 100, "unit": "mm"}, {"value": 200, "unit": "mm"}, {"value": 50, "unit": "mm"}])),
        "typical_tolerance": range(0.1, 0.2, "mm"),
        "setup_cost": money(100., 200., "per_order"),
        "tooling_cost": null, "tool_life_parts": null,
        "rate": money(5., 10., "per_part"), "rate_includes_material": false,
        "throughput": null, "time_basis": null,
        "machine_time_model": "fixture", "lead_time": range(5., 10., "d"), "notes": "fixture"});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

fn fixture() -> (String, String) {
    let materials = json!({"schema_version": 1, "materials": [
        material("resin", money(10., 20., "per_kg")),
        material("unpriced", missing()),
    ]});
    let processes = json!({"schema_version": 1, "processes": [
        process("cast", &["resin", "unpriced"], json!({
            "tooling_cost": money(500., 600., "per_tool"),
            "tool_life_parts": range(20., 25., "1")})),
        process("mill", &["resin"], json!({
            "rate": money(60., 120., "per_hour"),
            "throughput": range(500., 1000., "cm3/h"),
            "time_basis": "removed_volume"})),
        process("print", &["resin"], json!({
            "rate": money(0.5, 1., "per_cm3"), "rate_includes_material": true})),
    ]});
    (materials.to_string(), processes.to_string())
}

fn lib() -> Library {
    let (m, p) = fixture();
    Library::load(&m, &p).unwrap()
}

fn geometry() -> GeometrySummary {
    // 100 cm3 part in a 0.1 x 0.1 x 0.02 m (200 cm3) box.
    GeometrySummary {
        volume_m3: 1e-4,
        bbox_m: [0.1, 0.1, 0.02],
        min_wall_m: Some(0.003),
        surface_area_m2: None,
    }
}

fn close(a: Option<f64>, b: f64) {
    let a = a.unwrap();
    assert!((a - b).abs() < 1e-9 * b.max(1.), "{a} != {b}");
}

#[test]
fn builtin_library_loads_and_every_known_value_is_cited() {
    let library = Library::builtin().unwrap();
    assert_eq!(library.materials.len(), 13);
    assert_eq!(library.processes.len(), 6);
    let value = serde_json::to_value(&library).unwrap();
    let mut stack = vec![&value];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(o) => {
                if o.get("availability") == Some(&json!("known")) {
                    assert_eq!(o["synthetic"], json!(false), "{v}");
                    let provenance = o["provenance"].as_str().unwrap();
                    assert!(
                        provenance.contains("http") && provenance.contains("accessed"),
                        "{provenance}"
                    );
                }
                stack.extend(o.values());
            }
            Value::Array(a) => stack.extend(a),
            _ => {}
        }
    }
}

#[test]
fn library_rejects_bad_ranges_units_and_references() {
    let (m, p) = fixture();
    let swap = |s: &str, a: &str, b: &str| s.replacen(a, b, 1);
    assert!(Library::load(&swap(&m, "\"kg/m3\"", "\"kg\""), &p).is_err());
    assert!(Library::load(&swap(&m, "1000.0", "3000.0"), &p).is_err());
    assert!(Library::load(&m, &swap(&p, "\"unpriced\"", "\"ghost\"")).is_err());
    assert!(Library::load(&m, &swap(&p, "\"print\"", "\"cast\"")).is_err());
    assert!(Library::load(&swap(&m, "synthetic test fixture", " "), &p).is_err());
}

#[test]
fn cost_amortizes_setup_and_whole_tools() {
    let library = lib();
    let estimate = unit_cost(&library, &geometry(), "resin", "cast", 30).unwrap();
    assert!(estimate.missing.is_empty());
    // material: 1e-4 m3 x 1.1 x (1000 kg/m3 x $10 .. 2000 x $20)
    let material = 1e-4 * MATERIAL_SCRAP_FACTOR;
    // tooling: 30 parts need ceil(30/25)=2 .. ceil(30/20)=2 moulds
    let low = material * 1000. * 10. + 5. + 100. / 30. + 500. * 2. / 30.;
    let high = material * 2000. * 20. + 10. + 200. / 30. + 600. * 2. / 30.;
    close(estimate.low, low);
    close(estimate.high, high);
    // 50 parts: 2 moulds at life 25, 3 at life 20.
    let estimate = unit_cost(&library, &geometry(), "resin", "cast", 50).unwrap();
    let tooling = &estimate
        .breakdown
        .iter()
        .find(|t| t.name == "tooling")
        .unwrap();
    close(tooling.low, 500. * 2. / 50.);
    close(tooling.high, 600. * 3. / 50.);

    // per_hour on removed volume: 100 cm3 removed at 1000..500 cm3/h.
    let milled = unit_cost(&library, &geometry(), "resin", "mill", 1).unwrap();
    let process = milled
        .breakdown
        .iter()
        .find(|t| t.name == "process")
        .unwrap();
    close(process.low, 60. * 0.1);
    close(process.high, 120. * 0.2);
    let material = milled
        .breakdown
        .iter()
        .find(|t| t.name == "material")
        .unwrap();
    close(material.low, 2e-4 * MATERIAL_SCRAP_FACTOR * 1000. * 10.);

    // per_cm3 with material included: no separate material charge.
    let printed = unit_cost(&library, &geometry(), "resin", "print", 10).unwrap();
    close(printed.low, 0. + 100. * 0.5 + 10.);
    close(printed.high, 100. * 1. + 20.);
}

#[test]
fn missing_input_is_reported_not_fabricated() {
    let estimate = unit_cost(&lib(), &geometry(), "unpriced", "cast", 10).unwrap();
    assert_eq!(estimate.missing, ["unpriced.material_cost"]);
    assert_eq!((estimate.low, estimate.high), (None, None));
    let material = estimate
        .breakdown
        .iter()
        .find(|t| t.name == "material")
        .unwrap();
    assert_eq!((material.low, material.high), (None, None));
    assert!(estimate.breakdown.iter().any(|t| t.low.is_some()));
}

#[test]
fn manufacturability_flags_thin_wall_envelope_and_unknowns() {
    let library = lib();
    let mut g = geometry();
    assert!(
        manufacturability(&library, &g, "resin", "cast")
            .unwrap()
            .is_empty()
    );
    g.min_wall_m = Some(0.0005);
    g.bbox_m = [0.06, 0.06, 0.25];
    let found = manufacturability(&library, &g, "resin", "cast").unwrap();
    let has = |c, s| found.iter().any(|v| v.check == c && v.severity == s);
    assert!(has(Check::MinWall, Severity::Violation));
    assert!(has(Check::Envelope, Severity::Violation));
    g.min_wall_m = Some(0.0015);
    g.bbox_m = [0.2, 0.05, 0.1]; // fits only rotated
    let found = manufacturability(&library, &g, "resin", "cast").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Marginal);
    g.min_wall_m = None;
    let found = manufacturability(&library, &g, "unpriced", "mill").unwrap();
    assert!(
        found
            .iter()
            .any(|v| v.check == Check::Compatibility && v.severity == Severity::Violation)
    );
    assert!(
        found
            .iter()
            .any(|v| v.check == Check::MinWall && v.severity == Severity::Unknown)
    );
}

#[test]
fn service_margins_use_conservative_limits() {
    let library = lib();
    let margins = service_margins(&library, "resin", 363.15, 238.15, Some(10e6), 2.).unwrap();
    // max service low end 80 degC < 90 degC demand
    assert_eq!(margins[0].passes, Some(false));
    assert!((margins[0].margin.unwrap() + 10.).abs() < 1e-9);
    // min service high end -30 degC vs -35 degC demand
    assert_eq!(margins[1].passes, Some(false));
    // 40 MPa / 2 = 20 MPa vs 10 MPa
    assert!((margins[2].margin.unwrap() - 1.).abs() < 1e-12);
    assert!(service_margins(&library, "resin", 300., 310., None, 2.).is_err());
    assert!(matches!(
        library.material("resin").unwrap().solar_absorptance,
        PhysicalInput::Missing { .. }
    ));
}
