// GPL-3.0: adapter for vrcnx/mw2-recompiled-ios, commit 00856e9.
// Including the unmodified implementation keeps its private globals in one TU.
#include "runtime/bots/brain.cpp"
#include <mutex>

extern "C" {
struct RcPlayer {
    uint32_t num, team, alive, grounded;
    int32_t health;
    float origin[3], view[3];
};
struct RcCommand { float pitch, yaw; int32_t forward, right; uint32_t buttons; };
using Visible = bool (*)(void*, const float*, const float*, uint32_t);
}
static std::mutex rc_mutex;

extern "C" bool rc_reset(const char* map, const char* csv) noexcept {
    std::lock_guard<std::mutex> lock(rc_mutex);
    try {
        g_nodes.clear(); g_map = map; g_learned = false;
        for (Mind& m : g_minds) m = Mind{};
        for (int& n : g_lastNode) n = -1;
        std::istringstream file(csv);
        std::string line;
        std::getline(file, line);
        while (std::getline(file, line)) {
            std::stringstream fields(line);
            std::string position, links, type;
            std::getline(fields, position, ',');
            std::getline(fields, links, ',');
            std::getline(fields, type, ',');
            Node node;
            if (std::sscanf(position.c_str(), "%f %f %f", &node.at.x, &node.at.y, &node.at.z) != 3)
                return false;
            std::stringstream ids(links);
            for (int id; ids >> id;) node.links.push_back(id);
            node.crouch = type == "crouch";
            node.climb = type == "climb";
            g_nodes.push_back(std::move(node));
        }
        for (const Node& n : g_nodes) for (int id : n.links)
            if (id < 0 || id >= int(g_nodes.size())) return false;
        return !g_nodes.empty();
    } catch (...) { g_nodes.clear(); return false; }
}

extern "C" bool rc_think(uint32_t num, const RcPlayer* players, size_t count,
                         double time, void* context, Visible visible, RcCommand* out) noexcept {
    std::lock_guard<std::mutex> lock(rc_mutex);
    try {
        bots::brain::World world;
        world.time = time;
        world.skill = bots::brain::Skill::Regular;
        for (size_t i = 0; i < count; i++) {
            const auto& p = players[i];
            bots::brain::Player player;
            player.num = p.num; player.team = p.team; player.health = p.health;
            player.alive = p.alive != 0; player.onGround = p.grounded != 0;
            player.origin = {p.origin[0],p.origin[1],p.origin[2]};
            player.view = {p.view[0],p.view[1],p.view[2]};
            world.players.push_back(player);
            if (!player.alive) bots::brain::Dead(player.num);
        }
        world.visible = [&](const Vec3& a, const Vec3& b, uint32_t pass) {
            const float from[] = {a.x,a.y,a.z}, to[] = {b.x,b.y,b.z};
            return visible(context, from, to, pass);
        };
        const auto cmd = bots::brain::Think(num, world);
        *out = {cmd.pitch, cmd.yaw, cmd.forward, cmd.right, cmd.buttons};
        return true;
    } catch (...) { *out = {}; return false; }
}

extern "C" void rc_dead(uint32_t num) noexcept {
    std::lock_guard<std::mutex> lock(rc_mutex);
    bots::brain::Dead(num);
}
