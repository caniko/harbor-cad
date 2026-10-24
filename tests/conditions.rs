use harbor_cad::{
    conditions::{Conditions, Point},
    manufacturing::{Library, QuantityRange},
    materials::PhysicalInput,
};

fn every_input(c: &Conditions) -> Vec<&PhysicalInput<QuantityRange>> {
    let mut all: Vec<_> = c
        .classes
        .iter()
        .flat_map(|k| {
            [
                &k.low_air_temperature,
                &k.high_air_temperature,
                &k.low_relative_humidity,
                &k.high_relative_humidity,
                &k.solar_irradiance,
                &k.wind_speed,
            ]
        })
        .collect();
    all.extend(c.films.iter().map(|f| &f.coefficient));
    all.extend(
        c.finishes
            .iter()
            .flat_map(|f| [&f.solar_absorptance, &f.emissivity]),
    );
    all
}

#[test]
fn builtin_loads_with_cited_non_synthetic_values() {
    let c = Conditions::builtin().unwrap();
    assert_eq!(c.classes.len(), 4);
    for input in every_input(&c) {
        if let PhysicalInput::Known {
            provenance,
            synthetic,
            ..
        } = input
        {
            assert!(!synthetic && provenance.contains("https://"));
            assert!(provenance.contains("accessed 2026-10-24"), "{provenance}");
        }
    }
}

#[test]
fn design_points_are_ordered_and_follow_the_standard() {
    let c = Conditions::builtin().unwrap();
    for class in &c.classes {
        let hot = c.design_point(&class.id, Point::Hot).unwrap();
        let cold = c.design_point(&class.id, Point::Cold).unwrap();
        assert!(hot.ambient_temperature.value > cold.ambient_temperature.value);
        assert!(hot.solar_irradiance.value > 0. && cold.solar_irradiance.value == 0.);
        // Hot uses the low still-air film, cold the high design-wind film.
        assert!(hot.film_coefficient.value < cold.film_coefficient.value);
    }
    let hot = c.design_point("etsi-4-1", Point::Hot).unwrap();
    assert!((hot.ambient_temperature.value - 313.15).abs() < 1e-9);
    assert_eq!(hot.solar_irradiance.value, 1120.);
    assert_eq!(hot.film_coefficient.value, 2.2);
    let cold = c.design_point("etsi-4-1", Point::Cold).unwrap();
    assert!((cold.ambient_temperature.value - 240.15).abs() < 1e-9);
    assert_eq!(
        c.lookup("condition:etsi-4-2h.hot.ambient_temperature")
            .unwrap()
            .value,
        273.15 + 55.
    );
    assert_eq!(
        c.lookup("finish:powder-coat-light-grey.solar_absorptance")
            .unwrap()
            .value,
        0.47
    );
    // The closed-cavity film: the caller names the conservative (low) end.
    let cavity = c.lookup("film:enclosed-cavity-natural.low").unwrap();
    assert_eq!((cavity.value, cavity.unit), (1.2, "W/(m2*K)"));
    assert!(cavity.provenance.contains("Churchill"));
    assert_eq!(
        c.lookup("film:enclosed-cavity-natural.high").unwrap().value,
        8.2
    );
}

#[test]
fn finish_fills_missing_absorptance_but_glazing_stays_refused() {
    let c = Conditions::builtin().unwrap();
    let library = Library::builtin().unwrap();
    assert!(matches!(
        library.material("pa12-mjf").unwrap().solar_absorptance,
        PhysicalInput::Missing { .. }
    ));
    let (alpha, eps) = c
        .surface_optics("pa12-mjf", "pigment-black-polymer")
        .unwrap();
    assert!(matches!(alpha, PhysicalInput::Known { .. }));
    assert!(matches!(eps, PhysicalInput::Known { .. }));
    // A missing finish value falls back to the material's own data.
    let (alpha, _) = c.surface_optics("al-6061-t6", "bare").unwrap();
    assert!(
        matches!(&alpha, PhysicalInput::Known { value, .. } if value.si("dimensionless").unwrap().1 == 0.3)
    );
    let (alpha, _) = c.surface_optics("pa12-mjf", "bare").unwrap();
    assert!(matches!(alpha, PhysicalInput::Missing { .. }));
    // Transparent glazing keeps transmittance semantics; no finish applies.
    for finish in ["bare", "pigment-black-polymer", "powder-coat-white"] {
        assert!(c.surface_optics("pc-uv", finish).is_err());
    }
    // Incompatible pairs and unknown ids are refused.
    assert!(
        c.surface_optics("pa12-mjf", "powder-coat-light-grey")
            .is_err()
    );
    assert!(c.surface_optics("nope", "bare").is_err());
    assert!(c.surface_optics("al-6061-t6", "nope").is_err());
    assert!(c.design_point("nope", Point::Hot).is_err());
    for key in [
        "condition:etsi-4-1.warm.ambient_temperature",
        "condition:etsi-4-1.hot.wind",
        "condition:etsi-4-1.hot",
        "finish:pigment-white-polymer.solar_absorptance",
        "finish:bare.color",
        "film:enclosed-cavity-natural.mid",
        "film:nope.low",
        "other:x",
    ] {
        assert!(c.lookup(key).is_err(), "{key}");
    }
}

#[test]
fn strict_validation_refuses_uncited_and_inconsistent_data() {
    let conditions = include_str!("../data/conditions.json");
    let finishes = include_str!("../data/finishes.json");
    let load = |c: &str, f: &str| Conditions::load(c, f, Library::builtin().unwrap());
    load(conditions, finishes).unwrap();
    // A closed-cavity (internal) film is natural convection: no wind speed.
    let internal = conditions.rfind("\"wind_speed\": null").unwrap();
    let windy = format!(
        "{}\"wind_speed\": {{\"value\": 1, \"unit\": \"m/s\"}}{}",
        &conditions[..internal],
        &conditions[internal + "\"wind_speed\": null".len()..]
    );
    let edits = [
        (windy, true),
        (conditions.replacen("accessed 2026-10-24", "seen", 1), true),
        (
            conditions.replacen("\"synthetic\": false", "\"synthetic\": true", 1),
            true,
        ),
        (
            conditions.replacen("\"etsi-4-1e\"", "\"etsi-4-1\"", 1),
            true,
        ),
        (
            conditions.replacen("\"value\": 40,", "\"value\": -40,", 1),
            true,
        ),
        (
            finishes.replacen("\"ss304\"\n      ]", "\"pc-uv\"\n      ]", 1),
            false,
        ),
        (
            finishes.replacen("\"value\": 0.22", "\"value\": 1.5", 1),
            false,
        ),
        (
            finishes.replacen("\"unit\": \"1\"", "\"unit\": \"K\"", 1),
            false,
        ),
    ];
    for (edited, in_conditions) in edits {
        assert!(
            edited != conditions && edited != finishes,
            "edit did not apply"
        );
        let result = if in_conditions {
            load(&edited, finishes)
        } else {
            load(conditions, &edited)
        };
        assert!(result.is_err(), "accepted an invalid edit");
    }
}
