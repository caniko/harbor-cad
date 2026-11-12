//! Read-only robustness arithmetic over supplied numbers, never an execution gate.
use crate::{
    Result,
    contracts::{digest, invalid, token},
    design::{self, Constraint, Metrics, Objective, ObjectiveKind, ObjectiveTerm, Standing},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

type Point = BTreeMap<String, f64>;

fn distinct_map<'de, D, T>(deserializer: D) -> std::result::Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    crate::json::object(deserializer)?
        .into_iter()
        .map(|(name, raw)| {
            serde_json::from_str(raw.get())
                .map(|value| (name, value))
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppliedCandidate {
    pub name: String,
    #[serde(deserialize_with = "distinct_map")]
    pub metrics: Metrics,
    #[serde(default, deserialize_with = "distinct_map")]
    pub gates: BTreeMap<String, Option<bool>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    /// Independent proposed box endpoints; this does not prove native feasibility.
    #[serde(deserialize_with = "distinct_map")]
    pub axes: BTreeMap<String, [f64; 2]>,
    #[serde(deserialize_with = "distinct_map")]
    pub baseline: Point,
    pub search: Vec<Point>,
    pub fidelity_reruns: u32,
    pub repeats: u32,
    pub max_evaluations: u32,
    pub max_wall_seconds: u64,
    pub max_artifact_bytes: u64,
    pub estimated_bytes_per_evaluation: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub provenance: String,
    pub terms: Vec<ObjectiveTerm>,
    pub constraints: Vec<Constraint>,
    pub gate_names: Vec<String>,
    pub candidates: Vec<SuppliedCandidate>,
    pub ledger: Ledger,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CandidateReport {
    pub standing: Standing,
    pub objective: Option<f64>,
    pub weighted_minimum: Option<f64>,
    pub weighted_margins: BTreeMap<String, Option<f64>>,
    pub bottlenecks: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LedgerReport {
    pub corners: u32,
    pub additional_baseline: u32,
    pub search: u32,
    pub fidelity_reruns: u32,
    pub repeats: u32,
    pub total: u32,
    pub max_evaluations: u32,
    pub budget_spare: u32,
    pub hard_cap_headroom: u32,
    pub max_wall_seconds: u64,
    pub max_artifact_bytes: u64,
    pub estimated_artifact_bytes: Option<u64>,
    pub estimate_within_artifact_ceiling: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Report {
    pub schema_version: u32,
    pub scope: &'static str,
    pub input_digest: String,
    pub qualified: bool,
    pub envelope_validated: bool,
    pub worker_executed: bool,
    pub physical_validation: &'static str,
    pub candidates: BTreeMap<String, CandidateReport>,
    /// Display ties only; raw engine scores and hard feasibility are unchanged.
    pub nominal_winners: Vec<String>,
    pub display_decimal_places: u32,
    pub ledger: LedgerReport,
}

fn shown(number: f64) -> String {
    format!("{:.12}", if number == 0. { 0. } else { number })
}

fn ledger(input: &Ledger) -> Result<LedgerReport> {
    if input.axes.is_empty()
        || input.axes.len() > design::MAX_CONTINUOUS
        || input.axes.iter().any(|(name, [low, high])| {
            !token(name) || !low.is_finite() || !high.is_finite() || low >= high
        })
        || input.search.len() > design::MAX_EVALUATIONS as usize
        || !(1..=design::MAX_EVALUATIONS).contains(&input.max_evaluations)
        || input.fidelity_reruns > design::MAX_REFINE
        || input.repeats > design::MAX_EVALUATIONS
        || !(1..=design::MAX_WALL_SECONDS).contains(&input.max_wall_seconds)
        || input.max_artifact_bytes > design::MAX_TOTAL_ARTIFACT_BYTES
    {
        return Err(invalid(
            "bounded nondegenerate box, resource ceilings, <=32 evaluations and <=3 fidelity reruns required",
        ));
    }
    let point = |p: &Point| -> Result<Vec<f64>> {
        if p.len() != input.axes.len()
            || input.axes.iter().any(|(name, [low, high])| {
                p.get(name)
                    .is_none_or(|v| !v.is_finite() || v < low || v > high)
            })
        {
            return Err(invalid(
                "point must name every box axis exactly once and lie inside its bounds",
            ));
        }
        Ok(input.axes.keys().map(|name| p[name]).collect())
    };
    let corners = 1u32 << input.axes.len();
    let mut points: Vec<Vec<f64>> = (0..corners)
        .map(|bits| {
            input
                .axes
                .values()
                .enumerate()
                .map(|(axis, ends)| ends[((bits >> axis) & 1) as usize])
                .collect()
        })
        .collect();
    let baseline = point(&input.baseline)?;
    let additional_baseline = u32::from(!points.contains(&baseline));
    if additional_baseline != 0 {
        points.push(baseline);
    }
    for candidate in &input.search {
        let candidate = point(candidate)?;
        if points.contains(&candidate) {
            return Err(invalid(
                "search points must be distinct from corners, baseline and each other; use repeats for deliberate reruns",
            ));
        }
        points.push(candidate);
    }
    let total = corners
        + additional_baseline
        + input.search.len() as u32
        + input.fidelity_reruns
        + input.repeats;
    if total > input.max_evaluations {
        return Err(invalid(
            "corners, additional baseline, search, fidelity reruns and repeats exceed the proposed evaluation budget",
        ));
    }
    let estimated_artifact_bytes = input
        .estimated_bytes_per_evaluation
        .map(|bytes| {
            bytes
                .checked_mul(u64::from(input.max_evaluations))
                .ok_or_else(|| invalid("artifact estimate overflow"))
        })
        .transpose()?;
    Ok(LedgerReport {
        corners,
        additional_baseline,
        search: input.search.len() as u32,
        fidelity_reruns: input.fidelity_reruns,
        repeats: input.repeats,
        total,
        max_evaluations: input.max_evaluations,
        budget_spare: input.max_evaluations - total,
        hard_cap_headroom: design::MAX_EVALUATIONS - input.max_evaluations,
        max_wall_seconds: input.max_wall_seconds,
        max_artifact_bytes: input.max_artifact_bytes,
        estimated_artifact_bytes,
        estimate_within_artifact_ceiling: estimated_artifact_bytes
            .map(|bytes| bytes <= input.max_artifact_bytes),
    })
}

pub fn screen(input: &Request) -> Result<Report> {
    if input.schema_version != 1
        || input.provenance.trim().is_empty()
        || input.provenance.len() > 4096
        || input.candidates.is_empty()
        || input.candidates.len() > design::MAX_EVALUATIONS as usize
        || input.gate_names.len() > design::MAX_METRICS
        || input.gate_names.iter().any(|name| !token(name))
        || input.gate_names.iter().collect::<BTreeSet<_>>().len() != input.gate_names.len()
        || input.constraints.iter().any(|c| {
            !token(&c.metric)
                || c.unit.trim().is_empty()
                || c.unit.len() > 32
                || input.gate_names.contains(&c.metric)
        })
    {
        return Err(invalid(
            "screen schema 1, provenance, bounded named candidates, declared numeric units and distinct boolean gates required",
        ));
    }
    let objective = Objective {
        kind: ObjectiveKind::MaximizeMinMargin,
        terms: input.terms.clone(),
    };
    let units = input
        .constraints
        .iter()
        .map(|c| (c.metric.as_str(), c.unit.as_str()))
        .collect();
    design::validate_scoring(&objective, &input.constraints, &units)?;
    let ledger = ledger(&input.ledger)?;
    let mut candidates = BTreeMap::new();
    for candidate in &input.candidates {
        if !token(&candidate.name)
            || candidates.contains_key(&candidate.name)
            || candidate
                .metrics
                .keys()
                .any(|name| !units.contains_key(name.as_str()))
            || candidate
                .metrics
                .values()
                .flatten()
                .any(|value| !value.is_finite())
            || candidate
                .gates
                .keys()
                .any(|name| !input.gate_names.contains(name))
        {
            return Err(invalid(
                "candidate names must be unique; only declared metrics/gates allowed; unknown or non-finite metrics must be null",
            ));
        }
        let assessment = design::assess_scoring(&objective, &input.constraints, &candidate.metrics);
        let gates: Vec<_> = input
            .gate_names
            .iter()
            .map(|name| candidate.gates.get(name).copied().flatten())
            .collect();
        let standing =
            if assessment.standing == Standing::Infeasible || gates.contains(&Some(false)) {
                Standing::Infeasible
            } else if gates.contains(&None) {
                Standing::Unknown
            } else {
                assessment.standing
            };
        let weighted_margins: BTreeMap<_, _> = input
            .terms
            .iter()
            .map(|term| {
                let margin = assessment
                    .constraints
                    .iter()
                    .find(|c| c.metric == term.metric)
                    .and_then(|c| c.margin)
                    .map(|margin| margin * term.weight)
                    .filter(|margin| margin.is_finite());
                (term.metric.clone(), margin)
            })
            .collect();
        let score = (standing == Standing::Feasible)
            .then_some(assessment.objective)
            .flatten();
        let minimum = score.map(|score| -score);
        let bottlenecks = weighted_margins
            .iter()
            .filter_map(|(metric, value)| match (value, minimum) {
                (Some(value), Some(minimum)) if shown(*value) == shown(minimum) => {
                    Some(metric.clone())
                }
                _ => None,
            })
            .collect();
        let mut blockers: Vec<String> = assessment
            .constraints
            .iter()
            .filter_map(|constraint| match constraint.satisfied {
                Some(false) => Some(format!("constraint {} failed", constraint.metric)),
                None => Some(format!("metric {} unknown", constraint.metric)),
                Some(true) => None,
            })
            .collect();
        for (name, status) in input.gate_names.iter().zip(&gates) {
            match status {
                Some(false) => blockers.push(format!("gate {name} false")),
                None => blockers.push(format!("gate {name} unknown")),
                Some(true) => {}
            }
        }
        if assessment.objective.is_none() && blockers.is_empty() {
            blockers.push("objective unknown (non-finite arithmetic)".into());
        }
        candidates.insert(
            candidate.name.clone(),
            CandidateReport {
                standing,
                objective: score,
                weighted_minimum: minimum,
                weighted_margins,
                bottlenecks,
                blockers,
            },
        );
    }
    let best = candidates
        .values()
        .filter_map(|c| c.objective)
        .min_by(f64::total_cmp);
    let nominal_winners = candidates
        .iter()
        .filter_map(|(name, candidate)| match (candidate.objective, best) {
            (Some(score), Some(best)) if shown(score) == shown(best) => Some(name.clone()),
            _ => None,
        })
        .collect();
    Ok(Report {
        schema_version: 1,
        scope: "supplied-metric-reference-only",
        input_digest: digest(input)?,
        qualified: false,
        envelope_validated: false,
        worker_executed: false,
        physical_validation: "unqualified",
        candidates,
        nominal_winners,
        display_decimal_places: 12,
        ledger,
    })
}

/// Render directly from the same computed report; no separately calculated prose.
pub fn markdown(report: &Report) -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "# Supplied-metric robustness reference\n\nNot an approved envelope, native execution or physical qualification. Units are those declared by the caller. Terms select distinct constraints with finite positive weights; boolean gates are not objective terms. M is the minimum weighted normalized margin, O = -M.\n\n| Candidate | Standing | M | O (lower is better) | Bottlenecks / blockers |\n|---|---|---|---|---|\n",
    );
    for (name, candidate) in &report.candidates {
        let status = match candidate.standing {
            Standing::Feasible => "feasible",
            Standing::Infeasible => "infeasible",
            Standing::Unknown => "unknown",
        };
        let _ = writeln!(
            out,
            "| {name} | {status} | {} | {} | {} |",
            candidate
                .weighted_minimum
                .map(shown)
                .unwrap_or_else(|| "—".into()),
            candidate.objective.map(shown).unwrap_or_else(|| "—".into()),
            if candidate.blockers.is_empty() {
                candidate.bottlenecks.join(", ")
            } else {
                candidate.blockers.join(", ")
            }
        );
    }
    let _ = writeln!(
        out,
        "\nNominal winner(s) at {} displayed decimal places: {}. Raw engine scores and hard-feasibility comparisons are not rounded for acceptance. Infeasible and unknown rows have no selectable objective.\n",
        report.display_decimal_places,
        if report.nominal_winners.is_empty() {
            "none".into()
        } else {
            report.nominal_winners.join(", ")
        }
    );
    let l = &report.ledger;
    let _ = writeln!(
        out,
        "Ledger: {} distinct corners + {} additional baseline + {} search + {} fidelity reruns + {} repeats = {} evaluations. Proposed cap: {}; spare within that cap: {}. Engine hard cap: {}; headroom above the proposed cap: {}.\n",
        l.corners,
        l.additional_baseline,
        l.search,
        l.fidelity_reruns,
        l.repeats,
        l.total,
        l.max_evaluations,
        l.budget_spare,
        design::MAX_EVALUATIONS,
        l.hard_cap_headroom
    );
    let _ = writeln!(
        out,
        "Proposed ceilings: {} seconds and {} artifact bytes. Estimated artifact demand across the entire proposed budget: {}. Estimate within artifact ceiling: {}. These are supplied-number estimates, not validated child plans. A body-size cap is not supplied by this report.\n",
        l.max_wall_seconds,
        l.max_artifact_bytes,
        l.estimated_artifact_bytes
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unknown (no per-evaluation estimate supplied)".into()),
        l.estimate_within_artifact_ceiling
            .map(|within| if within { "yes" } else { "no" })
            .unwrap_or("unknown")
    );
    let _ = writeln!(
        out,
        "Next gate: obtain missing source-bound inputs, validate the actual envelope and effective worker, then obtain separate execution authorization. Native geometry, product physics and assembled tests remain unqualified.\n\nInput digest: `{}`.",
        report.input_digest
    );
    out
}
