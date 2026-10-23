"""Official SDK v2.3 MCPServer API, verified against tag source and a real client.

Source: https://github.com/modelcontextprotocol/python-sdk/tree/2118f14f8a19bc158d8a1cf90af58d85d187f849
"""

import argparse
from typing import Any

from mcp.server import MCPServer
from mcp.server.mcpserver.exceptions import ToolError

from .client import WorkerError
from .client import request as worker_request


async def request(operation: str, **payload: object) -> dict:
    try:
        return await worker_request(operation, **payload)
    except WorkerError as error:
        # The pinned SDK deliberately hides unexpected exceptions. Rust's
        # bounded typed validation/admission failures are expected tool errors.
        raise ToolError(str(error)) from error


def build_server(profile: str) -> MCPServer:
    if profile not in {"cad", "simulation", "results", "all"}:
        raise ValueError("unknown MCP profile")
    server = MCPServer(f"harbor-cad-{profile}")

    @server.tool()
    async def doctor() -> dict[str, Any]:
        """Inspect read-only inventory, compiled dependency locks/source pins and independent qualification states; no Nix evaluation or native imports."""
        return await request("doctor")

    @server.tool()
    async def backend_list() -> list[dict[str, Any]]:
        """List model/backend qualification; no native imports or GPU launch."""
        return await request("backend_list")

    @server.tool()
    async def execution_preflight(plan: dict[str, Any]) -> dict[str, Any]:
        """Inspect exact packages, registered-source checks, policy and declared RAM/disk using submission's pre-staging checks. No input retention, approval, resource reservation, live capacity guarantee, native execution or qualification."""
        return await request("execution_preflight", plan=plan)

    if profile in {"cad", "simulation", "all"}:

        @server.tool()
        async def case_validate(case: dict[str, Any]) -> dict[str, Any]:
            """Validate explicit units, named regions and applicability."""
            return await request("validate", case=case)

    if profile in {"simulation", "results", "all"}:

        @server.tool()
        async def retained_cooling_plan(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Plan independently approved native CPU cooling of exact stationary registered wetting originals; explicit thermal state and immutable job_submit approval required."""
            return await request("plan_retained_cooling", request=request_spec)

    if profile in {"simulation", "all"}:

        @server.tool()
        async def cad_plan_spectral_transport(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Independently approve direct-only original-facet CAD irradiance/power/dose; missing ageing remains unknown and no source is opened by the planner."""
            return await request("plan_cad_spectral_transport", request=request_spec)

        @server.tool()
        async def study_prepare(study: dict[str, Any]) -> dict[str, Any]:
            """Check up to 16 explicit CPU case approvals and total output allowance; no solve."""
            return await request("prepare_study", request=study)

        @server.tool()
        async def study_submit(
            study: dict[str, Any], idempotency_key: str
        ) -> dict[str, Any]:
            """Persist an immutable study intent and submit each case through ordinary worker jobs."""
            return await request(
                "submit_study", request=study, idempotency_key=idempotency_key
            )

        @server.tool()
        async def study_status(study_id: str) -> dict[str, Any]:
            """Read original case identities and current job states; completion is not numerical qualification."""
            return await request("study_status", study_id=study_id)

    if profile in {"cad", "simulation", "all"}:

        @server.tool()
        async def case_plan(case: dict[str, Any]) -> dict[str, Any]:
            """Prepare a CPU analytical-reference plan; never solve implicitly."""
            return await request("plan", case=case)

        @server.tool()
        async def case_plan_openlb_reference(case: dict[str, Any]) -> dict[str, Any]:
            """Plan explicit native CPU OpenLB with FreeCAD geometry; systemd policy required."""
            return await request("plan_openlb_reference", case=case)

        @server.tool()
        async def case_plan_b1(
            case: dict[str, Any], selections: dict[str, Any]
        ) -> dict[str, Any]:
            """Plan required HIP or CUDA flow, independent EGL/VAAPI devices, and offline bundle.

            Device selectors are explicit requests; planning does not qualify hardware.
            """
            return await request("plan_b1", case=case, selections=selections)

    if profile in {"cad", "all"}:

        @server.tool()
        async def cad_plan_mesh(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Plan a CPU mesh from one registered authorized CAD solid; preserve exact BREP/world placement."""
            return await request("plan_cad_mesh", request=request_spec)

        @server.tool()
        async def cad_mesh_submit(
            plan: dict[str, Any], approved_digest: str, idempotency_key: str
        ) -> dict[str, Any]:
            """Submit only an approved source-bound mesh; return its durable job ID."""
            if plan.get("schema_version") != 7 or [
                s.get("operation") for s in plan.get("stages", [])
            ] != ["cad_mesh", "bundle"]:
                raise ValueError("exact approved imported CAD mesh DAG required")
            return await request(
                "submit",
                plan=plan,
                approved_digest=approved_digest,
                idempotency_key=idempotency_key,
            )

        @server.tool()
        async def cad_plan_inspection(
            case: dict[str, Any], max_artifact_bytes: int = 67108864
        ) -> dict[str, Any]:
            """Plan digest-approved source inspection in FreeCAD's sandbox; no solver.

            Source paths are relative to the worker's allowed root. Approval and
            ordinary job submission are required before any document is opened.
            """
            return await request(
                "plan_cad_inspection", case=case, max_artifact_bytes=max_artifact_bytes
            )

        @server.tool()
        async def cad_submit(
            plan: dict[str, Any], approved_digest: str, idempotency_key: str
        ) -> dict[str, Any]:
            """Submit only an approved CAD inspection or controlled-copy variant; return its durable job ID."""
            operations = [stage.get("operation") for stage in plan.get("stages", [])]
            if plan.get("schema_version") not in {1, 16} or operations != [
                "cad_inspect",
                "bundle",
            ]:
                raise ValueError("exact approved CAD inspection DAG required")
            return await request(
                "submit",
                plan=plan,
                approved_digest=approved_digest,
                idempotency_key=idempotency_key,
            )

        @server.tool()
        async def cad_plan_variant(variant: dict[str, Any]) -> dict[str, Any]:
            """Approve one registered native box copy with explicit dimensions and preserved placement; no source document is opened while planning."""
            return await request("plan_cad_variant", request=variant)

    if profile in {"cad", "results", "all"}:

        @server.tool()
        async def cad_prepare_spectral_scene(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Bind explicit optical properties to all original named CAD triangles; retain missing inputs and never solve implicitly."""
            return await request("prepare_cad_spectral_scene", request=request_spec)

        @server.tool()
        async def cad_regions(job_id: str) -> dict[str, Any]:
            """Read bounded checksummed named-solid metadata from one native CAD job."""
            return await request("cad_regions", job_id=job_id)

    if profile in {"simulation", "all"}:

        @server.tool()
        async def snow_openings_validate(spec: dict[str, Any]) -> dict[str, Any]:
            """Measure explicit prescribed snow/planar-aperture geometry; no deposition, flow, permeability or convection inference."""
            return await request("validate_snow_openings", spec=spec)

        @server.tool()
        async def freezing_reference_validate(spec: dict[str, Any]) -> dict[str, Any]:
            """Validate explicit synthetic fixed-volume Stefan inputs and SI conversion; no native solve or retained-water transfer."""
            return await request("validate_freezing_reference", spec=spec)

        @server.tool()
        async def case_plan_freezing_reference(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan immutable CPU conduction solidification with bounded originals; no implicit retained-water transfer."""
            return await request("plan_freezing_reference", spec=spec)

        @server.tool()
        async def case_plan_fem_reference(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan an explicit static synthetic CPU FEM job; no native imports or solve."""
            return await request("plan_fem_reference", spec=spec)

        @server.tool()
        async def fem_plan_imported(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Plan a synthetic CPU static FEM reference from registered authorized world-space CAD."""
            return await request("plan_fem_imported", request=request_spec)

        @server.tool()
        async def case_plan_thermal_reference(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan explicit synthetic transient CPU heat transfer; prescribed convection and physical histories."""
            return await request("plan_thermal_reference", spec=spec)

        @server.tool()
        async def snow_reference_validate(spec: dict[str, Any]) -> dict[str, Any]:
            """Check prescribed dry snow resistance and explicit storage/melting applicability; execution remains false."""
            return await request("validate_snow_reference", spec=spec)

        @server.tool()
        async def spectral_reference_validate(spec: dict[str, Any]) -> dict[str, Any]:
            """Prepare explicit angular UV source, optical absorption, ageing weighting and prescribed dose; no native transport execution."""
            return await request("validate_spectral_reference", spec=spec)

        @server.tool()
        async def spectral_reference_plan(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan immutable native directional UV observations, optical weights and prescribed dose; approve and submit through the same worker."""
            return await request("plan_spectral_reference", spec=spec)

        @server.tool()
        async def atmospheric_reference_validate(
            spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Prepare pinned molecular UV atmosphere and full ordered angular observations; no isotropic substitution or execution qualification."""
            return await request("validate_atmospheric_reference", spec=spec)

        @server.tool()
        async def atmospheric_reference_plan(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan immutable bounded native molecular UV fields; approve and submit through the same durable worker."""
            return await request("plan_atmospheric_reference", spec=spec)

        @server.tool()
        async def spectral_reflection_reference_validate(
            spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Prepare bounded synthetic Lambertian UV reflection and distinct footprint/shadow model error; no native execution."""
            return await request("validate_spectral_reflection_reference", spec=spec)

        @server.tool()
        async def case_plan_snow_reference(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan native plane-wall heat transfer with approval-bound full-face prescribed snow; no deposition or opening-flow inference."""
            return await request("plan_snow_reference", spec=spec)

        @server.tool()
        async def case_plan_wetting_reference(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan synthetic equal-property planar CPU wetting with original phase/velocity observations; explicit approval required."""
            return await request("plan_wetting_reference", spec=spec)

        @server.tool()
        async def case_plan_contact_reference(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan synthetic planar CPU preload/thermal opening with explicit SI inputs and two static states; requires approval."""
            return await request("plan_contact_reference", spec=spec)

        @server.tool()
        async def case_plan_thermal_contact(spec: dict[str, Any]) -> dict[str, Any]:
            """Plan one-way native thermal, conservative projection and contact; submission requires explicit approval."""
            return await request("plan_thermal_contact", spec=spec)

        @server.tool()
        async def filter_plan(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Plan required HIP point-gradient filtering from one registered retained time.

            A numerical compute stage with independent approval; no solver or rendering.
            """
            return await request("plan_filter", request=request_spec)

        @server.tool()
        async def cold_restart_validate(case: dict[str, Any]) -> dict[str, Any]:
            """Validate cold-restart histories and material ranges, preserving missing inputs.

            Returns prescribed heater energy and bounded moisture screening only;
            does not launch a transient solver or infer boot reliability.
            """
            return await request("validate_cold_restart", case=case)

        @server.tool()
        async def job_submit(
            plan: dict[str, Any], approved_digest: str, idempotency_key: str
        ) -> dict[str, Any]:
            """Submit an immutable approved plan and immediately return a durable job ID."""
            return await request(
                "submit",
                plan=plan,
                approved_digest=approved_digest,
                idempotency_key=idempotency_key,
            )

        @server.tool()
        async def job_cancel(job_id: str) -> dict[str, Any]:
            """Cancel only a tracked owned service tree, with bounded escalation."""
            return await request("cancel", job_id=job_id)

    if profile in {"cad", "simulation", "results", "all"}:

        @server.tool()
        async def job_status(job_id: str) -> dict[str, Any]:
            """Read durable state; process success does not imply physical validation."""
            return await request("status", job_id=job_id)

        @server.tool()
        async def job_logs(
            job_id: str, after: int = 0, limit: int = 20
        ) -> list[dict[str, Any]]:
            """Read at most 100 bounded structured events using a durable cursor."""
            return await request("logs", job_id=job_id, after=after, limit=limit)

    if profile in {"results", "all"}:

        @server.tool()
        async def results_cad_optical(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Read independently verified original-facet optical power/dose for one exact region/seed; missing ageing stays absent and no solve or seed averaging is performed."""
            return await request("cad_optical_results", request=request_spec)

        @server.tool()
        async def render_plan(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Plan retained-field presentation with exact source binding and independent devices."""
            return await request("plan_presentation", request=request_spec)

        @server.tool()
        async def video_plan(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Plan independent hardware encoding from registered rendered frames."""
            return await request("plan_video", request=request_spec)

        @server.tool()
        async def presentation_submit(
            plan: dict[str, Any], approved_digest: str, idempotency_key: str
        ) -> dict[str, Any]:
            """Submit only an approved source-bound presentation; return a durable job ID."""
            if plan.get("schema_version") not in {2, 3} or not isinstance(
                plan.get("source"), dict
            ):
                raise ValueError("source-bound presentation plan required")
            return await request(
                "submit",
                plan=plan,
                approved_digest=approved_digest,
                idempotency_key=idempotency_key,
            )

        @server.tool()
        async def results_describe(job_id: str) -> dict[str, Any]:
            """Return hashes/provenance and artifact descriptors, never scientific arrays."""
            return await request("describe", job_id=job_id)

        @server.tool()
        async def results_sample(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Read at most 64 exact registered static FEM native locations with units and source identities."""
            return await request("results_sample", request=request_spec)

        @server.tool()
        async def results_compare(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Compare registered static FEM fields on the same exact mesh; no interpolation or acceptance promotion."""
            return await request("results_compare", request=request_spec)

        @server.tool()
        async def results_sample_thermal(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Sample up to 64 native thermal nodes at one exact approved retained time."""
            return await request("results_sample_thermal", request=request_spec)

        @server.tool()
        async def results_compare_thermal(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Compare same-mesh thermal nodes at explicit retained times; right minus left."""
            return await request("results_compare_thermal", request=request_spec)

        @server.tool()
        async def results_sample_freezing(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Read exact registered Float64 freezing values at retained time and up to 64 original grid points."""
            return await request("results_sample_freezing", request=request_spec)

        @server.tool()
        async def results_retain_wetting(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Retain complete source-bound phase/velocity by conservative explicit nodal extrusion; report missing thermal state and phase overshoots."""
            return await request("results_retain_wetting", request=request_spec)

        @server.tool()
        async def results_prepare_retained_cooling(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Prepare conserved stationary original-phase enthalpy with explicit synthetic thermal inputs; refuse moving or overshooting sources and do not solve."""
            return await request(
                "results_prepare_retained_cooling", request=request_spec
            )

        @server.tool()
        async def results_sample_retained_cooling(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Read original cooling subcontrols at one exact retained time/region with full source history and provenance; no interpolation or physical lifetime inference."""
            return await request(
                "results_sample_retained_cooling", request=request_spec
            )

        @server.tool()
        async def results_transfer_atmosphere(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Prepare registered native anisotropic original midpoint transfer and distinct optical dose; no transport execution or isotropic replacement."""
            return await request("results_transfer_atmosphere", request=request_spec)

        @server.tool()
        async def atmospheric_transport_plan(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Plan immutable CPU spectral transport using succeeded registered atmospheric originals and explicit receiver; requires approval before job_submit."""
            return await request("plan_atmospheric_transport", request=request_spec)

        @server.tool()
        async def results_compare_freezing(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Compare same-grid registered freezing points in SI units; no interpolation or engineering acceptance."""
            return await request("results_compare_freezing", request=request_spec)

        @server.tool()
        async def results_moisture(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Screen a complete native box surface; explicit air inputs, missing or inapplicable."""
            return await request("results_moisture", request=request_spec)

        @server.tool()
        async def results_transfer_temperature(
            request_spec: dict[str, Any],
        ) -> dict[str, Any]:
            """Project the complete verified native thermal box to a congruent uniform block; explicit Kelvin loss and conservation bounds."""
            return await request("results_transfer_temperature", request=request_spec)

        @server.tool()
        async def qualification_report(job_id: str) -> dict[str, Any]:
            """Inspect historical job evidence bound to its exact source/runtime/device; never promote qualification."""
            return await request("qualification_report", job_id=job_id)

        @server.tool()
        async def artifact_list(
            job_id: str, after: str | None = None, limit: int = 20
        ) -> dict[str, Any]:
            """Read a byte-bounded page; pass next_after until null to retain every descriptor.

            For a stable full traversal, wait for a terminal job state. Export always copies all records.
            """
            if not 1 <= limit <= 100:
                raise ValueError("artifact page limit 1..100 required")
            return await request("artifacts", job_id=job_id, after=after, limit=limit)

        @server.tool()
        async def results_correlate(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Score a bounded measured CSV (relative to the worker input root) against one registered thermal history: per-probe bias/MAE/RMSE/max error and tolerance pass, sensor accuracy reported separately. Linear interpolation inside the overlap only; correlation is not validation of untested conditions."""
            return await request("results_correlate", request=request_spec)

    if profile in {"simulation", "all"}:

        @server.tool()
        async def materials_list() -> dict[str, Any]:
            """List sourced material/process ids and process rate bases; typical published ranges, not qualified properties."""
            return await request("materials_list")

        @server.tool()
        async def cost_estimate(request_spec: dict[str, Any]) -> dict[str, Any]:
            """Estimate per-unit USD low/high with breakdown and manufacturability screen; missing inputs stay missing. Not a supplier quote."""
            return await request("cost_estimate", request=request_spec)

    if profile in {"simulation", "all"}:

        @server.tool()
        async def design_plan(envelope: dict[str, Any]) -> dict[str, Any]:
            """Validate a bounded design envelope and strict template substitution under the worker policy; return its approval digest. No job is created."""
            return await request("design_plan", envelope=envelope)

        @server.tool()
        async def design_submit(
            envelope: dict[str, Any], approved_digest: str, idempotency_key: str
        ) -> dict[str, Any]:
            """Persist an approved design intent; candidates run as ordinary worker jobs and the worker advances the bounded loop."""
            return await request(
                "design_submit",
                envelope=envelope,
                approved_digest=approved_digest,
                idempotency_key=idempotency_key,
            )

        @server.tool()
        async def design_propose(
            design_id: str, candidate: dict[str, Any]
        ) -> dict[str, Any]:
            """Evaluate one on-grid in-bounds candidate exactly like a proposal; it consumes the approved evaluation budget."""
            return await request(
                "design_propose", design_id=design_id, candidate=candidate
            )

        @server.tool()
        async def design_status(design_id: str) -> dict[str, Any]:
            """Read iterations, metrics, feasibility, incumbent, Pareto set and budget use; completion is not physical validation."""
            return await request("design_status", design_id=design_id)

        @server.tool()
        async def design_export(design_id: str) -> dict[str, Any]:
            """Report the winner, its child job IDs and simulated/unknown/calibrated constraint evidence; never promotes qualification."""
            return await request("design_export", design_id=design_id)

    return server


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--profile", choices=["cad", "simulation", "results", "all"], default="all"
    )
    args = parser.parse_args()
    build_server(args.profile).run(transport="stdio")


if __name__ == "__main__":
    main()
