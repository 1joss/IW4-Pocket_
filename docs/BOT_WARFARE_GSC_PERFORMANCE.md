# Bot Warfare GSC performance build 27

Branch: `feature/bot-warfare-gsc-performance`, based on the clean Pocket full-GSC build 26 (`ea539be6`). The full Bot Warfare controller, objectives, combat and waypoint navigation remain in GSC. This package does not restore the V5/V6 controller.

## Changes

- The interpreter borrows source locations instead of cloning their strings on every instruction. Short argument lists use inline storage; spawned arguments are copied once. The audited `isdefined` and `getarraykeys` builtins avoid unnecessary deep array copies, while overridden natives retain conservative value semantics.
- Thread, notification and timer lookup use indexes instead of repeatedly searching all threads/waiters/buckets. Registration order, cancellation, inline yielding and restored-world lookup remain covered by regression tests. Deleted-receiver checks avoid allocating per-thread receiver lists.
- Garbage collection reuses its worklist and visited sets. It still runs every tick and traces the same roots.
- Collision preparation runs once per unchanged geometry state per tick. Script geometry writes and unclassified native calls invalidate it; overridden natives lose their read-only classification. World-only rays skip player hit-volume preparation.
- Bot Warfare's two-part visibility predicate has a compact native implementation. It preserves the bullet/glass and sight conjunction, short-circuits blocked rays, avoids constructing a GSC trace-result array, and caches up to 1,024 exact endpoint pairs per unchanged geometry epoch. Geometry changes and tick transitions invalidate results.
- New, unknown enemy discovery defaults to 10 Hz, staggered by client slot. Remembered/current targets, forced targets and remote logic retain their original 20 Hz updates. Initial discovery can take up to 50 ms longer. `bots_perf_discovery_hz=20` restores full-rate discovery for comparison. FOV and smoke-distance rejection happen before expensive visibility work.
- Each spawned bot has one persistent GSC aim interpolation worker instead of continually creating and cancelling workers. Aim/movement updates retain the 50 ms cadence; death, respawn, disconnect and match end terminate the worker.
- Player `gettagorigin` no longer returns the player's feet for every bone. It uses cached authoritative hit-volume bone centres, with stance-aware fallbacks for recognized missing head/chest/ankle tags. These are targeting positions, not a claim of exact retail animation bone pivots. Pose and tick changes refresh them. This addresses one identified cause of reluctance to fire around cover; gameplay confirmation remains necessary.
- Off mode skips native profiling timers. Summary diagnostics now count collision preparation/reuse, visibility caching/avoided sight tests, tag cache work, consumed attack commands and accepted weapon shots. Per-bot records include fire attempts, button presses and the current firing gate (target, visibility, reaction, alignment, ADS, cooldown, weapon/range or disabled). Live bot totals now use the GSC roster.

The upstream hashes remain in `PROVENANCE.json`; local hashes identify the two modified GSC modules. Copy-on-write arrays, a new bytecode design, parallel script execution and a Rust controller translation remain separate architectural projects. No speedup percentage is claimed before testing on the iPad.

## Installation

Build 27 keeps `com.markussela.iw4l`, version 0.4.0, the existing executable name and Documents layout. Install over the existing app using the same signing account and bundle identifier. **Do not uninstall first.** There is no game-file migration or second game-data copy. A signing tool that rewrites the identifier can create a different installation.

## Testing and diagnostics

Use the same skill (default 3), bot count, teams and match settings as build 26. Start with 11 bots and Summary diagnostics. Run Terminal TDM, Highrise TDM and Afghan Domination for at least two minutes each; include doorway traffic, leaving spawn, shooting around cover, deaths/respawns and objective captures. Export before more than three sessions overwrite older captures; round restarts count as sessions.

For scaling, repeat a map at 5 and 17 bots. For instrumentation cost, repeat the same map/count with diagnostics Off after restarting the app. Detailed is reserved for a short reproduction because it changes performance. Record approximate times for bots refusing to shoot, pile-ups, freezes or slowdowns.

Export the entire `Documents/BotWarfareDiagnostics` folder plus `iw4l-boot.log` and `iw4l-boot.prev.log`. `tools/bw_report.py` reports aggregate performance counters and individual windows, preserving compatibility with build-26 captures. Counters describe retained windows, not necessarily the whole match. Aggregate weapon shots count accepted weapon-fire events, not individual shotgun pellets. Firing gate snapshots are sampled; they must be interpreted together with cumulative press and accepted-shot counters.

Compare scheduler p50/p95/max, native trace time, instructions, resumed/created threads, allocation/GC work and errors under matched scenarios. New cache counters show whether collision reuse survives actual scripts. Compare attack commands with accepted shots and firing gates before attributing reluctance to aim logic. Host tests and synthetic workloads do not substitute for device gameplay or predict its frame rate.
