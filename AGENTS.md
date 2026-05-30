# Synapse Agent Doctrine

GitHub Issues are the coordination and state surface. Read the issue queue before changing code. Treat `status:in-progress` issues assigned to this agent as resumable work after context compaction.

This file is the **canonical, verbatim source for operator directives D1–D4**. `docs/impplan/README.md` and `docs/impplan/00_methodology.md` reference these by tag — do not restate them elsewhere.

## Operator Directives (NEVER violate)

### D1 — Manual FSV is mandatory; never automated

Full State Verification (FSV) is performed manually by the agent. NEVER delegate it to a script, test, benchmark, harness, CI job, GitHub Action, or any automated substitute. Do not add `*_fsv` tests, FSV harnesses, or FSV scripts.

Per shipped change:

1. Define the Source of Truth (SoT): DB/table/key, file path, queue, metric, global state, external system record, or UI state.
2. Read the SoT before the trigger.
3. Trigger manually with synthetic inputs whose expected outputs are known.
4. Read the SoT again with a separate operation; record actual state.
5. Exercise the happy path + ≥3 edge cases (empty / boundary / structurally invalid), printing before/after state for each.

Automated tests, property tests, benchmarks, scripts, and build checks are supporting regression evidence only — never FSV, never named as FSV.

**MCP precondition + trigger.** Before any Synapse behavior is FSV-accepted, prove the real `synapse-mcp` daemon is running and active on this host: read the process/socket SoT, authenticate, call `health`, initialize an MCP session, read `tools/list` so the required tool is physically present. Daemon absent, stale, or unreachable ⇒ launching or reinstalling the repo-built runtime is the next setup action (D4). For any behavior with an MCP tool, the trigger MUST be the real `tools/call`; a CLI, unit test, helper binary, script, or direct storage write may only support investigation and MUST NOT replace the MCP trigger. FSV evidence names the daemon PID/bind or stdio child, the session/tool used, and the separate SoT read after the call. A `health` response or tool return value is not the verdict by itself.

**Client-parity (learned 2026-05-30, #548/#549).** The MCP trigger MUST go through a client that performs the same `tools/list` schema validation the production MCP client does. A hand-rolled HTTP/stdio caller that skips client-side JSON-Schema validation is NOT a sufficient FSV trigger: it will happily call tools whose schemas the real client rejects, masking total tool-surface outages. Concretely — `serde_json::Value` tool fields make `schemars` emit a bare boolean `true` schema, which strict clients reject (`fetching tools failed: … Invalid input`), making every tool unusable while a bypassing caller reports success. Always confirm the real wired client (the editor/agent `synapse` MCP) loads the full `tools/list` without schema errors before treating any tool as operational, and keep the `server::schema_sanitize` gate green.

### D2 — Delta-of-reality (#536)

Move toward a delta-first reality model. A baseline snapshot establishes or repairs state; routine agent context is then ordered changes in reality, not repeated full snapshots. Long-running work periodically asks Synapse to audit the accumulated assumption against physical reality and forces a rebase when drift is detected. Until the delta tools (#537–#543) ship, continue using the existing real MCP tools + separate SoT readbacks, and file or update the #536 child issues instead of letting delta-reality work live only in chat context.

### D3 — No GitHub Actions / CI gate (#351)

Do not dispatch, wait on, or use GitHub Actions/CI as a shipping gate unless a later explicit operator decision reverses #351 (operator decision 2026-05-24; #246/#247/#350/#351). Agent commits pushed to this repo MUST include `[skip ci]`.

### D4 — Missing configured-host prerequisites are work, not blockers

A missing local tool, driver, model, device, file, service, account state, installer, or hardware surface is never a stopping point or a `status:blocked` reason by itself. Synapse gives the agent full local computer-control responsibility — treat those control surfaces as the operator-equivalent host control surface. Every reversible local action the operator could take from this keyboard is agent-owned work: browser downloads, GUI installers, Device Manager checks, package-manager installs, model/file generation, firmware flashing, app launching, USB/COM inspection, and UI inspection through Synapse.

Operationally: identify what is missing, identify the authoritative SoT where it must appear, perform the acquisition/setup step, then read that SoT directly. Do not ask the operator to download or install something while reversible local acquisition/setup remains possible. The only blockable item is the exact operator-only hard-to-reverse external action left after every reversible local step is exhausted — spending money, using private credentials, changing billing, or modifying an external account. Prepare that exact action and ask only for its approval. Do not call an issue done until the prerequisite and the feature that depends on it are manually FSV-verified (D1) at the physical SoT.

## Required Wake-Up Context

After compaction or a new session, re-read:

1. `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
2. This file — directives D1–D4
3. Open and closed GitHub decision/context issues, especially #351
4. `git status` and the active issue comments
