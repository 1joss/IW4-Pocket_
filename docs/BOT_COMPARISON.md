# IW4 Bot Compare

Baseline: V6 commit `d9e2f27fe3cfb000013dc2fce5f05254ace861a6`, matching the latest successful existing IPA build.

This experimental IPA installs beside IW4 Pocket with bundle identifier
`com.1joss.iw4pocket.botcomparison`. It uses a separate iOS Documents folder:
import your own MW2 game data into this app as you did for IW4 Pocket.
The IPA is unsigned, matching this project's existing sideloading workflow.

Open **IW4 Pocket options → Bot comparison**, choose a mode, then completely
close and relaunch the app. Auto is Pocket. Bot count remains the lobby setting.

| Selection | Decisions and combat | Navigation |
|---|---|---|
| Pocket | Existing Pocket controller | Current V6 generated graph and controller |
| Pocket + BW | Same Pocket controller | Bundled Bot Warfare directed waypoint graph |
| Recompiled + BW | Actual pinned Recompiled C++ brain, Regular skill | Same bundled Bot Warfare waypoint data |

The bundled maps are **Highrise, Afghan, Terminal**. In the two BW modes,
other maps hold bots idle and report missing coverage in the log; there is no
silent fallback to the generated graph. Both waypoint modes recognize crouch
and climb/jump flags as Recompiled does. Tactical waypoint tags such as grenade,
claymore, prone and javelin are ordinary positions in this integration, matching
Recompiled's interpretation; full Bot Warfare tactical scripts are not included.
Movement still depends on Pocket's physics and supported traversal mechanics.

Mode and graph coverage are logged at load and the active mode is logged with
periodic metrics. Recompiled uses synchronous sight callbacks without Pocket's
96-query controller quota, preserving its original decision cadence. Its
controller time and visibility calls are logged separately; native task/stall
counters do not describe that controller. Recompiled also uses enemy positions
for roaming goal choice as upstream does, unlike Pocket's perception contract.

For comparison, use the same map, game type, bot count, match length and device
settings. Start with Team Deathmatch. Test both directions between Highrise
spawn areas, Afghan spawn exits, and Terminal doorway passage with several bots.
Repeat each mode to account for spawn and target randomness. Record the mode
with any logs and observations. Objective-mode parity and on-device performance
are not established by compilation or unit checks.

Sources and redistribution information: ../crates/bots/recompiled/README.md.
