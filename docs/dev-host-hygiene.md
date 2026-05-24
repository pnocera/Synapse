# Dev-host hygiene

Recommendations for keeping a long-lived synapse development host (Windows or Linux) lean and predictable. None of this is enforced at build time; it's collected here so contributors don't have to rediscover the trade-offs each time.

## Coordinates: `act_aim` / `act_click({x,y})` / `act_drag` / `act_scroll`

Synapse interprets all `{x, y}` mouse coordinates as **physical (DPI-aware) pixels** — the same units `GetCursorPos` returns from a per-monitor-DPI-aware process, and the same units UI Automation bounding boxes use. This matches the daemon's own DPI awareness (synapse-mcp is built as per-monitor V2) and `mouse_coordinates.rs::normalize_absolute_mouse_point` which feeds `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`.

Source-of-truth verifiers must use a DPI-aware reader or they will read **logical** (virtualised) coords and disagree with synapse by the monitor scale factor. The most common gotcha: PowerShell 5.1's `[System.Windows.Forms.Cursor]::Position` is DPI-unaware by default. The fix in PowerShell:

```powershell
Add-Type @'
using System;
using System.Runtime.InteropServices;
public struct POINT { public int X; public int Y; }
public class W { [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
                 [DllImport("user32.dll")] public static extern int SetProcessDPIAware(); }
'@
[W]::SetProcessDPIAware() | Out-Null
$p = New-Object POINT; [W]::GetCursorPos([ref]$p) | Out-Null
"$($p.X),$($p.Y)"
```

The `mouse_coordinates.rs::tests` cases double as a reference for the expected mapping on multi-monitor setups.

Element-id-based targets (`act_click({element_id: …})`) are unaffected — UIA bboxes are already in the synapse-native coord space.

## Release-profile build OOM on Windows (#236)

The default `[profile.release]` plus a full-workspace install can blow past 32 GB of pagefile commit during the link step, especially on multi-DLL Windows targets. Three independent amplifiers:

1. **Fat LTO over many crates.** `lto = "fat"` requires whole-program optimisation in a single unit; combined with the synapse-mcp dep graph (rmcp + chromiumoxide + uiautomation + windows-capture + perception) it routinely exceeds 32 GB commit.
2. **Default `CARGO_BUILD_JOBS`.** Without the env, cargo defaults to `num_cpus`; that multiplies the spike across cores.
3. **WSL-mounted `target/`.** Every artifact write hits 9P and amplifies memory pressure.

### Recommended install recipe for 32 GB workstations

```powershell
$env:CARGO_BUILD_JOBS = '2'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_TARGET_DIR  = 'C:\cargo-target\synapse'   # native NTFS, not WSL
cargo install --path crates/synapse-mcp --force --locked
```

Optional `Cargo.toml` adjustment (not yet applied; tracked under #236):

```toml
[profile.release]
codegen-units = 4   # was 16; 4 trades a bit of build time for ~40-60% lower peak commit
lto           = "thin"
# strip       = "debuginfo"   # if you don't need backtraces for release crashes
```

For very link-heavy builds, swapping the linker:

```toml
[target.x86_64-pc-windows-msvc]
linker    = "rust-lld.exe"
rustflags = ["-Clink-arg=/STACK:16777216"]
```

### Pagefile guidance

Microsoft's official recommendation is system-managed pagefile for dev workstations. If you keep manual sizing, target ≥ `2 × RAM` and put the file on the same drive as `target/`. The 16 / 32 GB cap on a 32 GB-RAM host has been observed to OOM the release link of this workspace.

## `target/` directory hygiene (#240)

Cargo never garbage-collects `target/`; on this repo it routinely reaches 8-10 GB on a single dev host. Use [`cargo-sweep`](https://github.com/holmgr/cargo-sweep) for selective pruning that preserves the incremental cache for files you touch:

```powershell
cargo install cargo-sweep
cargo sweep -t 30           # delete artifacts older than 30 days
cargo clean --doc            # docs alone can hit 1 GB+
cargo clean -p chromiumoxide --release   # surgical purge of the largest single dep
```

For multi-repo developers, set a shared cache in `~/.cargo/config.toml`:

```toml
[build]
target-dir = "C:/cargo-target/shared"
```

Be aware that `cargo clean` then nukes the shared cache for every repo using it.

`CARGO_INCREMENTAL=0` is the default for `release`; set it explicitly in install scripts to override stray env vars from CI tooling.

## Ephemeral FSV / operator-run output (#242)

The canonical location for ad-hoc run artifacts (`.log`, `.ndjson`, scratch JSON, debug screenshots) is:

* **Repo-local:** `./.runs/<run-id>/<file>` — easy to grep / diff / reference from issue comments
* **OS-cache (Windows):** `%LOCALAPPDATA%\synapse\runs\` — never pollutes git, survives `git clean -fdx`
* **OS-cache (Linux/macOS):** `$XDG_CACHE_HOME/synapse/runs/` (fallback `~/.cache/synapse/runs/`)

The legacy `fsv-<NNN>/` pattern (e.g. `fsv-218/`) at the repo root is **deprecated** and excluded by `.gitignore`. Existing content can be migrated to `.runs/<id>/` or deleted.

A small `scripts/clean-runs.ps1` (tracked under #242) prunes `.runs/` subdirs older than 30 days.

## Bench result retention (#243)

We have a `bench_results/<commit-hash>/` directory tracked in git. Per Criterion's own FAQ this is **not recommended** — baselines are regenerable, grow unboundedly, and per-commit references go stale fast.

Going forward (tracked under #243):

* Use [`critcmp`](https://github.com/BurntSushi/critcmp) for local A/B compare of named baselines (no commit required)
* Or wire CI to a benchmarking tracker ([Bencher](https://bencher.dev/) / self-hosted Conbench) with JSON exports
* If we must keep baselines in-tree, pin to release tags only (`bench_results/v1.x.y/`), not per-commit hashes
