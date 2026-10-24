//! Bounded approval-bound design loop over ordinary immutable worker jobs.
//!
//! One envelope approval covers a strict template substitution space. Every
//! candidate becomes ordinary child jobs submitted through the worker's normal
//! submission path; this module never schedules or executes anything itself.
use crate::{
    Error, Result,
    contracts::{
        CaseSpec, ExecutionPlan, HostExecutionProfile, MAX_MESSAGE, Operation, digest, invalid,
        token,
    },
    materials::PhysicalInput,
    science::Quantity,
    storage::{Job, Store, now},
};
use rusqlite::{OptionalExtension, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

pub const MAX_CONTINUOUS: usize = 8;
pub const MAX_CATEGORICAL: usize = 2;
pub const MAX_CHOICES: usize = 6;
pub const MAX_EVALUATIONS: u32 = 32;
pub const MAX_REFINE: u32 = 3;
const MAX_CHILDREN: usize = 4;
const MAX_METRICS: usize = 8;
const MAX_METRIC_SOURCES: usize = 24;
/// Planning operations a chained template may request. Each is resolved by the
/// worker's own dispatch, exactly as the CLI/MCP planning call would be.
pub const CHAIN_OPERATIONS: [&str; 7] = [
    "plan",
    "plan_thermal_reference",
    "plan_cad_variant",
    "plan_cad_mesh",
    "plan_fem_imported",
    "plan_cad_solid_mesh",
    "plan_enclosure_thermal",
];
fn chain_allowed(op: &Operation) -> bool {
    matches!(
        op,
        Operation::Plan { .. }
            | Operation::PlanThermalReference { .. }
            | Operation::PlanCadVariant { .. }
            | Operation::PlanCadMesh { .. }
            | Operation::PlanFemImported { .. }
            | Operation::PlanCadSolidMesh { .. }
            | Operation::PlanEnclosureThermal { .. }
    )
}
const MAX_WALL_SECONDS: u64 = 7 * 24 * 3600;
const MAX_TOTAL_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContinuousParam {
    pub name: String,
    pub low: f64,
    pub high: f64,
    /// Manufacturing grid and pattern-search step floor, in `unit`.
    pub step: f64,
    pub unit: String,
}
impl ContinuousParam {
    fn steps(&self) -> i64 {
        ((self.high - self.low) / self.step).round() as i64
    }
    fn value(&self, grid: i64) -> f64 {
        let value = self.low + grid as f64 * self.step;
        // Drop binary noise (0.33999999999999997 -> 0.34); grid() checks membership.
        format!("{value:.12e}").parse().unwrap_or(value)
    }
    fn grid(&self, value: f64) -> Option<i64> {
        let grid = ((value - self.low) / self.step).round();
        (value.is_finite()
            && (0.0..=self.steps() as f64).contains(&grid)
            && (self.value(grid as i64) - value).abs() <= 1e-6 * self.step)
            .then_some(grid as i64)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CategoricalParam {
    pub name: String,
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DesignSpace {
    pub continuous: Vec<ContinuousParam>,
    #[serde(default)]
    pub categorical: Vec<CategoricalParam>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveKind {
    Minimize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveTerm {
    pub metric: String,
    pub weight: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    pub kind: ObjectiveKind,
    pub terms: Vec<ObjectiveTerm>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintOp {
    Le,
    Ge,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub metric: String,
    pub op: ConstraintOp,
    pub limit: f64,
    pub unit: String,
    /// Optional factor >= 1 that tightens the limit.
    #[serde(default)]
    pub safety_factor: Option<f64>,
}
impl Constraint {
    pub fn effective_limit(&self) -> f64 {
        let factor = self.safety_factor.unwrap_or(1.);
        match (self.op, self.limit >= 0.) {
            (ConstraintOp::Le, true) | (ConstraintOp::Ge, false) => self.limit / factor,
            (ConstraintOp::Le, false) | (ConstraintOp::Ge, true) => self.limit * factor,
        }
    }
    /// Normalized signed margin; negative means violated.
    pub fn margin(&self, value: f64) -> f64 {
        let limit = self.effective_limit();
        let scale = if self.limit == 0. {
            1.
        } else {
            self.limit.abs()
        };
        match self.op {
            ConstraintOp::Le => (limit - value) / scale,
            ConstraintOp::Ge => (value - limit) / scale,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_evaluations: u32,
    pub max_wall_seconds: u64,
    pub max_artifact_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fidelity {
    /// Template set evaluated for every candidate.
    pub screen: String,
    #[serde(default)]
    pub refine_top_k: Option<u32>,
    /// Template set re-evaluating the best screened candidates.
    #[serde(default)]
    pub refine: Option<String>,
}

/// Either a complete existing `plan` or a chained `plan_request`. Plans may
/// contain `$param`/`$input` placeholders; plan requests additionally `$job`
/// placeholders naming an upstream role, resolved once `after` succeeded.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChildTemplate {
    pub role: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub plan: Value,
    /// An allowlisted planning operation (`CHAIN_OPERATIONS`) in worker request form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_request: Option<Value>,
    /// Earlier role in the same set whose successful job this request waits for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Approved output allowance bound for the derived plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_artifact_bytes: Option<u64>,
}

impl ChildTemplate {
    pub fn complete(role: &str, plan: Value) -> Self {
        Self {
            role: role.into(),
            plan,
            plan_request: None,
            after: None,
            max_artifact_bytes: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetricSource {
    /// A scalar in one registered checksummed JSON artifact of a child job.
    Job {
        role: String,
        artifact: String,
        pointer: String,
    },
    /// The candidate's own continuous parameter value.
    Parameter { param: String },
    /// A registered local evaluator reading only the declared parameters.
    Provider { name: String, params: Vec<String> },
    /// An explicitly declared input; `missing` stays unknown, never a default.
    Declared { input: PhysicalInput<f64> },
    /// Pooled RMSE (`correlation::objective`) of `results correlate` against
    /// the role's succeeded job; `request` omits `job_id`.
    Correlation { role: String, request: Value },
    /// A cited normative value (`conditions::Conditions::lookup`):
    /// `condition:<class>.<hot|cold>.<field>` or `finish:<id>.<field>`, in
    /// its SI unit. Usable through `$input` like a declared input.
    Sourced { key: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetricSpec {
    pub metric: String,
    pub unit: String,
    pub source: MetricSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DesignEnvelope {
    pub schema_version: u32,
    pub name: String,
    pub provenance: String,
    pub seed: u32,
    pub space: DesignSpace,
    pub objective: Objective,
    pub constraints: Vec<Constraint>,
    pub budget: Budget,
    pub fidelity: Fidelity,
    pub templates: BTreeMap<String, Vec<ChildTemplate>>,
    pub metric_sources: Vec<MetricSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ParamValue {
    Number(f64),
    Choice(String),
}

pub type Params = BTreeMap<String, ParamValue>;

/// A candidate supplied by a human or agent; evaluated exactly like proposals.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalCandidate {
    pub params: Params,
    pub provenance: String,
}

/// Local metric hook. Providers are pure functions of declared parameters and
/// already-extracted job/parameter metrics. `None` or `Err` means unknown.
pub trait MetricProvider: Sync {
    fn name(&self) -> &'static str;
    fn unit(&self) -> &'static str;
    /// `calibrated` only when backed by calibrated data; otherwise `estimated`.
    fn evidence(&self) -> &'static str;
    fn evaluate(&self, params: &Params, metrics: &BTreeMap<String, f64>) -> Result<Option<f64>>;
}

/// Registered local providers (estimate-only W1 manufacturing screens).
pub static PROVIDERS: &[&dyn MetricProvider] = crate::design_providers::ALL;

fn provider(name: &str) -> Option<&'static dyn MetricProvider> {
    PROVIDERS.iter().copied().find(|p| p.name() == name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Screen,
    Refine,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreparedDesign {
    pub schema_version: u32,
    pub approval_digest: String,
    pub policy: String,
    pub combinations: u32,
    pub screen_per_combination: u32,
    pub candidate_artifact_bytes: BTreeMap<String, u64>,
    pub required_artifact_bytes: u64,
    pub executed: bool,
    pub physical_validation: String,
}

fn bounded_text(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.len() <= limit
}

fn relative_path(path: &str) -> bool {
    bounded_text(path, 256)
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

impl DesignEnvelope {
    fn refine_k(&self) -> u32 {
        self.fidelity.refine_top_k.unwrap_or(0)
    }
    fn combinations(&self) -> Vec<Vec<usize>> {
        let mut combos = vec![vec![]];
        for variable in &self.space.categorical {
            combos = combos
                .into_iter()
                .flat_map(|prefix| {
                    (0..variable.choices.len()).map(move |choice| {
                        let mut next = prefix.clone();
                        next.push(choice);
                        next
                    })
                })
                .collect();
        }
        combos
    }
    fn screen_per_combination(&self) -> u32 {
        let available = self.budget.max_evaluations - self.refine_k();
        (2 * self.space.continuous.len() as u32 + 1)
            .min(available / self.combinations().len().max(1) as u32)
    }
    fn set(&self, level: Level) -> Result<&str> {
        match level {
            Level::Screen => Ok(&self.fidelity.screen),
            Level::Refine => self
                .fidelity
                .refine
                .as_deref()
                .ok_or_else(|| invalid("design has no refine fidelity")),
        }
    }
    fn metric(&self, name: &str) -> Option<&MetricSpec> {
        self.metric_sources.iter().find(|m| m.metric == name)
    }
}

/// How `$job` placeholders resolve during substitution.
enum Bind<'a> {
    /// Complete plan templates: no `$job` placeholders.
    Static,
    /// Validation of a chained request: names must be upstream roles; dummies.
    Check(&'a BTreeSet<String>),
    /// Derivation: succeeded upstream role -> job id.
    Jobs(&'a Store, &'a BTreeMap<String, String>),
}

fn placeholder_keys(map: &serde_json::Map<String, Value>, head: &str, extra: &[&str]) -> bool {
    map.keys().all(|k| k == head || extra.contains(&k.as_str()))
}

fn optional_str<'v>(map: &'v serde_json::Map<String, Value>, key: &str) -> Result<Option<&'v str>> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(invalid(format!("placeholder {key} must be a string"))),
    }
}

/// A `Sourced` metric as a known, cited, non-synthetic input (SI unit).
fn sourced(key: &str) -> Result<PhysicalInput<f64>> {
    let value = crate::conditions::Conditions::builtin()?.lookup(key)?;
    Ok(PhysicalInput::Known {
        value: value.value,
        provenance: value.provenance,
        synthetic: false,
    })
}

/// A value in `unit`: a bare SI number for `si`, a Quantity otherwise.
fn emit(value: f64, unit: &str, si: Option<&str>) -> Result<Value> {
    Ok(match si {
        Some(dimension) => json!(
            Quantity {
                value,
                unit: unit.into()
            }
            .si(dimension)?
        ),
        None => json!({"value": value, "unit": unit}),
    })
}

/// Strict placeholder substitution. `{"$param": NAME[, "unit"|"si"]}` names a
/// design parameter, `{"$input": METRIC[, "si"]}` a declared input and
/// `{"$job": ROLE[, "artifact", "pointer"[, "unit"]]}` an upstream job id or a
/// finite number from its registered JSON artifact. `si: DIMENSION` emits a
/// bare SI number for plan fields that carry no unit.
fn substitute(
    value: &Value,
    env: &DesignEnvelope,
    params: &Params,
    used: &mut BTreeSet<String>,
    bind: &Bind,
) -> Result<Value> {
    Ok(match value {
        Value::Object(map) if map.keys().any(|k| k.starts_with('$')) => {
            if let Some(role) = map.get("$job") {
                return job_placeholder(map, role, bind);
            }
            if let Some(name) = map.get("$input") {
                let name = name
                    .as_str()
                    .filter(|_| placeholder_keys(map, "$input", &["si"]))
                    .ok_or_else(|| {
                        invalid("only {\"$input\": METRIC[, \"si\": DIMENSION]} placeholders are allowed")
                    })?;
                let si = optional_str(map, "si")?;
                let (unit, input) = match env.metric(name) {
                    Some(MetricSpec {
                        unit,
                        source: MetricSource::Declared { input },
                        ..
                    }) => (unit, input.clone()),
                    Some(MetricSpec {
                        unit,
                        source: MetricSource::Sourced { key },
                        ..
                    }) => (unit, sourced(key)?),
                    _ => {
                        return Err(invalid(format!(
                            "$input {name} must name a declared or sourced metric source"
                        )));
                    }
                };
                return match (&input, bind) {
                    (PhysicalInput::Known { value, .. }, _) => emit(*value, unit, si),
                    (PhysicalInput::Missing { .. }, Bind::Check(_)) => emit(1., unit, si),
                    (PhysicalInput::Missing { reason }, _) => Err(invalid(format!(
                        "declared input {name} is missing: {reason}"
                    ))),
                };
            }
            let name = map
                .get("$param")
                .and_then(Value::as_str)
                .filter(|_| placeholder_keys(map, "$param", &["unit", "si"]))
                .ok_or_else(|| {
                    invalid(
                        "only {\"$param\": NAME[, \"unit\": UNIT | \"si\": DIMENSION]} placeholders are allowed",
                    )
                })?;
            let unit = optional_str(map, "unit")?;
            let si = optional_str(map, "si")?;
            if unit.is_some() && si.is_some() {
                return Err(invalid("placeholder takes unit or si, not both"));
            }
            used.insert(name.into());
            if let Some(param) = env.space.continuous.iter().find(|p| p.name == name) {
                if unit.is_some_and(|unit| unit != param.unit) {
                    return Err(invalid(format!(
                        "placeholder unit differs from parameter {name} unit {}",
                        param.unit
                    )));
                }
                match params.get(name) {
                    Some(ParamValue::Number(v)) => emit(*v, &param.unit, si)?,
                    _ => return Err(invalid(format!("numeric value for {name} required"))),
                }
            } else if env.space.categorical.iter().any(|p| p.name == name) {
                if unit.is_some() || si.is_some() {
                    return Err(invalid("categorical placeholders carry no unit"));
                }
                match params.get(name) {
                    Some(ParamValue::Choice(choice)) => Value::String(choice.clone()),
                    _ => return Err(invalid(format!("choice for {name} required"))),
                }
            } else {
                return Err(invalid(format!("unknown design placeholder {name}")));
            }
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), substitute(v, env, params, used, bind)?)))
                .collect::<Result<_>>()?,
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| substitute(v, env, params, used, bind))
                .collect::<Result<_>>()?,
        ),
        other => other.clone(),
    })
}

fn job_placeholder(
    map: &serde_json::Map<String, Value>,
    role: &Value,
    bind: &Bind,
) -> Result<Value> {
    let role = role
        .as_str()
        .filter(|_| placeholder_keys(map, "$job", &["artifact", "pointer", "unit"]))
        .ok_or_else(|| {
            invalid(
                "only {\"$job\": ROLE[, \"artifact\", \"pointer\"[, \"unit\"]]} placeholders are allowed",
            )
        })?;
    let artifact = optional_str(map, "artifact")?;
    let pointer = optional_str(map, "pointer")?;
    let unit = optional_str(map, "unit")?;
    if artifact.is_some() != pointer.is_some() || (unit.is_some() && artifact.is_none()) {
        return Err(invalid(
            "$job artifact and pointer go together; unit needs both",
        ));
    }
    let number = match bind {
        Bind::Static => {
            return Err(invalid(
                "$job placeholders are only allowed in chained plan_request templates",
            ));
        }
        Bind::Check(upstream) => {
            if !upstream.contains(role)
                || artifact.is_some_and(|a| !relative_path(a))
                || pointer.is_some_and(|p| !p.starts_with('/') || p.len() > 256)
            {
                return Err(invalid(format!(
                    "$job {role} must name an upstream (after-chain) role with a relative artifact and JSON pointer"
                )));
            }
            if artifact.is_none() {
                return Ok(json!(uuid::Uuid::nil().to_string()));
            }
            json!(1)
        }
        Bind::Jobs(store, jobs) => {
            let id = jobs
                .get(role)
                .ok_or_else(|| invalid(format!("upstream {role} has no succeeded job")))?;
            let (Some(artifact), Some(pointer)) = (artifact, pointer) else {
                return Ok(json!(id));
            };
            crate::qualification::registered_json(store, id, artifact)?
                .and_then(|(_, v)| v.pointer(pointer).cloned())
                .filter(|v| v.as_f64().is_some_and(f64::is_finite))
                .ok_or_else(|| {
                    invalid(format!(
                        "upstream {role} {artifact}{pointer} is not a registered finite number"
                    ))
                })?
        }
    };
    Ok(match unit {
        Some(unit) => json!({"value": number, "unit": unit}),
        None => number,
    })
}

/// Roles a template transitively waits for (its `after` chain).
fn upstream(templates: &[ChildTemplate], template: &ChildTemplate) -> BTreeSet<String> {
    let mut roles = BTreeSet::new();
    let mut next = template.after.as_deref();
    while let Some(role) = next {
        if !roles.insert(role.to_string()) {
            break;
        }
        next = templates
            .iter()
            .find(|t| t.role == role)
            .and_then(|t| t.after.as_deref());
    }
    roles
}

/// Resolve a chained planning request into an allowlisted worker operation.
fn request_operation(
    env: &DesignEnvelope,
    template: &ChildTemplate,
    params: &Params,
    used: &mut BTreeSet<String>,
    bind: &Bind,
) -> Result<Operation> {
    let request = template
        .plan_request
        .as_ref()
        .ok_or_else(|| invalid("chained template plan_request required"))?;
    let value = substitute(request, env, params, used, bind)?;
    let op: Operation = serde_json::from_value(value).map_err(|e| {
        invalid(format!(
            "template {} plan_request schema: {e}",
            template.role
        ))
    })?;
    if !chain_allowed(&op) {
        return Err(invalid(format!(
            "template {} plan_request operation must be one of {CHAIN_OPERATIONS:?}",
            template.role
        )));
    }
    Ok(op)
}

/// A derived plan passes unchanged plan validation under the effective policy
/// and stays inside the template's approved output allowance.
fn admit_derived(template: &ChildTemplate, plan: &ExecutionPlan, policy: &str) -> Result<()> {
    plan.validate()?;
    if plan.policy != policy {
        return Err(invalid("derived plan policy differs from effective policy"));
    }
    if Some(plan.observation.max_artifact_bytes) > template.max_artifact_bytes {
        return Err(invalid(
            "derived plan output allowance exceeds the template's approved max_artifact_bytes",
        ));
    }
    Ok(())
}

/// The worker's own planning dispatch (the same path the CLI/MCP call).
pub type Planner<'a> = dyn FnMut(Operation) -> Result<ExecutionPlan> + 'a;

fn derive(
    store: &Store,
    env: &DesignEnvelope,
    template: &ChildTemplate,
    params: &Params,
    jobs: &BTreeMap<String, String>,
    policy: &str,
    planner: &mut Planner,
) -> Result<ExecutionPlan> {
    let op = request_operation(
        env,
        template,
        params,
        &mut BTreeSet::new(),
        &Bind::Jobs(store, jobs),
    )?;
    let plan = planner(op)?;
    admit_derived(template, &plan, policy)?;
    Ok(plan)
}

/// Resolve one template set: validated immutable plans for complete
/// templates and `None` for chained ones (checked with dummy upstreams).
pub fn child_plans(
    env: &DesignEnvelope,
    level: Level,
    params: &Params,
    policy: &str,
) -> Result<Vec<(String, Option<ExecutionPlan>)>> {
    let templates = env
        .templates
        .get(env.set(level)?)
        .ok_or_else(|| invalid("template set missing"))?;
    let mut used = BTreeSet::new();
    let plans = templates
        .iter()
        .map(|template| {
            if template.plan_request.is_some() {
                let roles = upstream(templates, template);
                request_operation(env, template, params, &mut used, &Bind::Check(&roles))?;
                return Ok((template.role.clone(), None));
            }
            let value = substitute(&template.plan, env, params, &mut used, &Bind::Static)?;
            let plan: ExecutionPlan = serde_json::from_value(value)
                .map_err(|e| invalid(format!("template {} plan schema: {e}", template.role)))?;
            plan.validate()?;
            if plan.policy != policy {
                return Err(invalid(
                    "template plan policy differs from effective policy",
                ));
            }
            Ok((template.role.clone(), Some(plan)))
        })
        .collect::<Result<Vec<_>>>()?;
    for spec in &env.metric_sources {
        match &spec.source {
            MetricSource::Parameter { param } => {
                used.insert(param.clone());
            }
            MetricSource::Provider { params, .. } => used.extend(params.iter().cloned()),
            MetricSource::Job { .. }
            | MetricSource::Declared { .. }
            | MetricSource::Correlation { .. }
            | MetricSource::Sourced { .. } => {}
        }
    }
    let declared: BTreeSet<String> = env
        .space
        .continuous
        .iter()
        .map(|p| p.name.clone())
        .chain(env.space.categorical.iter().map(|p| p.name.clone()))
        .collect();
    if let Some(unused) = declared.difference(&used).next() {
        return Err(invalid(format!(
            "design parameter {unused} is used by no template or metric source"
        )));
    }
    Ok(plans)
}

fn corner(env: &DesignEnvelope, high: bool) -> Params {
    env.space
        .continuous
        .iter()
        .map(|p| {
            let grid = if high { p.steps() } else { 0 };
            (p.name.clone(), ParamValue::Number(p.value(grid)))
        })
        .chain(env.space.categorical.iter().map(|p| {
            let choice = if high {
                p.choices.last()
            } else {
                p.choices.first()
            };
            (
                p.name.clone(),
                ParamValue::Choice(choice.cloned().unwrap_or_default()),
            )
        }))
        .collect()
}

/// Lowest-corner complete plans of every used template set, for pre-intent
/// admission checks. Chained plans are admitted when derived.
pub fn probe_plans(env: &DesignEnvelope, policy: &str) -> Result<Vec<ExecutionPlan>> {
    let mut plans = vec![];
    for level in [Level::Screen, Level::Refine] {
        if env.set(level).is_ok() {
            plans.extend(
                child_plans(env, level, &corner(env, false), policy)?
                    .into_iter()
                    .filter_map(|(_, plan)| plan),
            );
        }
    }
    Ok(plans)
}

/// Validate the complete envelope. Without an explicit policy the templates'
/// uniform policy is reported; the worker always binds its effective policy.
pub fn prepare(env: &DesignEnvelope, policy: Option<&str>) -> Result<PreparedDesign> {
    if env.schema_version != 1
        || !token(&env.name)
        || !bounded_text(&env.provenance, 4096)
        || serde_json::to_vec(env)?.len() as u64 > MAX_MESSAGE - 4096
    {
        return Err(invalid(
            "design schema 1, token name, provenance and bounded serialized envelope required",
        ));
    }
    let space = &env.space;
    if space.continuous.len() > MAX_CONTINUOUS
        || space.categorical.len() > MAX_CATEGORICAL
        || space.continuous.len() + space.categorical.len() == 0
    {
        return Err(invalid(
            "1..8 continuous and at most 2 categorical design parameters required",
        ));
    }
    let mut names = BTreeSet::new();
    for p in &space.continuous {
        let steps = (p.high - p.low) / p.step;
        if !token(&p.name)
            || p.name.len() > 32
            || !names.insert(p.name.as_str())
            || !bounded_text(&p.unit, 32)
            || !(p.low.is_finite() && p.high.is_finite() && p.step.is_finite())
            || p.low >= p.high
            || p.step <= 0.
            || !(1. ..=1e6).contains(&steps.round())
            || (steps - steps.round()).abs() > 1e-6
        {
            return Err(invalid(format!(
                "continuous parameter {} needs a unique name, unit and finite low < high on an integral step grid",
                p.name
            )));
        }
    }
    for p in &space.categorical {
        let unique: BTreeSet<_> = p.choices.iter().collect();
        if !token(&p.name)
            || p.name.len() > 32
            || !names.insert(p.name.as_str())
            || p.choices.is_empty()
            || p.choices.len() > MAX_CHOICES
            || unique.len() != p.choices.len()
            || p.choices.iter().any(|c| !token(c) || c.len() > 64)
        {
            return Err(invalid(format!(
                "categorical parameter {} needs a unique name and 1..6 unique token choices",
                p.name
            )));
        }
    }
    let k = env.refine_k();
    let budget = &env.budget;
    let combos = env.combinations().len() as u32;
    if !(1..=MAX_EVALUATIONS).contains(&budget.max_evaluations)
        || !(1..=MAX_WALL_SECONDS).contains(&budget.max_wall_seconds)
        || budget.max_artifact_bytes > MAX_TOTAL_ARTIFACT_BYTES
        || k > MAX_REFINE
        || k >= budget.max_evaluations
        || combos > budget.max_evaluations - k
    {
        return Err(invalid(
            "1..32 evaluations, bounded wall time, refine_top_k <= 3 and every categorical combination screenable within the evaluation budget required",
        ));
    }
    let mut used_sets = BTreeSet::from([env.fidelity.screen.as_str()]);
    match (&env.fidelity.refine, env.fidelity.refine_top_k) {
        (None, None) => {}
        (Some(set), Some(1..)) => {
            used_sets.insert(set);
        }
        _ => {
            return Err(invalid(
                "refine template set and refine_top_k 1..3 must be declared together",
            ));
        }
    }
    if env
        .templates
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != used_sets
    {
        return Err(invalid(
            "template sets must be exactly the declared screen/refine fidelities",
        ));
    }
    for (set, templates) in &env.templates {
        let roles: BTreeSet<_> = templates.iter().map(|t| t.role.as_str()).collect();
        if !token(set)
            || templates.is_empty()
            || templates.len() > MAX_CHILDREN
            || roles.len() != templates.len()
            || roles.iter().any(|r| !token(r) || r.len() > 32)
        {
            return Err(invalid(
                "each template set needs 1..4 children with unique token roles",
            ));
        }
        for (position, t) in templates.iter().enumerate() {
            let chained = match (&t.plan, &t.plan_request) {
                (Value::Object(_), None) => t.after.is_none() && t.max_artifact_bytes.is_none(),
                (Value::Null, Some(request)) => {
                    request
                        .get("operation")
                        .and_then(Value::as_str)
                        .is_some_and(|op| CHAIN_OPERATIONS.contains(&op))
                        && t.max_artifact_bytes
                            .is_some_and(|b| (1..=MAX_TOTAL_ARTIFACT_BYTES).contains(&b))
                        && t.after
                            .as_ref()
                            .is_none_or(|a| templates[..position].iter().any(|u| &u.role == a))
                }
                _ => false,
            };
            if !chained {
                return Err(invalid(format!(
                    "template {} needs either a complete plan, or a plan_request with an allowlisted operation {CHAIN_OPERATIONS:?}, max_artifact_bytes and an optional after naming an earlier role",
                    t.role
                )));
            }
        }
    }
    let mut metrics = BTreeSet::new();
    if env.metric_sources.is_empty() || env.metric_sources.len() > MAX_METRIC_SOURCES {
        return Err(invalid("1..24 metric sources required"));
    }
    let every_set_has = |role: &str| {
        used_sets
            .iter()
            .all(|set| env.templates[*set].iter().any(|t| t.role == role))
    };
    for spec in &env.metric_sources {
        if !token(&spec.metric)
            || spec.metric.len() > 32
            || !metrics.insert(spec.metric.as_str())
            || !bounded_text(&spec.unit, 32)
        {
            return Err(invalid(
                "unique token metric names with explicit units required",
            ));
        }
        let valid = match &spec.source {
            MetricSource::Job {
                role,
                artifact,
                pointer,
            } => {
                every_set_has(role)
                    && relative_path(artifact)
                    && pointer.starts_with('/')
                    && pointer.len() <= 256
            }
            MetricSource::Parameter { param } => space
                .continuous
                .iter()
                .any(|p| &p.name == param && p.unit == spec.unit),
            MetricSource::Provider { name, params } => {
                let declared: BTreeSet<_> = params.iter().collect();
                provider(name).is_some_and(|p| p.unit() == spec.unit)
                    && declared.len() == params.len()
                    && params.iter().all(|n| names.contains(n.as_str()))
            }
            MetricSource::Declared { input } => {
                input.validate().is_ok()
                    && match input {
                        PhysicalInput::Known { value, .. } => value.is_finite(),
                        PhysicalInput::Missing { .. } => true,
                    }
            }
            MetricSource::Correlation { role, request } => {
                every_set_has(role)
                    && spec.unit == "K"
                    && correlation_request(request, &uuid::Uuid::nil().to_string()).is_ok()
            }
            MetricSource::Sourced { key } => crate::conditions::Conditions::builtin()
                .and_then(|c| c.lookup(key))
                .is_ok_and(|s| s.unit == spec.unit),
        };
        if !valid {
            return Err(invalid(format!(
                "metric {} source must name a template role in every fidelity with a relative artifact and JSON pointer, a same-unit continuous parameter, a registered same-unit provider over declared parameters, a finite declared input with provenance or missing reason, a role with a valid job-less correlation request in K, or a resolvable sourced condition/finish key in its SI unit",
                spec.metric
            )));
        }
    }
    if env.objective.terms.is_empty()
        || env.objective.terms.len() > MAX_METRICS
        || env.objective.terms.iter().any(|t| {
            !metrics.contains(t.metric.as_str()) || !t.weight.is_finite() || t.weight == 0.
        })
    {
        return Err(invalid(
            "1..8 objective terms over declared metrics with finite nonzero weights required",
        ));
    }
    if env.constraints.len() > MAX_METRICS
        || env.constraints.iter().any(|c| {
            env.metric(&c.metric).is_none_or(|m| m.unit != c.unit)
                || !c.limit.is_finite()
                || c.safety_factor.is_some_and(|f| !f.is_finite() || f < 1.)
        })
    {
        return Err(invalid(
            "at most 8 constraints over declared same-unit metrics with finite limits and safety factors >= 1",
        ));
    }
    // Derived plans always bind the worker's effective policy; without a
    // complete template and an explicit policy there is nothing to compare.
    let policy = match policy {
        Some(policy) => policy.to_string(),
        None => match env.templates[&env.fidelity.screen]
            .iter()
            .find(|t| t.plan_request.is_none())
        {
            Some(t) => t
                .plan
                .get("policy")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("template plan policy required"))?
                .to_string(),
            None => "worker".into(),
        },
    };
    let mut bytes = BTreeMap::new();
    for level in [Level::Screen, Level::Refine] {
        let Ok(set) = env.set(level) else { continue };
        let mut total = 0u64;
        for high in [false, true] {
            let plans = child_plans(env, level, &corner(env, high), &policy)?;
            total = total.max(
                plans
                    .iter()
                    .zip(&env.templates[set])
                    .map(|((_, p), t)| {
                        p.as_ref().map_or(t.max_artifact_bytes.unwrap_or(0), |p| {
                            p.observation.max_artifact_bytes
                        })
                    })
                    .try_fold(0u64, u64::checked_add)
                    .ok_or_else(|| invalid("design output allowance overflow"))?,
            );
        }
        bytes.insert(set.to_string(), total);
    }
    let screen = bytes[&env.fidelity.screen];
    let refine = env.fidelity.refine.as_ref().map_or(0, |set| bytes[set]);
    let required = screen
        .checked_mul(u64::from(budget.max_evaluations - k))
        .and_then(|s| s.checked_add(refine.checked_mul(u64::from(k))?))
        .ok_or_else(|| invalid("design output allowance overflow"))?;
    if required > budget.max_artifact_bytes {
        return Err(invalid(
            "budget max_artifact_bytes must cover every evaluation's unchanged child output allowance",
        ));
    }
    Ok(PreparedDesign {
        schema_version: 1,
        approval_digest: digest(env)?,
        policy,
        combinations: combos,
        screen_per_combination: env.screen_per_combination(),
        candidate_artifact_bytes: bytes,
        required_artifact_bytes: required,
        executed: false,
        physical_validation: "unqualified".into(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Point {
    combo: Vec<usize>,
    grid: Vec<i64>,
}

fn point(env: &DesignEnvelope, params: &Params) -> Result<Point> {
    if params.len() != env.space.continuous.len() + env.space.categorical.len() {
        return Err(invalid(
            "candidate must assign exactly every declared parameter",
        ));
    }
    let grid = env
        .space
        .continuous
        .iter()
        .map(|p| match params.get(&p.name) {
            Some(ParamValue::Number(v)) => p.grid(*v).ok_or_else(|| {
                invalid(format!(
                    "{} must lie on its step grid within [{}, {}] {}",
                    p.name, p.low, p.high, p.unit
                ))
            }),
            _ => Err(invalid(format!("numeric {} required", p.name))),
        })
        .collect::<Result<_>>()?;
    let combo = env
        .space
        .categorical
        .iter()
        .map(|p| match params.get(&p.name) {
            Some(ParamValue::Choice(c)) => p
                .choices
                .iter()
                .position(|x| x == c)
                .ok_or_else(|| invalid(format!("{} is not a declared {} choice", c, p.name))),
            _ => Err(invalid(format!("choice for {} required", p.name))),
        })
        .collect::<Result<_>>()?;
    Ok(Point { combo, grid })
}

fn params_of(env: &DesignEnvelope, point: &Point) -> Params {
    env.space
        .continuous
        .iter()
        .zip(&point.grid)
        .map(|(p, g)| (p.name.clone(), ParamValue::Number(p.value(*g))))
        .chain(
            env.space
                .categorical
                .iter()
                .zip(&point.combo)
                .map(|(p, c)| (p.name.clone(), ParamValue::Choice(p.choices[*c].clone()))),
        )
        .collect()
}

/// Bounds, step-grid, choice and provenance checks for an external candidate.
pub fn validate_candidate(env: &DesignEnvelope, candidate: &ExternalCandidate) -> Result<()> {
    if !bounded_text(&candidate.provenance, 1024) {
        return Err(invalid("external candidate provenance required"));
    }
    point(env, &candidate.params).map(|_| ())
}

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Seeded Latin-hypercube screen per categorical combination, snapped to the grid.
fn screen_points(env: &DesignEnvelope) -> Vec<Point> {
    let per = env.screen_per_combination() as usize;
    let mut out: Vec<Point> = vec![];
    for (index, combo) in env.combinations().into_iter().enumerate() {
        let mut rng =
            SplitMix(u64::from(env.seed) ^ (index as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
        let perms: Vec<Vec<usize>> = env
            .space
            .continuous
            .iter()
            .map(|_| {
                let mut perm: Vec<usize> = (0..per).collect();
                for i in (1..per).rev() {
                    perm.swap(i, (rng.next() % (i as u64 + 1)) as usize);
                }
                perm
            })
            .collect();
        for j in 0..per {
            let grid = env
                .space
                .continuous
                .iter()
                .zip(&perms)
                .map(|(p, perm)| {
                    let u = (perm[j] as f64 + rng.unit()) / per as f64;
                    (u * p.steps() as f64).round() as i64
                })
                .collect();
            let point = Point {
                combo: combo.clone(),
                grid,
            };
            if !out.contains(&point) {
                out.push(point);
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    Feasible,
    Infeasible,
    Unknown,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ConstraintAssessment {
    pub metric: String,
    pub value: Option<f64>,
    pub margin: Option<f64>,
    pub satisfied: Option<bool>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Assessment {
    pub standing: Standing,
    pub objective: Option<f64>,
    pub violation: f64,
    pub min_margin: Option<f64>,
    pub constraints: Vec<ConstraintAssessment>,
}

pub type Metrics = BTreeMap<String, Option<f64>>;

/// Feasibility-first assessment; a missing metric never passes a constraint.
pub fn assess(env: &DesignEnvelope, metrics: &Metrics) -> Assessment {
    let value = |name: &str| {
        metrics
            .get(name)
            .copied()
            .flatten()
            .filter(|v| v.is_finite())
    };
    let constraints: Vec<_> = env
        .constraints
        .iter()
        .map(|c| {
            let v = value(&c.metric);
            let margin = v.map(|v| c.margin(v));
            ConstraintAssessment {
                metric: c.metric.clone(),
                value: v,
                margin,
                satisfied: margin.map(|m| m >= 0.),
            }
        })
        .collect();
    let objective = env
        .objective
        .terms
        .iter()
        .map(|t| value(&t.metric).map(|v| t.weight * v))
        .sum::<Option<f64>>();
    let violation = constraints
        .iter()
        .filter_map(|c| c.margin)
        .map(|m| (-m).max(0.))
        .sum();
    let unknown = constraints.iter().any(|c| c.margin.is_none());
    let min_margin = if unknown {
        None
    } else {
        constraints
            .iter()
            .filter_map(|c| c.margin)
            .min_by(f64::total_cmp)
    };
    let standing = if constraints.iter().any(|c| c.satisfied == Some(false)) {
        Standing::Infeasible
    } else if unknown || objective.is_none() {
        Standing::Unknown
    } else {
        Standing::Feasible
    };
    Assessment {
        standing,
        objective,
        violation,
        min_margin,
        constraints,
    }
}

fn rank_key(a: &Assessment) -> (u8, f64) {
    match a.standing {
        Standing::Feasible => (0, a.objective.unwrap_or(f64::INFINITY)),
        Standing::Infeasible => (1, a.violation),
        Standing::Unknown => (2, 0.),
    }
}

/// Sort `(index, assessment)` pairs: feasible by objective, infeasible by
/// total normalized violation, unknown last; ties keep the earlier index.
pub fn rank(items: &mut [(u32, Assessment)]) {
    items.sort_by(|(ia, a), (ib, b)| {
        let (ka, kb) = (rank_key(a), rank_key(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1)).then(ia.cmp(ib))
    });
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChildRecord {
    pub role: String,
    /// `None` until a chained child's plan is derived.
    pub plan_digest: Option<String>,
    pub idempotency_key: Option<String>,
    pub max_artifact_bytes: u64,
    /// Upstream job a chained plan was derived from (bound in its key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_job: Option<String>,
    /// Why a chained child was never submitted; its metrics stay unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub index: u32,
    /// `proposer` or `external`.
    pub origin: String,
    pub provenance: Option<String>,
    pub fidelity: Level,
    pub parent: Option<u32>,
    pub params: Params,
    pub children: Vec<ChildRecord>,
    /// `pending` until every child is terminal, then `evaluated`.
    pub state: String,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub params: Params,
    pub fidelity: Level,
    pub parent: Option<u32>,
}

/// Deterministic next candidate from the envelope and complete history.
// ponytail: pattern search; swap for a GP/BO surrogate if the evaluation budget binds
pub fn propose(env: &DesignEnvelope, history: &[Candidate]) -> Result<Option<Proposal>> {
    let max = env.budget.max_evaluations as usize;
    if history.len() >= max {
        return Ok(None);
    }
    let k = env.refine_k() as usize;
    let mut screens = vec![];
    for c in history.iter().filter(|c| c.fidelity == Level::Screen) {
        screens.push((point(env, &c.params)?, c.index, assess(env, &c.metrics)));
    }
    let evaluated: BTreeSet<&Point> = screens.iter().map(|(p, ..)| p).collect();
    let screen = |p: &Point| {
        Some(Proposal {
            params: params_of(env, p),
            fidelity: Level::Screen,
            parent: None,
        })
    };
    if screens.len() < max - k {
        if let Some(p) = screen_points(env).iter().find(|p| !evaluated.contains(p)) {
            return Ok(screen(p));
        }
        let mut ranked: Vec<(u32, Assessment)> =
            screens.iter().map(|(_, i, a)| (*i, a.clone())).collect();
        rank(&mut ranked);
        let mut combos_done = BTreeSet::new();
        for (index, assessment) in &ranked {
            let center = &screens.iter().find(|(_, i, _)| i == index).unwrap().0;
            if assessment.standing == Standing::Unknown || !combos_done.insert(&center.combo) {
                continue;
            }
            // Keep the step that reached the incumbent; halve only after a failed poll.
            let finest = (0..64)
                .find(|level| level_steps(env, *level).iter().all(|s| *s == 1))
                .unwrap_or(0);
            let start = (0..=finest)
                .find(|level| {
                    let steps = level_steps(env, *level);
                    evaluated
                        .iter()
                        .any(|y| polls(env, y, &steps, false).contains(center))
                })
                .unwrap_or(0);
            for level in start..=finest {
                if let Some(p) = polls(env, center, &level_steps(env, level), level == finest)
                    .into_iter()
                    .find(|p| !evaluated.contains(p))
                {
                    return Ok(screen(&p));
                }
            }
        }
    }
    if k > 0 {
        let refined: BTreeSet<u32> = history.iter().filter_map(|c| c.parent).collect();
        let mut ranked: Vec<(u32, Assessment)> = screens
            .iter()
            .filter(|(_, _, a)| a.standing != Standing::Unknown)
            .map(|(_, i, a)| (*i, a.clone()))
            .collect();
        rank(&mut ranked);
        if let Some((index, _)) = ranked.iter().take(k).find(|(i, _)| !refined.contains(i)) {
            let parent = &history[*index as usize];
            return Ok(Some(Proposal {
                params: parent.params.clone(),
                fidelity: Level::Refine,
                parent: Some(*index),
            }));
        }
    }
    Ok(None)
}

/// Grid steps per axis at a refinement level: a quarter range, halved per level, floored at one step.
fn level_steps(env: &DesignEnvelope, level: u32) -> Vec<i64> {
    env.space
        .continuous
        .iter()
        .map(|p| (p.steps() >> (level + 2).min(62)).max(1))
        .collect()
}

/// Compass poll (±step per axis), clamped to bounds. At the step floor,
/// pairwise diagonals follow so a diagonal constraint boundary cannot stall it.
fn polls(env: &DesignEnvelope, center: &Point, steps: &[i64], diagonals: bool) -> Vec<Point> {
    let d = steps.len();
    let mut moves: Vec<Vec<i64>> = vec![];
    for i in 0..d {
        for sign in [1, -1] {
            let mut m = vec![0; d];
            m[i] = sign * steps[i];
            moves.push(m);
        }
    }
    for i in 0..d * usize::from(diagonals) {
        for j in i + 1..d {
            for (si, sj) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let mut m = vec![0; d];
                m[i] = si * steps[i];
                m[j] = sj * steps[j];
                moves.push(m);
            }
        }
    }
    moves
        .into_iter()
        .map(|m| Point {
            combo: center.combo.clone(),
            grid: center
                .grid
                .iter()
                .zip(&m)
                .zip(&env.space.continuous)
                .map(|((g, delta), p)| (g + delta).clamp(0, p.steps()))
                .collect(),
        })
        .filter(|p| p != center)
        .collect()
}

/// A metric source's correlation request bound to one thermal job.
fn correlation_request(
    request: &Value,
    job_id: &str,
) -> Result<crate::correlation::CorrelationRequest> {
    let mut request = request
        .as_object()
        .filter(|r| !r.contains_key("job_id"))
        .cloned()
        .ok_or_else(|| invalid("correlation request object without job_id required"))?;
    request.insert("job_id".into(), json!(job_id));
    let request: crate::correlation::CorrelationRequest =
        serde_json::from_value(Value::Object(request))
            .map_err(|e| invalid(format!("correlation request schema: {e}")))?;
    request.validate()?;
    Ok(request)
}

/// Combine job-extracted values with parameter, declared and provider metrics.
pub fn compute_metrics(
    env: &DesignEnvelope,
    params: &Params,
    job_values: &Metrics,
    providers: &[&dyn MetricProvider],
) -> Metrics {
    let mut metrics = Metrics::new();
    for spec in &env.metric_sources {
        let value = match &spec.source {
            MetricSource::Job { .. } | MetricSource::Correlation { .. } => {
                job_values.get(&spec.metric).copied().flatten()
            }
            MetricSource::Parameter { param } => match params.get(param) {
                Some(ParamValue::Number(v)) => Some(*v),
                _ => None,
            },
            MetricSource::Declared { input } => match input {
                PhysicalInput::Known { value, .. } => Some(*value),
                PhysicalInput::Missing { .. } => None,
            },
            MetricSource::Sourced { key } => match sourced(key) {
                Ok(PhysicalInput::Known { value, .. }) => Some(value),
                _ => None,
            },
            MetricSource::Provider { .. } => continue,
        };
        metrics.insert(spec.metric.clone(), value.filter(|v| v.is_finite()));
    }
    let known: BTreeMap<String, f64> = metrics
        .iter()
        .filter_map(|(k, v)| v.map(|v| (k.clone(), v)))
        .collect();
    for spec in &env.metric_sources {
        if let MetricSource::Provider {
            name,
            params: names,
        } = &spec.source
        {
            let selected: Params = params
                .iter()
                .filter(|(k, _)| names.contains(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let value = providers
                .iter()
                .find(|p| p.name() == name)
                .and_then(|p| p.evaluate(&selected, &known).ok().flatten())
                .filter(|v| v.is_finite());
            metrics.insert(spec.metric.clone(), value);
        }
    }
    metrics
}

/// CI-runnable example over the analytical channel reference. Minimize
/// `0.1*height_mm + acceleration` subject to mean velocity >= 0.1 m/s; the
/// known grid optimum is height 6 mm, acceleration 0.34 m/s2.
pub fn example() -> Result<DesignEnvelope> {
    let template = |resolution: u32| -> Result<Value> {
        let mut case = CaseSpec::reference();
        case.resolution = resolution;
        let mut plan = serde_json::to_value(ExecutionPlan::reference(case)?)?;
        plan["case"]["channel_height"] = json!({"$param": "height", "unit": "mm"});
        plan["case"]["acceleration"] = json!({"$param": "acceleration", "unit": "m/s2"});
        Ok(plan)
    };
    Ok(DesignEnvelope {
        schema_version: 1,
        name: "channel_drive_height".into(),
        provenance: "synthetic analytical channel reference with a known grid optimum; not an enclosure model".into(),
        seed: 7,
        space: DesignSpace {
            continuous: vec![
                ContinuousParam {
                    name: "height".into(),
                    low: 4.,
                    high: 20.,
                    step: 1.,
                    unit: "mm".into(),
                },
                ContinuousParam {
                    name: "acceleration".into(),
                    low: 0.05,
                    high: 0.5,
                    step: 0.01,
                    unit: "m/s2".into(),
                },
            ],
            categorical: vec![],
        },
        objective: Objective {
            kind: ObjectiveKind::Minimize,
            terms: vec![
                ObjectiveTerm {
                    metric: "height".into(),
                    weight: 0.1,
                },
                ObjectiveTerm {
                    metric: "acceleration".into(),
                    weight: 1.,
                },
            ],
        },
        constraints: vec![Constraint {
            metric: "mean_velocity".into(),
            op: ConstraintOp::Ge,
            limit: 0.1,
            unit: "m/s".into(),
            safety_factor: None,
        }],
        budget: Budget {
            max_evaluations: 32,
            max_wall_seconds: 600,
            max_artifact_bytes: 32 * 1024 * 1024,
        },
        fidelity: Fidelity {
            screen: "screen".into(),
            refine_top_k: Some(1),
            refine: Some("refine".into()),
        },
        templates: BTreeMap::from([
            (
                "screen".into(),
                vec![ChildTemplate::complete("flow", template(33)?)],
            ),
            (
                "refine".into(),
                vec![ChildTemplate::complete("flow", template(65)?)],
            ),
        ]),
        metric_sources: vec![
            MetricSpec {
                metric: "mean_velocity".into(),
                unit: "m/s".into(),
                source: MetricSource::Job {
                    role: "flow".into(),
                    artifact: "validation.json".into(),
                    pointer: "/mean_velocity_m_s".into(),
                },
            },
            MetricSpec {
                metric: "height".into(),
                unit: "mm".into(),
                source: MetricSource::Parameter {
                    param: "height".into(),
                },
            },
            MetricSpec {
                metric: "acceleration".into(),
                unit: "m/s2".into(),
                source: MetricSource::Parameter {
                    param: "acceleration".into(),
                },
            },
        ],
    })
}

// ---- durable state ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignIntent {
    pub schema_version: u32,
    pub id: String,
    pub envelope: DesignEnvelope,
    pub envelope_digest: String,
    pub profile: HostExecutionProfile,
    pub profile_digest: String,
    pub created_at: i64,
}

fn child_key(intent: &DesignIntent, index: u32, role: &str) -> Result<String> {
    Ok(format!(
        "design_{}",
        digest(&(&intent.id, &intent.envelope_digest, index, role))?
    ))
}

/// A derived plan is additionally bound to the upstream job it came from.
fn derived_key(
    intent: &DesignIntent,
    index: u32,
    role: &str,
    upstream: Option<&str>,
) -> Result<String> {
    Ok(format!(
        "design_{}",
        digest(&(&intent.id, &intent.envelope_digest, index, role, upstream))?
    ))
}

pub fn retain_intent(
    store: &Store,
    env: &DesignEnvelope,
    key: &str,
    profile: &HostExecutionProfile,
) -> Result<DesignIntent> {
    prepare(env, Some(&profile.policy))?;
    if !token(key) {
        return Err(invalid("bounded design idempotency key required"));
    }
    let tx = rusqlite::Transaction::new_unchecked(
        &store.connection,
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let prior: Option<String> = tx
        .query_row("SELECT intent FROM designs WHERE idem=?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    if let Some(prior) = prior {
        let intent: DesignIntent = serde_json::from_str(&prior)?;
        if intent.envelope_digest != digest(env)? || intent.profile_digest != digest(profile)? {
            return Err(Error::IdempotencyConflict);
        }
        tx.commit()?;
        return Ok(intent);
    }
    let intent = DesignIntent {
        schema_version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        envelope: env.clone(),
        envelope_digest: digest(env)?,
        profile: profile.clone(),
        profile_digest: digest(profile)?,
        created_at: now(),
    };
    tx.execute(
        "INSERT INTO designs(id,idem,intent,state) VALUES(?1,?2,?3,'running')",
        params![intent.id, key, serde_json::to_string(&intent)?],
    )?;
    tx.commit()?;
    Ok(intent)
}

fn load(store: &Store, id: &str) -> Result<(DesignIntent, String, Option<String>)> {
    let (raw, state, error): (String, String, Option<String>) = store
        .connection
        .query_row(
            "SELECT intent,state,error FROM designs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| invalid("unknown design identity"))?;
    let intent: DesignIntent = serde_json::from_str(&raw)?;
    if intent.schema_version != 1
        || intent.id != id
        || intent.envelope_digest != digest(&intent.envelope)?
        || intent.profile_digest != digest(&intent.profile)?
    {
        return Err(invalid(
            "persisted design intent differs from its approved envelope or profile",
        ));
    }
    Ok((intent, state, error))
}

fn candidates(store: &Store, id: &str) -> Result<Vec<Candidate>> {
    let mut stmt = store
        .connection
        .prepare("SELECT idx,candidate FROM design_candidates WHERE design=?1 ORDER BY idx")?;
    let rows = stmt
        .query_map([id], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .enumerate()
        .map(|(position, (index, raw))| {
            let candidate: Candidate = serde_json::from_str(&raw)?;
            if candidate.index != index || index as usize != position {
                return Err(invalid(
                    "persisted design candidate sequence is not contiguous",
                ));
            }
            Ok(candidate)
        })
        .collect()
}

pub fn running(store: &Store) -> Result<Vec<String>> {
    let mut stmt = store
        .connection
        .prepare("SELECT id FROM designs WHERE state='running' ORDER BY rowid")?;
    Ok(stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Record a terminal non-transient advancement failure.
pub fn fail(store: &Store, id: &str, error: &str) -> Result<()> {
    store.connection.execute(
        "UPDATE designs SET state='failed',error=?1 WHERE id=?2 AND state='running'",
        params![&error[..error.len().min(1024)], id],
    )?;
    Ok(())
}

fn finish(store: &Store, id: &str, state: &str) -> Result<()> {
    store.connection.execute(
        "UPDATE designs SET state=?1 WHERE id=?2 AND state='running'",
        params![state, id],
    )?;
    Ok(())
}

fn job_by_key(store: &Store, key: &str) -> Result<Option<Job>> {
    let id: Option<String> = store
        .connection
        .query_row("SELECT id FROM jobs WHERE idem=?1", [key], |r| r.get(0))
        .optional()?;
    id.map(|id| store.job(&id)).transpose()
}

fn terminal(state: &str) -> bool {
    matches!(state, "succeeded" | "failed" | "cancelled" | "interrupted")
}

pub type Submit<'a> = dyn FnMut(&ExecutionPlan, &str) -> Result<()> + 'a;

fn save(store: &Store, id: &str, candidate: &Candidate) -> Result<()> {
    store.connection.execute(
        "UPDATE design_candidates SET candidate=?1 WHERE design=?2 AND idx=?3",
        params![serde_json::to_string(candidate)?, id, candidate.index],
    )?;
    Ok(())
}

/// Resubmit missing children idempotently, derive chained children whose
/// upstream succeeded (blocking them when it did not), and extract metrics
/// once every child is terminal or blocked. Persists every change.
fn refresh(
    store: &Store,
    intent: &DesignIntent,
    candidate: &mut Candidate,
    submit: &mut Submit,
    planner: &mut Planner,
) -> Result<bool> {
    let env = &intent.envelope;
    let policy = &intent.profile.policy;
    let templates = &env.templates[env.set(candidate.fidelity)?];
    if templates.len() != candidate.children.len()
        || templates
            .iter()
            .zip(&candidate.children)
            .any(|(t, c)| t.role != c.role)
    {
        return Err(invalid(
            "persisted candidate children differ from the approved templates",
        ));
    }
    let mut jobs = vec![];
    for child in &candidate.children {
        jobs.push(match &child.idempotency_key {
            Some(key) => job_by_key(store, key)?,
            None => None,
        });
    }
    let missing_static = templates
        .iter()
        .zip(&jobs)
        .any(|(t, j)| t.plan_request.is_none() && j.is_none());
    if missing_static {
        let plans = child_plans(env, candidate.fidelity, &candidate.params, policy)?;
        for ((child, (_, plan)), job) in candidate.children.iter().zip(&plans).zip(&mut jobs) {
            let (Some(plan), Some(key)) = (plan, &child.idempotency_key) else {
                continue;
            };
            if Some(plan.id()?) != child.plan_digest {
                return Err(invalid(
                    "regenerated child plan differs from its recorded digest",
                ));
            }
            if job.is_none() {
                submit(plan, key)?;
                *job = job_by_key(store, key)?;
            }
        }
    }
    let mut changed = false;
    for i in 0..templates.len() {
        let template = &templates[i];
        if template.plan_request.is_none() || candidate.children[i].blocked.is_some() {
            continue;
        }
        let succeeded: BTreeMap<String, String> = candidate
            .children
            .iter()
            .zip(&jobs)
            .filter_map(|(c, j)| {
                j.as_ref()
                    .filter(|j| j.state == "succeeded")
                    .map(|j| (c.role.clone(), j.id.clone()))
            })
            .collect();
        if let Some(key) = candidate.children[i].idempotency_key.clone() {
            if jobs[i].is_none() {
                // Crash between recording and submitting: derive the same plan again.
                let plan = derive(
                    store,
                    env,
                    template,
                    &candidate.params,
                    &succeeded,
                    policy,
                    planner,
                )?;
                if Some(plan.id()?) != candidate.children[i].plan_digest {
                    return Err(invalid(
                        "re-derived chained plan differs from its recorded digest",
                    ));
                }
                submit(&plan, &key)?;
                jobs[i] = job_by_key(store, &key)?;
            }
            continue;
        }
        let upstream_job = match &template.after {
            None => None,
            Some(after) => {
                let u = templates
                    .iter()
                    .position(|t| &t.role == after)
                    .ok_or_else(|| invalid("chained template upstream role missing"))?;
                let blocked = match (&candidate.children[u].blocked, &jobs[u]) {
                    (Some(reason), _) => {
                        Some(format!("upstream {after} never submitted: {reason}"))
                    }
                    (None, None) => continue,
                    (None, Some(job)) if !terminal(&job.state) => continue,
                    (None, Some(job)) if job.state != "succeeded" => {
                        Some(format!("upstream {after} {}", job.state))
                    }
                    (None, Some(_)) => None,
                };
                if let Some(reason) = blocked {
                    candidate.children[i].blocked = Some(reason);
                    changed = true;
                    continue;
                }
                jobs[u].as_ref().map(|j| j.id.clone())
            }
        };
        match derive(
            store,
            env,
            template,
            &candidate.params,
            &succeeded,
            policy,
            planner,
        ) {
            Ok(plan) => {
                let key = derived_key(
                    intent,
                    candidate.index,
                    &template.role,
                    upstream_job.as_deref(),
                )?;
                let child = &mut candidate.children[i];
                child.plan_digest = Some(plan.id()?);
                child.idempotency_key = Some(key.clone());
                child.upstream_job = upstream_job;
                // Intent first: the derived binding is durable before submission.
                save(store, &intent.id, candidate)?;
                submit(&plan, &key)?;
                jobs[i] = job_by_key(store, &key)?;
            }
            // Transient state and missing host capability are not per-candidate facts.
            Err(error @ (Error::Io(_) | Error::Sql(_) | Error::Unqualified(_))) => {
                return Err(error);
            }
            Err(error) => {
                candidate.children[i].blocked = Some(format!("chained planning refused: {error}"));
                changed = true;
            }
        }
    }
    let mut values = Metrics::new();
    let mut pending = false;
    for (child, job) in candidate.children.iter().zip(&jobs) {
        if child.blocked.is_some() {
            continue;
        }
        let Some(job) = job else {
            if child.idempotency_key.is_some() {
                return Err(invalid("child submission not durable"));
            }
            pending = true;
            continue;
        };
        if Some(&job.plan_digest) != child.plan_digest.as_ref() {
            return Err(Error::IdempotencyConflict);
        }
        if !terminal(&job.state) {
            pending = true;
            continue;
        }
        if job.state != "succeeded" {
            continue;
        }
        for spec in &env.metric_sources {
            let value = match &spec.source {
                MetricSource::Job {
                    role,
                    artifact,
                    pointer,
                } if role == &child.role => {
                    crate::qualification::registered_json(store, &job.id, artifact)
                        .ok()
                        .flatten()
                        .and_then(|(_, v)| v.pointer(pointer).and_then(Value::as_f64))
                }
                MetricSource::Correlation { role, request } if role == &child.role => {
                    correlation_request(request, &job.id)
                        .and_then(|request| {
                            crate::correlation::run(
                                store,
                                Path::new(&intent.profile.allowed_input_root),
                                &request,
                            )
                        })
                        .ok()
                        .map(|report| crate::correlation::objective(&report))
                }
                _ => continue,
            };
            values.insert(spec.metric.clone(), value);
        }
    }
    if pending {
        if changed {
            save(store, &intent.id, candidate)?;
        }
        return Ok(false);
    }
    candidate.metrics = compute_metrics(env, &candidate.params, &values, PROVIDERS);
    candidate.state = "evaluated".into();
    save(store, &intent.id, candidate)?;
    Ok(true)
}

fn allocated(history: &[Candidate]) -> u64 {
    history
        .iter()
        .flat_map(|c| &c.children)
        .map(|c| c.max_artifact_bytes)
        .sum()
}

fn create(
    store: &Store,
    intent: &DesignIntent,
    history: &[Candidate],
    proposal: Proposal,
    origin: (&str, Option<String>),
    submit: &mut Submit,
) -> Result<Candidate> {
    let env = &intent.envelope;
    let index = history.len() as u32;
    let plans = child_plans(
        env,
        proposal.fidelity,
        &proposal.params,
        &intent.profile.policy,
    )?;
    let templates = &env.templates[env.set(proposal.fidelity)?];
    let children = plans
        .iter()
        .zip(templates)
        .map(|((role, plan), template)| {
            Ok(match plan {
                Some(plan) => ChildRecord {
                    role: role.clone(),
                    plan_digest: Some(plan.id()?),
                    idempotency_key: Some(child_key(intent, index, role)?),
                    max_artifact_bytes: plan.observation.max_artifact_bytes,
                    upstream_job: None,
                    blocked: None,
                },
                // Chained: the approved allowance is reserved before derivation.
                None => ChildRecord {
                    role: role.clone(),
                    plan_digest: None,
                    idempotency_key: None,
                    max_artifact_bytes: template.max_artifact_bytes.unwrap_or(0),
                    upstream_job: None,
                    blocked: None,
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let bytes: u64 = children.iter().map(|c| c.max_artifact_bytes).sum();
    if allocated(history).saturating_add(bytes) > env.budget.max_artifact_bytes {
        return Err(Error::Resource("design artifact budget exhausted".into()));
    }
    let candidate = Candidate {
        index,
        origin: origin.0.into(),
        provenance: origin.1,
        fidelity: proposal.fidelity,
        parent: proposal.parent,
        params: proposal.params,
        children,
        state: "pending".into(),
        metrics: Metrics::new(),
    };
    // Intent first: a crash after this row resumes the same children by key.
    store.connection.execute(
        "INSERT INTO design_candidates(design,idx,candidate) VALUES(?1,?2,?3)",
        params![intent.id, index, serde_json::to_string(&candidate)?],
    )?;
    for (child, (_, plan)) in candidate.children.iter().zip(&plans) {
        if let (Some(plan), Some(key)) = (plan, &child.idempotency_key) {
            submit(plan, key)?;
        }
    }
    Ok(candidate)
}

/// Advance one running design: refresh pending children (deriving chained
/// plans through `planner`), then propose the next deterministic candidate
/// when nothing is pending. Execution stays with the ordinary worker queue
/// and shared admission.
pub fn advance(
    store: &Store,
    id: &str,
    profile: &HostExecutionProfile,
    submit: &mut Submit,
    planner: &mut Planner,
) -> Result<()> {
    let (intent, state, _) = load(store, id)?;
    if state != "running" {
        return Ok(());
    }
    if intent.profile_digest != digest(profile)? {
        return Err(Error::Unqualified(
            "worker host profile differs from the design's bound profile".into(),
        ));
    }
    let mut history = candidates(store, id)?;
    let mut pending = false;
    for candidate in history.iter_mut().filter(|c| c.state == "pending") {
        pending |= !refresh(store, &intent, candidate, submit, planner)?;
    }
    if pending {
        return Ok(());
    }
    let env = &intent.envelope;
    if history.len() >= env.budget.max_evaluations as usize {
        return finish(store, id, "budget_exhausted");
    }
    if now().saturating_sub(intent.created_at) as u64 > env.budget.max_wall_seconds {
        return finish(store, id, "wall_time_exhausted");
    }
    match propose(env, &history)? {
        None => finish(store, id, "completed"),
        Some(proposal) => {
            match create(
                store,
                &intent,
                &history,
                proposal,
                ("proposer", None),
                submit,
            ) {
                Err(Error::Resource(_)) => finish(store, id, "artifact_budget_exhausted"),
                // Root plan requests derive immediately rather than next tick.
                Ok(mut candidate) => {
                    refresh(store, &intent, &mut candidate, submit, planner).map(|_| ())
                }
                Err(error) => Err(error),
            }
        }
    }
}

/// Evaluate an external candidate exactly like a proposal. Repeating an
/// already-screened point returns the existing candidate.
pub fn propose_external(
    store: &Store,
    id: &str,
    candidate: &ExternalCandidate,
    profile: &HostExecutionProfile,
    submit: &mut Submit,
) -> Result<Candidate> {
    let (intent, state, _) = load(store, id)?;
    validate_candidate(&intent.envelope, candidate)?;
    let wanted = point(&intent.envelope, &candidate.params)?;
    let history = candidates(store, id)?;
    for c in &history {
        if c.fidelity == Level::Screen && point(&intent.envelope, &c.params)? == wanted {
            return Ok(c.clone());
        }
    }
    if state != "running" {
        return Err(invalid(format!("design is {state}; no further candidates")));
    }
    if intent.profile_digest != digest(profile)? {
        return Err(Error::Unqualified(
            "worker host profile differs from the design's bound profile".into(),
        ));
    }
    if history.len() >= intent.envelope.budget.max_evaluations as usize {
        return Err(Error::Resource("design evaluation budget exhausted".into()));
    }
    create(
        store,
        &intent,
        &history,
        Proposal {
            params: params_of(&intent.envelope, &wanted),
            fidelity: Level::Screen,
            parent: None,
        },
        ("external", Some(candidate.provenance.clone())),
        submit,
    )
}

// ---- status and export --------------------------------------------------

#[derive(Debug, Serialize, JsonSchema)]
pub struct JobRef {
    pub role: String,
    pub job_id: Option<String>,
    pub state: Option<String>,
    /// Chained child never submitted (upstream failure or planning refusal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Iteration {
    pub index: u32,
    pub origin: String,
    pub fidelity: Level,
    pub parent: Option<u32>,
    pub state: String,
    pub params: Params,
    pub metrics: Metrics,
    pub standing: Standing,
    pub objective: Option<f64>,
    pub violation: f64,
    pub min_margin: Option<f64>,
    pub jobs: Vec<JobRef>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BudgetUse {
    pub max_evaluations: u32,
    pub evaluations: u32,
    pub max_wall_seconds: u64,
    pub elapsed_seconds: u64,
    pub max_artifact_bytes: u64,
    pub allocated_artifact_bytes: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DesignStatus {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub envelope_digest: String,
    pub state: String,
    pub error: Option<String>,
    pub budget: BudgetUse,
    pub iterations: Vec<Iteration>,
    pub incumbent: Option<u32>,
    pub feasible: Vec<u32>,
    /// Non-dominated in (minimize objective, maximize minimum constraint margin).
    pub pareto: Vec<u32>,
    pub physical_validation: String,
    /// Declared metric inputs labelled synthetic; nonempty means every
    /// result derived from them is synthetic demonstration output.
    pub synthetic_inputs: Vec<String>,
}

fn synthetic_inputs(env: &DesignEnvelope) -> Vec<String> {
    env.metric_sources
        .iter()
        .filter(|m| matches!(&m.source, MetricSource::Declared { input } if input.is_synthetic()))
        .map(|m| m.metric.clone())
        .collect()
}

fn jobs(store: &Store, candidate: &Candidate) -> Result<Vec<JobRef>> {
    candidate
        .children
        .iter()
        .map(|child| {
            let job = match &child.idempotency_key {
                Some(key) => job_by_key(store, key)?,
                None => None,
            };
            Ok(JobRef {
                role: child.role.clone(),
                job_id: job.as_ref().map(|j| j.id.clone()),
                state: job.map(|j| j.state),
                blocked: child.blocked.clone(),
            })
        })
        .collect()
}

/// Best feasible candidate, preferring refined (higher-fidelity) evaluations.
pub fn incumbent(env: &DesignEnvelope, history: &[Candidate]) -> Option<u32> {
    let best = |level: Level| {
        let mut items: Vec<(u32, Assessment)> = history
            .iter()
            .filter(|c| c.fidelity == level)
            .map(|c| (c.index, assess(env, &c.metrics)))
            .filter(|(_, a)| a.standing == Standing::Feasible)
            .collect();
        rank(&mut items);
        items.first().map(|(i, _)| *i)
    };
    best(Level::Refine).or_else(|| best(Level::Screen))
}

pub fn status(store: &Store, id: &str) -> Result<DesignStatus> {
    let (intent, state, error) = load(store, id)?;
    prepare(&intent.envelope, Some(&intent.profile.policy))?;
    let env = &intent.envelope;
    let history = candidates(store, id)?;
    let assessments: Vec<Assessment> = history.iter().map(|c| assess(env, &c.metrics)).collect();
    let mut iterations = vec![];
    for (c, a) in history.iter().zip(&assessments) {
        iterations.push(Iteration {
            index: c.index,
            origin: c.origin.clone(),
            fidelity: c.fidelity,
            parent: c.parent,
            state: c.state.clone(),
            params: c.params.clone(),
            metrics: c.metrics.clone(),
            standing: a.standing,
            objective: a.objective,
            violation: a.violation,
            min_margin: a.min_margin,
            jobs: jobs(store, c)?,
        });
    }
    let both: Vec<(u32, f64, f64)> = iterations
        .iter()
        .filter_map(|i| Some((i.index, i.objective?, i.min_margin?)))
        .collect();
    let pareto = both
        .iter()
        .filter(|(_, o, m)| {
            !both
                .iter()
                .any(|(_, o2, m2)| o2 <= o && m2 >= m && (o2 < o || m2 > m))
        })
        .map(|(i, ..)| *i)
        .collect();
    Ok(DesignStatus {
        schema_version: 1,
        id: intent.id.clone(),
        name: env.name.clone(),
        envelope_digest: intent.envelope_digest.clone(),
        state,
        error,
        budget: BudgetUse {
            max_evaluations: env.budget.max_evaluations,
            evaluations: history.len() as u32,
            max_wall_seconds: env.budget.max_wall_seconds,
            elapsed_seconds: now().saturating_sub(intent.created_at) as u64,
            max_artifact_bytes: env.budget.max_artifact_bytes,
            allocated_artifact_bytes: allocated(&history),
        },
        incumbent: incumbent(env, &history),
        feasible: iterations
            .iter()
            .filter(|i| i.standing == Standing::Feasible)
            .map(|i| i.index)
            .collect(),
        pareto,
        iterations,
        physical_validation: "unqualified".into(),
        synthetic_inputs: synthetic_inputs(env),
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConstraintEvidence {
    pub metric: String,
    pub op: ConstraintOp,
    pub limit: f64,
    pub effective_limit: f64,
    pub unit: String,
    pub safety_factor: Option<f64>,
    pub value: Option<f64>,
    pub margin: Option<f64>,
    pub satisfied: Option<bool>,
    /// `simulated` (child job output), `calibrated`/`estimated` (local
    /// provider), `prescribed` (design parameter) or `unknown` (missing).
    pub evidence: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Winner {
    pub index: u32,
    pub fidelity: Level,
    pub params: Params,
    pub objective: Option<f64>,
    pub jobs: Vec<JobRef>,
    pub constraints: Vec<ConstraintEvidence>,
    /// Present when the objective minimizes measured-correlation RMSE: the
    /// winner's parameters are then `calibrated` against that test only.
    pub calibration: Vec<Calibration>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Calibration {
    pub label: String,
    pub metric: String,
    pub rmse_k: Option<f64>,
    pub role: String,
    pub measurement_csv: Value,
    pub test_provenance: Value,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DesignReport {
    pub schema_version: u32,
    pub design_id: String,
    pub name: String,
    pub envelope_digest: String,
    pub state: String,
    pub evaluations: u32,
    pub feasible: u32,
    pub winner: Option<Winner>,
    pub statement: String,
    pub physical_validation: String,
    pub synthetic_inputs: Vec<String>,
}

pub fn export(store: &Store, id: &str) -> Result<DesignReport> {
    let status = status(store, id)?;
    let (intent, ..) = load(store, id)?;
    let env = &intent.envelope;
    let history = candidates(store, id)?;
    let winner = match status.incumbent {
        None => None,
        Some(index) => {
            let c = &history[index as usize];
            let a = assess(env, &c.metrics);
            let constraints = env
                .constraints
                .iter()
                .zip(a.constraints)
                .map(|(spec, ca)| ConstraintEvidence {
                    metric: spec.metric.clone(),
                    op: spec.op,
                    limit: spec.limit,
                    effective_limit: spec.effective_limit(),
                    unit: spec.unit.clone(),
                    safety_factor: spec.safety_factor,
                    evidence: match (ca.value, env.metric(&spec.metric).map(|m| &m.source)) {
                        (None, _) | (_, None) => "unknown",
                        (_, Some(MetricSource::Job { .. })) => "simulated",
                        (
                            _,
                            Some(
                                MetricSource::Parameter { .. }
                                | MetricSource::Declared { .. }
                                | MetricSource::Sourced { .. },
                            ),
                        ) => "prescribed",
                        (_, Some(MetricSource::Correlation { .. })) => "measured",
                        (_, Some(MetricSource::Provider { name, .. })) => {
                            provider(name).map_or("unknown", |p| p.evidence())
                        }
                    }
                    .into(),
                    value: ca.value,
                    margin: ca.margin,
                    satisfied: ca.satisfied,
                })
                .collect();
            // Parameters minimizing a measured-correlation objective are
            // calibrated values, labelled with the approved test provenance.
            let calibration = env
                .objective
                .terms
                .iter()
                .filter_map(|t| match env.metric(&t.metric).map(|m| &m.source) {
                    Some(MetricSource::Correlation { role, request }) => Some(Calibration {
                        label: "calibrated".into(),
                        metric: t.metric.clone(),
                        rmse_k: c.metrics.get(&t.metric).copied().flatten(),
                        role: role.clone(),
                        measurement_csv: request["measurement_csv"].clone(),
                        test_provenance: request["provenance"].clone(),
                    }),
                    _ => None,
                })
                .collect();
            Some(Winner {
                index,
                fidelity: c.fidelity,
                params: c.params.clone(),
                objective: a.objective,
                jobs: jobs(store, c)?,
                constraints,
                calibration,
            })
        }
    };
    Ok(DesignReport {
        schema_version: 1,
        design_id: status.id,
        name: status.name,
        envelope_digest: status.envelope_digest,
        evaluations: status.budget.evaluations,
        feasible: status.feasible.len() as u32,
        statement: format!(
            "{}Design state {}: winner is the best feasible evaluated candidate under the approved envelope. Process success is not physical validation; simulated constraints carry only their child jobs' numerical scope, and unknown constraints never pass.",
            if status.synthetic_inputs.is_empty() {
                String::new()
            } else {
                format!(
                    "SYNTHETIC: declared inputs {} are synthetic demonstration values, so every result derived from them is synthetic and is not design evidence. ",
                    status.synthetic_inputs.join(", ")
                )
            },
            status.state
        ),
        state: status.state,
        winner,
        physical_validation: "unqualified".into(),
        synthetic_inputs: status.synthetic_inputs,
    })
}
