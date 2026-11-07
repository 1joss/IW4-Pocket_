//! Deterministic team-wide objective distribution without per-tick path searches.
use crate::observation::{ModeObjective, ObjectiveAction};

/// Rank among living allied bots, stable for a given snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct TeamSlot { pub rank: usize, pub count: usize }

/// A modest preference, never a forced assignment: urgent objectives and
/// verified route feasibility remain in charge.
pub fn assignment_bonus(slot: TeamSlot, obj: ModeObjective, objectives: &[ModeObjective]) -> f32 {
    if slot.count <= 1 || objectives.len() <= 1 || obj.touching { return 0.0; }
    if !matches!(obj.action, ObjectiveAction::Capture | ObjectiveAction::Defend) { return 0.0; }
    let mut ids: Vec<(u32, u32)> = objectives.iter()
        .filter(|o| matches!(o.action, ObjectiveAction::Capture | ObjectiveAction::Defend))
        .map(|o| (o.round, o.id)).collect();
    ids.sort_unstable(); ids.dedup();
    if ids.len() <= 1 { return 0.0; }
    let assigned = ids[slot.rank % ids.len()];
    if (obj.round, obj.id) == assigned { 18.0 } else { -10.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_and_single_objective_have_no_preference() {
        let obj = ModeObjective::at([0.0; 3]);
        assert_eq!(assignment_bonus(TeamSlot {rank: 0, count: 4}, obj, &[obj]), 0.0);
    }
}
