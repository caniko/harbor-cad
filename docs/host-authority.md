# Host authority and shared admission

`HostAuthority` is an explicit version-1 authorization contract separate from
the existing version-1 case, plan and host-profile records. It declares:

- The exact Fleetix source revision and contract digest.
- Aggregate same-user RAM capacity and reserved headroom.
- One canonical scoped root, application byte budget and free-space headroom
  per filesystem. Multiple roots on the same filesystem cannot create separate
  budgets.
- Exact allowed role/backend/PCI/UUID identities, authoritative routes and
  individually recorded overrides. An absent route stays disabled.
- One VRAM budget and headroom allowance per physical card, shared across roles.
- Allowed input roots and immutable native-runtime manifests.

All values are operator-supplied policy, never inferred from an imported document
or a plan's device selection. Export the generated `HostAuthority` JSON Schema
with `harbor-cad schema` when preparing the configuration. Route records must
come from the resolved Fleetix host configuration; an alternative belongs in
an explicit override with its reason.

```sh
harbor-cad authority install /absolute/authority.json
harbor-cad worker --state /absolute/private/state \
  --profile /absolute/profile.json --authority /absolute/authority.json
```

Workers using the authority require systemd execution. The same-user admission
journal is anchored under the **OS account's canonical home**, at
`.local/state/harbor-cad/admission/admission.sqlite3`; `HOME` and XDG environment
variables cannot select another pool. Its owned private database and sidecars
reject symlinks and hardlinked/foreign inodes. The journal persists independently
of workers and the user runtime directory. Conflicting worker authorities reject.

Explicit policy installation can promote a new configuration only after every
reservation has been released through verified closure. Registered roots must
remain within its filesystem scopes and retained bytes must fit its budgets.
Already-bound jobs are never rewritten or upgraded. A worker or queued job using
the superseded authority receives a compatibility rejection; submit a new job
under the deliberately selected policy.

## Immutable execution authorization

Each authorized submission atomically commits an `ExecutionAuthorization` with
the job/profile/execution binding. It includes the full authority and binds the
exact plan, profile and execution-binding digests. Idempotent retries retain the
first authorization. Launch and the executing runner verify it independently.
Successful execution archives `execution-authorization.json`; terminal exports
include it in `execution.json` even when execution failed before artifact creation.
Historical jobs export `execution_authorization: null`. Their original approved
JSON/digests remain unchanged; an authority-enabled worker cannot silently
upgrade their execution policy.

## Admission and lifecycle

One SQLite immediate transaction checks and records aggregate reservations before
service launch. A durable reservation includes the canonical state root, job ID,
authorization digest, RAM, filesystem staging allowance and shared card identities.
An occupied pool leaves the job queued; no service runtime deadline is spent
waiting for capacity. The runner requires the pre-launch reservation before its
running transition.

Filesystem accounting includes every registered state's retained artifacts,
quarantined/incomplete bytes, database files and the complete native-copy staging
allowance. Nested mounts, missing registered roots, arithmetic overflow and
ambiguous identities reject conservatively. Live `MemAvailable` and filesystem
available bytes are rechecked with explicit headroom. Active bytes plus complete
reservations are deliberately conservative rather than discounted using an
unverified progress estimate. External filesystem usage is reflected in live
free space; this is not a universal filesystem quota.

Heavy GPU admission is exclusive by physical PCI card. It checks existing card
anchors before recording ownership; production probe helpers also honor durable
reservations. Authorized runners acquire their anchors without a runtime wait.
AMD `mem_info_vram_total`/`mem_info_vram_used` supply a conservative pre-launch
headroom check; unavailable or inconsistent telemetry does not qualify a backend.
Reservations are application admission, not hard VRAM quotas or measured peaks.

Worker death does not release a reservation for an independent surviving service.
Terminal status alone does not release it either: cleanup checks the recorded
unit/invocation and complete cgroup tree. Missing/ambiguous ownership retains
capacity. A queued cancellation can release after proving no owned service lives.
Services explicitly receive `MemorySwapMax=0` in addition to RAM/CPU/task limits.

### Pruning deleted state roots

A registered state root that no longer exists blocks every admission and policy
installation, because its retained bytes and ownership cannot be accounted.
Deleting a state directory does not unregister it; the operator does so
explicitly:

```sh
harbor-cad admission prune --dry-run   # report only, read-only snapshot
harbor-cad admission prune
```

Pruning runs in the same SQLite immediate transaction (writer lease) as every
admission mutation, so it waits for, and never interleaves with, a concurrent
reservation. Existing roots are always kept. A missing root is refused while any
durable reservation references it, or while any live same-user process still holds
an open file or working directory beneath it (such as an unlinked
`worker.lock`); a lock holder whose descriptors cannot be inspected refuses every
missing root. Release such a reservation through verified closure first. The JSON
report lists `pruned`, `kept` and `refused` roots with reasons. Missing roots
still fail admission closed until pruned. There is deliberately no MCP tool.

Legacy workers without `--authority` retain the previously documented per-root
CPU-reference behavior. They do not qualify aggregate capacity or GPU execution.
HIP submissions require this authority and the independently correlated exact
PCI/UUID. Its single-KFD-GPU worker policy rejects another exposed compute GPU;
durable GPU qualification and multi-GPU exclusion remain separate evidence gates.

### KFD multi-GPU exclusion (`harbor-cad-native-hip-kfd-exclusion-v1`)

Once a second GPU is bound to amdgpu it appears as a second live KFD node and
the single-KFD-GPU policy refuses all HIP compute. An optional
`kfd_exclusion` receipt on `HostAuthority` lifts that refusal for one host only,
travelling the same `authority install` trust path as every other device fact
(no ad-hoc file beside a job). `HipIdentity::from_topology` admits the selected
card beside a foreign one **only** when the installed receipt matches the exact
running kernel release and `/proc/version` digest, the selected compute PCI, and
the foreign-PCI set exactly; any drift — a different kernel, a changed PCI set,
an extra third GPU, or a missing receipt — restores the `Unqualified` refusal.
`HIP_VISIBLE_DEVICES` stays non-enforcing and there is no env-var bypass. The
admitted identity records `applied_policy =
harbor-cad-native-hip-kfd-exclusion-v1`; a single-GPU host records the
single-KFD policy.

The receipt's evidence is produced by `scripts/verify_kfd_exclusion.py`, which
inside the exact HIP sandbox attempts `ACQUIRE_VM`, `ALLOC_MEMORY_OF_GPU` and
`CREATE_QUEUE` against the foreign `gpu_id` (expecting `EINVAL` each time) with
the selected card as positive control, bound to the matched running-kernel
amdkfd source. It is on security hold until the opt-in amdgpu boot entry lands.

## Qualification

```sh
python3 scripts/verify_shared_admission.py \
  --executable /nix/store/CLI/bin/harbor-cad \
  --output /absolute/new/qualification-directory
```

This opt-in verifier installs a deliberately scoped application authority, starts
two packaged workers and uses labelled synthetic CPU jobs. It verifies queued
cross-root RAM/disk contention before service creation, reservation survival after
worker death, cross-root progress after complete service closure, checksummed
authorization exports, and the existing restart/forced-death/cancellation
qualifier with the same authority. It signals only its recorded job units.
Numerical GPU, KFD, multi-GPU, per-process VRAM peaks and logout/reboot behavior
require independent qualification.
