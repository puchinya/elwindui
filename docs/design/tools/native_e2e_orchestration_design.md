# Native E2E orchestration design

This document is the durable internal architecture for native end-to-end (E2E) orchestration.
It defines the shared behavior that a future runner must provide while leaving platform-specific
automation to the repository-owned AppKit and WinUI3 drivers. It does not implement a runner,
change a driver command catalog, or redefine a public ElwindUI contract.

The current operational authorities remain [`appkit-e2e.md`](../../agents/appkit-e2e.md) and
[`winui3-e2e.md`](../../agents/winui3-e2e.md) until the shared runner exists. Durable product
case definitions belong under [`tests/e2e/`](../../../tests/e2e/README.md); platform drivers and
their self-tests remain under `tools/macos-ui-driver/` and `tools/windows-ui-driver/`.

## 1. Scope and authority

The shared architecture applies to backend-neutral product cases. A case describes product
behavior, setup, actions, observable postconditions, visual checkpoints when required, and
cleanup. The shared runner is the only layer that consumes and interprets the backend-neutral
compiled plan during execution. It selects the backend and translates each plan operation into
calls to the selected platform driver's primitive command surface. AppKit and WinUI3 drivers do
not consume or interpret the shared plan itself; they execute platform automation primitives and
normalize adapter-specific mechanics, results, and errors. They do not parse durable-case or
compiled-plan schemas, decide shared case sequencing, or own the shared product acceptance meaning.

This is an internal tooling design. It does not require a `docs/specs/` change, a Rust API, a
driver refactor, or a provider launcher/configuration change. No future capability described here
is available merely because it is documented.

## 2. Script-first execution model

The target architecture is:

```text
durable case definition
        |
        v
deterministic case compiler
        |
        v
reusable compiled plan/script
        |
        v
shared E2E runner
        |
        +--> AppKit driver
        |
        +--> WinUI3 driver
        |
        +--> optional NEEDS_VISION checkpoint
        |
        `--> result.json + immutable run evidence
```

The compiler produces a deterministic plan from a durable case and its declared dependencies.
The runner prepares that plan, executes a normal deterministic case as a bounded batch, and
collects structured output from the selected platform driver. It should not return control to an
AI tester between every primitive. The AI tester is a bounded executor and visual observer, not an
interactive test designer that reconstructs the plan after each command.

The runner may eventually expose a concrete plan or batch interface. Until implementation exists,
driver READMEs and operational guides must not present fictitious `batch`, `run-plan`, or
`capture-sequence` commands as usable commands.

## 3. Durable cases and deterministic compilation

The backend-neutral case under `tests/e2e/` is the source of truth. A future case/compiler schema
must be versioned and must make plan dependencies explicit. The compiled plan contains structure:
ordered operations, declared waits, parameter sources, bounded budgets, and evidence requirements.
It does not contain process-specific or screen-specific runtime values.

The conceptual plan fingerprint is derived from the inputs that materially affect plan structure:

1. normalized durable case definition;
2. case and runner schema version;
3. target backend;
4. declared plan dependencies;
5. platform driver command-surface/version fingerprint; and
6. plan compiler version.

The fingerprint must not be a hash of the entire repository. A repository HEAD change alone is
not a plan invalidation key when the declared plan dependencies are unchanged.

## 4. Local compiled-plan cache

Compiled plans are reusable generated artifacts. The conceptual local cache is:

```text
.agent-state/e2e-cache/<plan-fingerprint>/
```

The cache is local/generated and remains Git-ignored. When the dependency fingerprint is
unchanged, the runner reuses the compiled plan instead of regenerating it. A cache hit is an
optimization of plan preparation, not evidence that the case is accepted.

The new run evidence records the plan fingerprint or hash, including on a cache hit. If a declared
case, runner, driver command surface, compiler, target backend, or other declared plan dependency
changes, the fingerprint changes and the plan is regenerated.

Generated plans are not checked into Git in this first architecture. That alternative may be
reconsidered only after concrete evidence establishes a repository-managed artifact requirement.

## 5. Plan reuse versus runtime and result reuse

The runner separates three lifetimes:

| Artifact | Reuse rule |
|---|---|
| Compiled plan/script | Reusable while its dependency fingerprint is unchanged. |
| Runtime values | Resolved afresh for every run. |
| PASS/FAIL evidence | Fresh, immutable evidence bound to the tested HEAD and runtime environment. |

Every run reacquires values that can change without changing plan structure, including:

- process ID and `CGWindowID`/HWND;
- current window bounds;
- DPI and monitor state;
- runtime AX/UIA element identity;
- absolute screen coordinates;
- temporary and evidence paths; and
- timestamps.

A product change requires acceptance to be rerun even when the plan remains structurally valid.
Prior PASS/FAIL evidence is never reused because a plan cache hit occurred. Each run records the
tested committed HEAD, relevant environment, the plan fingerprint, driver identity, and fresh
structured/image evidence in a new immutable run directory.

## 6. Runner, driver, and tester responsibilities

The future shared runner owns case loading, deterministic compilation, cache lookup, parameter
resolution, bounded execution, classification, and immutable evidence assembly. The AppKit and
WinUI3 drivers own their platform primitives and adapter-specific error details. Driver
`success: true` proves only that the requested driver operation executed; it does not prove a
product postcondition. Drivers never infer product acceptance semantics from a shared plan, and
their primitive behavior remains independently contract-testable without loading a durable case or
shared-plan parser.

If a future driver supports a multi-primitive request, the shared runner first translates the
compiled plan into a platform-specific primitive batch and the driver executes that ordered batch.
The driver still does not parse or interpret the backend-neutral case or compiled-plan schema. A
driver-owned `run-plan <shared-plan>` shape is forbidden when it would make the driver a second
shared-plan interpreter. Caching, vision checkpoints, result classification, and animation
sequencing remain shared-runner concerns; only platform-specific primitive batching mechanics may
belong to a driver.

For a shared operation such as semantic invoke, the runner reads the backend-neutral operation,
selects AppKit or WinUI3, and maps it to the corresponding AX or UIA primitive before invoking the
driver. The driver executes that primitive; it does not reinterpret the operation or decide what
product postcondition makes the case pass.

The tester receives a fixed, case-scoped instruction sheet or prepared plan. The tester may make a
bounded visual determination at an explicit checkpoint, but must not silently change the case,
invent an alternative command, or redesign the sequence interactively. Current provider-specific
tester procedures remain in the platform guides while the shared runner is unimplemented.

## 7. Parameter resolution and vision checkpoints

Parameter resolution follows this fixed order:

```text
case constant
-> previous structured runner/driver output
-> AX/UIA query
-> OS/window metadata
-> explicit visual parameter checkpoint
```

Vision must not rediscover a PID, window ID, geometry, AX/UIA state, value, or other mechanically
available fact. A stable vision-derived parameter may be reused only when it is explicitly declared
as a compile-time visual parameter and covered by the same plan fingerprint. Runtime visual
assertions are never cached as test results.

Before an AI vision checkpoint, the runner obtains geometry mechanically, crops to the smallest
useful region of interest (ROI), and performs safe mechanical image checks where applicable.
Vision is used only for a parameter with no reliable mechanical source or for an inherently visual
acceptance criterion. Full-window screenshots are not the default for a local visual assertion,
and mechanical comparison must account for font, renderer, and OS variability rather than require
unconditional pixel-perfect golden images.

An explicit visual checkpoint must identify the ROI, the question to answer, the evidence path,
and the allowed bounded outcome. Ambiguous visual evidence is not a product failure.

## 8. Conditions, foreground, and platform boundaries

Correctness waits are bounded and condition-driven. Prefer existing AX/UIA `wait-for`-style
semantics; polling is acceptable when event observation is unavailable, but it ends as soon as the
condition is satisfied. Fixed sleeps are not correctness mechanisms, and global timeout inflation
is not a flakiness strategy.

Foreground is required only for an operation whose delivery depends on real frontmost input or
whose subject is foreground behavior. For AppKit, the normal non-foreground-gated operations are
`find`, `dump-tree`, `set-focus`, `wait-for`, `click --via ax-press`, and `capture-window`.
Foreground and input routing remain prerequisites for `click --via mouse`, `point-click`, `drag`,
`resize`, and synthesized keyboard input when delivery depends on the frontmost application.
`capture-window` is screenshot capture and must not be documented as requiring `focus-window`.

WinUI3 UIA pattern operations similarly do not require foreground. Real input remains distinct and
uses the Windows driver's input-routing behavior; `focus-window` is diagnostic or case-specific,
not a universal gate. These platform-specific rules are operationally authoritative in the
platform guides until a shared runner is implemented.

## 9. Result classification and cleanup

The runner uses `PASS`, `FAIL`, `NOT RUN`, and `BLOCKED` with strict boundaries:

- `FAIL` requires valid target identity, delivery of the intended action to the product, completion
  of the required bounded postcondition wait, and a definitive wrong final product state. For
  visual acceptance, the required evidence must be acquired and the visual determination must be
  definite.
- `BLOCKED` means a host, tool, session, security, permission, or foreground condition prevented
  the product from being exercised. It is not a product failure.
- `NOT RUN` means the required action or evidence was never executed or collected.
- `PASS` requires the required product postcondition or evidence, not just a successful driver
  envelope.

Ambiguous vision is never product `FAIL`. A missing host prerequisite, unavailable GUI session,
or unusable driver is `BLOCKED`; an omitted action or missing required evidence is `NOT RUN`.
After abnormal results, cleanup is still required, including bounded termination of any launched
application and preservation of the exact stdout/stderr needed to explain the classification.

## 10. Animation capture lifecycle

Animation acceptance uses one bounded capture sequence:

1. resolve the target and ROI mechanically;
2. arm the bounded capture sequence;
3. confirm that capture is armed;
4. trigger the animation;
5. record the action's monotonic timestamp;
6. capture a small bounded set of timestamped frames;
7. run mechanical checks where sufficient;
8. use at most one visual-analysis checkpoint when visual interpretation is required; and
9. verify the deterministic final logical state separately when it is mechanically observable.

The runner must not implement animation sampling as an AI-to-single-screenshot loop that returns
to the tester before every next frame. `capture-sequence` is a planned future capability and must
not be documented as an existing command until implemented.

## 11. Enforceable execution budgets

The future runner defines budgets per case and records consumption in run evidence. At minimum,
the budgets bound:

- runner/driver round trips;
- vision checkpoints;
- screenshots and animation frames; and
- retries.

The default retry policy permits at most one controlled retry, and only after restoring the
required prerequisites, target identity, geometry, and expected precondition. “Take another
screenshot just in case” is not an unbounded evidence strategy.

## 12. Tester model and effort policy

The required routing policy is:

```text
Codex tester:       gpt-5.6-luna, reasoning effort explicitly medium
Claude Code tester: Claude Haiku 4.5, normal/default reasoning configuration
```

The parent agent's reasoning effort must not be inherited as the effective Codex child effort.
When the provider exposes the metadata, execution evidence should attest the effective tester
model and reasoning effort. Claude Code remains on its normal/default configuration unless its
launcher later requires an explicit anti-inheritance setting.

This is a desired routing policy, not a claim of current enforcement. The repository currently
does not prove that Codex child effort is explicitly pinned, and `.codex/config.toml` does not
provide an established setting for that purpose. If the supported child-launch mechanism cannot
set or attest the effort, that is a tooling gap for future implementation; no unsupported config
key is invented here.

## 13. Current state and target boundary

The platform drivers and their adapter/self-test responsibilities are implemented according to
their current guides and designs. The following shared capabilities are architectural targets,
not current product tooling:

| Capability | Current state |
|---|---|
| AppKit and WinUI3 platform drivers | Implemented platform adapters; see their command/design documents. |
| Shared durable product case | No durable shared product case is defined yet. |
| Case compiler and reusable plan | Planned; not implemented. |
| Local compiled-plan cache | Planned; not implemented. |
| Batch runner / shared execution | Planned; not implemented. |
| ROI and mechanical-image pipeline | Planned; not implemented. |
| Vision checkpoint protocol | Architectural target; no AI vision implementation is claimed. |
| Animation capture sequence | Planned; `capture-sequence` is not implemented. |
| Explicit Codex child-effort enforcement/attestation | Known tooling gap; policy is documented but not proven enforced. |

## 14. Alternatives and rejected choices

- **AI-controlled command-by-command redesign:** rejected because it increases round trips and
  makes execution nondeterministic; compile once and execute a bounded plan instead.
- **Hashing the entire repository:** rejected because unrelated commits should not invalidate a
  plan; use explicit structural dependencies.
- **Reusing prior results on a cache hit:** rejected because runtime identity, product state, HEAD,
  and evidence environment can change independently of plan structure.
- **Universal AppKit foreground gates:** rejected because semantic AX operations and screenshot
  capture do not require real frontmost input; retain foreground gates for real input.
- **Global sleeps or unbounded screenshots:** rejected in favor of condition-driven waits and
  enforceable capture/retry budgets.
- **Checked-in generated plans:** rejected for the first architecture; use the local ignored cache.
- **A separate AppKit orchestration architecture:** rejected because the shared design owns
  backend-neutral orchestration and platform guides should remain operational adapters.
