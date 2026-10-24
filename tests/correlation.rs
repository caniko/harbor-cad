use harbor_cad::{correlation::*, results::SampleLocation, science::Quantity};
use std::collections::BTreeMap;

fn q(value: f64, unit: &str) -> Quantity {
    Quantity {
        value,
        unit: unit.into(),
    }
}

fn request() -> CorrelationRequest {
    CorrelationRequest {
        schema_version: 1,
        job_id: uuid::Uuid::new_v4().to_string(),
        measurement_csv: "test.csv".into(),
        dimension: "temperature".into(),
        thermal_stage: None,
        probe_map: [("tc1", 1), ("tc2", 8)]
            .map(|(name, node_id)| (name.to_string(), SampleLocation::Node { node_id }))
            .into(),
        time_offset_s: 10.,
        time_window: [20., 90.],
        tolerance: Tolerance {
            abs: q(0.65, "delta_degC"),
            rel: 0.,
        },
        provenance: MeasurementProvenance {
            test_id: "CC-007".into(),
            rig: "climate chamber A".into(),
            sensor_type: "type T thermocouple".into(),
            sensor_accuracy: q(0.5, "K"),
            date: "2026-10-01".into(),
        },
    }
}

/// Manufactured truth: simulated T(t) = 293.15 + 0.1 t (K) at every snapshot,
/// so linear interpolation is exact. Measured = truth(t + offset) - offset_err
/// ± noise, written in degC.
fn fixture(noise: f64) -> (Vec<u8>, BTreeMap<String, Series>) {
    let truth = |t: f64| 293.15 + 0.1 * t;
    let series: Vec<_> = (0..=10)
        .map(|i| (i as f64 * 10., truth(i as f64 * 10.)))
        .collect();
    let simulated = [("tc1", series.clone()), ("tc2", series)]
        .map(|(n, s)| (n.to_string(), s))
        .into();
    let mut csv = String::from("time,probe,value,unit\r\n");
    for (probe, bias) in [("tc1", 0.5), ("tc2", -0.25)] {
        // measured times 0..=100 s; simulated time = measured + 10 s.
        for i in 0..=20 {
            let t = i as f64 * 5.;
            let sign = if i % 2 == 0 { 1. } else { -1. };
            let measured_k = truth(t + 10.) - bias + sign * noise;
            csv.push_str(&format!("{t},{probe},{},degC\n", measured_k - 273.15));
        }
    }
    (csv.into_bytes(), simulated)
}

#[test]
fn manufactured_truth_recovers_known_bias_and_rmse() {
    let (csv, simulated) = fixture(0.1);
    let measured = parse_measurements(&csv, "temperature").unwrap();
    assert_eq!(measured.rows, 42);
    let report = correlate(&request(), &measured, &simulated).unwrap();
    // Window [20, 90] in simulated time ⇒ measured t ∈ [10, 80]: 15 points each.
    let [tc1, tc2] = &report.probes[..] else {
        panic!()
    };
    assert_eq!((tc1.metrics.n, tc1.excluded_outside_window), (15, 6));
    // Alternating ±0.1 noise over 15 points starting at i = 2 (even): 8×(+), 7×(-).
    let noise_mean = 0.1 * (8. - 7.) / 15.;
    assert!((tc1.metrics.bias - (0.5 - noise_mean)).abs() < 1e-9);
    assert!((tc2.metrics.bias - (-0.25 - noise_mean)).abs() < 1e-9);
    let rmse1 = ((8. * 0.4f64.powi(2) + 7. * 0.6f64.powi(2)) / 15.).sqrt();
    assert!((tc1.metrics.rmse - rmse1).abs() < 1e-9);
    assert!((tc1.metrics.max_abs_error - 0.6).abs() < 1e-9);
    assert!((tc1.metrics.mae - (8. * 0.4 + 7. * 0.6) / 15.).abs() < 1e-9);
    assert!(tc1.metrics.pass && tc2.metrics.pass && report.pass);
    assert_eq!(report.overall.n, 30);
    assert!((objective(&report) - report.overall.rmse).abs() < 1e-15);
    assert_eq!(report.sensor_accuracy_si, 0.5);
    assert!(report.claim.contains("not validation"));
    assert!(serde_json::to_vec(&report).unwrap().len() < 65536);

    let mut strict = request();
    strict.tolerance.abs = q(0.5, "K");
    let failed = correlate(&strict, &measured, &simulated).unwrap();
    assert!(!failed.pass && !failed.probes[0].metrics.pass && failed.probes[1].metrics.pass);
    assert_eq!(failed.probes[0].metrics.failures, 7);
    // A relative term over |T| in Kelvin restores the pass.
    strict.tolerance.rel = 0.001;
    assert!(correlate(&strict, &measured, &simulated).unwrap().pass);
}

#[test]
fn interpolation_between_snapshots_is_linear_and_exact_on_knots() {
    let csv = b"time,probe,value,unit\n0,tc1,300,K\n5,tc1,300,K\n10,tc1,300,K\n";
    let measured = parse_measurements(csv, "temperature").unwrap();
    let mut r = request();
    r.probe_map.remove("tc2");
    r.time_offset_s = 0.;
    r.time_window = [0., 10.];
    let simulated = [("tc1".to_string(), vec![(0., 300.), (10., 302.)])].into();
    let report = correlate(&r, &measured, &simulated).unwrap();
    let m = &report.probes[0].metrics;
    assert!((m.bias - 1.).abs() < 1e-12);
    assert_eq!((m.max_abs_error, m.max_abs_error_time_s), (2., 10.));
}

#[test]
fn refuses_unsorted_unknown_unmapped_and_extrapolated_inputs() {
    let (csv, simulated) = fixture(0.);
    let r = request();
    let ok = parse_measurements(&csv, "temperature").unwrap();
    for bad in [
        &b"time,probe,value,unit\n1,tc1,20,degC\n1,tc1,21,degC\n"[..],
        b"time,probe,value,unit\n2,tc1,20,degC\n1,tc1,21,degC\n",
        b"time,probe,value,unit\n1,tc1,20,degF\n",
        b"time,probe,value,unit\n1,tc1,NaN,degC\n",
        b"time,probe,value,unit\ninf,tc1,1,degC\n",
        b"time,probe,value,unit\n1,tc 1,1,degC\n",
        b"time,probe,value,unit\n1,tc1,1,degC,x\n",
        b"time,probe,value\n1,tc1,1\n",
        b"time,probe,value,unit\n",
        b"time,probe,value,unit\n1,tc1,1,degC\n\n",
    ] {
        assert!(parse_measurements(bad, "temperature").is_err());
    }
    let mut too_many = String::from("time,probe,value,unit\n");
    for i in 0..65 {
        too_many.push_str(&format!("0,p{i},1,K\n"));
    }
    assert!(parse_measurements(too_many.as_bytes(), "temperature").is_err());

    let mut unmapped = r.clone();
    unmapped.probe_map.remove("tc2");
    assert!(correlate(&unmapped, &ok, &simulated).is_err());
    let mut missing = r.clone();
    missing
        .probe_map
        .insert("tc3".into(), SampleLocation::Node { node_id: 2 });
    assert!(correlate(&missing, &ok, &simulated).is_err());
    for window in [[-1., 50.], [20., 101.], [50., 50.], [f64::NAN, 1.]] {
        let mut outside = r.clone();
        outside.time_window = window;
        assert!(correlate(&outside, &ok, &simulated).is_err());
    }
    // Window inside the history but containing no shifted measurement.
    let mut empty = r.clone();
    empty.time_offset_s = 1000.;
    assert!(correlate(&empty, &ok, &simulated).is_err());
    let changes: [fn(&mut CorrelationRequest); 5] = [
        |r: &mut CorrelationRequest| r.provenance.rig = " ".into(),
        |r: &mut CorrelationRequest| r.provenance.sensor_accuracy = q(0.5, "degF"),
        |r: &mut CorrelationRequest| r.tolerance.rel = -1.,
        |r: &mut CorrelationRequest| r.dimension = "pressure".into(),
        |r: &mut CorrelationRequest| r.schema_version = 2,
    ];
    for change in changes {
        let mut bad = r.clone();
        change(&mut bad);
        assert!(correlate(&bad, &ok, &simulated).is_err());
    }
}

#[test]
fn worker_entry_validates_csv_before_store_and_confines_paths() {
    let root = tempfile::tempdir().unwrap();
    let store = harbor_cad::storage::Store::open(&root.path().join("state")).unwrap();
    let (csv, _) = fixture(0.);
    std::fs::write(root.path().join("test.csv"), &csv).unwrap();
    std::fs::write(
        root.path().join("bad.csv"),
        "time,probe,value,unit\n1,tc1,1,furlong\n",
    )
    .unwrap();
    let mut r = request();
    let error = run(&store, root.path(), &r).unwrap_err().to_string();
    assert!(!error.contains("invalid input"), "{error}");
    for path in ["bad.csv", "../test.csv", "/etc/passwd"] {
        r.measurement_csv = path.into();
        let error = run(&store, root.path(), &r).unwrap_err().to_string();
        assert!(error.contains("invalid input"), "{error}");
    }
}
