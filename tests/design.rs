use harbor_cad::{design::*, science::channel_reference, storage::Store, worker::request};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn num(params: &Params, name: &str) -> f64 {
    match &params[name] {
        ParamValue::Number(v) => *v,
        ParamValue::Choice(_) => panic!("numeric {name}"),
    }
}

/// Pure loop: propose, evaluate with `job`, record, until the proposer stops.
fn simulate(env: &DesignEnvelope, job: impl Fn(&Params, Level) -> Metrics) -> Vec<Candidate> {
    let mut history: Vec<Candidate> = vec![];
    while let Some(p) = propose(env, &history).unwrap() {
        let metrics = compute_metrics(env, p.fidelity, &p.params, &job(&p.params, p.fidelity), &[]);
        history.push(Candidate {
            index: history.len() as u32,
            origin: "proposer".into(),
            provenance: None,
            fidelity: p.fidelity,
            parent: p.parent,
            params: p.params,
            children: vec![],
            state: "evaluated".into(),
            metrics,
        });
        assert!(history.len() <= env.budget.max_evaluations as usize);
    }
    history
}

fn channel(params: &Params, _: Level) -> Metrics {
    let reference = channel_reference(
        num(params, "acceleration"),
        num(params, "height") * 1e-3,
        1e-5,
        33,
    )
    .unwrap();
    BTreeMap::from([("mean_velocity".into(), Some(reference.mean_velocity_m_s))])
}

/// Brute-force grid optimum of the example: (objective, height, acceleration).
fn example_optimum() -> (f64, f64, f64) {
    let mut best = (f64::INFINITY, 0., 0.);
    for h in 0..=16 {
        for a in 0..=45 {
            let (h, a) = (4. + h as f64, 0.05 + a as f64 * 0.01);
            let u = channel_reference(a, h * 1e-3, 1e-5, 33)
                .unwrap()
                .mean_velocity_m_s;
            let objective = 0.1 * h + a;
            if u >= 0.1 && objective < best.0 {
                best = (objective, h, a);
            }
        }
    }
    best
}

fn quadratic_env(seed: u32) -> DesignEnvelope {
    let mut env = example().unwrap();
    env.seed = seed;
    env.space.continuous = vec![
        ContinuousParam {
            name: "height".into(),
            low: 4.,
            high: 20.,
            step: 0.5,
            unit: "mm".into(),
        },
        ContinuousParam {
            name: "acceleration".into(),
            low: 0.05,
            high: 0.5,
            step: 0.01,
            unit: "m/s2".into(),
        },
    ];
    env
}

#[test]
fn example_file_is_the_valid_init_template() {
    let env = example().unwrap();
    let file: DesignEnvelope = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/design-envelope.json"
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&file).unwrap(),
        serde_json::to_value(&env).unwrap()
    );
    let prepared = prepare(&env, Some("ci")).unwrap();
    assert_eq!(prepared.policy, "ci");
    assert_eq!(prepared.screen_per_combination, 5);
    assert!(!prepared.executed);
    assert!(prepare(&env, Some("research")).is_err(), "policy is bound");
}

#[test]
fn envelope_refusals_cover_bounds_steps_limits_and_strict_placeholders() {
    let base = example().unwrap();
    let refuse = |change: &dyn Fn(&mut DesignEnvelope), why: &str| {
        let mut env = base.clone();
        change(&mut env);
        assert!(prepare(&env, Some("ci")).is_err(), "{why} accepted");
    };
    refuse(&|e| e.space.continuous[0].low = 30., "low >= high");
    refuse(
        &|e| e.space.continuous[0].step = 0.7,
        "non-integral step grid",
    );
    refuse(&|e| e.space.continuous[0].step = 0., "zero step");
    refuse(
        &|e| e.space.continuous[1].high = f64::NAN,
        "non-finite bound",
    );
    refuse(
        &|e| {
            let extra = e.space.continuous[0].clone();
            for i in 0..7 {
                let mut p = extra.clone();
                p.name = format!("extra{i}");
                e.space.continuous.push(p);
            }
        },
        ">8 continuous parameters",
    );
    refuse(&|e| e.budget.max_evaluations = 33, ">32 evaluations");
    refuse(&|e| e.budget.max_evaluations = 0, "zero evaluations");
    refuse(&|e| e.fidelity.refine_top_k = Some(4), "refine_top_k > 3");
    refuse(&|e| e.fidelity.refine = None, "refine_top_k without set");
    refuse(
        &|e| e.budget.max_artifact_bytes = 1024,
        "understated output budget",
    );
    refuse(
        &|e| {
            e.space.categorical.push(CategoricalParam {
                name: "material".into(),
                choices: (0..7).map(|i| format!("m{i}")).collect(),
            })
        },
        ">6 choices",
    );
    refuse(
        &|e| {
            e.templates.get_mut("screen").unwrap()[0].plan["case"]["length"] =
                json!({"$param": "wall"})
        },
        "unknown placeholder",
    );
    refuse(
        &|e| {
            e.templates.get_mut("screen").unwrap()[0].plan["case"]["length"] =
                json!({"$param": "height", "scale": 2})
        },
        "extended placeholder",
    );
    refuse(
        &|e| {
            e.templates.get_mut("screen").unwrap()[0].plan["case"]["channel_height"] =
                json!({"$param": "height", "unit": "m"})
        },
        "placeholder unit mismatch",
    );
    refuse(
        &|e| {
            e.space.continuous.push(ContinuousParam {
                name: "unused".into(),
                low: 0.,
                high: 1.,
                step: 0.5,
                unit: "m".into(),
            })
        },
        "unused parameter",
    );
    refuse(
        &|e| {
            // A known dimension still fails existing plan validation.
            e.space.continuous[0].unit = "m/s2".into();
            e.templates.get_mut("screen").unwrap()[0].plan["case"]["channel_height"] =
                json!({"$param": "height"});
            e.metric_sources[1].unit = "m/s2".into();
        },
        "wrong physical dimension",
    );
    refuse(
        &|e| e.constraints[0].unit = "km/h".into(),
        "constraint unit",
    );
    refuse(
        &|e| e.constraints[0].safety_factor = Some(0.5),
        "weak safety factor",
    );
    refuse(
        &|e| e.objective.terms[0].metric = "undeclared".into(),
        "undeclared objective metric",
    );
    refuse(
        &|e| {
            e.metric_sources[0].source = MetricSource::Job {
                role: "missing".into(),
                artifact: "validation.json".into(),
                pointer: "/mean_velocity_m_s".into(),
            }
        },
        "unknown role",
    );
    refuse(
        &|e| {
            e.metric_sources[0].source = MetricSource::Job {
                role: "flow".into(),
                artifact: "../escape.json".into(),
                pointer: "/x".into(),
            }
        },
        "traversing artifact",
    );
    refuse(
        &|e| {
            e.metric_sources[0].source = MetricSource::Provider {
                name: "unregistered".into(),
                params: vec![],
            }
        },
        "unregistered provider",
    );
    refuse(
        &|e| {
            e.templates
                .insert("spare".into(), e.templates["screen"].clone());
        },
        "unused template set",
    );
    let mut unknown = serde_json::to_value(&base).unwrap();
    unknown["surrogate"] = json!("gp");
    assert!(serde_json::from_value::<DesignEnvelope>(unknown).is_err());
}

#[test]
fn proposer_is_seeded_deterministic_and_always_on_grid() {
    let mut env = quadratic_env(11);
    env.space.categorical.push(CategoricalParam {
        name: "material".into(),
        choices: vec!["pc".into(), "asa".into(), "pa12".into()],
    });
    let job = |p: &Params, _: Level| channel(p, Level::Screen);
    env.templates.get_mut("screen").unwrap()[0].plan["case"]["material"]["name"] =
        json!({"$param": "material"});
    env.templates.get_mut("refine").unwrap()[0].plan["case"]["material"]["name"] =
        json!({"$param": "material"});
    prepare(&env, Some("ci")).unwrap();
    let first = simulate(&env, job);
    let second = simulate(&env, job);
    assert_eq!(
        first.iter().map(|c| &c.params).collect::<Vec<_>>(),
        second.iter().map(|c| &c.params).collect::<Vec<_>>()
    );
    let mut other = env.clone();
    other.seed = 12;
    assert_ne!(
        simulate(&other, job)
            .iter()
            .map(|c| c.params.clone())
            .collect::<Vec<_>>(),
        first.iter().map(|c| c.params.clone()).collect::<Vec<_>>()
    );
    let materials: std::collections::BTreeSet<_> = first
        .iter()
        .map(|c| match &c.params["material"] {
            ParamValue::Choice(c) => c.clone(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(materials.len(), 3, "every categorical combination screened");
    for c in &first {
        validate_candidate(
            &env,
            &ExternalCandidate {
                params: c.params.clone(),
                provenance: "replay".into(),
            },
        )
        .unwrap();
        for (param, value) in &c.params {
            if let ParamValue::Number(v) = value {
                let p = env
                    .space
                    .continuous
                    .iter()
                    .find(|p| &p.name == param)
                    .unwrap();
                assert!((p.low..=p.high).contains(v));
                let g = (v - p.low) / p.step;
                assert!((g - g.round()).abs() < 1e-9);
            }
        }
        // Every proposal also materializes to validated plans.
        child_plans(&env, c.fidelity, &c.params, "ci").unwrap();
    }
    assert!(
        first.iter().filter(|c| c.fidelity == Level::Screen).count()
            <= (env.budget.max_evaluations - 1) as usize
    );
}

#[test]
fn feasibility_first_ranking_never_passes_unknown_metrics() {
    let env = example().unwrap();
    let metrics = |u: Option<f64>, h: f64, a: f64| {
        BTreeMap::from([
            ("mean_velocity".to_string(), u),
            ("height".to_string(), Some(h)),
            ("acceleration".to_string(), Some(a)),
        ])
    };
    let feasible_cheap = assess(&env, &metrics(Some(0.2), 5., 0.3));
    let feasible_dear = assess(&env, &metrics(Some(0.5), 10., 0.4));
    let slightly_infeasible = assess(&env, &metrics(Some(0.09), 4., 0.05));
    let very_infeasible = assess(&env, &metrics(Some(0.01), 4., 0.05));
    let unknown = assess(&env, &metrics(None, 4., 0.05));
    assert_eq!(feasible_cheap.standing, Standing::Feasible);
    assert_eq!(unknown.standing, Standing::Unknown);
    assert_eq!(unknown.constraints[0].satisfied, None);
    assert!(unknown.min_margin.is_none());
    assert_eq!(slightly_infeasible.standing, Standing::Infeasible);
    assert!((slightly_infeasible.violation - 0.1).abs() < 1e-12);
    let mut items = vec![
        (0, unknown),
        (1, very_infeasible),
        (2, feasible_dear),
        (3, slightly_infeasible),
        (4, feasible_cheap),
    ];
    rank(&mut items);
    assert_eq!(
        items.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        [4, 2, 3, 1, 0]
    );
    let mut safe = env.clone();
    safe.constraints[0].safety_factor = Some(2.);
    assert_eq!(
        assess(&safe, &metrics(Some(0.15), 5., 0.3)).standing,
        Standing::Infeasible
    );
    // A missing objective metric is unknown, not feasible.
    let mut missing = metrics(Some(0.2), 5., 0.3);
    missing.insert("height".into(), None);
    assert_eq!(assess(&env, &missing).standing, Standing::Unknown);
}

#[test]
fn pattern_search_converges_on_analytic_quadratic_within_budget() {
    // Unconstrained smooth bowl with a known on-grid minimum at (13.5 mm, 0.21).
    let f = |p: &Params, _: Level| {
        let (h, a) = (num(p, "height"), num(p, "acceleration"));
        BTreeMap::from([(
            "mean_velocity".into(),
            Some((h - 13.5).powi(2) + 400. * (a - 0.21).powi(2)),
        )])
    };
    for seed in 0..24 {
        let mut bowl = quadratic_env(seed);
        bowl.constraints.clear();
        bowl.fidelity.refine_top_k = None;
        bowl.fidelity.refine = None;
        bowl.templates.remove("refine");
        bowl.objective.terms = vec![ObjectiveTerm {
            metric: "mean_velocity".into(),
            weight: 1.,
        }];
        prepare(&bowl, Some("ci")).unwrap();
        let history = simulate(&bowl, f);
        let best = &history[incumbent(&bowl, &history).unwrap() as usize];
        let trace: Vec<_> = history
            .iter()
            .map(|c| (num(&c.params, "height"), num(&c.params, "acceleration")))
            .collect();
        assert!(history.len() <= 32);
        assert_eq!(num(&best.params, "height"), 13.5, "seed {seed}: {trace:?}");
        assert!(
            (num(&best.params, "acceleration") - 0.21).abs() < 1e-12,
            "seed {seed}: {trace:?}"
        );
    }
}

#[test]
fn example_loop_reaches_the_known_constrained_optimum() {
    let env = example().unwrap();
    let history = simulate(&env, channel);
    let (optimum, h, a) = example_optimum();
    assert!(h == 6. && (a - 0.34).abs() < 1e-12, "{h} {a}");
    let best = &history[incumbent(&env, &history).unwrap() as usize];
    assert_eq!(best.fidelity, Level::Refine);
    let objective = assess(&env, &best.metrics).objective.unwrap();
    assert!(
        objective <= optimum * 1.02,
        "incumbent {objective} vs optimum {optimum}: {:?}",
        best.params
    );
}

#[test]
fn constrained_example_seed_sweep_beats_the_single_start_baseline() {
    // Single-start compass proposer at trunk c8a66b5 over seeds 0..40:
    // worst gap 27.66 %, median 5.32 %, 6 seeds exactly on the optimum.
    const BEFORE_WORST: f64 = 0.2766;
    const BEFORE_MEDIAN: f64 = 0.0532;
    const BEFORE_EXACT: usize = 6;
    let (optimum, ..) = example_optimum();
    let mut gaps: Vec<f64> = (0..40)
        .map(|seed| {
            let mut env = example().unwrap();
            env.seed = seed;
            let history = simulate(&env, channel);
            let best = &history[incumbent(&env, &history).unwrap() as usize];
            (assess(&env, &best.metrics).objective.unwrap() - optimum) / optimum
        })
        .collect();
    gaps.sort_by(f64::total_cmp);
    let (worst, median) = (gaps[39], (gaps[19] + gaps[20]) / 2.);
    let exact = gaps.iter().filter(|g| **g < 1e-12).count();
    let stats = format!("worst {worst:.4} median {median:.4} exact {exact}/40");
    assert!(worst < BEFORE_WORST, "{stats}");
    assert!(median <= BEFORE_MEDIAN, "{stats}");
    assert!(exact >= BEFORE_EXACT, "{stats}");
}

#[test]
fn proposals_regenerate_from_persisted_history_after_restart() {
    // Resume re-reads the candidate rows and calls `propose` again, so every
    // proposal must be a pure function of the stored history, including an
    // interrupted (unknown) evaluation and an external candidate mid-run.
    let env = example().unwrap();
    let mut history: Vec<Candidate> = vec![];
    let mut proposals = vec![];
    while let Some(p) = propose(&env, &history).unwrap() {
        let index = history.len() as u32;
        let (origin, params) = if index == 12 {
            let mut params = p.params.clone();
            params.insert("height".into(), ParamValue::Number(19.));
            params.insert("acceleration".into(), ParamValue::Number(0.49));
            ("external", params)
        } else {
            proposals.push((index, p.clone()));
            ("proposer", p.params.clone())
        };
        let mut metrics = compute_metrics(
            &env,
            p.fidelity,
            &params,
            &channel(&params, p.fidelity),
            &[],
        );
        if index == 9 {
            metrics.insert("mean_velocity".into(), None);
        }
        history.push(Candidate {
            index,
            origin: origin.into(),
            provenance: (origin == "external").then(|| "agent".into()),
            fidelity: if origin == "external" {
                Level::Screen
            } else {
                p.fidelity
            },
            parent: if origin == "external" { None } else { p.parent },
            params,
            children: vec![],
            state: "evaluated".into(),
            metrics,
        });
    }
    assert_eq!(history.len(), env.budget.max_evaluations as usize);
    let screened: std::collections::BTreeSet<_> = history
        .iter()
        .filter(|c| c.fidelity == Level::Screen)
        .map(|c| serde_json::to_string(&c.params).unwrap())
        .collect();
    assert_eq!(
        screened.len(),
        history.len() - 1,
        "no screen point is proposed twice"
    );
    for (index, expected) in proposals {
        let stored: Vec<Candidate> =
            serde_json::from_str(&serde_json::to_string(&history[..index as usize]).unwrap())
                .unwrap();
        assert_eq!(
            propose(&env, &stored).unwrap().as_ref(),
            Some(&expected),
            "{index}"
        );
    }
}

struct Constant;
impl MetricProvider for Constant {
    fn name(&self) -> &'static str {
        "constant_cost"
    }
    fn unit(&self) -> &'static str {
        "EUR"
    }
    fn evidence(&self) -> &'static str {
        "estimated"
    }
    fn evaluate(
        &self,
        params: &Params,
        metrics: &BTreeMap<String, f64>,
    ) -> harbor_cad::Result<Option<f64>> {
        assert_eq!(params.len(), 1, "only declared parameters reach providers");
        Ok(metrics.get("mean_velocity").map(|u| u + 1.))
    }
}

#[test]
fn provider_metrics_see_declared_params_and_missing_inputs_stay_unknown() {
    let mut env = example().unwrap();
    env.metric_sources.push(MetricSpec {
        metric: "unit_cost".into(),
        unit: "EUR".into(),
        source: MetricSource::Provider {
            name: "constant_cost".into(),
            params: vec!["height".into()],
        },
    });
    let params = Params::from([
        ("height".into(), ParamValue::Number(6.)),
        ("acceleration".into(), ParamValue::Number(0.34)),
    ]);
    let known = BTreeMap::from([("mean_velocity".into(), Some(0.5))]);
    let metrics = compute_metrics(&env, Level::Screen, &params, &known, &[&Constant]);
    assert_eq!(metrics["unit_cost"], Some(1.5));
    assert_eq!(metrics["height"], Some(6.));
    let missing = compute_metrics(&env, Level::Screen, &params, &BTreeMap::new(), &[&Constant]);
    assert_eq!(missing["unit_cost"], None);
    assert_eq!(missing["mean_velocity"], None);
    assert_eq!(
        compute_metrics(&env, Level::Screen, &params, &known, &[])["unit_cost"],
        None
    );
}

#[test]
fn external_candidates_are_bounded_on_grid_and_complete() {
    let env = example().unwrap();
    let candidate = |h: serde_json::Value, a: serde_json::Value| -> ExternalCandidate {
        serde_json::from_value(json!({"params":{"height":h,"acceleration":a},"provenance":"agent"}))
            .unwrap()
    };
    validate_candidate(&env, &candidate(json!(6.0), json!(0.34))).unwrap();
    for (h, a) in [
        (json!(6.5), json!(0.34)),
        (json!(3.0), json!(0.34)),
        (json!(6.0), json!(0.51)),
        (json!(6.0), json!(0.345)),
        (json!("pc"), json!(0.34)),
    ] {
        assert!(
            validate_candidate(&env, &candidate(h.clone(), a.clone())).is_err(),
            "{h} {a}"
        );
    }
    let mut extra = candidate(json!(6.0), json!(0.34));
    extra.params.insert("wall".into(), ParamValue::Number(2.));
    assert!(validate_candidate(&env, &extra).is_err());
    let mut missing = candidate(json!(6.0), json!(0.34));
    missing.params.remove("acceleration");
    assert!(validate_candidate(&env, &missing).is_err());
    let mut anonymous = candidate(json!(6.0), json!(0.34));
    anonymous.provenance = " ".into();
    assert!(validate_candidate(&env, &anonymous).is_err());
    assert!(
        serde_json::from_value::<ExternalCandidate>(
            json!({"params":{},"provenance":"x","priority":1})
        )
        .is_err()
    );
}

// ---- real worker under the CI profile ------------------------------------

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn profile() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/profiles/ci.json"))
}
fn start(root: &Path) -> Worker {
    let _ = std::fs::remove_file(root.join("worker.sock"));
    let child = Command::new(env!("CARGO_BIN_EXE_harbor-cad"))
        .args(["worker", "--state"])
        .arg(root)
        .arg("--profile")
        .arg(profile())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now();
    while !root.join("worker.sock").exists() {
        assert!(deadline.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    Worker(child)
}
fn call(root: &Path, operation: serde_json::Value) -> harbor_cad::worker::Response {
    request(
        &root.join("worker.sock"),
        serde_json::from_value(operation).unwrap(),
    )
    .unwrap()
}
fn design_status(root: &Path, id: &str) -> serde_json::Value {
    let response = call(root, json!({"operation":"design_status","design_id":id}));
    assert!(response.ok, "{response:?}");
    response.data.unwrap()
}
fn wait_until(
    root: &Path,
    id: &str,
    done: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now();
    loop {
        let status = design_status(root, id);
        if done(&status) {
            return status;
        }
        assert!(deadline.elapsed() < Duration::from_secs(120), "{status}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn job_count(root: &Path) -> i64 {
    Store::open(root)
        .unwrap()
        .connection
        .query_row("SELECT count(*) FROM jobs", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn worker_runs_approved_design_to_known_optimum_and_resumes_without_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let env = example().unwrap();
    let mut worker = start(&root);

    let planned = call(&root, json!({"operation":"design_plan","envelope":env}));
    assert!(planned.ok, "{planned:?}");
    let digest = planned.data.unwrap()["approval_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let rejected = call(
        &root,
        json!({"operation":"design_submit","envelope":env,"approved_digest":"0".repeat(64),"idempotency_key":"loop"}),
    );
    assert_eq!(rejected.error.unwrap()["code"], "invalid_input");
    assert_eq!(job_count(&root), 0);

    let submitted = call(
        &root,
        json!({"operation":"design_submit","envelope":env,"approved_digest":digest,"idempotency_key":"loop"}),
    );
    assert!(submitted.ok, "{submitted:?}");
    let submitted = submitted.data.unwrap();
    let id = submitted["id"].as_str().unwrap().to_string();
    assert_eq!(submitted["iterations"].as_array().unwrap().len(), 1);
    assert!(submitted["iterations"][0]["jobs"][0]["job_id"].is_string());

    // Crash the worker mid-loop and restart it on the same state root.
    wait_until(&root, &id, |s| {
        s["budget"]["evaluations"].as_u64().unwrap() >= 4
    });
    drop(worker);
    worker = start(&root);
    let status = wait_until(&root, &id, |s| s["state"] != "running");
    // Multi-start spends the whole budget unless every walk converges first.
    assert!(
        ["completed", "budget_exhausted"].contains(&status["state"].as_str().unwrap()),
        "{status}"
    );
    let iterations = status["iterations"].as_array().unwrap();
    assert!(iterations.len() <= 32);
    let children: usize = iterations
        .iter()
        .map(|i| i["jobs"].as_array().unwrap().len())
        .sum();
    assert_eq!(
        job_count(&root) as usize,
        children,
        "no duplicate child jobs"
    );
    let ids: std::collections::BTreeSet<_> = iterations
        .iter()
        .flat_map(|i| i["jobs"].as_array().unwrap())
        .map(|j| j["job_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), children);

    // Restart regenerated the same deterministic sequence unless the crash
    // interrupted a child (which then stays an unknown evaluation).
    if iterations
        .iter()
        .all(|i| i["jobs"][0]["state"] == "succeeded")
    {
        let expected: Vec<_> = simulate(&env, channel)
            .into_iter()
            .map(|c| serde_json::to_value(c.params).unwrap())
            .collect();
        let observed: Vec<_> = iterations.iter().map(|i| i["params"].clone()).collect();
        assert_eq!(observed, expected);
    }
    let (optimum, ..) = example_optimum();
    let best = &iterations[status["incumbent"].as_u64().unwrap() as usize];
    assert_eq!(best["fidelity"], "refine");
    assert!(
        best["objective"].as_f64().unwrap() <= optimum * 1.02,
        "{best}"
    );

    let report = call(&root, json!({"operation":"design_export","design_id":id}));
    assert!(report.ok, "{report:?}");
    let report = report.data.unwrap();
    assert_eq!(report["winner"]["index"], best["index"]);
    assert_eq!(report["winner"]["constraints"][0]["evidence"], "simulated");
    assert_eq!(report["physical_validation"], "unqualified");
    assert_eq!(report["requirements"], json!([]));
    assert!(
        report["statement"]
            .as_str()
            .unwrap()
            .contains("not physical validation")
    );
    let winner_job = report["winner"]["jobs"][0]["job_id"].as_str().unwrap();
    assert_eq!(
        call(&root, json!({"operation":"status","job_id":winner_job}))
            .data
            .unwrap()["state"],
        "succeeded"
    );

    // Repeating the approval resumes the same design; a changed envelope under
    // the same key is rejected and creates nothing.
    let before = job_count(&root);
    let repeat = call(
        &root,
        json!({"operation":"design_submit","envelope":env,"approved_digest":digest,"idempotency_key":"loop"}),
    );
    assert_eq!(repeat.data.unwrap()["id"], id);
    let mut changed = env.clone();
    changed.provenance.push_str(" changed");
    let changed_digest = prepare(&changed, Some("ci")).unwrap().approval_digest;
    let conflict = call(
        &root,
        json!({"operation":"design_submit","envelope":changed,"approved_digest":changed_digest,"idempotency_key":"loop"}),
    );
    assert_eq!(conflict.error.unwrap()["code"], "idempotency_conflict");
    assert_eq!(job_count(&root), before);

    // External proposals are refused once the design has stopped, but an
    // already-evaluated point is returned rather than re-run.
    let replay = call(
        &root,
        json!({"operation":"design_propose","design_id":id,"candidate":{"params":iterations[0]["params"],"provenance":"agent replay"}}),
    );
    assert_eq!(replay.data.unwrap()["candidate"]["index"], 0);
    let fresh = call(
        &root,
        json!({"operation":"design_propose","design_id":id,"candidate":{"params":{"height":19.0,"acceleration":0.49},"provenance":"agent"}}),
    );
    assert_eq!(fresh.error.unwrap()["code"], "invalid_input");
    drop(worker);
}

#[test]
fn external_proposal_is_evaluated_like_a_proposal_through_the_cli() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let _worker = start(&root);
    let mut env = example().unwrap();
    env.budget.max_evaluations = 8;
    env.budget.max_artifact_bytes = 8 * 1024 * 1024;
    let envelope = temp.path().join("envelope.json");
    std::fs::write(&envelope, serde_json::to_vec(&env).unwrap()).unwrap();
    let cli = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_harbor-cad"))
            .arg("--socket")
            .arg(root.join("worker.sock"))
            .args(args)
            .output()
            .unwrap();
        (
            output.status.success(),
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        )
    };
    let (ok, planned) = cli(&["design", "plan", envelope.to_str().unwrap()]);
    assert!(ok, "{planned}");
    let digest = planned["approval_digest"].as_str().unwrap();
    let (ok, submitted) = cli(&[
        "design",
        "submit",
        envelope.to_str().unwrap(),
        "--approve",
        digest,
        "--idempotency-key",
        "external",
    ]);
    assert!(ok, "{submitted}");
    let id = submitted["data"]["id"].as_str().unwrap().to_string();
    let candidate = temp.path().join("candidate.json");
    std::fs::write(
        &candidate,
        serde_json::to_vec(
            &json!({"params":{"height":6.0,"acceleration":0.34},"provenance":"LLM agent via MCP"}),
        )
        .unwrap(),
    )
    .unwrap();
    let (ok, proposed) = cli(&["design", "propose", &id, candidate.to_str().unwrap()]);
    assert!(ok, "{proposed}");
    assert_eq!(proposed["data"]["candidate"]["origin"], "external");
    let index = proposed["data"]["candidate"]["index"].as_u64().unwrap();
    std::fs::write(
        &candidate,
        serde_json::to_vec(
            &json!({"params":{"height":6.5,"acceleration":0.34},"provenance":"off grid"}),
        )
        .unwrap(),
    )
    .unwrap();
    let (ok, refused) = cli(&["design", "propose", &id, candidate.to_str().unwrap()]);
    assert!(!ok && refused["data"].is_null(), "{refused}");
    let status = wait_until(&root, &id, |s| s["state"] != "running");
    assert_eq!(status["state"], "budget_exhausted", "{status}");
    let external = &status["iterations"][index as usize];
    assert_eq!(external["standing"], "feasible");
    assert_eq!(external["jobs"][0]["state"], "succeeded");
    assert!(external["metrics"]["mean_velocity"].as_f64().unwrap() >= 0.1);
    let (ok, report) = cli(&["design", "export", &id]);
    assert!(ok, "{report}");
    // The refined re-evaluation of the external optimum wins.
    assert_eq!(report["data"]["winner"]["params"], external["params"]);
    assert_eq!(report["data"]["winner"]["fidelity"], "refine");
}

// ---- chained templates, declared inputs and manufacturing providers -------

const MIB: u64 = 1024 * 1024;

/// CI-runnable two-stage chain: `derived` is planned by the ordinary `plan`
/// operation from `flow`'s registered mean velocity (used as its drive), so a
/// derived metric proves the upstream output reached the downstream plan.
/// Synthetic plumbing fixture with no physical meaning.
fn chain_env(evaluations: u32) -> DesignEnvelope {
    let mut env = example().unwrap();
    env.fidelity = Fidelity {
        screen: "screen".into(),
        refine_top_k: None,
        refine: None,
    };
    env.templates.remove("refine");
    env.budget.max_evaluations = evaluations;
    env.budget.max_artifact_bytes = u64::from(evaluations) * 2 * MIB;
    let mut case = env.templates["screen"][0].plan["case"].clone();
    case["acceleration"] = json!({"$job": "flow", "artifact": "validation.json",
        "pointer": "/mean_velocity_m_s", "unit": "m/s2"});
    env.templates
        .get_mut("screen")
        .unwrap()
        .push(ChildTemplate {
            role: "derived".into(),
            plan: serde_json::Value::Null,
            plan_request: Some(json!({"operation": "plan", "case": case})),
            after: Some("flow".into()),
            max_artifact_bytes: Some(MIB),
        });
    env.metric_sources.push(MetricSpec {
        metric: "derived_velocity".into(),
        unit: "m/s".into(),
        source: MetricSource::Job {
            role: "derived".into(),
            artifact: "validation.json".into(),
            pointer: "/mean_velocity_m_s".into(),
        },
    });
    env
}

fn chained(env: &mut DesignEnvelope) -> &mut ChildTemplate {
    &mut env.templates.get_mut("screen").unwrap()[1]
}

fn declared(metric: &str, unit: &str, input: serde_json::Value) -> MetricSpec {
    serde_json::from_value(json!({"metric": metric, "unit": unit,
        "source": {"kind": "declared", "input": input}}))
    .unwrap()
}

#[test]
fn chained_templates_are_allowlisted_upstream_bound_and_strict() {
    let base = chain_env(4);
    let prepared = prepare(&base, Some("ci")).unwrap();
    assert_eq!(prepared.candidate_artifact_bytes["screen"], 2 * MIB);
    let refuse = |change: &dyn Fn(&mut DesignEnvelope), why: &str| {
        let mut env = base.clone();
        change(&mut env);
        assert!(prepare(&env, Some("ci")).is_err(), "{why} accepted");
    };
    refuse(
        &|e| chained(e).plan_request.as_mut().unwrap()["operation"] = json!("submit"),
        "non-planning operation",
    );
    refuse(
        &|e| {
            chained(e).plan_request = Some(
                json!({"operation": "plan_cad_inspection", "case": {}, "max_artifact_bytes": 1}),
            )
        },
        "planning operation outside the allowlist",
    );
    refuse(
        &|e| chained(e).after = Some("derived".into()),
        "self dependency",
    );
    refuse(
        &|e| chained(e).after = Some("missing".into()),
        "unknown upstream",
    );
    refuse(&|e| chained(e).after = None, "$job without an upstream");
    refuse(
        &|e| chained(e).max_artifact_bytes = None,
        "unbounded derived output",
    );
    refuse(
        &|e| {
            let plan = e.templates["screen"][0].plan.clone();
            chained(e).plan = plan;
        },
        "plan and plan_request together",
    );
    refuse(
        &|e| chained(e).plan_request.as_mut().unwrap()["case"]["surrogate"] = json!(1),
        "request schema",
    );
    refuse(
        &|e| {
            chained(e).plan_request.as_mut().unwrap()["case"]["acceleration"] =
                json!({"$job": "flow", "artifact": "validation.json"})
        },
        "artifact without pointer",
    );
    refuse(
        &|e| {
            chained(e).plan_request.as_mut().unwrap()["case"]["acceleration"] =
                json!({"$job": "flow", "artifact": "../x.json", "pointer": "/a", "unit": "m/s2"})
        },
        "traversing upstream artifact",
    );
    refuse(
        &|e| {
            e.templates.get_mut("screen").unwrap()[0].plan["case"]["name"] = json!({"$job": "flow"})
        },
        "$job in a complete plan",
    );
    refuse(
        &|e| {
            chained(e).plan_request.as_mut().unwrap()["case"]["max_time_s"] =
                json!({"$input": "undeclared", "si": "time"})
        },
        "$input naming no declared source",
    );
    refuse(
        &|e| {
            e.metric_sources.push(declared(
                "ambient",
                "K",
                json!({"availability": "missing", "reason": " "}),
            ))
        },
        "declared input without a reason",
    );
    let correlation = |request: serde_json::Value, unit: &str| -> MetricSpec {
        serde_json::from_value(json!({"metric": "rmse", "unit": unit,
            "source": {"kind": "correlation", "role": "flow", "request": request}}))
        .unwrap()
    };
    let request = json!({"schema_version": 1, "measurement_csv": "run.csv", "dimension": "temperature",
        "probe_map": {"tc1": {"association": "node", "node_id": 1}}, "time_offset_s": 0.0,
        "time_window": [0.0, 1.0], "tolerance": {"abs": {"value": 1.0, "unit": "K"}, "rel": 0.0},
        "provenance": {"test_id": "t", "rig": "r", "sensor_type": "s",
            "sensor_accuracy": {"value": 0.5, "unit": "K"}, "date": "d"}});
    let mut accepted = base.clone();
    accepted
        .metric_sources
        .push(correlation(request.clone(), "K"));
    accepted.metric_sources.push(declared(
        "ambient",
        "K",
        json!({"availability": "missing", "reason": "site not chosen"}),
    ));
    prepare(&accepted, Some("ci")).unwrap();
    let mut with_job = request.clone();
    with_job["job_id"] = json!(uuid_nil());
    refuse(
        &|e| e.metric_sources.push(correlation(with_job.clone(), "K")),
        "correlation request carrying its own job",
    );
    refuse(
        &|e| e.metric_sources.push(correlation(request.clone(), "degC")),
        "correlation metric not in K",
    );
}

fn uuid_nil() -> String {
    "00000000-0000-0000-0000-000000000000".into()
}

#[test]
fn declared_inputs_resolve_in_requests_and_missing_stays_unknown() {
    let mut env = chain_env(4);
    env.metric_sources.push(declared(
        "horizon",
        "min",
        json!({"availability": "known", "value": 0.5, "provenance": "test", "synthetic": true}),
    ));
    env.metric_sources.push(declared(
        "ambient",
        "K",
        json!({"availability": "missing", "reason": "site not chosen"}),
    ));
    chained(&mut env).plan_request.as_mut().unwrap()["case"]["max_time_s"] =
        json!({"$input": "ambient", "si": "time"});
    assert!(
        prepare(&env, Some("ci")).is_err(),
        "a K input cannot become a time"
    );
    chained(&mut env).plan_request.as_mut().unwrap()["case"]["max_time_s"] =
        json!({"$input": "horizon", "si": "time"});
    prepare(&env, Some("ci")).unwrap();
    let params = Params::from([
        ("height".into(), ParamValue::Number(6.)),
        ("acceleration".into(), ParamValue::Number(0.34)),
    ]);
    let metrics = compute_metrics(&env, Level::Screen, &params, &BTreeMap::new(), &[]);
    assert_eq!(metrics["horizon"], Some(0.5));
    assert_eq!(metrics["ambient"], None);
}

fn provider_value(name: &str, params: &Params, metrics: &[(&str, f64)]) -> Option<f64> {
    let metrics: BTreeMap<String, f64> = metrics.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    PROVIDERS
        .iter()
        .find(|p| p.name() == name)
        .unwrap()
        .evaluate(params, &metrics)
        .ok()
        .flatten()
}

#[test]
fn component_margin_uses_the_hotter_of_board_and_internal_air() {
    let none = Params::new();
    let limit = ("component_max_temperature_k", 398.15);
    let board = ("pcb_max_temperature_k", 330.);
    let m =
        |metrics: &[(&str, f64)]| provider_value("component_temperature_margin_k", &none, metrics);
    assert_eq!(m(&[limit, board]), Some(398.15 - 330.));
    assert_eq!(
        m(&[limit, board, ("internal_air_temperature_k", 345.)]),
        Some(398.15 - 345.)
    );
    assert_eq!(
        m(&[limit, board, ("internal_air_temperature_k", 320.)]),
        Some(398.15 - 330.)
    );
    assert_eq!(m(&[limit, ("internal_air_temperature_k", 345.)]), None);
}

#[test]
fn power_sweep_superposes_two_solves_clamps_and_verifies_on_refine() {
    let al = Params::from([(
        "route".into(),
        ParamValue::Choice("al-6061-t6__cnc-3axis-al__powder-coat-white".into()),
    )]);
    let sweep = PROVIDERS
        .iter()
        .find(|p| p.name() == "max_supported_pcb_power_w")
        .unwrap();
    let run = |level, params: &Params, metrics: &[(&str, f64)]| {
        let metrics: BTreeMap<String, f64> =
            metrics.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        sweep.evaluate_at(level, params, &metrics).unwrap()
    };
    // Board: 340 K at 1 W rising 5 K/W; housing: 330 K rising 3 K/W. The
    // 6061-T6 conservative service maximum is 373.15 K, less the 10 K margin.
    let base = vec![
        ("pcb_power_low_w", 1.),
        ("pcb_power_high_w", 15.),
        ("component_max_temperature_k", 358.15),
        ("housing_margin_required_k", 10.),
        ("pcb_max_temperature_k", 340.),
        ("internal_air_temperature_k", 335.),
        ("max_temperature_k", 330.),
        ("hi_pcb_max_temperature_k", 410.),
        ("hi_internal_air_temperature_k", 380.),
        ("hi_max_temperature_k", 372.),
    ];
    let with = |changes: &[(&'static str, f64)]| {
        let mut m = base.clone();
        for (k, v) in changes {
            match m.iter_mut().find(|(n, _)| n == k) {
                Some(entry) => entry.1 = *v,
                None => m.push((k, *v)),
            }
        }
        m
    };
    let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-9);
    // The component limit on the board binds ...
    let board = 1. + 18.15 / 5.;
    assert!(close(run(Level::Screen, &al, &base), board));
    // ... or on the internal air (components sit between board and air) ...
    assert!(close(
        run(
            Level::Screen,
            &al,
            &with(&[
                ("internal_air_temperature_k", 350.),
                ("hi_internal_air_temperature_k", 420.)
            ])
        ),
        1. + 8.15 / 5.
    ));
    // ... else the housing service limit less the margin binds ...
    let housing = 1. + 33.15 / 3.;
    assert!(close(
        run(
            Level::Screen,
            &al,
            &with(&[("component_max_temperature_k", 400.)])
        ),
        housing
    ));
    // ... and the result is clamped to the swept range.
    let cool = with(&[
        ("component_max_temperature_k", 500.),
        ("hi_max_temperature_k", 340.),
    ]);
    assert!(close(run(Level::Screen, &al, &cool), 15.));
    // Even 1 W fails, power does not raise a peak, or an input is unknown: None.
    assert_eq!(
        run(
            Level::Screen,
            &al,
            &with(&[("component_max_temperature_k", 339.)])
        ),
        None
    );
    assert_eq!(
        run(Level::Screen, &al, &with(&[("hi_max_temperature_k", 330.)])),
        None
    );
    let unknown: Vec<_> = base
        .iter()
        .copied()
        .filter(|(k, _)| *k != "hi_internal_air_temperature_k")
        .collect();
    assert_eq!(run(Level::Screen, &al, &unknown), None);
    assert_eq!(run(Level::Screen, &Params::new(), &base), None);
    // Refine: the third solve at the supported power must match the
    // superposed prediction within 0.5 K, else the metric is unknown.
    assert_eq!(run(Level::Refine, &al, &base), None);
    let t = (board - 1.) / 14.;
    let check = |offset: f64| {
        with(&[
            ("check_pcb_max_temperature_k", 340. + 70. * t + offset),
            ("check_internal_air_temperature_k", 335. + 45. * t),
            ("check_max_temperature_k", 330. + 42. * t),
        ])
    };
    assert!(close(run(Level::Refine, &al, &check(0.)), board));
    assert!(close(run(Level::Refine, &al, &check(0.4)), board));
    assert_eq!(run(Level::Refine, &al, &check(0.6)), None);
    // Screen ignores a verification solve's absence and presence alike.
    assert!(close(run(Level::Screen, &al, &check(0.6)), board));
}

#[test]
fn manufacturing_providers_reuse_w1_and_never_fabricate_missing_inputs() {
    use harbor_cad::manufacturing::{GeometrySummary, Library, manufacturability, unit_cost};
    let library = Library::builtin().unwrap();
    let route = |r: &str| Params::from([("route".into(), ParamValue::Choice(r.into()))]);
    let al = route("al-6061-t6__cnc-3axis-al");
    let geometry = [
        ("volume_m3", 4e-4),
        ("xmin_m", 0.),
        ("xmax_m", 0.2),
        ("ymin_m", 0.),
        ("ymax_m", 0.15),
        ("zmin_m", 0.01),
        ("zmax_m", 0.09),
        ("min_wall_m", 0.004),
    ];
    let with = |extra: &[(&'static str, f64)]| [&geometry[..], extra].concat();
    let summary = GeometrySummary {
        volume_m3: 4e-4,
        bbox_m: [0.2, 0.15, 0.08],
        min_wall_m: Some(0.004),
        surface_area_m2: None,
    };
    let estimate = unit_cost(&library, &summary, "al-6061-t6", "cnc-3axis-al", 50).unwrap();
    for (name, expected) in [
        ("unit_cost_low_usd", estimate.low),
        ("unit_cost_high_usd", estimate.high),
    ] {
        let value = provider_value(name, &al, &with(&[("quantity", 50.)])).unwrap();
        assert!((value - expected.unwrap()).abs() < 1e-9, "{name}");
    }
    assert!(estimate.high > estimate.low);
    assert_eq!(provider_value("unit_cost_high_usd", &al, &geometry), None);
    assert_eq!(
        provider_value("unit_cost_high_usd", &al, &with(&[("quantity", 50.5)])),
        None
    );
    assert_eq!(
        provider_value(
            "unit_cost_high_usd",
            &Params::new(),
            &with(&[("quantity", 50.)])
        ),
        None,
        "no route, no estimate"
    );
    let incompatible = route("pa12-mjf__cnc-3axis-al");
    assert_eq!(
        provider_value(
            "unit_cost_high_usd",
            &incompatible,
            &with(&[("quantity", 50.)])
        ),
        None
    );
    assert!(
        provider_value("manufacturability_violations", &incompatible, &geometry).unwrap() >= 1.
    );
    let found = manufacturability(&library, &summary, "al-6061-t6", "cnc-3axis-al").unwrap();
    assert_eq!(
        provider_value("manufacturability_violations", &al, &geometry),
        Some(found.len() as f64)
    );
    let no_wall: Vec<_> = geometry
        .iter()
        .copied()
        .filter(|(k, _)| *k != "min_wall_m")
        .collect();
    assert_eq!(
        provider_value("manufacturability_violations", &al, &no_wall),
        None,
        "unknown wall is not zero violations"
    );
    let thin = with(&[])
        .into_iter()
        .map(|(k, v)| (k, if k == "min_wall_m" { 0.0003 } else { v }))
        .collect::<Vec<_>>();
    assert!(provider_value("manufacturability_violations", &al, &thin).unwrap() >= 1.);
    let mass = provider_value("mass_kg", &al, &geometry).unwrap();
    assert!((mass - 4e-4 * 2700.).abs() < 1e-9);
    // al-6061-t6: conservative max service 100 degC, min service -100 degC.
    let margin =
        provider_value("temperature_margin_k", &al, &[("max_temperature_k", 330.)]).unwrap();
    assert!((margin - (373.15 - 330.)).abs() < 1e-9);
    let both = provider_value(
        "temperature_margin_k",
        &al,
        &[("max_temperature_k", 330.), ("min_temperature_k", 180.)],
    )
    .unwrap();
    assert!(
        (both - (180. - 173.15)).abs() < 1e-9,
        "the tighter of both margins"
    );
    assert_eq!(provider_value("temperature_margin_k", &al, &geometry), None);
    assert_eq!(
        provider_value("stress_margin", &al, &[("max_von_mises_pa", 50e6)]),
        None,
        "no stated safety factor"
    );
    let stress = provider_value(
        "stress_margin",
        &al,
        &[("max_von_mises_pa", 50e6), ("stress_safety_factor", 2.)],
    )
    .unwrap();
    assert!((stress - (240e6 / 2. / 50e6 - 1.)).abs() < 1e-9);
    assert!(PROVIDERS.iter().all(|p| p.evidence() == "estimated"));
}

#[test]
fn enclosure_examples_approve_at_least_the_planner_output_floor() {
    use harbor_cad::estimates::{NATIVE_OUTPUT_BYTES, solid_output_bytes, variant_output_bytes};
    for name in ["design-enclosure.json", "design-enclosure-demo.json"] {
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("examples")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap();
        let mut per_set = BTreeMap::new();
        for (set, templates) in raw["templates"].as_object().unwrap() {
            let templates = templates.as_array().unwrap();
            let elements = |role: &serde_json::Value| {
                let mesh = templates.iter().find(|t| t["role"] == *role).unwrap();
                mesh["plan_request"]["request"]["mesh"]["max_elements"]
                    .as_u64()
                    .unwrap()
            };
            let mut total = 0;
            for t in templates {
                let request = &t["plan_request"]["request"];
                // Source-independent part of the planner's estimates::minimum;
                // the copied document/BREP bytes come on top of it.
                let floor = NATIVE_OUTPUT_BYTES
                    + match t["plan_request"]["operation"].as_str().unwrap() {
                        "plan_cad_variant" => variant_output_bytes(
                            request["regions"].as_array().unwrap().len() as u64 - 1,
                        ),
                        "plan_cad_solid_mesh" => solid_output_bytes(
                            request["mesh"]["max_elements"].as_u64().unwrap(),
                            false,
                        ),
                        "plan_enclosure_thermal" => {
                            solid_output_bytes(elements(&request["mesh_job"]["$job"]), true)
                        }
                        other => panic!("{name}: unexpected chained operation {other}"),
                    }
                    .unwrap();
                let approved = t["max_artifact_bytes"].as_u64().unwrap();
                assert!(
                    floor <= approved,
                    "{name} {set}/{}: planner floor {floor} exceeds approved {approved}",
                    t["role"]
                );
                total += approved;
            }
            per_set.insert(set.as_str(), total);
        }
        let (budget, fidelity) = (&raw["budget"], &raw["fidelity"]);
        let evaluations = budget["max_evaluations"].as_u64().unwrap();
        let top_k = fidelity["refine_top_k"].as_u64().unwrap_or(0);
        let refine = fidelity["refine"].as_str().map_or(0, |set| per_set[set]);
        let required =
            per_set[fidelity["screen"].as_str().unwrap()] * (evaluations - top_k) + refine * top_k;
        assert!(
            required <= budget["max_artifact_bytes"].as_u64().unwrap(),
            "{name}: budget below {required} bytes"
        );
    }
}

#[test]
fn enclosure_examples_keep_the_rim_precondition_and_screen_at_the_refine_mesh_size() {
    use harbor_cad::cad_sheet_variant::enclosure_rim_precondition;
    // adapters/enclosure_fixture.py: groove_width 2 mm, centred in the wall.
    const GROOVE_WIDTH_M: f64 = 0.002;
    for name in ["design-enclosure.json", "design-enclosure-demo.json"] {
        let raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("examples")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap();
        let wall = raw["space"]["continuous"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "wall")
            .unwrap();
        assert_eq!(wall["unit"], "mm");
        let (low, step) = (
            wall["low"].as_f64().unwrap(),
            wall["step"].as_f64().unwrap(),
        );
        // No grid wall is refused by the planner, and the bound is tight.
        enclosure_rim_precondition(Some(low * 1e-3), Some(GROOVE_WIDTH_M)).unwrap();
        assert!(
            enclosure_rim_precondition(Some((low - step) * 1e-3), Some(GROOVE_WIDTH_M)).is_err()
        );
        // Sub-mm features meshed at 6 mm gave isolated slivers below the 0.05
        // quality gate that pass at 4 mm, so screen meshes at the refine size.
        for templates in raw["templates"].as_object().unwrap().values() {
            for t in templates.as_array().unwrap() {
                if let Some(mesh) = t["plan_request"]["request"].get("mesh") {
                    assert_eq!(mesh["characteristic_length_max_m"], 0.004, "{name}");
                    assert_eq!(mesh["min_quality"], 0.05, "{name}");
                }
            }
        }
    }
}

#[test]
fn enclosure_and_calibration_examples_validate() {
    let read = |name: &str| -> DesignEnvelope {
        serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("examples")
                    .join(name),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let enclosure = read("design-enclosure.json");
    let prepared = prepare(&enclosure, Some("research")).unwrap();
    assert_eq!(prepared.combinations, 5);
    // The single categorical lists only compatible MATERIAL__PROCESS__FINISH
    // triples, so no evaluation is spent on a refused route x finish pair.
    let library = harbor_cad::manufacturing::Library::builtin().unwrap();
    let conditions = harbor_cad::conditions::Conditions::builtin().unwrap();
    assert_eq!(enclosure.space.categorical.len(), 1);
    for choice in &enclosure.space.categorical[0].choices {
        let (material, process, finish) =
            harbor_cad::design_providers::split_route(choice).unwrap();
        let finish = finish.unwrap();
        let process = library.process(process).unwrap();
        assert!(
            process.compatible_materials.iter().any(|m| m == material),
            "{choice}"
        );
        assert!(
            conditions
                .finish(finish)
                .unwrap()
                .compatible_materials
                .iter()
                .any(|m| m == material),
            "{choice}"
        );
        // ... and every finish gives the housing a sourced solar absorptance.
        let (absorptance, _) = conditions.surface_optics(material, finish).unwrap();
        assert!(
            matches!(
                absorptance,
                harbor_cad::materials::PhysicalInput::Known { .. }
            ),
            "{choice}"
        );
    }
    let roles = |set: &str| -> Vec<(String, Option<String>)> {
        enclosure.templates[set]
            .iter()
            .map(|t| (t.role.clone(), t.after.clone()))
            .collect()
    };
    let pair = |role: &str, after: Option<&str>| (role.to_string(), after.map(String::from));
    assert_eq!(
        roles("screen"),
        [
            pair("variant", None),
            pair("mesh", Some("variant")),
            pair("thermal_lo", Some("mesh")),
            pair("thermal_hi", Some("mesh"))
        ]
    );
    // The refine tier chains both sweep solves before the verification solve
    // at the superposed supported power.
    assert_eq!(
        roles("refine"),
        [
            pair("variant", None),
            pair("mesh", Some("variant")),
            pair("thermal_lo", Some("mesh")),
            pair("thermal_hi", Some("thermal_lo")),
            pair("thermal_check", Some("thermal_hi"))
        ]
    );
    let constraint = |metric: &str| {
        enclosure
            .constraints
            .iter()
            .find(|c| c.metric == metric)
            .unwrap()
    };
    assert_eq!(constraint("max_supported_pcb_power_w").limit, 1.);
    assert_eq!(
        enclosure.pareto_metric.as_deref(),
        Some("max_supported_pcb_power_w")
    );
    // Every enclosure solve is a linear power_sweep solve (the linearized
    // radiation bound is allowed); superposition is refused on the nonlinear
    // T^4 model or outside sweep mode.
    for (pointer, value) in [("radiation", json!(true)), ("power_sweep", json!(false))] {
        let mut bad = enclosure.clone();
        bad.templates.get_mut("screen").unwrap()[3]
            .plan_request
            .as_mut()
            .unwrap()["request"]["spec"][pointer] = value;
        let error = prepare(&bad, Some("research")).unwrap_err().to_string();
        assert!(error.contains("power_sweep"), "{pointer}: {error}");
    }
    // The component limit and board conductivity are cited defaults, the sweep
    // range and housing margin declared requirements; no operator input is missing.
    let params = Params::from([(
        "route".into(),
        ParamValue::Choice("pa12-mjf__mjf-pa12__pigment-black-polymer".into()),
    )]);
    let metrics = compute_metrics(
        &enclosure,
        Level::Screen,
        &params,
        &BTreeMap::new(),
        PROVIDERS,
    );
    assert_eq!(metrics["quantity"], Some(50.));
    assert_eq!(metrics["ambient_hot_k"], Some(313.15));
    assert_eq!(metrics["solar_hot_w_m2"], Some(1120.));
    assert_eq!(metrics["film_hot_w_m2_k"], Some(2.2));
    assert_eq!(metrics["internal_film_coefficient"], Some(1.2));
    assert_eq!(metrics["min_temperature_k"], Some(273.15 - 33.));
    assert!((metrics["component_max_temperature_k"].unwrap() - 358.15).abs() < 1e-9);
    assert_eq!(metrics["pcb_conductivity"], Some(0.26));
    assert_eq!(
        (metrics["pcb_power_low_w"], metrics["pcb_power_high_w"]),
        (Some(1.), Some(15.))
    );
    assert_eq!(
        metrics["housing_margin_required_k"],
        Some(constraint("temperature_margin_k").limit)
    );
    assert!(
        enclosure
            .metric_sources
            .iter()
            .all(|m| !matches!(&m.source, MetricSource::Declared { input } if !matches!(input, harbor_cad::materials::PhysicalInput::Known { synthetic: false, .. })))
    );
    assert_eq!(metrics["temperature_margin_k"], None);
    assert_eq!(metrics["max_supported_pcb_power_w"], None);
    assert_eq!(metrics["unit_cost_high_usd"], None);
    // The resolved in-plane board conductivity and component limit carry their
    // data entries' explicit product requirements into the export; the
    // through-plane low end bounds every board and carries none.
    let required = requirements(&enclosure, &metrics).unwrap();
    assert_eq!(
        required
            .iter()
            .map(|r| (r.metric.as_str(), r.source.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "pcb_in_plane_conductivity",
                "electronics:pcb-fr4-copper-in-plane.low"
            ),
            (
                "component_max_temperature_k",
                "electronics:component-limit-industrial.low"
            ),
        ]
    );
    assert!(required[0].requirement.contains("continuous copper planes"));
    assert!(required[1].requirement.contains("+85 degC"));
    let mut unresolved = metrics.clone();
    unresolved.insert("pcb_in_plane_conductivity".into(), None);
    assert_eq!(requirements(&enclosure, &unresolved).unwrap().len(), 1);
    let demo = read("design-enclosure-demo.json");
    let demo_metrics = compute_metrics(&demo, Level::Screen, &params, &BTreeMap::new(), PROVIDERS);
    assert!(requirements(&demo, &demo_metrics).unwrap().is_empty());
    // With a simulated housing peak, PA12's -40 degC minimum leaves 7 K at the
    // class 4.1 cold point, below the 10 K requirement; aluminium keeps 67 K
    // cold-side, so its margin is the hot side.
    let simulated = BTreeMap::from([("max_temperature_k".to_string(), Some(330.))]);
    let metrics = compute_metrics(&enclosure, Level::Screen, &params, &simulated, PROVIDERS);
    assert!((metrics["temperature_margin_k"].unwrap() - 7.).abs() < 1e-9);
    let al = Params::from([(
        "route".into(),
        ParamValue::Choice("al-6061-t6__cnc-3axis-al__powder-coat-white".into()),
    )]);
    let metrics = compute_metrics(&enclosure, Level::Screen, &al, &simulated, PROVIDERS);
    assert!((metrics["temperature_margin_k"].unwrap() - (373.15 - 330.)).abs() < 1e-9);
    // Two simulated sweep solves give the supported power on screen; refine
    // without its verification solve leaves it unknown.
    let solves = BTreeMap::from(
        [
            ("max_temperature_k", 330.),
            ("pcb_max_temperature_k", 340.),
            ("internal_air_temperature_k", 335.),
            ("hi_max_temperature_k", 372.),
            ("hi_pcb_max_temperature_k", 410.),
            ("hi_internal_air_temperature_k", 380.),
        ]
        .map(|(k, v)| (k.to_string(), Some(v))),
    );
    let screen = compute_metrics(&enclosure, Level::Screen, &al, &solves, PROVIDERS);
    assert!((screen["max_supported_pcb_power_w"].unwrap() - (1. + 18.15 / 5.)).abs() < 1e-9);
    let refine = compute_metrics(&enclosure, Level::Refine, &al, &solves, PROVIDERS);
    assert_eq!(refine["max_supported_pcb_power_w"], None);
    // The glazing's solar bound and the linearized radiation bound are carried
    // into the export.
    let bounds = conservative_bounds(&enclosure);
    assert_eq!(bounds.len(), 2);
    assert!(bounds[0].starts_with("radiation=linearized_lower_bound: conservative_bound"));
    assert!(
        bounds[1].starts_with("transparent_solar=deposit_incident_in_cavity: conservative_bound")
    );
    assert!(conservative_bounds(&read("design-enclosure-demo.json")).is_empty());
    let calibrate = read("design-calibrate.json");
    prepare(&calibrate, Some("research")).unwrap();
}

/// Raw durable candidate row, for asserting the recorded chain binding.
fn candidate_row(root: &Path, id: &str, index: u64) -> serde_json::Value {
    let raw: String = Store::open(root)
        .unwrap()
        .connection
        .query_row(
            "SELECT candidate FROM design_candidates WHERE design=?1 AND idx=?2",
            rusqlite::params![id, index as i64],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn submit_design(root: &Path, env: &DesignEnvelope, key: &str) -> String {
    let digest = prepare(env, Some("ci")).unwrap().approval_digest;
    let submitted = call(
        root,
        json!({"operation":"design_submit","envelope":env,"approved_digest":digest,"idempotency_key":key}),
    );
    assert!(submitted.ok, "{submitted:?}");
    submitted.data.unwrap()["id"].as_str().unwrap().to_string()
}

fn job_refs(status: &serde_json::Value) -> usize {
    status["iterations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|i| i["jobs"].as_array().unwrap())
        .filter(|j| j["job_id"].is_string())
        .count()
}

#[test]
fn worker_derives_chained_children_from_upstream_output_and_resumes_without_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let env = chain_env(4);
    let mut worker = start(&root);
    let id = submit_design(&root, &env, "chain");
    wait_until(&root, &id, |s| {
        s["iterations"][0]["jobs"][1]["job_id"].is_string()
    });
    // Crash after the first derivation and restart on the same state root.
    drop(worker);
    worker = start(&root);
    let status = wait_until(&root, &id, |s| s["state"] != "running");
    assert_eq!(status["state"], "budget_exhausted", "{status}");
    let iterations = status["iterations"].as_array().unwrap();
    assert_eq!(iterations.len(), 4);
    assert_eq!(
        job_count(&root) as usize,
        job_refs(&status),
        "no duplicate children"
    );
    let mut derived_checked = 0;
    for (index, iteration) in iterations.iter().enumerate() {
        let jobs = iteration["jobs"].as_array().unwrap();
        assert_eq!(jobs[1]["role"], "derived");
        if jobs[0]["state"] != "succeeded" {
            // A crash-interrupted upstream blocks its dependent; nothing is fabricated.
            assert!(jobs[1]["job_id"].is_null() && jobs[1]["blocked"].is_string());
            continue;
        }
        let row = candidate_row(&root, &id, index as u64);
        let child = &row["children"][1];
        assert_eq!(child["upstream_job"], jobs[0]["job_id"], "{row}");
        assert!(child["plan_digest"].is_string() && child["idempotency_key"].is_string());
        assert_ne!(
            child["idempotency_key"],
            row["children"][0]["idempotency_key"]
        );
        if jobs[1]["state"] == "succeeded" {
            // u2 = u1 h^2 / (12 nu): the derived drive is the upstream velocity.
            let h = num(
                &serde_json::from_value(iteration["params"].clone()).unwrap(),
                "height",
            ) * 1e-3;
            let u1 = iteration["metrics"]["mean_velocity"].as_f64().unwrap();
            let u2 = iteration["metrics"]["derived_velocity"].as_f64().unwrap();
            assert!(
                (u2 - u1 * h * h / 12e-5).abs() <= 1e-9 * u2.abs(),
                "{iteration}"
            );
            derived_checked += 1;
        }
    }
    assert!(derived_checked >= 3, "{status}");
    // Re-advancing a finished design submits nothing new.
    let before = job_count(&root);
    let repeat = call(
        &root,
        json!({"operation":"design_submit","envelope":env,
            "approved_digest":prepare(&env, Some("ci")).unwrap().approval_digest,"idempotency_key":"chain"}),
    );
    assert_eq!(repeat.data.unwrap()["id"], id);
    assert_eq!(job_count(&root), before);
    drop(worker);
}

#[test]
fn metric_placeholders_derive_from_upstream_metrics_and_pareto_metric_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let mut env = chain_env(3);
    // The same drive as the $job chain, but through a metric evaluated from
    // the upstream job (the velocity reused numerically as the drive).
    env.metric_sources.push(MetricSpec {
        metric: "drive".into(),
        unit: "m/s2".into(),
        source: MetricSource::Job {
            role: "flow".into(),
            artifact: "validation.json".into(),
            pointer: "/mean_velocity_m_s".into(),
        },
    });
    chained(&mut env).plan_request.as_mut().unwrap()["case"]["acceleration"] =
        json!({"$metric": "drive"});
    env.pareto_metric = Some("derived_velocity".into());
    for (bad, why) in [
        (json!({"$metric": "undeclared"}), "undeclared metric"),
        (json!({"$metric": "drive", "unit": "m/s2"}), "extra key"),
    ] {
        let mut e = env.clone();
        chained(&mut e).plan_request.as_mut().unwrap()["case"]["acceleration"] = bad;
        assert!(prepare(&e, Some("ci")).is_err(), "{why} accepted");
    }
    let mut e = env.clone();
    e.templates.get_mut("screen").unwrap()[0].plan["case"]["acceleration"] =
        json!({"$metric": "drive"});
    assert!(
        prepare(&e, Some("ci")).is_err(),
        "$metric in a complete plan"
    );
    let mut e = env.clone();
    e.pareto_metric = Some("undeclared".into());
    assert!(prepare(&e, Some("ci")).is_err(), "undeclared pareto metric");

    let worker = start(&root);
    let id = submit_design(&root, &env, "metric");
    let status = wait_until(&root, &id, |s| s["state"] != "running");
    let mut checked = 0;
    for iteration in status["iterations"].as_array().unwrap() {
        if iteration["jobs"][1]["state"] != "succeeded" {
            continue;
        }
        let h = num(
            &serde_json::from_value(iteration["params"].clone()).unwrap(),
            "height",
        ) * 1e-3;
        let u1 = iteration["metrics"]["mean_velocity"].as_f64().unwrap();
        let u2 = iteration["metrics"]["derived_velocity"].as_f64().unwrap();
        assert!((u2 - u1 * h * h / 12e-5).abs() <= 1e-9 * u2.abs());
        checked += 1;
    }
    assert!(checked >= 2, "{status}");
    // The Pareto set now trades the objective against the declared metric
    // (maximized) over feasible candidates only.
    assert_eq!(status["pareto_metric"], "derived_velocity");
    let pareto = status["pareto"].as_array().unwrap();
    assert!(!pareto.is_empty());
    for index in pareto {
        let i = &status["iterations"][index.as_u64().unwrap() as usize];
        assert_eq!(i["standing"], "feasible", "{status}");
    }
    let report = call(&root, json!({"operation":"design_export","design_id":id}));
    let report = report.data.unwrap();
    assert_eq!(report["pareto"].as_array().unwrap().len(), pareto.len());
    assert_eq!(report["pareto_metric"], "derived_velocity");
    drop(worker);
}

#[test]
fn failed_upstream_or_refused_derivation_never_submits_the_dependent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let _worker = start(&root);

    // The analytical residual (~1e-15) exceeds this tolerance: every upstream job fails.
    let mut failing = chain_env(3);
    failing.templates.get_mut("screen").unwrap()[0].plan["case"]["applicability"]["numerical_tolerance"] =
        json!(1e-300);
    let id = submit_design(&root, &failing, "failing");
    let status = wait_until(&root, &id, |s| s["state"] != "running");
    assert_ne!(status["state"], "failed", "{status}");
    for iteration in status["iterations"].as_array().unwrap() {
        let jobs = iteration["jobs"].as_array().unwrap();
        assert_eq!(jobs[0]["state"], "failed", "{iteration}");
        assert!(jobs[1]["job_id"].is_null(), "{iteration}");
        assert_eq!(jobs[1]["blocked"], "upstream flow failed");
        assert!(iteration["metrics"]["derived_velocity"].is_null());
        assert_eq!(iteration["standing"], "unknown");
    }
    assert_eq!(job_count(&root) as usize, job_refs(&status));

    // A derived plan larger than the approved allowance is refused per candidate.
    let mut undersized = chain_env(2);
    undersized.provenance.push_str(" undersized");
    chained(&mut undersized).max_artifact_bytes = Some(1024);
    let before = job_count(&root);
    let id = submit_design(&root, &undersized, "undersized");
    let status = wait_until(&root, &id, |s| s["state"] != "running");
    for iteration in status["iterations"].as_array().unwrap() {
        let derived = &iteration["jobs"][1];
        assert!(derived["job_id"].is_null(), "{iteration}");
        assert!(
            derived["blocked"]
                .as_str()
                .unwrap()
                .contains("approved max_artifact_bytes"),
            "{iteration}"
        );
    }
    assert_eq!(job_count(&root) - before, 2, "only the upstream jobs ran");
}
