# Bot Warfare GSC baseline (build 26)

Branch: `feature/bot-warfare-gsc`, based directly on standard `origin/main` commit
`2cc1559`. The comparison branch, V5/V6 controller and navigation build patches
are not applied. The old native controller is not scheduled. This branch has one
bot backend: the full Bot Warfare 2.3.0 GSC controller, not three comparison modes.

The import pins `ineedbots/iw4_bot_warfare` at
`31415e49e5baf160bd369c9ef4d61295018ffef2`. All upstream maps, scripts and scriptdata
are retained, including combat, objectives, waypoint routing, congestion costs,
manager, chat, menu and waypoint editor modules. Only the Pocket adapter and an
observational event hook differ. `crates/sim/bot_warfare/PROVENANCE.json` lists the
upstream hashes and changes; `python3 tools/test_bw_import.py` verifies them.
Stock MW2 scripts and game assets still come from the installed game data.

## Install over the current app

The package keeps `com.markussela.iw4l`, the original app identity, executable and
Documents layout. Build number is 26 (previous comparison build: 25).
Install/update it with the same sideloading account and bundle identifier used
for the current IPA. **Do not uninstall first.** An in-place update preserves the
existing iOS data container and game files; changing the signing identity or
rewriting the bundle identifier can change that behavior. The IPA contains no
installer, deletion or migration of game files. No second copy is required.

Use the usual Private Match BOTS setting. Live `bot add N` requests also increase
the GSC manager's fill target. The adapter defaults to skill 3/7 rather than
random maximum difficulty, and grants the first human local host access. Existing
`IW4L_BOT_MODE` settings from the comparison build do not select a backend here.
Old native-only dummy/hold/teleport/fire console tools are not GSC controls.

## Diagnostics and testing

In IW4 Pocket options, **Bot Warfare diagnostics** has Auto/Summary, Detailed,
and Off. Restart after changing it. The corresponding `iw4l-env.txt` setting is
`IW4L_BW_PROFILE=summary`, `detail`, or `off`. Auto is Summary.

First run Terminal TDM, Highrise TDM and Afghan Domination, with the same bot count,
match settings and skill. Let each run for at least two minutes after spawning.
Include spawn exits, the Terminal doorways and Domination captures. Record the
map, bot count, approximate time and what you observed. Compare 5, 11 and 17 bots
in separate runs to expose scaling. Export before starting more than three sessions (round restarts also count).

Export **the entire `Documents/BotWarfareDiagnostics` folder plus
`iw4l-boot.log` and `iw4l-boot.prev.log`** through the app's Files container. Summary
is intended for ordinary testing. Use Detailed for short reproductions after a
problem is identified; its extra work changes performance. Use Off with the same
scenario to estimate instrumentation overhead against the normal frame reports.
The standard Send logs button opens a report page; it does not attach this folder.

Each map load or in-place round restart gets a timestamped session with:

- `manifest.json`: schema/backend, pinned source, build, compiled module hashes,
  function/native identity tables, settings mode and capture limits.
- `trace-00..03.jsonl`: sequenced rotating records. Once per second: map/mode,
  settings, RNG, waypoint count, objective state, player poses, health, teams,
  kills/deaths, bot path/goal/target/visibility/skill/input state, heap and waiter
  counts, and cumulative script errors. Every five seconds: timing percentiles,
  maxima, per-function instructions and self time, per-native counts/time/errors,
  allocations, GC time and diagnostic/bridge overhead. Normal teardown writes
  the final partial window.
- Detailed additionally records scheduler frames, script resumptions, explicit script-object/client mappings, source-line
  instruction counts, engine calls/results (bounded result snapshots) and the
  generated per-bot command stream. This helps isolate routing, target selection,
  aiming and engine-boundary behavior before translating individual systems.
- `errors.json`: first 32 distinct script faults, with source locations and caller
  chains, retained outside trace rotation; a later fatal fault replaces the final
  slot. All repeated fault counts remain in periodic world snapshots. Native
  failures also include the calling source line in the trace. Boot logging remains.
- `latest-load-error.json` in the parent folder preserves the latest compile,
  binding or entry failure even if a runtime session could not start.

Storage is bounded to three sessions, four 4 MiB trace segments each (about 48 MiB
plus manifests/errors). The pending queue is capped at 4096 records and 2 MiB;
queue/write losses are reported, and record sequences expose overwritten history.
Files flush once per second, on initial errors, and on normal teardown. A sudden
kill may lose the last second and the partial window. An unavailable filesystem
is reported to the boot log; it cannot be made lossless.

Run `python3 tools/bw_report.py <session-folder> --out report.json` for a sorted
summary of retained windows, hotspots and errors. Keep the raw captures too.

## Interpreting performance and planning Rust work

Timings are elapsed wall time, not sampled CPU time. Scheduler totals include
stock gametype scripts and garbage collection. Function self times exclude
native calls and nested callbacks. Native timings include their callbacks, so
summing every native and script figure would double-count nested work. Function
`segments` counts resumed/entered timing slices, not semantic calls. Allocations
count created GSC objects/arrays, not allocator bytes. Heap counts are live counts.

Use instruction and source-line counts to distinguish expensive interpretation
from expensive engine traces. Use per-native call rates and maxima to distinguish
redundant visibility work from costly geometry calls. Check heap/GC growth and
waiters for leaks, bot inputs versus displacement for movement failures, goals and
objective flags for incentives, and target visibility/aim offsets/skills for
unfair combat. Errors must be resolved before treating behavior as controller
logic. The analyzer ranks retained aggregate costs; it does not average window
percentiles into a misleading global percentile.

For a Rust translation, keep these record types and client/tick identities. Port
one subsystem at a time, compare goals/paths/targets and commands under matched
scenarios, then compare time at each bot count. The bounded native result snapshots
have depth/element/string limits and object references; these logs are **not a
complete deterministic replay**. Exact replay needs a separate world/state and
RNG fixture harness. Source-line counts are available only in Detailed mode.
This first build establishes that baseline, not an assertion that every possible
optimization or on-device gameplay issue has been diagnosed already.

## Validation

- `cargo test -p sim --lib`: simulation, held-action/human isolation, respawn input
  reset, temporary bot admission/connect/begin/disconnect/slot reuse and persistent/nonpersistent round restarts, nested GSC profiling/error accounting and bounded diagnostic queue tests.
- `cargo check -p console`: complete desktop application dependency integration.
- Import/analyzer, iOS memory/package/menu and release-source contract tests.
- `cargo run -p sim --example bw_compile -- <stock-script-root>` compiles the full
  dependency closure, binds all natives, executes the adapter loads the bundled Terminal/Highrise/Afghan CSVs and runs the actual
  upstream A* against congestion, directional, isolated and empty graph fixtures.
  Its test-only trace native reports clear space; it does not validate map geometry.
- iOS workflow runs tests, builds and verifies the actual unsigned IPA, including
  original identity, version and embedded GSC/diagnostic markers.

Compilation/fixtures are not device gameplay verification. The first device run
must confirm bot admission, objectives, death/respawn and map changes against the
user's actual game assets. Unsupported engine behavior remains visible as faults.

The diagnostic stress fixture is repeatable without changing controller sources:

```
IW4L_BW_PROFILE=summary IW4L_BW_DIAGNOSTICS=/tmp/bw-stress cargo run -p sim --example bw_compile -- <stock-script-root> --stress-diagnostics
python3 tools/bw_report.py /tmp/bw-stress/<session-folder> --out /tmp/bw-stress-report.json
```

It deliberately generates 90,000 invalid calls. Validation confirmed four bounded
segments, readable error retention after old trace records rotated away, zero
malformed JSON lines, and the final 53-frame partial window. Separate short runs
confirmed three-session retention and preservation of unmarked directories.
The Detailed fixture also retained its aggregate instruction/native counters when
its bounded native-I/O queue dropped startup detail. These are host-side
instrumentation tests, not measurements of iPad gameplay performance.
