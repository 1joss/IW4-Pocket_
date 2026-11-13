#!/usr/bin/env python3
"""Exercise the actual upstream C++ brain/bridge plus bundled route contracts."""
import hashlib
import pathlib
import plistlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
for name in ('mp_highrise', 'mp_afghan', 'mp_terminal'):
    lines = (ROOT / f'crates/bots/waypoints/{name}_wp.csv').read_text().splitlines()
    assert int(lines[0]) == len(lines) - 1
    for line in lines[1:]:
        fields = line.split(',')
        assert len(fields[0].split()) == 3
        assert fields[2] in ('stand', 'crouch', 'climb', 'prone', 'claymore', 'grenade', 'javelin'), fields[2]
        assert all(0 <= int(j) < int(lines[0]) for j in fields[1].split())

# Ensure the comparison actually uses the original implementation.
expected = '34d1bac73fcb607e9961a678007694f8fff155f068d8ea6a862943240ea1988d'
brain = ROOT / 'crates/bots/recompiled/runtime/bots/brain.cpp'
assert hashlib.sha256(brain.read_bytes()).hexdigest() == expected
plist = plistlib.loads((ROOT / 'ios/Info.plist').read_bytes())
assert plist['CFBundleIdentifier'] == 'com.markussela.iw4l'
assert plist['CFBundleDisplayName'] == 'IW4 Pocket'

harness = r'''
#include "bridge.cpp"
#include <cassert>
#include <cmath>
static bool clear(void*, const float*, const float*, uint32_t) { return true; }
int main() {
    assert(rc_reset("test", "3\n0 0 0,1,stand,,,\n150 0 0,2,crouch,,,\n300 0 0,,climb,,,\n"));
    assert(g_nodes.size() == 3);
    assert(Route(0,2).size() == 3);
    assert(Route(2,0).empty()); // no invented reverse links
    RcPlayer p{}; p.num=1; p.alive=1; p.grounded=1; p.health=100;
    RcCommand out{};
    assert(rc_think(1, &p, 1, 1.0, nullptr, clear, &out));
    // Isolate route following from random roaming/combat selection.
    auto& m = g_minds[1];
    m.path={0,1,2}; m.step=0; m.hasGoal=true; m.goal={300,0,0}; m.goalAt=1.0;
    assert(rc_think(1, &p, 1, 1.05, nullptr, clear, &out));
    assert(out.forward > 0 && (out.buttons & 0x200)); // crouch destination
    p.origin[0]=150;
    assert(rc_think(1, &p, 1, 1.10, nullptr, clear, &out));
    assert(out.buttons & 0x400); // climb destination presses jump
    rc_dead(1);
    assert(!g_minds[1].alive);
    p.origin[0]=0;
    assert(rc_think(1, &p, 1, 2.0, nullptr, clear, &out));
    assert(g_minds[1].alive);
    assert(std::isfinite(out.pitch) && std::isfinite(out.yaw));
    assert(rc_reset("test2", "1\n0 0 0,,stand,,,\n"));
    assert(!g_minds[1].hasGoal && g_nodes.size()==1);
    assert(!rc_reset("bad", "1\n0 0 0,999,stand,,,\n"));
}
'''
with tempfile.TemporaryDirectory() as tmp:
    source = pathlib.Path(tmp) / 'comparison_test.cpp'
    source.write_text(harness)
    binary = pathlib.Path(tmp) / 'comparison_test'
    subprocess.run(['c++', '-std=c++17', '-pthread', '-I', str(ROOT / 'crates/bots/recompiled'),
                    str(source), '-o', str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
print('Comparison routes, app identity, upstream integrity and controller execution passed.')
