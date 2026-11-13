//! Bounded memory of persistently obstructed engagements.
//! A denied trace is unknown, not proof of obstruction.
use sim::ClientId;
const OBSTRUCTED_TICKS: u32 = 60;
const COOLDOWN_TICKS: u32 = 120;
#[derive(Clone, Copy, Debug, Default)]
pub struct EngagementExperience {
    target: Option<ClientId>,
    obstructed_since: Option<u32>,
    avoid_until: u32,
}
impl EngagementExperience {
    pub fn reset(&mut self) { *self = Self::default(); }
    pub fn record(&mut self, target: ClientId, tick: u32, blocked: bool) {
        if self.target != Some(target) {
            self.target = Some(target);
            self.obstructed_since = None;
            self.avoid_until = 0;
        }
        if blocked {
            let since = *self.obstructed_since.get_or_insert(tick);
            if tick.saturating_sub(since) >= OBSTRUCTED_TICKS {
                self.avoid_until = tick.saturating_add(COOLDOWN_TICKS);
                self.obstructed_since = Some(tick);
            }
        } else { self.obstructed_since = None; self.avoid_until = 0; }
    }
    pub fn avoided(&self, target: ClientId, tick: u32) -> bool {
        self.target == Some(target) && tick < self.avoid_until
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_obstruction_triggers_temporary_avoidance() {
        let mut x = EngagementExperience::default(); let id = ClientId(1);
        x.record(id, 10, true); x.record(id, 70, true);
        assert!(x.avoided(id, 70)); assert!(!x.avoided(id, 190));
        x.record(id, 191, false); assert!(!x.avoided(id, 191));
    }
    #[test]
    fn switching_targets_clears_old_failure() {
        let mut x = EngagementExperience::default();
        x.record(ClientId(1), 0, true); x.record(ClientId(1), 60, true);
        x.record(ClientId(2), 61, true);
        assert!(!x.avoided(ClientId(2), 61));
    }
}


/// One bounded home for navigation, exploration and combat history.
/// Route and exploration records intentionally have different lifetimes.
#[derive(Clone, Debug, Default)]
pub struct ExperienceMemory {
    engagement: EngagementExperience,
    failed_routes: [Option<([f32; 3], u32)>; 12],
    failed_cursor: usize,
    explored: [Option<[f32; 3]>; 8],
    explored_cursor: usize,
}
impl ExperienceMemory {
    pub fn reset_life(&mut self) {
        self.engagement.reset();
        self.failed_routes.fill(None);
        self.failed_cursor = 0;
        self.explored.fill(None);
        self.explored_cursor = 0;
    }
    pub fn reset_navigation(&mut self) {
        self.failed_routes.fill(None);
        self.failed_cursor = 0;
        self.explored.fill(None);
        self.explored_cursor = 0;
    }
    pub fn engagement(&self) -> &EngagementExperience { &self.engagement }
    pub fn engagement_mut(&mut self) -> &mut EngagementExperience { &mut self.engagement }
    pub fn remember_failed_route(&mut self, goal: [f32; 3], tick: u32) {
        let slot = self.failed_routes.iter()
            .position(|entry| entry.is_none_or(|(_, until)| until <= tick))
            .unwrap_or(self.failed_cursor);
        self.failed_routes[slot] = Some((goal, tick.saturating_add(240)));
        self.failed_cursor = (slot + 1) % self.failed_routes.len();
    }
    pub fn failed_route_near(&self, candidate: [f32; 3], tick: u32, radius: f32) -> bool {
        self.failed_routes.iter().flatten().any(|(goal, until)| *until > tick && distance_sq(*goal, candidate) < radius * radius)
    }
    pub fn recently_explored(&self, candidate: [f32; 3]) -> bool {
        self.explored.iter().flatten().any(|old| distance_sq(*old, candidate) < 240.0 * 240.0)
    }
    pub fn remember_explored(&mut self, candidate: [f32; 3]) {
        self.explored[self.explored_cursor] = Some(candidate);
        self.explored_cursor = (self.explored_cursor + 1) % self.explored.len();
    }
}
fn planar_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0]-b[0]).powi(2) + (a[1]-b[1]).powi(2)
}
fn distance_sq(a: [f32; 3], b: [f32; 3]) -> f32 {
    planar_sq(a,b) + (a[2]-b[2]).powi(2)
}
#[cfg(test)]
mod unified_tests {
    use super::*;
    #[test]
    fn route_cooldown_expires_and_history_is_bounded() {
        let mut m = ExperienceMemory::default();
        for i in 0..20 { m.remember_failed_route([i as f32 * 200.0,0.0,0.0], 10); }
        assert_eq!(m.failed_routes.iter().flatten().count(), 12);
        assert!(m.failed_route_near([1800.0,0.0,0.0], 10, 72.0));
        assert!(!m.failed_route_near([1800.0,0.0,0.0], 250, 72.0));
    }
    #[test]
    fn route_failures_do_not_block_a_different_floor() {
        let mut m = ExperienceMemory::default();
        m.remember_failed_route([0.0, 0.0, 48.0], 10);
        assert!(m.failed_route_near([0.0, 0.0, 48.0], 11, 72.0));
        assert!(!m.failed_route_near([0.0, 0.0, 192.0], 11, 72.0));
    }
    #[test]
    fn explored_and_failure_memories_are_independent() {
        let mut m = ExperienceMemory::default();
        m.remember_explored([10.0,0.0,0.0]);
        m.remember_failed_route([300.0,0.0,0.0], 10);
        assert!(m.recently_explored([10.0,0.0,0.0]));
        m.reset_navigation();
        assert!(!m.recently_explored([10.0,0.0,0.0]));
        assert!(!m.failed_route_near([300.0,0.0,0.0], 10, 72.0));
    }
}
