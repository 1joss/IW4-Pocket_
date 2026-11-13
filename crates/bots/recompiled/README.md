# Recompiled controller comparison

`runtime/bots/brain.cpp` and `brain.h` are unmodified copies from
https://github.com/vrcnx/mw2-recompiled-ios at commit
00856e94fad5a2711b4fea1afa5cb90651bede9e (the iPadOS fork of
PaulCombal/mw2-recompiled). Copyright remains with the upstream authors.
Their GPL-3.0 license is reproduced in COPYING. The bridge and Rust adapter
are GPL-3.0 as part of this comparison integration. Distributions of this
combined comparison build must include the corresponding source under GPL-3.0;
the comparison workflow uploads a source archive beside the IPA.

Bot Warfare waypoint CSVs in ../waypoints are from INeedBots/INeedGames:
https://github.com/ineedbots/iw4_bot_warfare at commit
31415e49e5baf160bd369c9ef4d61295018ffef2. Credit to INeedBots and the
individual route authors credited by that project. Upstream permits reuse,
modification and merging with attribution (README, Credits).

The C++ brain is compiled, not rewritten. Its targeting, reaction, aiming,
combat movement, goal choice, A*, steering and recovery run unchanged.
`bridge.cpp` replaces engine hooks with a synchronous C ABI. Authored CSVs are
passed from embedded data instead of depending on a relative filesystem path.
Learning is disabled because this comparison always supplies authored routes.
Regular skill is fixed; native Pocket difficulty remains its existing default.
The engines, collision backends and bot tick rates differ from the upstream
app, so this is a controller comparison within IW4 Pocket, not a claim of
identical gameplay across engines. It is not complete original Bot Warfare.
