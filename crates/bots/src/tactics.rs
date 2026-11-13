//! Hierarchical tactical interruption policy. Mission selection is separate.
//! Pure, allocation-free rules so threat/mission precedence can be tested.
use crate::task::{SwitchReason, TaskKind};

#[derive(Clone, Copy, Debug)]
pub struct TacticalContext {
    pub has_objective: bool,
    pub touching: bool,
    pub enemy_distance: Option<f32>,
    pub effective_range: f32,
    pub target_avoided: bool,
    pub clip_empty: bool,
    pub wounded: bool,
    pub recent_contact: bool,
}

pub struct TacticalDecision {
    pub kind: TaskKind,
    pub reason: SwitchReason,
    /// Compatibility telemetry, not a utility competition.
    pub telemetry: [f32; 5],
}

pub fn select_action(c: TacticalContext) -> TacticalDecision {
    let close_threat = c.enemy_distance.is_some_and(|d| d <= 256.0);
    let viable_threat = c.enemy_distance.is_some_and(|d| {
        d <= c.effective_range && (!c.target_avoided || close_threat)
    });
    // An empty magazine is an immediate interruption when under threat.
    // Reloading is also allowed between encounters; do not invent a target.
    let (kind, reason) = if c.clip_empty && c.enemy_distance.is_some() {
        (TaskKind::Recover, SwitchReason::ClipEmpty)
    } else if close_threat && viable_threat {
        (TaskKind::Fight, SwitchReason::SawEnemy)
    } else if c.touching {
        (TaskKind::TouchObj, SwitchReason::ObjectiveReachable)
    } else if c.wounded && !viable_threat {
        (TaskKind::Recover, SwitchReason::LowHealth)
    } else if c.has_objective {
        // Midrange encounters interrupt only if viable; an objective persists.
        if viable_threat {
            (TaskKind::Fight, SwitchReason::SawEnemy)
        } else {
            (TaskKind::TouchObj, SwitchReason::ObjectiveReachable)
        }
    } else if viable_threat {
        (TaskKind::Fight, SwitchReason::SawEnemy)
    } else if c.recent_contact {
        (TaskKind::Investigate, SwitchReason::LostSight)
    } else {
        (TaskKind::Hunt, SwitchReason::Spawned)
    };
    let mut telemetry = [-1.0; 5];
    let slot = match kind {
        TaskKind::Hunt => 0, TaskKind::Investigate => 1,
        TaskKind::Fight => 2, TaskKind::Recover => 3,
        TaskKind::TouchObj => 4,
    };
    telemetry[slot] = 100.0;
    TacticalDecision { kind, reason, telemetry }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn objective() -> TacticalContext {
        TacticalContext { has_objective: true, touching: false, enemy_distance: None,
            effective_range: 1000.0, target_avoided: false, clip_empty: false,
            wounded: false, recent_contact: false }
    }
    #[test] fn mission_survives_distant_or_avoided_enemy() {
        let mut c = objective(); c.enemy_distance = Some(1800.0);
        assert_eq!(select_action(c).kind, TaskKind::TouchObj);
        c.enemy_distance = Some(600.0); c.target_avoided = true;
        assert_eq!(select_action(c).kind, TaskKind::TouchObj);
    }
    #[test] fn close_defense_interrupts_objective() {
        let mut c = objective(); c.enemy_distance = Some(120.0);
        assert_eq!(select_action(c).kind, TaskKind::Fight);
    }
    #[test] fn reload_interrupts_combat() {
        let mut c = objective(); c.enemy_distance = Some(120.0); c.clip_empty = true;
        assert_eq!(select_action(c).kind, TaskKind::Recover);
    }
    #[test] fn touch_priority_over_medium_threat() {
        let mut c = objective(); c.touching = true; c.enemy_distance = Some(500.0);
        assert_eq!(select_action(c).kind, TaskKind::TouchObj);
    }
    #[test] fn investigate_only_without_mission() {
        let mut c = objective(); c.has_objective = false; c.recent_contact = true;
        assert_eq!(select_action(c).kind, TaskKind::Investigate);
    }
}
