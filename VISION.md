# harbor-cad vision

harbor-cad is a local-first Rust CLI and persistent worker, with a narrow Python
MCP layer, for scientific CAD jobs. It takes FreeCAD designs through
environmental simulation, GPU analysis and rendering, hardware video encoding,
and portable scientific export. Its pilot product is an outdoor electronics
enclosure built in small batches, and its authoritative scope is
`docs/implementation-spec.md`.

## Problem

Scientific CAD work chains many specialised tools — FreeCAD, OpenLB, CalculiX,
libRadtran, Mitsuba, ParaView, FFmpeg — each with its own packaging, ABI, and
device assumptions. The result is easy to produce and hard to reproduce, easy to
call "validated" when only the process succeeded, and easy to overstate for
hardware that was never qualified.

## Principles

- Process success is never a physical-validation claim.
- Qualification is scoped and recorded. The acceptance ledger carries evidence
  for exact packages, formulations, and host scope; discovery does not qualify a
  changed runtime automatically.
- Preserve source CAD and existing repository work rather than regenerating over
  it.
- Keep the language seam narrow. Rust owns contracts, validation, scheduling,
  resources, lifecycle, and artifacts; Python owns the FreeCAD/ParaView/Mitsuba
  bridges and MCP; native solvers own the numerical algorithms.
- Pin immutable inputs: committed `flake.lock`, `Cargo.lock`, and `uv.lock`,
  recorded native hashes, compilers, and ABIs. No build-time hardware discovery,
  undeclared downloads, unqualified fast-math, or hidden `-march=native`.
- Isolate runtimes. FreeCAD, ParaView, Mitsuba, and the MCP use individually
  compatible interpreters, and exact packaged executables launch with controlled
  environments instead of global Python, loader, or Qt injection.
- Submit work with an approval digest and an idempotency key, over a Unix socket
  under an absolute private state directory.
- Stay local-first: no mandatory physics framework beyond the chosen stack, no
  cloud service, network database, or web dashboard.
- Decisions belong to the consuming project. The reusable CAD robustness skill
  guides requirement interviews, source-safe derivatives, bounded searches, and
  qualification without absorbing product decisions.

## Scope

The `harbor-cad` CLI (`case init`, `case plan`, job submission and status,
`results describe`, `artifact export`), the persistent worker and its profiles,
the MCP tool surface, native adapters for mesh, FEM, thermal, wetting, spectral,
filters, rendering, and encoding, a CPU analytical-reference workflow, and the
documentation set covering references, qualification, results, and design
conditions.

## Non-goals

No second mandatory physics framework, cloud service, network database, or web
dashboard. The repository does not qualify hardware it did not run on, does not
replace the consuming project's product decisions, and treats GUI or
presentation work as optional rather than required for baseline delivery.

## Direction

The source-level workstreams are closed out; what remains is qualification.
`tasks/todo.md` lists moving nixpkgs to the newest fully cached revision, then
building and GC-rooting each declared output (`default`, `mcp`, Python test
environment, CPU and HIP runtimes, native wetting and solver runtimes, CAD and
FEM runtimes, GUI) and recording the reports. Native render and media DRM
policies still await a real run.

## Place in the fleet

Harbor-CAD is a gitlinked component (`components/cad`) of the `harbor`
build-infrastructure superproject that canix pins, and is managed as a project
by canix's paperclip graphics organisation. Canix does not pin this repository
directly.
