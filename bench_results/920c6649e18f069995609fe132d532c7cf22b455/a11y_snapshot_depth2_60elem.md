# synapse-a11y M1 readback

Source of truth: `AccessibleSubtree` returned by `synapse_a11y::snapshot`, `WinEventHookReadback` returned by `subscribe_win_events`, and `CdpDiagnostics` returned by `probe_chromium_cdp`.

## Windows UIA readback

Command:

```text
C:\Temp\synapse-a11y-test\synapse_a11y_tests.exe --nocapture
```

Result:

```text
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
source_of_truth=winevent_hook edge=apartment after=WinEventHookReadback { thread_id: 23384, apartment: MainSta, hook_count: 10, event_ids: [3, 32773, 32782, 32780, 32768, 32769, 32774, 4, 5, 2] }
source_of_truth=uia_snapshot edge=depth2 after=root:0x280dae:0000002a00280dae nodes:6 max_depth:1
source_of_truth=uia_snapshot edge=round_trip after=root:0x280dae:0000002a00280dae nodes:1
source_of_truth=cdp_diagnostics edge=reachable_port after=CdpDiagnostics { process_name: "msedge.exe", status: Ok, endpoint: Some("http://127.0.0.1:57364"), reason_code: None, capabilities: [DomSnapshot, AccessibilityFullAxTree, DomQuerySelector, PageCaptureScreenshot] }
```

`MainSta` is the COM readback for the first initialized STA thread and is accepted as an STA-family apartment. The snapshot degraded from requested depth 2 to depth 1 after a cold UIA call exceeded the 25 ms threshold; the returned tree is marked truncated in code.

## Notepad warm snapshot benchmark

Command:

```text
SYNAPSE_A11Y_MANUAL_BENCH=1 SYNAPSE_A11Y_BENCH_ITERS=300 C:\Temp\synapse-a11y-test\uia_snapshot_bench.exe
```

Foreground target: Windows Notepad launched and activated immediately before the benchmark.

Result:

```text
source_of_truth=a11y_snapshot_bench after=status:ok iterations:300 nodes:6 p99_ms:0.000 max_ms:0.001
```

The first physical UIA snapshot populates the 50 ms warm cache. WinEvent callbacks invalidate the cache on foreground, focus, value, name, structure, selection, menu, and alert events.

## WSL edge-case readback

Command:

```text
cargo test -p synapse-a11y -- --nocapture
```

Result:

```text
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
source_of_truth=coalesced_events edge=empty after=[]
source_of_truth=coalesced_events edge=within_50ms after=[AccessibleEvent { seq: 2, at_ms: 49, window_id: 4660, element_id: Some(ElementId("0x1234:00000001")), kind: NameChanged, name: Some("new"), value: None }]
source_of_truth=coalesced_events edge=exact_50ms after=[AccessibleEvent { seq: 1, at_ms: 0, window_id: 4660, element_id: Some(ElementId("0x1234:00000002")), kind: FocusChanged, name: None, value: None }, AccessibleEvent { seq: 2, at_ms: 50, window_id: 4660, element_id: Some(ElementId("0x1234:00000002")), kind: FocusChanged, name: None, value: None }]
source_of_truth=debounced_events edge=rapid_typing after=[AccessibleEvent { seq: 0, at_ms: 0, window_id: 4660, element_id: Some(ElementId("0x1234:00000003")), kind: ValueChanged, name: None, value: Some("a") }, AccessibleEvent { seq: 5, at_ms: 100, window_id: 4660, element_id: Some(ElementId("0x1234:00000003")), kind: ValueChanged, name: None, value: Some("f") }]
source_of_truth=cdp_diagnostics edge=no_debug_port after=CdpDiagnostics { process_name: "chrome.exe", status: Unreachable, endpoint: None, reason_code: Some("A11Y_CDP_UNREACHABLE"), capabilities: [] }
```
