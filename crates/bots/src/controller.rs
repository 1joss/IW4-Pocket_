use playerstate_iw4::UserCmd;
use sim::{ClientId, ClientLifecycle};

use crate::intent::{BotIntent, MotorReport, MoveMode, PathOutcome};
use crate::memory::Memory;
use crate::experience::ExperienceMemory;
use crate::motor::Motor;
use crate::nav::{self, NavGraph, PathError};
use crate::observation::{
    BotEvent, BotObservation, Contact, KnowledgeSource, ModeObjective, ObjectiveAction, TeamRole,
    WeaponClass, WeaponDamageRange,
};
use crate::query::{QueryResult, QuerySubsystem, SightSample, WalkSample, WorldQuery};
use crate::task::{ActionStage, Decision, SwitchReason, Task, TaskKind};
use crate::strategy::{StrategicGoal, StrategyState, TacticalAction};
use crate::weapon::WeaponSkill;
use crate::team::{TeamSlot, assignment_bonus};
use crate::tactics::{select_action, TacticalContext};

const THINK_PERIOD_TICKS: u32 = 2;
const STEP_IN: f32 = 64.0;
const WAYPOINT_IN: f32 = 24.0;
const LAST_SEEN_FRESH_TICKS: u32 = 40;
/// How stale the positive observation behind a shot may be. Committing to a
/// target is a movement and aiming decision; firing needs the enemy to have
/// been seen, not merely remembered.
const FIRE_FRESH_TICKS: u32 = 4;
/// Free-space fallback ring, used only where there is no navigation graph.
const FIGHT_RING: u32 = 12;
/// How many baked supports one scan evaluates. Two queries each, so the whole
/// scan fits inside a few slices of the shared quota.
const FIGHT_CANDIDATES: usize = 6;
/// How far the threat may move before its firing position is re-derived. Far
/// wider than a step, so a moving target does not rebuild the goal every tick.
const FIGHT_COMMIT_MOVE: f32 = 128.0;
const FIGHT_SCAN_MAX_AGE_TICKS: u32 = 8;
/// A committed position is abandoned if the engagement has not resolved by
/// then, so a stale goal cannot outlive its fight.
const FIGHT_COMMIT_MAX_AGE_TICKS: u32 = 60;
/// Commanded movement without displacement for this long is a stuck bot.
const STUCK_TICKS: u32 = 40;
/// Beyond the weapon's falloff endpoint, aiming and movement continue but
/// shots are restricted by class-specific human-like engagement discipline.
/// These are AI tactical limits, not a change to weapon ballistics.
const ASSAULT_FIRE_MAX: f32 = 2200.0;
const SMG_FIRE_MAX: f32 = 1400.0;
const LMG_FIRE_MAX: f32 = 2800.0;
const SNIPER_FIRE_MAX: f32 = 4500.0;
const SHOTGUN_FIRE_MAX: f32 = 650.0;
const ROUTE_RETRY_TICKS: u32 = 24;
const FIGHT_REPOSITION_TICKS: u32 = 100;
const FAILED_GOAL_RADIUS: f32 = 72.0;
const GOAL_RESERVATION_RADIUS: f32 = 96.0;
const GOAL_RESERVATION_PENALTY: f32 = 24.0;

#[derive(Clone, Debug)]
pub struct HostController {
    aggression: f32,
    self_preserve: f32,
    memory: Memory,
    experience: ExperienceMemory,
    strategy: StrategyState,
    task: Task,
    intent: BotIntent,
    motor: Motor,
    last_think_tick: Option<u32>,
    reaction_until: u32,
    last_target: Option<ClientId>,
    /// The client a live fire authorization rests on, and the tick of the
    /// positive observation that granted it.
    fire_target: Option<(ClientId, u32)>,
    committed: Vec<nav::RouteStep>,
    route_goal: Option<[f32; 3]>,
    route_work: nav::RouteWork,
    fight_scan: Option<FightScan>,
    weapon: WeaponSkill,
    nav_key: Option<(u64, u64, u32, u32)>,
    roam: Option<[f32; 3]>,
    roam_gen: u32,
    route_blocked_since: Option<u32>,
    last_nav_diagnostic_tick: Option<u32>,
    fight_hold_since: Option<u32>,
    decision: Decision,
    prev_seen: Vec<ClientId>,
    was_touching: bool,
    last_cmd: UserCmd,
    life_sequence: Option<sim::LifeSequence>,
    report: MotorReport,
    progress_origin: Option<[f32; 3]>,
    progress_tick: u32,
    escape: Option<([f32; 3], u32)>,
    reservations: Vec<(ClientId, [f32; 3])>,
    team_slot: TeamSlot,
    last_origin: [f32; 3],
}

impl HostController {
    pub fn new(seed: u64) -> Self {
        let aggression = ((seed & 0xff) as f32) / 255.0;
        let self_preserve = (((seed >> 8) & 0xff) as f32) / 255.0;
        Self {
            aggression,
            self_preserve,
            memory: Memory::default(),
            experience: ExperienceMemory::default(),
            strategy: StrategyState::default(),
            task: Task::default(),
            intent: BotIntent::default(),
            motor: Motor::new(seed),
            last_think_tick: None,
            reaction_until: 0,
            last_target: None,
            fire_target: None,
            committed: Vec::new(),
            route_goal: None,
            route_work: nav::RouteWork::default(),
            fight_scan: None,
            weapon: WeaponSkill::default(),
            nav_key: None,
            roam: None,
            roam_gen: 0,
            route_blocked_since: None,
            last_nav_diagnostic_tick: None,
            fight_hold_since: None,
            decision: Decision::default(),
            prev_seen: Vec::new(),
            was_touching: false,
            last_cmd: UserCmd::default(),
            life_sequence: None,
            report: MotorReport::Executing,
            progress_origin: None,
            progress_tick: 0,
            escape: None,
            reservations: Vec::new(),
            team_slot: TeamSlot::default(),
            last_origin: [0.0; 3],
        }
    }

    pub(crate) fn cancel_navigation(&mut self) {
        self.route_work.cancel();
        self.fight_scan = None;
        self.committed.clear();
        self.route_goal = None;
        self.route_blocked_since = None;
    }

    pub fn decision(&self) -> Decision {
        self.decision
    }

    pub fn route_stats(&self) -> nav::RouteStats {
        self.route_work.stats()
    }

    /// Slices the live route request has waited, and its longest run without
    /// granted progress. `(0, 0)` when nothing is pending.
    pub fn route_age(&self) -> (u32, u32) {
        self.route_work.live_age()
    }

    /// A bot with a movement task that has not moved. Holding, interacting and
    /// waiting to respawn are not navigation failures.
    pub fn stuck(&self, tick: u32) -> bool {
        matches!(
            self.intent.move_mode,
            MoveMode::Walk | MoveMode::Jump | MoveMode::Drop | MoveMode::BreakGlass
        ) && tick.saturating_sub(self.progress_tick) >= STUCK_TICKS
    }

    /// The enemy this bot has committed to, for perception to refresh before it
    /// spends the same allowance looking for new ones. `None` when the bot is
    /// not fighting.
    pub fn focus_target(&self) -> Option<ClientId> {
        self.last_target
            .filter(|_| self.task.kind == TaskKind::Fight)
    }

    pub fn task(&self) -> Task {
        self.task
    }

    pub fn set_team_slot(&mut self, slot: TeamSlot) { self.team_slot = slot; }

    pub fn intent(&self) -> BotIntent {
        self.intent
    }

    pub fn report(&self) -> MotorReport {
        self.report
    }

    pub fn last_cmd(&self) -> UserCmd {
        self.last_cmd
    }

    pub fn position(&self) -> [f32; 3] {
        self.last_origin
    }

    pub fn drive(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        dt_ms: i32,
    ) -> UserCmd {
        self.drive_nav(obs, world, None, &mut 512, &[], dt_ms)
    }

    pub fn drive_nav(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        reservations: &[(ClientId, [f32; 3])],
        dt_ms: i32,
    ) -> UserCmd {
        self.last_origin = obs.self_state.origin;
        self.reservations.clear();
        self.reservations.extend_from_slice(reservations);
        world.enter(QuerySubsystem::Execution);
        let key = nav.map(|g| (g.generation, g.digest, g.schema, g.hull));
        if self.nav_key != key {
            self.nav_key = key;
            self.cancel_navigation();
            self.experience.reset_navigation();
            self.last_nav_diagnostic_tick = None;
        }
        if self.life_sequence != Some(obs.self_state.life_sequence) {
            self.life_sequence = Some(obs.self_state.life_sequence);
            self.cancel_navigation();
            self.weapon.reset();
            self.roam = None;
            if let Some(graph) = nav.filter(|g| !g.is_empty()) {
                if let Some(start) = nav::nearest_within(graph, obs.self_state.origin, nav::SNAP_IN) {
                    let reachable = nav::reachable_mask(graph, start);
                    let count = reachable.iter().filter(|&&yes| yes).count();
                    diag::info!(Sim, "bots: nav_spawn id={} node={} reachable={}/{} origin=({:.0},{:.0},{:.0})", obs.self_state.id.0, start, count, graph.nodes.len(), obs.self_state.origin[0], obs.self_state.origin[1], obs.self_state.origin[2]);
                } else {
                    diag::warn!(Sim, "bots: nav_spawn id={} no_start_support", obs.self_state.id.0);
                }
            }
            self.fight_hold_since = None;
            self.memory = Memory::default();
            self.experience.reset_life();
            self.strategy.reset_life();
            self.prev_seen.clear();
            self.was_touching = false;
            self.last_think_tick = None;
            self.last_target = None;
            self.fire_target = None;
            self.progress_origin = None;
            self.escape = None;
            self.decision = Decision::default();
            self.task = Task::default();
        }
        if obs.self_state.lifecycle != ClientLifecycle::Alive {
            self.strategy.resume();
            self.task = Task {
                kind: TaskKind::Hunt,
                reason: SwitchReason::Died,
            };
            self.intent = BotIntent::default();
            self.route_work.cancel();
            self.weapon.reset();
            self.fight_scan = None;
            self.fire_target = None;
            self.decision.stage = ActionStage::Failed;
            self.decision.objective = None;
            self.committed.clear();
            self.last_cmd = UserCmd {
                server_time: obs.time_ms,
                weapon: obs.self_state.weapon,
                weapon_mapped: obs.self_state.weapon,
                ..UserCmd::default()
            };
            return self.last_cmd;
        }
        if self.should_think(obs.tick, obs.self_state.id.0) || self.urgent_think(obs) {
            self.think(obs, world, nav, astar_budget);
            self.last_think_tick = Some(obs.tick);
        }
        // Intents are reused between scheduled thinks, so the authorization is
        // re-checked on the tick that actually emits the command.
        self.gate_fire(obs);
        if self.intent.move_mode == MoveMode::BreakGlass {
            self.intent.fire = false;
            self.intent.sprint = false;
            self.intent.use_button = false;
            if let Some(goal) = self.intent.move_goal {
                world.enter(QuerySubsystem::Tactical);
                let eye = obs.eye();
                // Try several body-height points on the obstructed corridor.
                for height in [obs.self_state.view_height.max(40.0), 40.0, 20.0] {
                    let target = [goal[0], goal[1], goal[2] + height];
                    if let Ok(SightSample::Blocked {
                        fraction,
                        obstacle: crate::query::ObstacleKind::Glass,
                    }) = world.shot_ray(eye, target, obs.self_state.id)
                    {
                        self.intent.look_at = Some(std::array::from_fn(|i| {
                            eye[i] + (target[i] - eye[i]) * fraction
                        }));
                        self.intent.fire = obs.self_state.ammo_clip > 0
                            && self.intent.weapon.is_none()
                            && !self.intent.reload
                            && (self.task.kind != TaskKind::Fight
                                || obs.tick >= self.reaction_until);
                        break;
                    }
                }
            }
        }
        let cmd = self.motor.drive(obs, &self.intent, dt_ms);
        self.report = self.motor.report(obs, &self.intent, &cmd);
        self.last_cmd = cmd;
        cmd
    }

    /// A retained target is never a standing permission to shoot: the shot has
    /// to stay backed by a positive observation of that same client, refreshed
    /// on every tick perception actually looked.
    fn gate_fire(&mut self, obs: &BotObservation) {
        if !self.intent.fire {
            return;
        }
        let Some((id, since)) = self.fire_target.as_mut() else {
            self.intent.fire = false;
            return;
        };
        if obs.seen.iter().any(|contact| contact.id == *id) {
            *since = obs.tick;
        }
        // A target may keep moving between decision ticks. A previous shot
        // authorization must not bypass the weapon's range gate.
        let range_ok = obs.seen.iter().find(|contact| contact.id == *id).is_none_or(|contact| {
            dist2(obs.self_state.origin, contact.origin).sqrt()
                <= engagement_limit(obs.self_state.weapon_class, None)
        });
        if obs.tick.saturating_sub(*since) > FIRE_FRESH_TICKS || !range_ok {
            self.intent.fire = false;
        }
    }

    /// Records what a shot decision rests on. `None` withdraws the
    /// authorization, which the emitted-command path then enforces.
    fn authorize_fire(&mut self, fire: bool, target: Contact) -> bool {
        self.fire_target = fire.then_some((target.id, target.seen_tick));
        fire
    }

    fn should_think(&self, tick: u32, id: u32) -> bool {
        match self.last_think_tick {
            None => true,
            Some(last) if last == tick => false,
            Some(_) => (tick.wrapping_add(id)).is_multiple_of(THINK_PERIOD_TICKS),
        }
    }

    fn urgent_think(&self, obs: &BotObservation) -> bool {
        if self.last_think_tick == Some(obs.tick) {
            return false;
        }
        if self.decision.objective.is_some_and(|old| {
            !obs.objectives.iter().chain(obs.objective.iter()).any(|o| {
                o.id == old.id
                    && o.round == old.round
                    && o.action == old.action
                    && o.use_button == old.use_button
                    && o.touching == old.touching
            })
        }) {
            return true;
        }
        obs.events_since(&self.prev_seen)
            .iter()
            .any(|event| matches!(event, BotEvent::Spotted { .. }))
    }

    fn think(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) {
        let started = std::time::Instant::now();
        let events = self.memory.ingest(obs);
        let touching_use = obs
            .objectives
            .iter()
            .copied()
            .chain(obs.objective)
            .find(|obj| obj.touching && obj.use_button);
        if touching_use.is_none()
            && let Some(intent) = self.unstick(obs, world)
        {
            self.intent = intent;
            self.task.reason = SwitchReason::ProgressLost;
            self.decision.stage = ActionStage::Approach;
            self.apply_weapon_skill(obs);
            return;
        }
        // A tactical interruption must not erase a still-valid mission.
        // Invalidated objectives can be replaced immediately, regardless of the
        // normal five-second commitment window.
        let remembered = match self.strategy.goal() {
            Some(StrategicGoal::Objective { id, round }) => obs.objectives
                .iter().copied().chain(obs.objective)
                .find(|o| o.id == id && o.round == round),
            _ => None,
        };
        if matches!(self.strategy.goal(), Some(StrategicGoal::Objective { .. }))
            && remembered.is_none()
        {
            self.strategy.clear_goal();
        }
        let mut objective = touching_use.or_else(|| {
            if !self.strategy.may_switch(obs.tick) {
                if let Some(obj) = remembered { return Some(obj); }
            }
            obs.objectives
                .iter()
                .copied()
                .chain(obs.objective)
                .max_by(|a, b| {
                    (objective_score(obs, *a, None, self.team_slot, self.route_goal, &self.reservations))
                        .total_cmp(&(objective_score(obs, *b, None, self.team_slot, self.route_goal, &self.reservations)))
                        .then_with(|| b.id.cmp(&a.id))
                })
        });
        // Mission selection precedes tactical interruptions. Unlike the old
        // flat utility contest, a valid objective is not displaced by roaming.
        // Route checks are performed only when no verified route is held.
        if touching_use.is_none() && objective.is_some() {
            objective = choose_objective(
                obs, world, nav, astar_budget,
                &mut self.committed, &mut self.route_goal, &mut self.route_work,
                &self.reservations, self.team_slot,
                remembered.filter(|_| !self.strategy.may_switch(obs.tick)),
            );
        }
        let next = self.select_tactical_task(
            obs, objective, world.weapon_damage_range(obs.self_state.weapon),
        );
        // A live mission receives a modest stability bonus, not an absolute
        // lock: emergencies and genuinely superior objectives still win.
        let switched = next.kind != self.task.kind;
        // Routes are keyed by destination. A temporary combat fallback must not
        // discard navigation progress on every think tick.
        self.task = next;
        match self.task.kind {
            TaskKind::Fight => self.strategy.interrupt(TacticalAction::Engage),
            TaskKind::Investigate => self.strategy.interrupt(TacticalAction::Investigate),
            TaskKind::Recover => self.strategy.interrupt(TacticalAction::Recover),
            _ => self.strategy.resume(),
        }
        let previous_stage = self.decision.stage;
        let previous = self.decision.objective;
        self.decision.objective = objective.filter(|_| self.task.kind == TaskKind::TouchObj);
        self.intent = self.intent_for(obs, world, nav, astar_budget, objective);
        if self.task.kind == TaskKind::TouchObj
            && !matches!(self.intent.path, PathOutcome::Blocked | PathOutcome::Unreachable)
        {
            if let Some(obj) = self.decision.objective {
                self.strategy.commit(StrategicGoal::Objective { id: obj.id, round: obj.round }, obs.tick);
            }
        } else if self.task.kind == TaskKind::Hunt {
            if let Some(destination) = self.roam {
                self.strategy.commit(StrategicGoal::Disperse { destination }, obs.tick);
            }
        }
        self.decision.stage = if matches!(
            self.intent.path,
            PathOutcome::Blocked | PathOutcome::Unreachable
        ) {
            ActionStage::Failed
        } else if previous.is_some_and(|old| {
            old.use_button
                && obs
                    .objectives
                    .iter()
                    .chain(obs.objective.iter())
                    .any(|new| {
                        old.id == new.id
                            && old.round == new.round
                            && old.action != new.action
                            && new.action == ObjectiveAction::Defend
                    })
        }) {
            ActionStage::Complete
        } else if self.intent.use_button {
            ActionStage::Interact
        } else if self.intent.move_mode == MoveMode::Hold {
            ActionStage::Hold
        } else if objective
            .is_some_and(|o| dist2(obs.self_state.origin, o.origin) < o.radius * o.radius)
        {
            ActionStage::Position
        } else {
            ActionStage::Approach
        };
        self.intent.sprint = self.intent.move_mode == MoveMode::Walk
            && self.intent.path == PathOutcome::Clear
            && self.memory.currently_seen().next().is_none()
            && !self.intent.use_button
            && !self.intent.fire
            && !self.intent.reload
            && !self.intent.crouch
            && self
                .intent
                .move_goal
                .is_some_and(|g| dist_xy(obs.self_state.origin, g) > 160.0);
        // Standing clearance also fits a crouched hull. Require a shot at crouched eye height.
        if self.intent.fire
            && self.intent.move_mode == MoveMode::Hold
            && obs.self_state.health < 60
            && let Some(target) = self.pick_fight_target(obs)
            && world.walk_hull(obs.self_state.origin, obs.self_state.origin)
                == Ok(WalkSample::Clear)
        {
            let mut eye = obs.self_state.origin;
            eye[2] += 40.0;
            self.intent.crouch = shot_allowed(
                world.shot_ray(eye, self.intent.look_at.unwrap(), obs.self_state.id),
                target.id,
            );
        }
        if (self.intent.path == PathOutcome::Unreachable
            || self.intent.path == PathOutcome::Blocked)
            && (self.task.kind == TaskKind::TouchObj || self.task.kind == TaskKind::Investigate)
        {
            self.task.reason = SwitchReason::PathFailed;
        }
        let touching_now = objective.is_some_and(|obj| obj.touching);
        if self.task.kind == TaskKind::TouchObj
            && self.was_touching
            && !touching_now
            && self.decision.stage != ActionStage::Complete
        {
            self.intent.path = PathOutcome::ProgressLost;
            self.task.reason = SwitchReason::ProgressLost;
        }
        self.was_touching = touching_now && self.task.kind == TaskKind::TouchObj;
        self.apply_weapon_skill(obs);
        self.prev_seen = self
            .memory
            .currently_seen()
            .map(|contact| contact.id)
            .collect();
        let weapon_act = self.weapon.act();
        if switched || previous_stage != self.decision.stage {
            let left = self
                .intent
                .move_goal
                .map(|goal| dist2(obs.self_state.origin, goal).sqrt())
                .unwrap_or(0.0);
            let know = self
                .memory
                .currently_seen()
                .next()
                .or_else(|| self.memory.last_seen().next())
                .map(|c| c.source);
            diag::info!(
                Sim,
                "bots: id={} tick={} task={:?} reason={:?} path={:?} know={know:?} origin=({:.0},{:.0},{:.0}) left={left:.0} events={events:?} decision={:?} weapon={weapon_act:?} think_us={} report={:?}",
                obs.self_state.id.0,
                obs.tick,
                self.task.kind,
                self.task.reason,
                self.intent.path,
                obs.self_state.origin[0],
                obs.self_state.origin[1],
                obs.self_state.origin[2],
                self.decision,
                started.elapsed().as_micros(),
                self.report,
            );
        }
    }

    /// The weapon channel has one owner. The skill commits to reloading or
    /// switching and keeps its request until the simulation confirms the
    /// outcome; movement and looking stay with the task.
    fn apply_weapon_skill(&mut self, obs: &BotObservation) {
        // An interaction already under way is not displaced by an empty magazine.
        self.weapon.update(obs, !self.intent.use_button);
        self.intent.weapon = self.weapon.requested_weapon();
        self.intent.reload = self.weapon.wants_reload();
        if self.intent.weapon.is_some() {
            self.intent.fire = false;
        }
        if self.intent.weapon.is_some() || self.intent.reload {
            self.intent.sprint = false;
        }
    }

    fn unstick(&mut self, obs: &BotObservation, world: &mut impl WorldQuery) -> Option<BotIntent> {
        let here = obs.self_state.origin;
        if self
            .progress_origin
            .is_none_or(|old| dist_xy(old, here) >= 16.0)
        {
            self.progress_origin = Some(here);
            self.progress_tick = obs.tick;
        }
        if let Some((goal, until)) = self.escape {
            if obs.tick < until && dist_xy(here, goal) > 8.0 {
                return Some(BotIntent {
                    look_at: Some(goal),
                    move_goal: Some(goal),
                    move_mode: MoveMode::Walk,
                    path: PathOutcome::Clear,
                    ..BotIntent::default()
                });
            }
            self.escape = None;
            self.committed.clear();
            self.roam = None;
            if obs.tick >= until && dist_xy(here, goal) > 8.0 {
                self.route_work.cancel();
                self.route_goal = None;
                self.roam_gen = self.roam_gen.saturating_add(1);
            }
        }
        let moving = matches!(
            self.intent.move_mode,
            MoveMode::Walk | MoveMode::Jump | MoveMode::Drop | MoveMode::BreakGlass
        ) || matches!(self.intent.path, PathOutcome::Blocked | PathOutcome::BudgetExhausted | PathOutcome::Unreachable | PathOutcome::IncompleteGraph);
        if !moving || obs.tick.saturating_sub(self.progress_tick) < 40 {
            return None;
        }
        self.progress_tick = obs.tick;
        // Pmove can be blocked by other players even when world traces are clear.
        // Different clients yield in different directions before rejoining their route.
        let salt = obs
            .self_state
            .id
            .0
            .wrapping_mul(3)
            .wrapping_add(obs.tick / 40);
        for offset in 0..4 {
            let angle = ((salt + offset * 2) % 8) as f32 * core::f32::consts::FRAC_PI_4;
            let goal = [
                here[0] + angle.cos() * 64.0,
                here[1] + angle.sin() * 64.0,
                here[2],
            ];
            if world.walk_hull(here, goal) != Ok(WalkSample::Clear) {
                continue;
            }
            let Ok(floor) = world.hull_trace(
                [goal[0], goal[1], goal[2] + 2.0],
                [goal[0], goal[1], goal[2] - nav::STEP_Z_IN],
            ) else {
                continue;
            };
            if floor.startsolid || floor.fraction >= 1.0 {
                continue;
            }
            // An escape must break the stale search, not resume the same
            // unreachable destination on the next think.
            if let Some(goal) = self.route_goal {
                self.remember_failed_goal(goal, obs.tick);
            }
            self.committed.clear();
            self.route_goal = None;
            self.route_work.cancel();
            self.roam = None;
            self.fight_scan = None;
            self.route_blocked_since = None;
            self.escape = Some((floor.endpos, obs.tick + 24));
            diag::info!(Sim, "bots: id={} tick={} nav_escape=started origin=({:.0},{:.0},{:.0}) target=({:.0},{:.0},{:.0})", obs.self_state.id.0, obs.tick, here[0], here[1], here[2], floor.endpos[0], floor.endpos[1], floor.endpos[2]);
            return Some(BotIntent {
                look_at: Some(floor.endpos),
                move_goal: Some(floor.endpos),
                move_mode: MoveMode::Walk,
                path: PathOutcome::Clear,
                ..BotIntent::default()
            });
        }
        // No safe escape was verified. Invalidate the current route so the
        // next scheduled think can seek a different supported destination.
        if let Some(goal) = self.route_goal {
            self.remember_failed_goal(goal, obs.tick);
        }
        self.committed.clear();
        self.route_goal = None;
        self.route_work.cancel();
        self.roam = None;
        self.fight_scan = None;
        None
    }

    /// Translate a persistent mission and immediate threats into an execution
    /// task. The five tasks are execution adapters, not competing missions.
    fn select_tactical_task(
        &mut self,
        obs: &BotObservation,
        objective: Option<ModeObjective>,
        damage: Option<WeaponDamageRange>,
    ) -> Task {
        let target = self.pick_fight_target(obs);
        let distance = target.map(|c| dist2(obs.self_state.origin, c.origin).sqrt());
        let avoided = target.is_some_and(|c| self.experience.engagement().avoided(c.id, obs.tick));
        let fresh = self.memory.last_seen().any(|c|
            obs.tick.saturating_sub(c.seen_tick) <= LAST_SEEN_FRESH_TICKS);
        let decision = select_action(TacticalContext {
            has_objective: objective.is_some(),
            touching: objective.is_some_and(|o| o.touching),
            enemy_distance: distance,
            effective_range: engagement_limit(obs.self_state.weapon_class, damage),
            target_avoided: avoided,
            clip_empty: obs.self_state.ammo_clip == 0,
            wounded: self.wounded(obs),
            recent_contact: fresh,
        });
        self.decision.scores = decision.telemetry;
        Task { kind: decision.kind, reason: decision.reason }
    }

    fn intent_for(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        objective: Option<ModeObjective>,
    ) -> BotIntent {
        match self.task.kind {
            TaskKind::Fight => self.fight(obs, world, nav, astar_budget),
            TaskKind::Investigate => self.seek(obs, world, nav, astar_budget, SeekKind::LastSeen),
            TaskKind::TouchObj => self.seek(
                obs,
                world,
                nav,
                astar_budget,
                SeekKind::Objective(objective),
            ),
            TaskKind::Recover => self.seek(obs, world, nav, astar_budget, SeekKind::BackOff),
            TaskKind::Hunt => self.hunt(obs, world, nav, astar_budget),
        }
    }

    fn hunt(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) -> BotIntent {
        let Some(goal) = self.ensure_roam(obs, nav) else {
            return BotIntent::default();
        };
        let (move_mode, path, move_goal) =
            self.route(obs.self_state.origin, goal, world, nav, astar_budget);
        // Incomplete coverage is not a wall: pick another roam destination
        // instead of holding in front of the truncated region forever.
        if matches!(
            path,
            PathOutcome::Blocked | PathOutcome::Unreachable | PathOutcome::IncompleteGraph
        ) {
            self.remember_failed_goal(goal, obs.tick);
            self.roam = None;
            self.committed.clear();
            self.route_goal = None;
        }
        BotIntent {
            look_at: Some([goal[0], goal[1], obs.eye()[2]]),
            move_goal,
            desired_range: None,
            move_mode,
            path,
            ..BotIntent::default()
        }
    }

    fn ensure_roam(&mut self, obs: &BotObservation, nav: Option<&NavGraph>) -> Option<[f32; 3]> {
        if let Some(StrategicGoal::Disperse { destination }) = self.strategy.goal()
            && dist2(obs.self_state.origin, destination) > 64.0 * 64.0
            && !self.experience.failed_route_near(destination, obs.tick, FAILED_GOAL_RADIUS)
        {
            self.roam = Some(destination);
            return Some(destination);
        }
        if let Some(goal) = self.roam
            && dist2(obs.self_state.origin, goal) > 64.0 * 64.0
        {
            return Some(goal);
        }
        if self.roam.is_some() {
            self.committed.clear();
        }
        self.roam_gen = self.roam_gen.saturating_add(1);
        let salt = obs.self_state.id.0 ^ self.roam_gen;
        let next = if let Some(graph) = nav.filter(|graph| !graph.is_empty()) {
            let mut best: Option<([f32; 3], f32)> = None;
            if let Some(start) = nav::nearest_within(graph, obs.self_state.origin, nav::SNAP_IN) {
                // A single directed traversal serves every candidate this selection.
                let reachable = nav::reachable_mask(graph, start);
                for attempt in 0..8u32 {
                    let Some(candidate) = nav::roam_node_reachable(
                        graph, obs.self_state.origin,
                        salt.wrapping_add(attempt.wrapping_mul(0x9E37_79B1)),
                        &reachable,
                    ) else { continue; };
                    if self.experience.failed_route_near(candidate, obs.tick, FAILED_GOAL_RADIUS) {
                        continue;
                    }
                    let crowd = reservation_penalty(obs.self_state.id, candidate, &self.reservations);
                    let repeat = if self.experience.recently_explored(candidate) { 40.0 } else { 0.0 };
                    let score = crowd + repeat;
                    if best.is_none_or(|(_, current)| score < current) {
                        best = Some((candidate, score));
                        if score == 0.0 { break; }
                    }
                }
            }
            best.map(|(candidate, _)| candidate)
        } else {
            let ang = (salt as f32) * 0.37;
            Some([
                obs.self_state.origin[0] + 160.0 * ang.cos(),
                obs.self_state.origin[1] + 160.0 * ang.sin(),
                obs.self_state.origin[2],
            ])
        };
        if let Some(candidate) = next { self.experience.remember_explored(candidate); }
        self.roam = next;
        next
    }

    fn fight(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) -> BotIntent {
        let Some(target) = self.pick_fight_target(obs) else {
            return BotIntent::default();
        };
        if self.last_target != Some(target.id) {
            self.last_target = Some(target.id);
            let reaction = (6.0 - self.aggression * 4.0).max(2.0) as u32;
            self.reaction_until = obs.tick.saturating_add(reaction);
        }
        let aim_at = [target.origin[0], target.origin[1], target.origin[2] + 48.0];
        if self.wounded(obs) {
            return self.fight_break(obs, world, nav, astar_budget, target);
        }
        // Perception can see a head over cover while the chest is occluded.
        // Aim only at a point validated by the weapon trace, not sight alone.
        world.enter(QuerySubsystem::Tactical);
        let mut aim_at = aim_at;
        let mut shot = world.shot_ray(obs.eye(), aim_at, obs.self_state.id);
        if matches!(
            shot,
            Ok(SightSample::Blocked {
                obstacle: crate::query::ObstacleKind::Glass,
                ..
            })
        ) {
            return BotIntent {
                look_at: Some(aim_at),
                move_goal: Some(target.origin),
                move_mode: MoveMode::BreakGlass,
                path: PathOutcome::Clear,
                ..BotIntent::default()
            };
        }
        if definitely_obstructed(shot, target.id) {
            for height in [64.0, 8.0] {
                let candidate = [
                    target.origin[0],
                    target.origin[1],
                    target.origin[2] + height,
                ];
                shot = world.shot_ray(obs.eye(), candidate, obs.self_state.id);
                if shot_allowed(shot, target.id) {
                    aim_at = candidate;
                    break;
                }
                if !definitely_obstructed(shot, target.id) {
                    break;
                }
            }
        }
        if shot.is_ok() {
            self.experience.engagement_mut().record(
                target.id, obs.tick, definitely_obstructed(shot, target.id));
        }
        let range = dist2(obs.self_state.origin, target.origin).sqrt();
        let damage = world.weapon_damage_range(obs.self_state.weapon);
        let limit = engagement_limit(obs.self_state.weapon_class, damage);
        // At longer distances, require both torso and upper-body shot traces.
        // A single pixel-sized line of fire no longer authorizes a cross-map shot.
        let exposure_ok = if range <= limit && range > limit * 0.55
            && shot_allowed(shot, target.id) && fresh_contact(obs, target)
        {
            let upper = [target.origin[0], target.origin[1], target.origin[2] + 60.0];
            let lower = [target.origin[0], target.origin[1], target.origin[2] + 32.0];
            shot_allowed(world.shot_ray(obs.eye(), upper, obs.self_state.id), target.id)
                && shot_allowed(world.shot_ray(obs.eye(), lower, obs.self_state.id), target.id)
        } else {
            true
        };
        let fire = self.authorize_fire(
            range <= limit && exposure_ok
                && obs.tick >= self.reaction_until
                && obs.self_state.ammo_clip != 0
                && fresh_contact(obs, target)
                && shot_allowed(shot, target.id),
            target,
        );
        let hold = self.hold_range(obs);
        let (move_mode, path, move_goal) = self.fight_position(
            obs,
            world,
            nav,
            astar_budget,
            aim_at,
            target,
            shot,
            range <= hold && range <= limit,
        );
        if matches!(
            path,
            PathOutcome::Unreachable | PathOutcome::Blocked | PathOutcome::IncompleteGraph
        ) && !shot_allowed(shot, target.id)
        {
            if let Some(obj) = choose_objective(
                obs,
                world,
                nav,
                astar_budget,
                &mut self.committed,
                &mut self.route_goal,
                &mut self.route_work,
                &self.reservations,
                self.team_slot,
                self.decision.objective,
            ) {
                self.task = Task {
                    kind: TaskKind::TouchObj,
                    reason: SwitchReason::PathFailed,
                };
                self.decision.objective = Some(obj);
                return self.seek(
                    obs,
                    world,
                    nav,
                    astar_budget,
                    SeekKind::Objective(Some(obj)),
                );
            }
            self.task = Task {
                kind: TaskKind::Hunt,
                reason: SwitchReason::PathFailed,
            };
            return self.hunt(obs, world, nav, astar_budget);
        }
        BotIntent {
            look_at: Some(aim_at),
            move_goal,
            desired_range: Some(hold),
            move_mode,
            fire,
            path,
            ..BotIntent::default()
        }
    }

    fn pick_fight_target(&self, obs: &BotObservation) -> Option<Contact> {
        let hold = self.hold_range(obs);
        self.memory
            .currently_seen()
            .max_by(|a, b| {
                fight_score(obs, a, hold, self.last_target).total_cmp(&fight_score(
                    obs,
                    b,
                    hold,
                    self.last_target,
                ))
            })
            .copied()
    }

    fn wounded(&self, obs: &BotObservation) -> bool {
        obs.self_state.health > 0
            && (obs.self_state.health as f32) <= 20.0 + 20.0 * self.self_preserve
    }

    fn hold_range(&self, obs: &BotObservation) -> f32 {
        (obs.self_state.weapon_class.fight_hold() - 40.0 * self.aggression).max(48.0)
    }

    fn fight_break(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        target: Contact,
    ) -> BotIntent {
        let aim_at = [target.origin[0], target.origin[1], target.origin[2] + 48.0];
        let away = away_from(obs.self_state.origin, target.origin, 192.0);
        world.enter(QuerySubsystem::Tactical);
        let range = dist2(obs.self_state.origin, target.origin).sqrt();
        let limit = engagement_limit(
            obs.self_state.weapon_class,
            world.weapon_damage_range(obs.self_state.weapon),
        );
        let fire = self.authorize_fire(
            range <= limit && self.aggression > self.self_preserve
                && obs.tick >= self.reaction_until
                && obs.self_state.ammo_clip != 0
                && fresh_contact(obs, target)
                && shot_allowed(
                    world.shot_ray(obs.eye(), aim_at, obs.self_state.id),
                    target.id,
                ),
            target,
        );
        let (move_mode, path, move_goal) =
            self.route(obs.self_state.origin, away, world, nav, astar_budget);
        BotIntent {
            look_at: Some(aim_at),
            move_goal,
            desired_range: None,
            move_mode,
            fire,
            path,
            ..BotIntent::default()
        }
    }

    /// Where to stand for this engagement. Both the reposition inside weapon
    /// range and the approach from outside it end at the same committed support:
    /// the bot moves to a place it can shoot from, never to the enemy's own
    /// coordinates, which change with every observation.
    #[allow(clippy::too_many_arguments)]
    fn fight_position(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        aim_at: [f32; 3],
        contact: Contact,
        shot: QueryResult<SightSample>,
        in_hold: bool,
    ) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
        let here = obs.self_state.origin;
        let target = contact.origin;
        let target_id = contact.id;
        world.enter(QuerySubsystem::Tactical);
        if shot.is_err() {
            return self.pending_movement(here);
        }
        // Already in range with a line of fire: this is the position.
        if in_hold && shot_allowed(shot, target_id) {
            let since = *self.fight_hold_since.get_or_insert(obs.tick);
            if obs.tick.saturating_sub(since) < FIGHT_REPOSITION_TICKS {
                return (MoveMode::Hold, PathOutcome::Clear, Some(here));
            }
            // Re-evaluate supports periodically, but don't force a blind move.
            self.fight_hold_since = Some(obs.tick);
            self.fight_scan = None;
        } else {
            self.fight_hold_since = None;
        }
        let hold = self.hold_range(obs);
        // The engagement, not the two exact coordinates: a threat that shifts a
        // little leaves the committed position standing. The bot's own position
        // is deliberately absent — walking to the position it chose must not
        // retire the choice.
        let key = FightScanKey {
            target: target_id,
            hold_in: hold as i32,
        };
        let mut scan = match self.fight_scan.take() {
            Some(scan) if scan.answers(key, target, obs.tick) => scan,
            _ => FightScan::begin(
                key,
                target,
                obs.tick,
                self.firing_candidates(obs, nav, target, hold),
            ),
        };
        // A committed position is walked to through the ordinary route executor,
        // which may take a detour; aim keeps following observations meanwhile.
        if let Some(pos) = scan.chosen {
            // Standing on the chosen support without a shot means it did not
            // deliver what it was chosen for; an unreachable one is no better.
            // Either way the next candidate is taken, never the same one again.
            let arrived = dist2(here, pos) <= WAYPOINT_IN * WAYPOINT_IN;
            let moved = (!arrived).then(|| self.route(here, pos, world, nav, astar_budget));
            let usable = moved.is_some_and(|moved| {
                !matches!(
                    moved.1,
                    PathOutcome::Unreachable | PathOutcome::IncompleteGraph
                )
            });
            if let Some(moved) = moved.filter(|_| usable) {
                scan.tick = obs.tick;
                self.fight_scan = Some(scan);
                return moved;
            }
            scan.commit_next(here);
            self.route_goal = None;
            self.committed.clear();
            if let Some(pos) = scan.chosen {
                let moved = self.route(here, pos, world, nav, astar_budget);
                scan.tick = obs.tick;
                self.fight_scan = Some(scan);
                return moved;
            }
        }
        let mut denied = false;
        let opened = scan.cursor;
        while (scan.cursor as usize) < scan.candidates.len() {
            let cand = scan.candidates[scan.cursor as usize];
            let eye = [
                cand[0],
                cand[1],
                cand[2]
                    + if obs.self_state.view_height > 1.0 {
                        obs.self_state.view_height
                    } else {
                        60.0
                    },
            ];
            let shot = world.shot_ray(eye, aim_at, obs.self_state.id);
            if shot.is_err() {
                denied = true;
                break;
            }
            if !shot_allowed(shot, target_id) {
                scan.cursor += 1;
                continue;
            }
            let threat_eye = [target[0], target[1], target[2] + 60.0];
            let chest = [cand[0], cand[1], cand[2] + 48.0];
            let return_fire = world.shot_ray(threat_eye, chest, target_id);
            if return_fire.is_err() {
                denied = true;
                break;
            }
            let exposed = return_fire_clear(return_fire, obs.self_state.id);
            // Travel and exposure in one comparable scale; reachability is left
            // to the route, so a position behind a corner is not thrown away.
            let score = dist2(here, cand).sqrt() + if exposed { 320.0 } else { 0.0 };
            scan.accepted.push((score, cand));
            scan.cursor += 1;
        }
        if !denied && scan.cursor as usize >= scan.candidates.len() {
            scan.commit_next(here);
        }
        let chosen = scan.chosen;
        let anchor = scan.anchor;
        // A visit that neither advanced the scan nor holds a position is not
        // progress, so it must not keep an exhausted scan alive: letting it age
        // out is what allows the engagement to be reconsidered.
        if scan.cursor != opened || chosen.is_some() {
            scan.tick = obs.tick;
        }
        self.fight_scan = Some(scan);
        let Some(pos) = chosen else {
            let path = if denied {
                PathOutcome::BudgetExhausted
            } else {
                PathOutcome::Blocked
            };
            // A visible enemy is not necessarily shootable. In range, let the
            // caller resume another goal when every firing position failed; out
            // of range, close on the engagement's anchor — a destination that
            // does not move with every observation and invalidate its own search.
            if denied {
                return self.pending_movement(here);
            }
            if in_hold {
                return (MoveMode::Hold, path, Some(here));
            }
            return self.route(here, anchor, world, nav, astar_budget);
        };
        self.route(here, pos, world, nav, astar_budget)
    }

    /// The bounded candidate set a scan is frozen on. Supports come from the
    /// baked graph; the ring is only for a runtime that has no graph at all,
    /// where there is no support to choose from.
    fn firing_candidates(
        &self,
        obs: &BotObservation,
        nav: Option<&NavGraph>,
        target: [f32; 3],
        hold: f32,
    ) -> Vec<[f32; 3]> {
        let here = obs.self_state.origin;
        let mut candidates = if let Some(graph) = nav.filter(|graph| !graph.is_empty()) {
            let supports = nav::firing_supports(graph, here, target, hold, FIGHT_CANDIDATES);
            if !supports.is_empty() {
                supports
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        if candidates.is_empty() {
            candidates = (0..FIGHT_RING)
                .map(|step| {
                    let ang = (step as f32) * (core::f32::consts::TAU / FIGHT_RING as f32);
                    [
                        target[0] + hold * ang.cos(),
                        target[1] + hold * ang.sin(),
                        here[2],
                    ]
                })
                .collect();
        }
        candidates.sort_by(|a, b| {
            reservation_penalty(obs.self_state.id, *a, &self.reservations)
                .total_cmp(&reservation_penalty(obs.self_state.id, *b, &self.reservations))
        });
        candidates
    }

    fn seek(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        kind: SeekKind,
    ) -> BotIntent {
        let (goal, use_button, look) = match kind {
            SeekKind::LastSeen => {
                let Some(last) = self.memory.last_seen().next().copied() else {
                    return BotIntent::default();
                };
                (last.origin, false, last.origin)
            }
            SeekKind::Objective(obj) => {
                let Some(obj) = obj else {
                    return BotIntent::default();
                };
                if obj.touching && obj.use_button {
                    return BotIntent {
                        look_at: Some(obj.origin),
                        move_goal: Some(obs.self_state.origin),
                        move_mode: MoveMode::Hold,
                        path: PathOutcome::Clear,
                        use_button: true,
                        ..BotIntent::default()
                    };
                }
                if obj.action == ObjectiveAction::Defend
                    || (obj.role == TeamRole::Cover && obj.active_user.is_some())
                {
                    let guard = guard_slot(obj.origin, obs.self_state.id);
                    if dist_xy(obs.self_state.origin, obj.origin) <= 220.0
                        && (obs.self_state.origin[2] - obj.origin[2]).abs()
                            <= nav::STEP_Z_IN * 2.0 + 8.0
                    {
                        let (move_mode, path, move_goal) =
                            self.route(obs.self_state.origin, guard, world, nav, astar_budget);
                        return BotIntent {
                            look_at: Some([guard[0], guard[1], guard[2] + 60.0]),
                            move_goal: (move_mode != MoveMode::Hold).then_some(move_goal).flatten(),
                            move_mode,
                            path,
                            ..BotIntent::default()
                        };
                    }
                }
                (
                    obj.origin,
                    false,
                    [obj.origin[0], obj.origin[1], obs.eye()[2]],
                )
            }
            SeekKind::BackOff => {
                let threat = self
                    .memory
                    .last_seen()
                    .next()
                    .or_else(|| self.memory.currently_seen().next())
                    .map(|c| c.origin)
                    .unwrap_or([
                        obs.self_state.origin[0] + 64.0,
                        obs.self_state.origin[1],
                        obs.self_state.origin[2],
                    ]);
                let away = away_from(obs.self_state.origin, threat, 192.0);
                (away, false, threat)
            }
        };
        let (move_mode, path, move_goal) =
            self.route(obs.self_state.origin, goal, world, nav, astar_budget);
        BotIntent {
            look_at: Some(look),
            move_goal,
            desired_range: None,
            move_mode,
            use_button,
            crouch: matches!(kind, SeekKind::BackOff),
            path,
            ..BotIntent::default()
        }
    }

    fn remember_failed_goal(&mut self, goal: [f32; 3], tick: u32) {
        self.experience.remember_failed_route(goal, tick);
    }

    // Query-budget denial isn't evidence of a wall. Retain only an existing
    // clear, walking waypoint; never infer that an unvalidated step is safe.
    fn pending_movement(&self, here: [f32; 3]) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
        if self.intent.path == PathOutcome::Clear
            && matches!(self.intent.move_mode, MoveMode::Walk | MoveMode::Drop)
            && self.intent.move_goal.is_some_and(|goal| dist2(here, goal) > WAYPOINT_IN * WAYPOINT_IN)
        {
            return (self.intent.move_mode, PathOutcome::Clear, self.intent.move_goal);
        }
        (MoveMode::Hold, PathOutcome::BudgetExhausted, Some(here))
    }

    fn route(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
        if let Some(graph) = nav.filter(|g| !g.is_empty()) {
            return self.route_nav(from, to, world, graph, astar_budget);
        }
        walk_step(from, to, world)
    }

    fn route_nav(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        world: &mut impl WorldQuery,
        graph: &NavGraph,
        astar_budget: &mut u32,
    ) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
        let need = self.committed.is_empty()
            || self
                .route_goal
                .is_none_or(|g| dist2(g, to) > WAYPOINT_IN * WAYPOINT_IN);
        if need {
            let objective = self.decision.objective.filter(|o| o.origin == to);
            match nav::find_route_resumable(
                world,
                graph,
                from,
                to,
                objective,
                astar_budget,
                &mut self.route_work,
            ) {
                Ok(path) => {
                    self.committed = path;
                    self.route_goal = Some(to);
                }
                Err(PathError::IncompleteGraph) => {
                    return (MoveMode::Hold, PathOutcome::IncompleteGraph, Some(to));
                }
                Err(PathError::BudgetExhausted { .. }) => {
                    if self.committed.is_empty() || self.route_goal.is_none_or(|goal| dist2(goal, to) > WAYPOINT_IN * WAYPOINT_IN) {
                        return (MoveMode::Hold, PathOutcome::BudgetExhausted, Some(to));
                    }
                    // Existing route has the same goal; keep executing its verified steps.
                }
                Err(
                    PathError::GraphChanged
                    | PathError::Unreachable
                    | PathError::EmptyGraph
                    | PathError::NoStartSupport
                    | PathError::NoGoalSupport,
                ) => {
                    // Rate-limit expensive directed connectivity diagnosis.
                    // The topology result is explanatory, never a substitute
                    // for the collision-validated route planner.
                    let tick = self.last_think_tick.unwrap_or(0);
                    if self.last_nav_diagnostic_tick.is_none_or(|last| tick.saturating_sub(last) >= 200) {
                        self.last_nav_diagnostic_tick = Some(tick);
                        let topology = nav::route_topology(graph, from, to);
                        diag::info!(Sim,
                            "bots: nav_route_failure tick={} from=({:.0},{:.0},{:.0}) to=({:.0},{:.0},{:.0}) topology={:?}",
                            tick, from[0], from[1], from[2], to[0], to[1], to[2], topology);
                    }
                    self.committed.clear();
                    return (MoveMode::Hold, PathOutcome::Unreachable, Some(to));
                }
            }
        }
        world.enter(QuerySubsystem::Execution);
        while let Some(step) = self.committed.first().copied() {
            let tolerance = if self.committed.len() == 1 && self.decision.objective.is_some() {
                4.0
            } else {
                WAYPOINT_IN
            };
            if dist2(from, step.position) > tolerance * tolerance {
                break;
            }
            if let Some(next) = self.committed.get(1) {
                // A special transition starts at its entry; do not cut its corner.
                if next.kind != nav::TraversalKind::Walk && dist2(from, next.entry) > 4.0 * 4.0 {
                    break;
                }
                if next.kind == nav::TraversalKind::Walk
                    && world.walk_hull(from, next.position) != Ok(WalkSample::Clear)
                    && dist_xy(from, step.position) > 4.0
                {
                    break;
                }
            }
            self.committed.remove(0);
        }
        let Some(step) = self.committed.first().copied() else {
            return (MoveMode::Hold, PathOutcome::Clear, Some(from));
        };
        let sample = if step.kind == nav::TraversalKind::Drop {
            nav::drop_clear(world, from, step.position)
        } else {
            world.walk_hull(from, step.position)
        };
        match sample {
            Ok(WalkSample::Clear) => {
                self.route_blocked_since = None;
                (
                if step.kind == nav::TraversalKind::Drop {
                    MoveMode::Drop
                } else {
                    MoveMode::Walk
                },
                PathOutcome::Clear,
                Some(step.position),
                )
            },
            Ok(WalkSample::BreakGlass) => (
                MoveMode::BreakGlass,
                PathOutcome::Clear,
                Some(step.position),
            ),
            Ok(WalkSample::Blocked) => {
                let attempts = self.route_blocked_since.get_or_insert(0);
                *attempts = attempts.saturating_add(1);
                // Try the native jump/mantle input only at a *nearby* upward
                // obstacle. This is not permission to traverse arbitrary gaps
                // or climb tall walls: normal blocked-route recovery still runs.
                // Pulse the attempt rather than holding jump every tick.
                let rise = step.position[2] - from[2];
                let near = dist_xy(from, step.position) <= 72.0;
                if near && (8.0..=48.0).contains(&rise)
                    && matches!(*attempts, 2 | 5 | 8)
                {
                    return (MoveMode::Jump, PathOutcome::Clear, Some(step.position));
                }
                if *attempts >= ROUTE_RETRY_TICKS / THINK_PERIOD_TICKS {
                    self.committed.clear();
                    self.route_goal = None;
                    self.route_work.cancel();
                    self.route_blocked_since = None;
                    self.roam = None;
                    self.fight_scan = None;
                }
                (MoveMode::Hold, PathOutcome::Blocked, Some(step.position))
            }
            Err(_) => (MoveMode::Hold, PathOutcome::BudgetExhausted, Some(step.position)),
        }
    }
}

/// The engagement a firing position was chosen for: a different threat or a
/// different weapon band retires the choice. The bot's own movement towards the
/// position does not, and neither does a small move by the threat — that is the
/// `anchor` distance below, not a grid cell, so no boundary makes one step
/// count as a new engagement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FightScanKey {
    target: ClientId,
    hold_in: i32,
}

/// One bounded evaluation of firing positions and the position it committed to.
/// The candidate list is frozen when the scan begins, so a denied slice resumes
/// at the same candidate instead of re-deriving the set.
#[derive(Clone, Debug, PartialEq)]
struct FightScan {
    key: FightScanKey,
    /// Where the threat was when this set was derived.
    anchor: [f32; 3],
    tick: u32,
    cursor: u32,
    candidates: Vec<[f32; 3]>,
    /// Candidates that passed their shot and exposure checks, with their score.
    /// A position that is taken or rejected leaves this list, so the next choice
    /// costs no queries and the same one is never re-picked.
    accepted: Vec<(f32, [f32; 3])>,
    chosen: Option<[f32; 3]>,
}

impl FightScan {
    fn begin(key: FightScanKey, anchor: [f32; 3], tick: u32, candidates: Vec<[f32; 3]>) -> Self {
        Self {
            key,
            anchor,
            tick,
            cursor: 0,
            candidates,
            accepted: Vec::new(),
            chosen: None,
        }
    }

    /// Whether this scan still answers the engagement in front of the bot.
    fn answers(&self, key: FightScanKey, at: [f32; 3], tick: u32) -> bool {
        self.key == key
            && dist2(self.anchor, at) <= FIGHT_COMMIT_MOVE * FIGHT_COMMIT_MOVE
            && !self.expired(tick)
    }

    /// Commits to the cheapest remaining accepted position the bot is not
    /// already standing on — one that close has just failed its own shot test
    /// from here, and the motor could not travel to it anyway.
    fn commit_next(&mut self, here: [f32; 3]) -> Option<[f32; 3]> {
        self.chosen = None;
        loop {
            let at = self
                .accepted
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
                .map(|(index, _)| index)?;
            let pos = self.accepted.remove(at).1;
            if dist2(here, pos) > WAYPOINT_IN * WAYPOINT_IN {
                self.chosen = Some(pos);
                return self.chosen;
            }
        }
    }

    /// An unfinished scan is short-lived; a commitment lives as long as the
    /// engagement it was made for, and no longer.
    fn expired(&self, tick: u32) -> bool {
        let age = tick.saturating_sub(self.tick);
        if self.chosen.is_some() {
            age > FIGHT_COMMIT_MAX_AGE_TICKS
        } else {
            age > FIGHT_SCAN_MAX_AGE_TICKS
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SeekKind {
    LastSeen,
    Objective(Option<ModeObjective>),
    BackOff,
}

fn objective_utility(obs: &BotObservation, obj: ModeObjective, route_length: Option<f32>) -> f32 {
    let travel_ms = route_length.unwrap_or_else(|| dist2(obs.self_state.origin, obj.origin).sqrt())
        / 180.0
        * 1000.0;
    let cost = (travel_ms / 1000.0).min(25.0);
    let interaction_left = obj.interaction_ms as f32 * (1.0 - obj.progress.clamp(0.0, 1.0));
    if obj.action == ObjectiveAction::Defuse
        && obj.role == TeamRole::Actor
        && obj.remaining_ms.is_some_and(|ms| {
            (ms as f32) < interaction_left + if obj.touching { 0.0 } else { travel_ms }
        })
    {
        return -1.0;
    }
    let mut value = match obj.action {
        ObjectiveAction::Capture => 40.0 - cost,
        ObjectiveAction::Plant => 70.0 - cost,
        ObjectiveAction::Defend => 55.0 - cost,
        ObjectiveAction::Defuse => {
            let needed =
                travel_ms + obj.interaction_ms as f32 * (1.0 - obj.progress.clamp(0.0, 1.0));
            let slack = obj.remaining_ms.unwrap_or(u32::MAX) as f32 - needed;
            if slack < 0.0 {
                20.0
            } else {
                110.0 + (30.0 - slack / 1000.0).clamp(0.0, 30.0)
            }
        }
    };
    if obj.role == TeamRole::Cover {
        value = value.min(62.0);
    }
    if obj.touching && obj.use_button {
        value = 200.0;
    }
    value
}

/// Single objective scoring policy shared by strategic selection and route validation.
fn objective_score(
    obs: &BotObservation,
    obj: ModeObjective,
    route_length: Option<f32>,
    team_slot: TeamSlot,
    route_goal: Option<[f32; 3]>,
    reservations: &[(ClientId, [f32; 3])],
) -> f32 {
    objective_utility(obs, obj, route_length)
        + assignment_bonus(team_slot, obj, &obs.objectives)
        - reservation_penalty(obs.self_state.id, obj.origin, reservations)
        + if route_goal == Some(obj.origin) { 6.0 } else { 0.0 }
}

fn choose_objective(
    obs: &BotObservation,
    world: &mut impl WorldQuery,
    nav: Option<&NavGraph>,
    astar_budget: &mut u32,
    committed: &mut Vec<nav::RouteStep>,
    route_goal: &mut Option<[f32; 3]>,
    route_work: &mut nav::RouteWork,
    reservations: &[(ClientId, [f32; 3])],
    team_slot: TeamSlot,
    preferred: Option<ModeObjective>,
) -> Option<ModeObjective> {
    let mut candidates = obs.objectives.clone();
    if candidates.is_empty()
        && let Some(obj) = obs.objective
    {
        candidates.push(obj);
    }
    candidates.sort_by(|a, b| {
        let score = |o: &ModeObjective| {
            objective_score(obs, *o, None, team_slot, *route_goal, reservations)
        };
        // Preserve an active commitment through route validation unless
        // its path is actually impossible. Touching interactions still win.
        let priority = |o: &ModeObjective| {
            if o.touching && o.use_button { 2 } else if preferred.is_some_and(|p| p.id == o.id && p.round == o.round) { 1 } else { 0 }
        };
        priority(b).cmp(&priority(a))
            .then_with(|| score(b).total_cmp(&score(a)))
            .then_with(|| a.id.cmp(&b.id))
    });
    let Some(graph) = nav.filter(|graph| !graph.is_empty()) else {
        return candidates.first().copied();
    };
    let mut deferred: Option<ModeObjective> = None;
    for obj in candidates {
        if *route_goal == Some(obj.origin)
            && (obj.touching || path_ends_in_use_volume(world, committed, obj))
        {
            return Some(obj);
        }
        match nav::find_route_resumable(
            world,
            graph,
            obs.self_state.origin,
            obj.origin,
            Some(obj),
            astar_budget,
            route_work,
        ) {
            Ok(path) => {
                if path_ends_in_use_volume(world, &path, obj) {
                    *committed = path;
                    *route_goal = Some(obj.origin);
                    return Some(obj);
                }
            }
            // Budget-deferred work stays pending: stop here and keep the slot.
            Err(PathError::BudgetExhausted { .. }) => {
                deferred = Some(obj);
                break;
            }
            // Coverage that cannot answer this request is not proof of global
            // unreachability, and waiting does not extend the published graph.
            // Record it and let a lower-scoring, routable objective be tried.
            Err(
                PathError::GraphChanged
                | PathError::IncompleteGraph
                | PathError::Unreachable
                | PathError::EmptyGraph
                | PathError::NoStartSupport
                | PathError::NoGoalSupport,
            ) => {}
        }
    }
    deferred
}

/// Prefer the actual weapon's falloff curve, while retaining a class cap for
/// no-falloff guns and a fallback when content is not yet loaded. A valid
/// finite bullet range is an absolute ceiling; the tactical limit is stricter.
fn engagement_limit(class: WeaponClass, damage: Option<WeaponDamageRange>) -> f32 {
    let class_cap = match class {
        WeaponClass::Assault => ASSAULT_FIRE_MAX,
        WeaponClass::Smg => SMG_FIRE_MAX,
        WeaponClass::Lmg => LMG_FIRE_MAX,
        WeaponClass::Sniper => SNIPER_FIRE_MAX,
        WeaponClass::Shotgun => SHOTGUN_FIRE_MAX,
    };
    let Some(damage) = damage else { return class_cap };
    let mut cap = class_cap;
    if damage.min_damage > 0 && damage.damage > damage.min_damage
        && damage.minimum_damage_range.is_finite()
        && damage.minimum_damage_range > 0.0
    {
        // Allow some space beyond the final falloff breakpoint, but never
        // make a short-range weapon behave like a sniper.
        cap = cap.min(damage.minimum_damage_range * 1.15);
    }
    if damage.bullet_range.is_finite() && damage.bullet_range > 0.0 {
        cap = cap.min(damage.bullet_range);
    }
    cap.max(64.0)
}

#[cfg(test)]
mod engagement_tests {
    use super::*;

    #[test]
    fn real_falloff_limits_smg() {
        let curve = WeaponDamageRange { damage: 40, min_damage: 20,
            full_damage_range: 600.0, minimum_damage_range: 850.0,
            bullet_range: 8000.0 };
        assert_eq!(engagement_limit(WeaponClass::Smg, Some(curve)), 977.5);
    }

    #[test]
    fn no_falloff_still_has_class_limit() {
        let curve = WeaponDamageRange { damage: 70, min_damage: 70,
            full_damage_range: 0.0, minimum_damage_range: 0.0,
            bullet_range: 9000.0 };
        assert_eq!(engagement_limit(WeaponClass::Sniper, Some(curve)), 4500.0);
    }

    #[test]
    fn never_exceeds_physical_bullet_range() {
        let curve = WeaponDamageRange { damage: 40, min_damage: 20,
            full_damage_range: 100.0, minimum_damage_range: 300.0,
            bullet_range: 250.0 };
        assert_eq!(engagement_limit(WeaponClass::Shotgun, Some(curve)), 250.0);
    }
}

/// A positive observation of this contact, recent enough to shoot at. A contact
/// held through unsensed ticks still aims and moves; it does not fire.
fn fresh_contact(obs: &BotObservation, target: Contact) -> bool {
    target.source == KnowledgeSource::CurrentlySeen
        && obs.tick.saturating_sub(target.seen_tick) <= FIRE_FRESH_TICKS
}

fn shot_allowed(sample: QueryResult<SightSample>, target: ClientId) -> bool {
    match sample {
        Ok(SightSample::Clear) => true,
        Ok(SightSample::HitPlayer { client, .. }) => client == target,
        Ok(SightSample::Unknown | SightSample::Blocked { .. }) | Err(_) => false,
    }
}

/// A trace that ran and found something in the way. A denied or unclassified
/// trace says nothing about the geometry, so it is not an obstruction.
fn definitely_obstructed(sample: QueryResult<SightSample>, target: ClientId) -> bool {
    matches!(
        sample,
        Ok(SightSample::Blocked { .. }) | Ok(SightSample::HitPlayer { .. })
    ) && !shot_allowed(sample, target)
}

fn return_fire_clear(sample: QueryResult<SightSample>, bot: ClientId) -> bool {
    match sample {
        Ok(SightSample::Clear) => true,
        Ok(SightSample::HitPlayer { client, .. }) => client == bot,
        Ok(SightSample::Unknown | SightSample::Blocked { .. }) | Err(_) => false,
    }
}

fn fight_score(obs: &BotObservation, contact: &Contact, hold: f32, last: Option<ClientId>) -> f32 {
    let range = dist2(obs.self_state.origin, contact.origin).sqrt();
    let dist = 1.0 - (range - hold).abs() / (hold + 80.0);
    let stick = if last == Some(contact.id) { 0.4 } else { 0.0 };
    dist + stick
}

fn away_from(here: [f32; 3], threat: [f32; 3], dist: f32) -> [f32; 3] {
    let dx = here[0] - threat[0];
    let dy = here[1] - threat[1];
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    [
        here[0] + dx / len * dist,
        here[1] + dy / len * dist,
        here[2],
    ]
}

fn path_ends_in_use_volume(
    world: &impl WorldQuery,
    path: &[nav::RouteStep],
    obj: ModeObjective,
) -> bool {
    path.last()
        .is_some_and(|end| world.objective_contains(obj, end.position))
}

fn reservation_penalty(
    client: ClientId,
    candidate: [f32; 3],
    reservations: &[(ClientId, [f32; 3])],
) -> f32 {
    reservations
        .iter()
        .filter(|(owner, goal)| {
            *owner != client && dist_xy(candidate, *goal) < GOAL_RESERVATION_RADIUS
        })
        .count() as f32
        * GOAL_RESERVATION_PENALTY
}

fn guard_slot(origin: [f32; 3], client: ClientId) -> [f32; 3] {
    let angle = (client.0 % 8) as f32 * core::f32::consts::TAU / 8.0;
    [
        origin[0] + angle.cos() * 144.0,
        origin[1] + angle.sin() * 144.0,
        origin[2],
    ]
}

fn side_step_targets(from: [f32; 3], to: [f32; 3]) -> [[f32; 3]; 3] {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let forward = [dx / length, dy / length];
    let diagonal = core::f32::consts::FRAC_1_SQRT_2;
    [
        [
            from[0] + STEP_IN * (forward[0] - forward[1]) * diagonal,
            from[1] + STEP_IN * (forward[1] + forward[0]) * diagonal,
            from[2],
        ],
        [
            from[0] + STEP_IN * (forward[0] + forward[1]) * diagonal,
            from[1] + STEP_IN * (forward[1] - forward[0]) * diagonal,
            from[2],
        ],
        [from[0] - STEP_IN * forward[0], from[1] - STEP_IN * forward[1], from[2]],
    ]
}

fn walk_step(
    from: [f32; 3],
    to: [f32; 3],
    world: &mut impl WorldQuery,
) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
    let d = [to[0] - from[0], to[1] - from[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len < 8.0 {
        return (MoveMode::Hold, PathOutcome::Clear, Some(to));
    }
    let scale = (STEP_IN / len).min(1.0);
    let step = [from[0] + d[0] * scale, from[1] + d[1] * scale, from[2]];
    match world.walk_hull(from, step) {
        Ok(WalkSample::Clear) => (MoveMode::Walk, PathOutcome::Clear, Some(to)),
        Ok(WalkSample::BreakGlass) => (MoveMode::BreakGlass, PathOutcome::Clear, Some(step)),
        Ok(WalkSample::Blocked) => {
            for detour in side_step_targets(from, to) {
                match world.walk_hull(from, detour) {
                    Ok(WalkSample::Clear) => {
                        return (MoveMode::Walk, PathOutcome::Clear, Some(detour));
                    }
                    Ok(WalkSample::BreakGlass) => {
                        return (MoveMode::BreakGlass, PathOutcome::Clear, Some(detour));
                    }
                    Ok(WalkSample::Blocked) | Err(_) => {}
                }
            }
            let path = if len > STEP_IN {
                PathOutcome::Unreachable
            } else {
                PathOutcome::Blocked
            };
            (MoveMode::Hold, path, Some(to))
        }
        Err(_) => (MoveMode::Hold, PathOutcome::BudgetExhausted, Some(to)),
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

fn dist_xy(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}
