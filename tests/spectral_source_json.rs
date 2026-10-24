use harbor_cad::{contracts::Operation, materials::PhysicalInput, radiation::SpectralSource};

#[test]
fn tagged_spectral_sources_keep_decimal_and_exponent_numbers() {
    let directional = r#"{
        "propagation_direction": [0.5, 0.0, -0.8660254037844386],
        "irradiance": [{"value": 1e-10, "unit": "W/(m2*nm)"}],
        "kind": "directional"
    }"#;
    let source: SpectralSource = serde_json::from_str(directional).unwrap();
    let SpectralSource::Directional {
        propagation_direction,
        irradiance,
    } = &source
    else {
        panic!("directional source changed variant");
    };
    assert_eq!(*propagation_direction, [0.5, 0.0, -0.8660254037844386]);
    assert_eq!(irradiance[0].value, 1e-10);
    let encoded = serde_json::to_string(&source).unwrap();
    let decoded: SpectralSource = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::to_value(source).unwrap()
    );

    let source: SpectralSource = serde_json::from_str(
        r#"{"radiance": [{"value": 0.25, "unit": "W/(m2*sr*nm)"}], "kind": "isotropic"}"#,
    )
    .unwrap();
    let SpectralSource::Isotropic { radiance } = source else {
        panic!("isotropic source changed variant");
    };
    assert_eq!(radiance[0].value, 0.25);
}

#[test]
fn tagged_spectral_sources_reject_duplicate_unknown_and_inactive_fields() {
    for input in [
        r#"{"kind":"isotropic","radiance":[],"radiance":[]}"#,
        r#"{"kind":"isotropic","kind":"isotropic","radiance":[]}"#,
        r#"{"kind":"isotropic","radiance":[],"extra":null}"#,
        r#"{"kind":"isotropic","radiance":[],"irradiance":[]}"#,
        r#"{"kind":"isotropic","radiance":[],"propagation_direction":null}"#,
        r#"{"kind":"directional","irradiance":[],"propagation_direction":[0,0,1],"radiance":null}"#,
        r#"{"kind":"directional","irradiance":[]}"#,
        r#"{"kind":"isotropic","radiance":null}"#,
        r#"{"kind":"isotropic","radiance":[{"value":"0.25","unit":"W/(m2*sr*nm)"}]}"#,
        r#"{"kind":"isotropic","radiance":[{"value":{"$serde_json::private::Number":"0.25"},"unit":"W/(m2*sr*nm)"}]}"#,
        r#"{"kind":"isotropic","radiance":[{"value":0.25,"value":0.5,"unit":"W/(m2*sr*nm)"}]}"#,
        r#"{"kind":"other","radiance":[]}"#,
    ] {
        assert!(
            serde_json::from_str::<SpectralSource>(input).is_err(),
            "accepted {input}"
        );
    }
}

#[test]
fn physical_inputs_preserve_generic_values_and_strict_variant_fields() {
    let input: PhysicalInput<f64> = serde_json::from_str(
        r#"{"value":1e-10,"provenance":"measurement","synthetic":false,"availability":"known"}"#,
    )
    .unwrap();
    assert!(matches!(input, PhysicalInput::Known { value, .. } if value == 1e-10));
    let borrowed: PhysicalInput<&str> = serde_json::from_str(
        r#"{"availability":"known","value":"original","provenance":"measurement","synthetic":false}"#,
    )
    .unwrap();
    assert!(matches!(
        borrowed,
        PhysicalInput::Known {
            value: "original",
            ..
        }
    ));
    for input in [
        r#"{"availability":"missing","reason":"unknown","synthetic":false}"#,
        r#"{"availability":"missing","reason":"unknown","value":null}"#,
        r#"{"availability":"known","value":0.25,"value":0.5,"provenance":"measurement","synthetic":false}"#,
        r#"{"availability":"known","value":0.25,"provenance":"measurement","synthetic":false,"reason":null}"#,
    ] {
        assert!(
            serde_json::from_str::<PhysicalInput<f64>>(input).is_err(),
            "accepted {input}"
        );
    }
}

#[test]
fn worker_operations_preserve_nested_decimal_fields_and_defaults() {
    let spec = include_str!("../examples/spectral-reference.json");
    let input = format!(r#"{{"spec":{spec},"operation":"validate_spectral_reference"}}"#);
    let operation: Operation = serde_json::from_str(&input).unwrap();
    assert!(matches!(
        operation,
        Operation::ValidateSpectralReference { .. }
    ));
    let from_value: Operation =
        serde_json::from_value(serde_json::from_str(&input).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(operation).unwrap(),
        serde_json::to_value(from_value).unwrap()
    );
    let operation: Operation =
        serde_json::from_str(r#"{"operation":"artifacts","job_id":"job"}"#).unwrap();
    assert!(matches!(
        operation,
        Operation::Artifacts {
            after: None,
            limit: 20,
            ..
        }
    ));
    let operation: Operation = serde_json::from_str(r#"{"operation":"doctor"}"#).unwrap();
    assert!(matches!(operation, Operation::Doctor {}));
    for input in [
        r#"{"operation":"doctor","extra":null}"#,
        r#"{"operation":"doctor","operation":"doctor"}"#,
        r#"{"operation":"artifacts","job_id":"first","job_id":"second"}"#,
    ] {
        assert!(
            serde_json::from_str::<Operation>(input).is_err(),
            "accepted {input}"
        );
    }
}
