//! Safe host adapter around the pinned, unmodified Recompiled bot brain.
//! All calls happen in the authority system; the C++ bridge serializes its
//! process-global state. Callbacks are synchronous and never retained.
use playerstate_iw4::UserCmd;
use sim::{BulletTraceQuery, SimWorld, TraceOutcome};
use sim::{ClientId, ClientLifecycle, Snapshot};
use std::ffi::{CString, c_char, c_void};

struct TraceContext<'a> {
    world: &'a mut SimWorld,
    calls: u32,
}

#[repr(C)]
struct Player {
    num: u32,
    team: u32,
    alive: u32,
    grounded: u32,
    health: i32,
    origin: [f32; 3],
    view: [f32; 3],
}
#[repr(C)]
#[derive(Default)]
struct Command {
    pitch: f32,
    yaw: f32,
    forward: i32,
    right: i32,
    buttons: u32,
}
unsafe extern "C" {
    fn rc_reset(map: *const c_char, csv: *const c_char) -> bool;
    fn rc_dead(num: u32);
    fn rc_think(
        num: u32,
        players: *const Player,
        count: usize,
        time: f64,
        context: *mut c_void,
        visible: unsafe extern "C" fn(*mut c_void, *const f32, *const f32, u32) -> bool,
        out: *mut Command,
    ) -> bool;
}

pub fn reset(map: &str, csv: &str) -> bool {
    let (Ok(map), Ok(csv)) = (CString::new(map), CString::new(csv)) else {
        return false;
    };
    // SAFETY: valid terminated strings, consumed within this call.
    unsafe { rc_reset(map.as_ptr(), csv.as_ptr()) }
}
pub fn dead(num: u32) {
    // SAFETY: upstream bounds-checks this id and the bridge serializes access.
    unsafe { rc_dead(num) }
}

unsafe extern "C" fn visible(
    context: *mut c_void,
    a: *const f32,
    b: *const f32,
    pass: u32,
) -> bool {
    // SAFETY: bridge passes two 3-float arrays and the live trace context supplied to rc_think.
    // A panic must never unwind through C++.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let context = unsafe { &mut *context.cast::<TraceContext<'_>>() };
        context.calls += 1;
        let a = unsafe { [*a, *a.add(1), *a.add(2)] };
        let b = unsafe { [*b, *b.add(1), *b.add(2)] };
        // The exact mask used by Recompiled's G_SightTrace adapter. This
        // excludes player bodies, rather than allowing any first player hit
        // to incorrectly make a wall behind that player transparent.
        matches!(
            context.world.sensor_trace(BulletTraceQuery {
                start: a,
                end: b,
                mask: 0x00801803,
                ignore: Some(ClientId(pass)),
                ignore_hit: None,
                ignore_model: None,
            }),
            TraceOutcome::Miss { .. }
        )
    }))
    .unwrap_or(false)
}

pub fn think(
    id: ClientId,
    snapshot: &Snapshot,
    world: &mut SimWorld,
    time_ms: i32,
) -> Option<(UserCmd, u32)> {
    let ps = &snapshot.players.iter().find(|(client, _)| *client == id)?.1;
    let players: Vec<_> = snapshot
        .players
        .iter()
        .filter_map(|(client, ps)| {
            let meta = snapshot.meta.for_client(*client)?;
            Some(Player {
                num: client.0,
                team: if snapshot.meta.kind.is_team() {
                    meta.client_state_team as u32
                } else {
                    0
                },
                alive: u32::from(meta.lifecycle == ClientLifecycle::Alive),
                grounded: u32::from(ps.ground_entity_num != 2047),
                health: ps.health,
                origin: ps.origin,
                view: ps.viewangles,
            })
        })
        .collect();
    let mut out = Command::default();
    let mut context = TraceContext { world, calls: 0 };
    // SAFETY: all buffers and callback context outlive this synchronous call.
    let ok = unsafe {
        rc_think(
            id.0,
            players.as_ptr(),
            players.len(),
            f64::from(time_ms) / 1000.0,
            (&raw mut context).cast(),
            visible,
            &mut out,
        )
    };
    if !ok {
        return None;
    }
    Some((
        UserCmd {
            server_time: time_ms,
            angles: net::look_angles_from_degrees([
                out.pitch - ps.delta_angles[0],
                out.yaw - ps.delta_angles[1],
                0.0,
            ]),
            forwardmove: out.forward.clamp(-127, 127) as i8,
            rightmove: out.right.clamp(-127, 127) as i8,
            buttons: out.buttons,
            weapon: ps.weapon as u16,
            weapon_mapped: ps.weapon as u16,
            ..UserCmd::default()
        },
        context.calls,
    ))
}
