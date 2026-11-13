//! Persistent strategic intention, independent of temporary tactical actions.
//! State is deliberately small and deterministic for host-authoritative iOS bots.
use crate::observation::ModeObjective;

const MIN_COMMIT_TICKS: u32 = 100; // Five seconds at 50 ms/tick.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StrategicGoal {
    Disperse { destination: [f32; 3] },
    Objective { id: u32, round: u32 },
    Patrol { destination: [f32; 3] },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TacticalAction {
    Advance,
    Engage,
    Investigate,
    Recover,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StrategyState {
    goal: Option<StrategicGoal>,
    committed_at: u32,
    tactical: Option<TacticalAction>,
}

impl StrategyState {
    pub fn goal(&self) -> Option<StrategicGoal> { self.goal }
    pub fn tactical(&self) -> Option<TacticalAction> { self.tactical }
    pub fn reset_life(&mut self) {
        self.goal = None;
        self.tactical = None;
        self.committed_at = 0;
    }
    pub fn clear_goal(&mut self) { self.goal = None; self.tactical = None; }
    pub fn interrupt(&mut self, action: TacticalAction) { self.tactical = Some(action); }
    pub fn resume(&mut self) { self.tactical = None; }
    pub fn commit(&mut self, goal: StrategicGoal, tick: u32) {
        if self.goal != Some(goal) {
            self.goal = Some(goal);
            self.committed_at = tick;
        }
    }
    pub fn may_switch(&self, tick: u32) -> bool {
        self.goal.is_none() || tick.saturating_sub(self.committed_at) >= MIN_COMMIT_TICKS
    }
    pub fn objective_valid(&self, objectives: &[ModeObjective]) -> bool {
        match self.goal {
            Some(StrategicGoal::Objective { id, round }) =>
                objectives.iter().any(|o| o.id == id && o.round == round),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tactical_interrupt_does_not_erase_mission() {
        let mut state = StrategyState::default();
        let goal = StrategicGoal::Patrol { destination: [1.0, 2.0, 3.0] };
        state.commit(goal, 10);
        state.interrupt(TacticalAction::Engage);
        assert_eq!(state.goal(), Some(goal));
        assert_eq!(state.tactical(), Some(TacticalAction::Engage));
        state.resume();
        assert_eq!(state.goal(), Some(goal));
        assert_eq!(state.tactical(), None);
    }
    #[test]
    fn commitment_window_and_life_reset() {
        let mut state = StrategyState::default();
        state.commit(StrategicGoal::Patrol { destination: [0.0; 3] }, 10);
        assert!(!state.may_switch(109));
        assert!(state.may_switch(110));
        state.reset_life();
        assert!(state.may_switch(11));
    }
}
