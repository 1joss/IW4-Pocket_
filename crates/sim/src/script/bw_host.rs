//! IW4x-compatible bot input and lifecycle natives for the pinned GSC mod.
use crate::frame::FrameWorld;
use crate::script::host::{
    args::{float, int, string, vector},
    natives::player::player,
};
use crate::script::{
    ArrayKey,
    Namespace::{Function, Method},
    NativeRegistry, Runtime, Value,
};
use bevy_ecs::prelude::World;
use playerstate_iw4::{UserCmd, buttons};
use std::collections::BTreeMap;
use std::path::{Component, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct BotInput {
    pub buttons: u32,
    pub forward: i8,
    pub right: i8,
    pub angles: [f32; 3],
    pub weapon: u16,
    pub melee_yaw: f32,
    pub melee_dist: u8,
    pub remote: [u8; 2],
    pub life: Option<crate::LifeSequence>,
    pub alive: bool,
}
impl Default for BotInput {
    fn default() -> Self {
        Self {
            buttons: 0,
            forward: 0,
            right: 0,
            angles: [0.; 3],
            weapon: 0,
            melee_yaw: 0.,
            melee_dist: 0,
            remote: [0; 2],
            life: None,
            alive: false,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct BotState {
    pub inputs: BTreeMap<u32, BotInput>,
    pub serial: u64,
}

fn bot<'a>(world: &'a mut World, receiver: &Value) -> Result<&'a mut BotInput, String> {
    let id = player(world, receiver)?;
    let pose = {
        let f = FrameWorld::from_world(world);
        f.player(crate::ClientId(id))
            .copied()
            .zip(f.client_meta(crate::ClientId(id)).map(|m| m.life_sequence))
    };
    let b = world
        .resource_mut::<Runtime>()
        .into_inner()
        .bw
        .inputs
        .get_mut(&id)
        .ok_or_else(|| "receiver is not a Bot Warfare test client".to_owned())?;
    if let Some((ps, life)) = pose {
        reset_life(b, &ps, life);
    }
    Ok(b)
}
fn reset_life(b: &mut BotInput, ps: &playerstate_iw4::PlayerState, life: crate::LifeSequence) {
    if b.life != Some(life) {
        *b = BotInput::default();
        b.life = Some(life);
        b.angles = ps.viewangles;
        b.weapon = ps.weapon as u16;
    }
}
fn data_root() -> PathBuf {
    if cfg!(target_os = "ios") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Documents/BotWarfare")
    } else {
        std::env::var_os("IW4L_BW_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("iw4l-artifacts/bot-warfare"))
    }
}
fn file_key(path: &str) -> Result<String, String> {
    let path = path.replace('\\', "/");
    if !path.starts_with("scriptdata/")
        || std::path::Path::new(&path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Bot Warfare files must be relative scriptdata paths".into());
    }
    Ok(path)
}
fn create_bot(world: &mut World, _: &Value, _: &[Value]) -> Result<Value, String> {
    let taken = FrameWorld::from_world(world).client_ids_sorted();
    // Humans use low client ids. Allocate test clients from the high end.
    let Some(id) = (0..18u32).rev().find(|n| !taken.iter().any(|i| i.0 == *n)) else {
        return Ok(Value::Undefined);
    };
    let client = crate::ClientId(id);
    let serial = {
        let mut r = world.resource_mut::<Runtime>();
        r.bw.serial += 1;
        r.bw.serial
    };
    let mut account = [0u8; 16];
    account[..8].copy_from_slice(b"BW-TEMP\0");
    account[8..].copy_from_slice(&serial.to_le_bytes());
    world
        .resource_mut::<crate::PersistentDataStore>()
        .admit_temporary(client, crate::AccountId(account))
        .map_err(|e| format!("bot account admission: {e:?}"))?;
    {
        let mut frame = FrameWorld::from_world(world);
        let meta = frame.client_meta_mut(client);
        meta.lifecycle = crate::ClientLifecycle::ChoosingClass;
        let name = format!("BW Bot {}", id + 1);
        meta.name = [0; 16];
        let len = name.len().min(15);
        meta.name[..len].copy_from_slice(&name.as_bytes()[..len]);
    }
    let object = world.resource_mut::<Runtime>().create_player(id)?;
    let pers = super::host::arrays::new_array(world, Vec::new())?;
    {
        let mut r = world.resource_mut::<Runtime>();
        if let Value::Array(a) = pers {
            let fields = r.arrays.get_mut(&a).unwrap();
            fields.insert(ArrayKey::String("isBot".into()), Value::Int(1));
            fields.insert(ArrayKey::String("isBotWarfare".into()), Value::Int(1));
        }
        r.set_object_field(object, "pers", pers);
        r.set_object_field(object, "name", Value::string(&format!("BW Bot {}", id + 1)));
        r.players
            .insert(id, super::host::players::PlayerSlot::new(object));
        r.bw.inputs.insert(id, BotInput::default());
    }
    let now = super::host::players::now_ms(world);
    super::runtime::run_now(
        world,
        "maps/mp/gametypes/_callbacksetup::codecallback_playerconnect",
        Value::Object(object),
        Vec::new(),
        now,
    )
    .map_err(|e| format!("bot connect: {e}"))?;
    super::host::players::answer_join(world, id);
    diag::info!(
        Sim,
        "bw: created client={id} revision={}",
        crate::bot_warfare::REVISION
    );
    Ok(Value::Object(object))
}
pub(crate) fn register(registry: &mut NativeRegistry) {
    registry.register(Function, "closer", |_, _, a| {
        let origin = vector(a, 0)?;
        let x = vector(a, 1)?;
        let y = vector(a, 2)?;
        let distance = |v: [f32; 3]| (0..3).map(|i| (v[i] - origin[i]).powi(2)).sum::<f32>();
        Ok(Value::Int((distance(x) < distance(y)).into()))
    });
    registry.register(Method, "getplayerviewheight", |w, r, _| {
        let id = player(w, r)?;
        let f = FrameWorld::from_world(w);
        Ok(Value::Float(
            f.player(crate::ClientId(id))
                .ok_or("player has no playerstate")?
                .view_height_current,
        ))
    });
    registry.register(Function, "addtestclient", create_bot);
    registry.register(Method, "getguid", |w, r, _| {
        let id = player(w, r)?;
        let guid = if w.resource::<Runtime>().bw.inputs.contains_key(&id) {
            format!("bot{id}")
        } else {
            format!("{:016x}", u64::from(id) + 1)
        };
        Ok(Value::string(&guid))
    });
    registry.register(Method, "isbot", |w, r, _| {
        let id = player(w, r)?;
        Ok(Value::Int(
            w.resource::<Runtime>().bw.inputs.contains_key(&id).into(),
        ))
    });
    registry.register(Method, "istestclient", |w, r, _| {
        let id = player(w, r)?;
        Ok(Value::Int(
            w.resource::<Runtime>().bw.inputs.contains_key(&id).into(),
        ))
    });
    registry.register(Method, "getping", |_, _, _| Ok(Value::Int(0)));
    registry.register(Method, "botstop", |w, r, _| {
        let id = player(w, r)?;
        let ps = FrameWorld::from_world(w)
            .player(crate::ClientId(id))
            .copied();
        let b = bot(w, r)?;
        let life = b.life;
        let alive = b.alive;
        *b = BotInput::default();
        b.life = life;
        b.alive = alive;
        if let Some(ps) = ps {
            b.angles = ps.viewangles;
            b.weapon = ps.weapon as u16;
        }
        Ok(Value::Undefined)
    });
    registry.register(Method, "botmovement", |w, r, a| {
        let f = int(a, 0)?.clamp(-128, 127) as i8;
        let s = int(a, 1)?.clamp(-128, 127) as i8;
        let b = bot(w, r)?;
        b.forward = f;
        b.right = s;
        Ok(Value::Undefined)
    });
    registry.register(Method, "botangles", |w, r, a| {
        let angles = [float(a, 0)?, float(a, 1)?, float(a, 2)?];
        if angles.iter().any(|v| !v.is_finite()) {
            return Err("non-finite bot angles".into());
        }
        bot(w, r)?.angles = angles;
        Ok(Value::Undefined)
    });
    registry.register(Method, "botaction", |w, r, a| {
        let action = string(a, 0)?.to_ascii_lowercase();
        let (sign, name) = action.split_at_checked(1).ok_or("empty bot action")?;
        if sign != "+" && sign != "-" {
            return Err("bot action requires + or -".into());
        }
        let bit = action_bit(name).ok_or_else(|| format!("unknown bot action {name}"))?;
        let b = bot(w, r)?;
        if sign == "+" {
            b.buttons |= bit
        } else {
            b.buttons &= !bit
        }
        Ok(Value::Undefined)
    });
    registry.register(Method, "botmeleeparams", |w, r, a| {
        let yaw = float(a, 0)?;
        let dist = float(a, 1)?.clamp(0., 255.) as u8;
        let b = bot(w, r)?;
        b.melee_yaw = yaw;
        b.melee_dist = dist;
        Ok(Value::Undefined)
    });
    registry.register(Method, "botremoteangles", |w, r, a| {
        let p = int(a, 0)? as u8;
        let y = int(a, 1)? as u8;
        bot(w, r)?.remote = [p, y];
        Ok(Value::Undefined)
    });
    registry.register(Method, "botweapon", |w, r, a| {
        let name = string(a, 0)?;
        let id = FrameWorld::from_world(w)
            .weapon_index_by_script_name(&name)
            .ok_or("unknown bot weapon")?;
        bot(w, r)?.weapon = id as u16;
        Ok(Value::Undefined)
    });
    registry.register(Function, "fileexists", |_, _, a| {
        let key = file_key(&string(a, 0)?)?;
        Ok(Value::Int(
            (data_root().join(&key).is_file() || crate::bot_warfare::file(&key).is_some()).into(),
        ))
    });
    registry.register(Function, "fileread", |_, _, a| {
        let key = file_key(&string(a, 0)?)?;
        match std::fs::read(data_root().join(&key)).or_else(|_| {
            crate::bot_warfare::file(&key)
                .map(Vec::from)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        }) {
            Ok(bytes) => Ok(Value::byte_string(&bytes)),
            Err(e) => Err(format!("{key}: {e}")),
        }
    });
    registry.register(Function, "filewrite", |_, _, a| {
        use std::io::Write;
        let key = file_key(&string(a, 0)?)?;
        let value = string(a, 1)?;
        let mode = string(a, 2)?;
        if mode != "write" && mode != "append" {
            return Err("filewrite mode must be write or append".into());
        }
        let path = data_root().join(&key);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(mode == "write")
            .append(mode == "append")
            .open(path)
            .map_err(|e| e.to_string())?;
        f.write_all(value.as_bytes()).map_err(|e| e.to_string())?;
        Ok(Value::Undefined)
    });
    registry.register(Function, "printconsole", |w, _, a| {
        let text = string(a, 0)?;
        super::benchmark::event(w, "console", &text);
        diag::info!(Sim, "bw: {}", text.trim_end());
        Ok(Value::Undefined)
    });
    registry.register(Function, "bw_event", |w, r, a| {
        let text = a
            .iter()
            .map(|v| super::runtime::to_text(v).unwrap_or_else(|| format!("{v:?}")))
            .collect::<Vec<_>>()
            .join(" ");
        super::benchmark::event(w, "bot_event", &format!("self={r:?} {text}"));
        Ok(Value::Undefined)
    });
    registry.register(Function, "bw_profile", |_, _, _| {
        Ok(Value::string(crate::bot_warfare::REVISION))
    });
    registry.register(Function, "kick", |w, _, a| {
        let id = int(a, 0)? as u32;
        if !w.resource::<Runtime>().bw.inputs.contains_key(&id) {
            return Err("Bot Warfare cannot kick a human client".into());
        }
        w.resource_mut::<Runtime>().disconnects.insert(id);
        Ok(Value::Undefined)
    });
}
fn action_bit(name: &str) -> Option<u32> {
    Some(match name {
        "fire" | "attack" => buttons::ATTACK,
        "sprint" => buttons::SPRINT,
        "melee" => buttons::MELEE_CHARGE,
        "activate" | "use" => buttons::USE,
        "reload" => buttons::RELOAD,
        "usereload" => buttons::USE_RELOAD,
        "goprone" | "prone" => buttons::PRONE,
        "gocrouch" | "crouch" => buttons::CROUCH,
        "gostand" | "stand" | "jump" => buttons::JUMP,
        "ads" => buttons::ADS | buttons::THROW,
        "holdbreath" | "breath" => buttons::BREATH,
        "frag" => buttons::FRAG,
        "smoke" => buttons::SMOKE,
        "remote" => buttons::REMOTE_CONTROL,
        "leanleft" => 0x40,
        "leanright" => 0x80,
        "actionslot1" => 1 << 20,
        "actionslot2" => 1 << 21,
        "actionslot3" => 1 << 22,
        "actionslot4" => 1 << 23,
        _ => return None,
    })
}
pub(crate) fn emit_commands(world: &mut World) {
    if !world
        .resource::<crate::step::StepRequest>()
        .reason
        .advances_authority_world()
    {
        return;
    }
    let started = std::time::Instant::now();
    let now =
        world.resource::<crate::step::StepRequest>().tick.0 as i32 * crate::MATCH_TICK_MS as i32;
    let ids: Vec<_> = world
        .resource::<Runtime>()
        .bw
        .inputs
        .keys()
        .copied()
        .collect();
    let mut commands = Vec::new();
    for id in ids {
        let (ps, meta) = {
            let f = FrameWorld::from_world(world);
            (
                f.player(crate::ClientId(id)).copied(),
                f.client_meta(crate::ClientId(id)).cloned(),
            )
        };
        let Some(meta) = meta else {
            world.resource_mut::<Runtime>().bw.inputs.remove(&id);
            continue;
        };
        let Some(ps) = ps else { continue };
        let mut r = world.resource_mut::<Runtime>();
        let b = r.bw.inputs.get_mut(&id).unwrap();
        let alive = meta.lifecycle == crate::ClientLifecycle::Alive;
        if !alive {
            b.buttons = 0;
            b.forward = 0;
            b.right = 0;
            b.alive = false;
            continue;
        }
        // Input methods synchronize before applying this life's new controls.
        // This fallback clears stale controls even if no script input ran yet.
        reset_life(b, &ps, meta.life_sequence);
        b.alive = true;
        let weapon = if b.weapon == 0 || !ps.weapons.contains(&(b.weapon as i32)) {
            ps.weapon as u16
        } else {
            b.weapon
        };
        commands.push(crate::PlayerCommand::unsequenced(
            crate::ClientId(id),
            UserCmd {
                server_time: now,
                buttons: b.buttons,
                angles: std::array::from_fn(|a| {
                    ((b.angles[a] - ps.delta_angles[a]) * 65536. / 360.) as i32
                }),
                forwardmove: b.forward,
                rightmove: b.right,
                weapon,
                weapon_mapped: weapon,
                melee_charge_yaw: b.melee_yaw,
                melee_charge_dist: b.melee_dist,
                remote_control: b.remote,
                ..Default::default()
            },
        ));
    }
    {
        let mut r = world.resource_mut::<Runtime>();
        let tick = r.benchmark.tick;
        if r.benchmark.detailed {
            for c in &commands {
                r.benchmark.queue(serde_json::json!({"type":"command","tick":tick,"client":c.client.0,"buttons":c.command.buttons,"angles":c.command.angles,"forward":c.command.forwardmove,"right":c.command.rightmove,"weapon":c.command.weapon,"remote":c.command.remote_control}));
            }
        }
        r.benchmark.bridge_ns += started.elapsed().as_nanos() as u64;
    }
    let mut step = world.resource_mut::<crate::step::StepRequest>();
    step.input
        .cmds
        .retain(|c| !commands.iter().any(|b| b.client == c.client));
    step.input.cmds.extend(commands);
    step.input.canonicalize();
}
