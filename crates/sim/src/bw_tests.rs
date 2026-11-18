use super::*;
use crate::script::{Catalog, Namespace, NativeRegistry, Program, Runtime, Value};
use std::collections::BTreeMap;

fn world(source: &str) -> SimWorld {
    let sources = BTreeMap::from([("maps/mp/bots/test".into(), source.into())]);
    let program = Program::load(&sources, &["maps/mp/bots/test"], &Catalog::iw4()).unwrap();
    let mut world = SimWorld::new();
    world
        .install_gsc_program(program, NativeRegistry::default(), Default::default())
        .unwrap();
    world
}
fn call(
    world: &mut SimWorld,
    name: &str,
    receiver: &Value,
    args: &[Value],
) -> Result<Value, String> {
    let native = world
        .ecs
        .resource::<NativeRegistry>()
        .get(Namespace::Method, name)
        .unwrap();
    native(&mut world.ecs, receiver, args)
}
#[test]
fn bot_warfare_input_is_held_and_human_control_is_rejected() {
    let mut world = world("main() { level.result = 1; }");
    let object = world
        .ecs
        .resource_mut::<Runtime>()
        .create_player(17)
        .unwrap();
    let human = world
        .ecs
        .resource_mut::<Runtime>()
        .create_player(0)
        .unwrap();
    world
        .ecs
        .resource_mut::<Runtime>()
        .bw
        .inputs
        .insert(17, Default::default());
    let receiver = Value::Object(object);
    call(
        &mut world,
        "botaction",
        &receiver,
        &[Value::string("+fire")],
    )
    .unwrap();
    call(
        &mut world,
        "botmovement",
        &receiver,
        &[Value::Int(200), Value::Int(-200)],
    )
    .unwrap();
    let bot = &world.ecs.resource::<Runtime>().bw.inputs[&17];
    assert_eq!(bot.buttons, playerstate_iw4::buttons::ATTACK);
    assert_eq!((bot.forward, bot.right), (127, -128));
    call(
        &mut world,
        "botaction",
        &receiver,
        &[Value::string("+sprint")],
    )
    .unwrap();
    call(
        &mut world,
        "botaction",
        &receiver,
        &[Value::string("-fire")],
    )
    .unwrap();
    assert_eq!(
        world.ecs.resource::<Runtime>().bw.inputs[&17].buttons,
        playerstate_iw4::buttons::SPRINT
    );
    assert!(
        call(
            &mut world,
            "botaction",
            &Value::Object(human),
            &[Value::string("+fire")]
        )
        .is_err()
    );
    assert!(call(&mut world, "botaction", &receiver, &[Value::string("fire")]).is_err());
    assert!(
        call(
            &mut world,
            "botaction",
            &receiver,
            &[Value::string("+invalid")]
        )
        .is_err()
    );
    call(&mut world, "botstop", &receiver, &[]).unwrap();
    let bot = &world.ecs.resource::<Runtime>().bw.inputs[&17];
    assert_eq!((bot.buttons, bot.forward, bot.right), (0, 0, 0));
}
#[test]
fn bot_warfare_profiles_nested_script_threads_and_errors() {
    let mut world = world(
        "main() { level.result=0; thread child(); child(); level.result=level.result+1; self botaction(\"bad\"); } child() { a=[]; a[0]=spawnstruct(); level.result=level.result+1; }",
    );
    world
        .start_gsc("maps/mp/bots/test::main", Value::level(), vec![])
        .unwrap();
    step(
        &mut world,
        Tick(1),
        &TickInput::default(),
        50,
        StepReason::AuthorityFrame,
    );
    let mut runtime = world.ecs.resource_mut::<Runtime>();
    assert_eq!(runtime.object_field(0, "result"), Value::Int(3));
    assert!(runtime.benchmark.spans.is_empty());
    assert!(runtime.benchmark.resumes >= 2);
    assert!(
        runtime
            .benchmark
            .functions
            .iter()
            .map(|s| s.instructions)
            .sum::<u64>()
            > 20
    );
    assert!(runtime.benchmark.allocations >= 4);
    assert_eq!(runtime.errors.values().sum::<u64>(), 1);
    assert!(runtime.benchmark.natives.iter().any(|s| s.errors == 1));
    assert!(runtime.benchmark.io_errors == 0);
}
#[test]
fn bot_warfare_diagnostics_bound_queue_and_sequence() {
    let mut world = world("main() { }");
    let mut runtime = world.ecs.resource_mut::<Runtime>();
    let b = &mut runtime.benchmark;
    b.pending.clear();
    for _ in 0..5000 {
        b.queue(serde_json::json!({"type":"test"}));
    }
    assert_eq!(b.pending.len(), 4096);
    assert_eq!(b.dropped, 904);
    assert!(
        b.pending
            .windows(2)
            .all(|p| p[0]["sequence"].as_u64() < p[1]["sequence"].as_u64())
    );
}

#[test]
fn bot_warfare_new_life_clears_previous_controls_before_new_input() {
    let mut world = world("main() { }");
    let object = world
        .ecs
        .resource_mut::<Runtime>()
        .create_player(17)
        .unwrap();
    world
        .ecs
        .resource_mut::<Runtime>()
        .bw
        .inputs
        .insert(17, Default::default());
    {
        let mut frame = FrameWorld::from_world(&mut world.ecs);
        frame.ensure_player(ClientId(17)).viewangles = [0., 45., 0.];
        frame.client_meta_mut(ClientId(17)).life_sequence = crate::LifeSequence(1);
    }
    let receiver = Value::Object(object);
    call(
        &mut world,
        "botaction",
        &receiver,
        &[Value::string("+fire")],
    )
    .unwrap();
    call(
        &mut world,
        "botangles",
        &receiver,
        &[Value::Int(0), Value::Int(180), Value::Int(0)],
    )
    .unwrap();
    {
        let mut frame = FrameWorld::from_world(&mut world.ecs);
        frame.client_meta_mut(ClientId(17)).life_sequence = crate::LifeSequence(2);
        frame.player_mut(ClientId(17)).unwrap().viewangles = [0., 90., 0.];
    }
    call(
        &mut world,
        "botmovement",
        &receiver,
        &[Value::Int(127), Value::Int(0)],
    )
    .unwrap();
    let b = &world.ecs.resource::<Runtime>().bw.inputs[&17];
    assert_eq!(b.buttons, 0);
    assert_eq!(b.forward, 127);
    assert_eq!(b.angles, [0., 90., 0.]);
}

#[test]
fn bot_warfare_admission_connect_begin_disconnect_and_slot_reuse() {
    use structured_data_iw4::{DataType, Definition, DefinitionSet};
    let sources=BTreeMap::from([
        ("maps/mp/bots/test".into(),"main() { level.begins=0; if(getdvarint(\"fixture_started\")==0) { level.first=addtestclient(); level.second=addtestclient(); setdvar(\"fixture_started\",1); } } one() { level.replacement=addtestclient(); } restart(persist) { map_restart(persist); }".into()),
        ("maps/mp/gametypes/_callbacksetup".into(),"codecallback_playerconnect() { self waittill(\"begin\"); level.begins++; } codecallback_playerdisconnect() { self notify(\"disconnect\"); }".into()),
    ]);
    let program = Program::load(
        &sources,
        &["maps/mp/bots/test", "maps/mp/gametypes/_callbacksetup"],
        &Catalog::iw4(),
    )
    .unwrap();
    let definition = Definition {
        version: 1,
        checksum: 1,
        size: 12,
        root: DataType::Int,
        enums: vec![],
        structs: vec![],
        indexed_arrays: vec![],
        enum_arrays: vec![],
    };
    let mut level = crate::script::LevelData::default();
    level.schemas.insert(
        "mp/playerdata.def".into(),
        std::sync::Arc::new(DefinitionSet {
            definitions: vec![definition],
        }),
    );
    let mut world = SimWorld::new();
    world
        .install_gsc_program(program, NativeRegistry::default(), level)
        .unwrap();
    world
        .start_gsc("maps/mp/bots/test::main", Value::level(), vec![])
        .unwrap();
    step(
        &mut world,
        Tick(1),
        &TickInput::default(),
        50,
        StepReason::AuthorityFrame,
    );
    step(
        &mut world,
        Tick(2),
        &TickInput::default(),
        50,
        StepReason::AuthorityFrame,
    );
    assert_eq!(
        world.bot_warfare_clients(),
        vec![ClientId(16), ClientId(17)]
    );
    let account = world.persistent_data().account(ClientId(17)).unwrap();
    {
        let mut r = world.ecs.resource_mut::<Runtime>();
        assert_eq!(r.object_field(0, "begins"), Value::Int(2));
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(r.players[&17].begun && r.players[&16].begun);
    }
    world.retire_client(ClientId(17));
    step(
        &mut world,
        Tick(3),
        &TickInput::default(),
        50,
        StepReason::AuthorityFrame,
    );
    assert_eq!(world.bot_warfare_clients(), vec![ClientId(16)]);
    assert_eq!(world.persistent_data().account(ClientId(17)), None);
    world
        .start_gsc("maps/mp/bots/test::one", Value::level(), vec![])
        .unwrap();
    step(
        &mut world,
        Tick(4),
        &TickInput::default(),
        50,
        StepReason::AuthorityFrame,
    );
    assert_eq!(
        world.bot_warfare_clients(),
        vec![ClientId(16), ClientId(17)]
    );
    assert_ne!(
        world.persistent_data().account(ClientId(17)).unwrap(),
        account
    );
    let account_before_restart = world.persistent_data().account(ClientId(17)).unwrap();
    for (tick, persist) in [(5, 1), (8, 0)] {
        world
            .ecs
            .resource_mut::<Runtime>()
            .bw
            .inputs
            .get_mut(&17)
            .unwrap()
            .buttons = playerstate_iw4::buttons::ATTACK;
        world
            .start_gsc(
                "maps/mp/bots/test::restart",
                Value::Undefined,
                vec![Value::Int(persist)],
            )
            .unwrap();
        step(
            &mut world,
            Tick(tick),
            &TickInput::default(),
            50,
            StepReason::AuthorityFrame,
        );
        step(
            &mut world,
            Tick(tick + 1),
            &TickInput::default(),
            50,
            StepReason::AuthorityFrame,
        );
        step(
            &mut world,
            Tick(tick + 2),
            &TickInput::default(),
            50,
            StepReason::AuthorityFrame,
        );
        assert_eq!(
            world.bot_warfare_clients(),
            vec![ClientId(16), ClientId(17)]
        );
        assert_eq!(world.ecs.resource::<Runtime>().bw.inputs[&17].buttons, 0);
        assert_eq!(
            world.persistent_data().account(ClientId(17)),
            Some(account_before_restart)
        );
        let object = world.ecs.resource::<Runtime>().players[&17].object;
        assert_eq!(
            call(&mut world, "getguid", &Value::Object(object), &[]).unwrap(),
            Value::string("bot17")
        );
    }
    // A subsequent admission must not reuse an existing temporary account id.
    world
        .start_gsc("maps/mp/bots/test::one", Value::Undefined, vec![])
        .unwrap();
    step(
        &mut world,
        Tick(11),
        &TickInput::default(),
        50,
        StepReason::AuthorityFrame,
    );
    assert_eq!(
        world.bot_warfare_clients(),
        vec![ClientId(15), ClientId(16), ClientId(17)]
    );
    assert!(world.ecs.resource::<Runtime>().errors.is_empty());
}

#[test]
fn performance_player_bones_are_distinct_and_cache_tracks_pose_changes() {
    let mut world = world("main() { }");
    let object = world
        .ecs
        .resource_mut::<Runtime>()
        .create_player(0)
        .unwrap();
    {
        let mut f = FrameWorld::from_world(&mut world.ecs);
        let ps = f.ensure_player(ClientId(0));
        ps.origin = [100., 200., 300.];
        ps.view_height_current = 60.;
    }
    let receiver = Value::Object(object);
    let head = call(
        &mut world,
        "gettagorigin",
        &receiver,
        &[Value::string("j_head")],
    )
    .unwrap();
    let chest = call(
        &mut world,
        "gettagorigin",
        &receiver,
        &[Value::string("j_spineupper")],
    )
    .unwrap();
    let left = call(
        &mut world,
        "gettagorigin",
        &receiver,
        &[Value::string("j_ankle_le")],
    )
    .unwrap();
    let right = call(
        &mut world,
        "gettagorigin",
        &receiver,
        &[Value::string("j_ankle_ri")],
    )
    .unwrap();
    let (Value::Vector(h), Value::Vector(c), Value::Vector(l), Value::Vector(r)) =
        (head, chest, left, right)
    else {
        panic!("positions")
    };
    assert!(h[2] > c[2] && c[2] > l[2]);
    assert_ne!(l, r);
    assert_eq!(world.ecs.resource::<Runtime>().benchmark.perf.tag_builds, 1);
    {
        let mut f = FrameWorld::from_world(&mut world.ecs);
        let ps = f.player_mut(ClientId(0)).unwrap();
        ps.origin[0] += 50.;
        ps.view_height_current = 20.;
    }
    let Value::Vector(crouched) = call(
        &mut world,
        "gettagorigin",
        &receiver,
        &[Value::string("j_head")],
    )
    .unwrap() else {
        panic!()
    };
    assert!(crouched[2] < h[2]);
    assert_eq!(crouched[0], 150.);
    assert_eq!(world.ecs.resource::<Runtime>().benchmark.perf.tag_builds, 2);
}

#[test]
fn performance_scheduler_resumes_inline_children_after_clone_and_cancels_waits() {
    let mut world = world(
        "main() { level.result=0; thread child(1); thread child(10); wait 0.05; level notify(\"go\"); } child(n) { level endon(\"stop\"); level waittill(\"go\"); level.result+=n; wait 1; level.result+=100; } stop() {level notify(\"stop\");}",
    );
    world
        .start_gsc("maps/mp/bots/test::main", Value::level(), vec![])
        .unwrap();
    step(
        &mut world,
        Tick(1),
        &Default::default(),
        50,
        StepReason::AuthorityFrame,
    );
    assert!(world.ecs.resource::<Runtime>().thread_index.len() >= 2);
    let mut copy = world.clone();
    for w in [&mut world, &mut copy] {
        step(
            w,
            Tick(2),
            &Default::default(),
            50,
            StepReason::AuthorityFrame,
        );
        assert_eq!(
            w.ecs.resource_mut::<Runtime>().object_field(0, "result"),
            Value::Int(11)
        );
        w.start_gsc("maps/mp/bots/test::stop", Value::Undefined, vec![])
            .unwrap();
        for tick in 3..=25 {
            step(
                w,
                Tick(tick),
                &Default::default(),
                50,
                StepReason::AuthorityFrame,
            );
        }
        let mut r = w.ecs.resource_mut::<Runtime>();
        assert_eq!(r.object_field(0, "result"), Value::Int(11));
        assert!(r.thread_index.is_empty());
        assert_eq!(r.waiters.len(), 0);
        assert!(r.queued_at.is_empty());
    }
}

#[test]
fn performance_spawned_array_arguments_keep_value_semantics() {
    let mut w = world(
        "main() { a=[];a[0]=1;thread child(a);a[0]=9;level.defined=isdefined(a);level.keys=getarraykeys(a);wait 0.1;level.parent=a[0]; } child(a) {wait 0.05;level.child=a[0];a[0]=7;}",
    );
    w.start_gsc("maps/mp/bots/test::main", Value::level(), vec![])
        .unwrap();
    for tick in 1..=4 {
        step(
            &mut w,
            Tick(tick),
            &Default::default(),
            50,
            StepReason::AuthorityFrame,
        );
    }
    let mut r = w.ecs.resource_mut::<Runtime>();
    assert_eq!(r.object_field(0, "child"), Value::Int(1));
    assert_eq!(r.object_field(0, "parent"), Value::Int(9));
    assert_eq!(r.object_field(0, "defined"), Value::Int(1));
    assert!(r.errors.is_empty());
}

#[test]
fn performance_collision_settlement_reuses_and_invalidates() {
    let mut w = world("main() { }");
    crate::script::host::presence::settle_collision(&mut w.ecs);
    crate::script::host::presence::settle_collision(&mut w.ecs);
    assert_eq!(w.ecs.resource::<Runtime>().benchmark.perf.settle_runs, 1);
    assert_eq!(w.ecs.resource::<Runtime>().benchmark.perf.settle_skipped, 1);
    w.ecs
        .resource_mut::<Runtime>()
        .set_object_field(0, "origin", Value::Vector([1., 2., 3.]));
    crate::script::host::presence::settle_collision(&mut w.ecs);
    assert_eq!(w.ecs.resource::<Runtime>().benchmark.perf.settle_runs, 2);
    let mut registry = NativeRegistry::default();
    assert!(registry.geometry_readonly(Namespace::Function, "isdefined"));
    registry.register(Namespace::Function, "isdefined", |_, _, _| {
        Ok(Value::Int(1))
    });
    assert!(!registry.geometry_readonly(Namespace::Function, "isdefined"));
}

#[test]
fn performance_visibility_cache_is_bounded_and_invalidated() {
    let mut w = world("main() { }");
    let visible = crate::script::host::natives::engine::bw_visible(
        &mut w.ecs,
        [0., 0., 10.],
        [100., 0., 10.],
    );
    assert!(visible);
    assert!(crate::script::host::natives::engine::bw_visible(
        &mut w.ecs,
        [0., 0., 10.],
        [100., 0., 10.]
    ));
    assert_eq!(
        w.ecs.resource::<Runtime>().benchmark.perf.visibility_cached,
        1
    );
    w.ecs
        .resource_mut::<Runtime>()
        .set_object_field(0, "angles", Value::Vector([0., 90., 0.]));
    assert!(crate::script::host::natives::engine::bw_visible(
        &mut w.ecs,
        [0., 0., 10.],
        [100., 0., 10.]
    ));
    assert_eq!(
        w.ecs.resource::<Runtime>().benchmark.perf.visibility_cached,
        1
    );
    for i in 0..1100 {
        crate::script::host::natives::engine::bw_visible(
            &mut w.ecs,
            [0., 0., 10.],
            [i as f32, 1., 10.],
        );
    }
    assert!(w.ecs.resource::<Runtime>().visibility_cache.len() <= 1024);
}

#[test]
fn performance_real_gsc_aim_worker_refreshes_without_thread_churn() {
    fn function(source: &str, name: &str) -> String {
        let begin = source.find(&format!("\n{name}(")).unwrap() + 1;
        let brace = source[begin..].find('{').unwrap() + begin;
        let mut depth = 0;
        for (i, c) in source[brace..].char_indices() {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    return source[begin..=brace + i].to_owned();
                }
            }
        }
        panic!("function bounds")
    }
    let source = crate::bot_warfare::source("maps/mp/bots/_bot_internal")
        .unwrap()
        .replace("\r\n", "\n");
    let mut fixture = String::from(
        "main() { level.gameended=false; self.bot=spawnstruct();self.bot.isfrozen=false; thread bot_look_worker();for(i=0;i<20;i++){bot_lookat((1000,1000,60),0.2,(0,0,0),false);wait 0.05;}level.done=1; } gameflag(x){return true;} botGetThirdPersonOffset(a){return (0,0,0);} BotBuiltinBotAngles(a){self botangles(a[0],a[1],a[2]);}\n",
    );
    for name in ["bot_lookat", "bot_look_step", "bot_look_worker"] {
        fixture.push_str(&function(&source, name));
        fixture.push('\n');
    }
    let mut w = world(&fixture);
    let object = w.ecs.resource_mut::<Runtime>().create_player(17).unwrap();
    let mut slot = crate::script::host::players::PlayerSlot::new(object);
    slot.begun = true;
    {
        let mut r = w.ecs.resource_mut::<Runtime>();
        r.players.insert(17, slot);
        r.bw.inputs.insert(17, Default::default());
    }
    {
        let mut f = FrameWorld::from_world(&mut w.ecs);
        let ps = f.ensure_player(ClientId(17));
        ps.health = 100;
        ps.view_height_current = 60.;
        ps.view_height_target = 60;
        f.client_meta_mut(ClientId(17)).lifecycle = crate::ClientLifecycle::Alive;
    }
    w.set_gsc_dvar("bots_play_aim", "1");
    w.start_gsc("maps/mp/bots/test::main", Value::Object(object), vec![])
        .unwrap();
    for tick in 1..=30 {
        step(
            &mut w,
            Tick(tick),
            &Default::default(),
            50,
            StepReason::AuthorityFrame,
        );
    }
    let mut r = w.ecs.resource_mut::<Runtime>();
    assert_eq!(r.object_field(0, "done"), Value::Int(1));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(
        r.next_serial, 2,
        "one controller fixture and one persistent worker"
    );
    assert!(r.bw.inputs[&17].angles.iter().all(|v| v.is_finite()));
}
