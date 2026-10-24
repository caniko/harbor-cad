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
        let metrics = compute_metrics(env, &p.params, &job(&p.params, p.fidelity), &[]);
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
    let metrics = compute_metrics(&env, &params, &known, &[&Constant]);
    assert_eq!(metrics["unit_cost"], Some(1.5));
    assert_eq!(metrics["height"], Some(6.));
    let missing = compute_metrics(&env, &params, &BTreeMap::new(), &[&Constant]);
    assert_eq!(missing["unit_cost"], None);
    assert_eq!(missing["mean_velocity"], None);
    assert_eq!(
        compute_metrics(&env, &params, &known, &[])["unit_cost"],
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
    assert_eq!(status["state"], "completed", "{status}");
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
    let metrics = compute_metrics(&env, &params, &BTreeMap::new(), &[]);
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
    assert_eq!(prepared.combinations, 6);
    let roles: Vec<_> = enclosure.templates["screen"]
        .iter()
        .map(|t| (t.role.as_str(), t.after.as_deref()))
        .collect();
    assert_eq!(
        roles,
        [
            ("variant", None),
            ("mesh", Some("variant")),
            ("thermal", Some("mesh"))
        ]
    );
    // Product-specific inputs are declared but missing: the derived FEM refuses
    // rather than run on invented values, so the thermal constraints stay unknown.
    // The environment comes from the cited ETSI EN 300 019-1-4 class 4.1 points.
    let params = Params::from([
        (
            "route".into(),
            ParamValue::Choice("pa12-mjf__mjf-pa12".into()),
        ),
        (
            "finish".into(),
            ParamValue::Choice("pigment-black-polymer".into()),
        ),
    ]);
    let metrics = compute_metrics(&enclosure, &params, &BTreeMap::new(), PROVIDERS);
    assert_eq!(metrics["quantity"], Some(50.));
    assert_eq!(metrics["ambient_hot_k"], Some(313.15));
    assert_eq!(metrics["solar_hot_w_m2"], Some(1120.));
    assert_eq!(metrics["film_hot_w_m2_k"], Some(2.2));
    assert_eq!(metrics["min_temperature_k"], Some(273.15 - 33.));
    assert_eq!(metrics["pcb_power_w"], None);
    assert_eq!(metrics["temperature_margin_k"], None);
    assert_eq!(metrics["component_temperature_margin_k"], None);
    assert_eq!(metrics["unit_cost_high_usd"], None);
    // With a simulated housing peak, PA12's -40 degC minimum leaves 7 K at the
    // class 4.1 cold point, below the 10 K requirement.
    let simulated = BTreeMap::from([("max_temperature_k".to_string(), Some(330.))]);
    let metrics = compute_metrics(&enclosure, &params, &simulated, PROVIDERS);
    assert!((metrics["temperature_margin_k"].unwrap() - 7.).abs() < 1e-9);
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
