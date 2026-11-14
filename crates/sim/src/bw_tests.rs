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
