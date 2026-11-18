#[derive(Clone, Debug, Default)]
pub(crate) struct PerfCounters {
    pub settle_runs: u64,
    pub settle_skipped: u64,
    pub visibility_calls: u64,
    pub sight_avoided: u64,
    pub visibility_cached: u64,
    pub settle_ns: u64,
    pub tag_builds: u64,
    pub tag_hits: u64,
    pub accepted_shots: u64,
    pub attack_commands: u64,
}
use crate::script::{Program, Runtime, Value};
use bevy_ecs::prelude::World;
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

#[derive(Clone, Debug, Default)]
pub(crate) struct Stat {
    pub instructions: u64,
    pub calls: u64,
    pub ns: u64,
    pub max_ns: u64,
    pub errors: u64,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Span {
    pub native_ns: u64,
    pub nested_ns: u64,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Benchmark {
    pub perf: PerfCounters,
    pub spans: Vec<Span>,
    pub functions: Vec<Stat>,
    pub lines: BTreeMap<(usize, usize), u64>,
    pub natives: Vec<Stat>,
    pub samples: Vec<u64>,
    pub tick: u32,
    pub native_total_ns: u64,
    pub frames: u64,
    pub total_ns: u64,
    pub max_ns: u64,
    pub gc_ns: u64,
    pub resumes: u64,
    pub pending: Vec<Json>,
    pub pending_bytes: usize,
    pub error_records: Vec<Json>,
    pub dropped: u64,
    pub write_ns: u64,
    pub io_errors: u64,
    pub manifest: Option<Json>,
    pub sequence: u64,
    pub directory_ready: bool,
    pub session: Option<String>,
    pub segment: u32,
    pub bytes: u64,
    pub initialized: bool,
    pub bridge_ns: u64,
    pub report_ns: u64,
    pub allocations: u64,
    pub detailed: bool,
    pub enabled: bool,
}
impl Benchmark {
    pub fn initialize(&mut self, p: &Program) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        if !p
            .modules
            .iter()
            .any(|m| m.module.starts_with("maps/mp/bots/"))
        {
            return;
        }
        let mode = std::env::var("IW4L_BW_PROFILE").unwrap_or_else(|_| "summary".into());
        self.enabled = mode != "off";
        self.detailed = mode == "detail";
        self.functions.resize(p.functions.len(), Stat::default());
        self.natives.resize(p.natives.len(), Stat::default());
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        self.session = Some(format!(
            "{}-{}-{:06}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            std::process::id(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let manifest = json!({"type":"manifest","schema":1,"source_revision":crate::bot_warfare::REVISION,"controller":"bot-warfare-gsc","profile":mode,"program_fingerprint":format!("{:02x?}",p.fingerprint()),"baseline":"1joss/IW4-Pocket_ main 2cc1559","modules":p.modules.iter().map(|m|json!({"module":m.module,"sha256":format!("{:02x?}",m.sha256)})).collect::<Vec<_>>(),"functions":p.functions.iter().map(|f|format!("{}::{}",f.location.module,f.location.function)).collect::<Vec<_>>(),"natives":p.natives.iter().map(|n|n.name).collect::<Vec<_>>(),"timing":"function self wall nanoseconds exclude natives and nested callbacks; native durations include callbacks; scheduler includes stock scripts and GC; windows exclude diagnostic reporting; native I/O in detail mode only","limits":{"sessions":3,"segments":4,"segment_bytes":4194304,"queued_records":4096,"queued_bytes":2097152,"snapshot_elements":64,"snapshot_depth":2,"snapshot_fields_per_container":32,"string_characters":1024},"build":option_env!("IW4L_BUILD_NUMBER")});
        self.manifest = Some(manifest.clone());
        self.queue(manifest);
    }
    pub fn queue(&mut self, mut v: Json) {
        if self.session.is_none() {
            return;
        }
        if v["type"] == "runtime_error" || v["type"] == "fatal_error" {
            if self.error_records.len() < 32 {
                self.error_records.push(v.clone());
            } else if v["type"] == "fatal_error" {
                self.error_records[31] = v.clone();
            }
        }
        self.sequence += 1;
        if let Some(object) = v.as_object_mut() {
            object.insert("sequence".into(), json!(self.sequence));
        }
        let bytes = serde_json::to_vec(&v).map_or(usize::MAX, |v| v.len());
        if self.pending.len() < 4096
            && bytes <= 1024 * 1024
            && self.pending_bytes.saturating_add(bytes) <= 2 * 1024 * 1024
        {
            self.pending_bytes += bytes;
            self.pending.push(v)
        } else {
            self.dropped += 1
        }
    }
    fn root(&self) -> PathBuf {
        if cfg!(target_os = "ios") {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Documents/BotWarfareDiagnostics")
        } else {
            std::env::var_os("IW4L_BW_DIAGNOSTICS")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("iw4l-artifacts/bot-warfare-diagnostics"))
        }
    }
    fn window(&self, partial: bool) -> Json {
        let mut samples = self.samples.clone();
        samples.sort_unstable();
        let pct = |n: usize| {
            samples
                .get(samples.len().saturating_sub(1) * n / 100)
                .copied()
                .unwrap_or(0)
        };
        let functions=self.functions.iter().enumerate().filter(|(_,s)|s.instructions>0).map(|(i,s)|json!({"id":i,"instructions":s.instructions,"segments":s.calls,"self_ns":s.ns,"max_slice_ns":s.max_ns})).collect::<Vec<_>>();
        let natives=self.natives.iter().enumerate().filter(|(_,s)|s.calls>0).map(|(i,s)|json!({"id":i,"calls":s.calls,"ns":s.ns,"max_ns":s.max_ns,"errors":s.errors})).collect::<Vec<_>>();
        json!({"type":"window","partial":partial,"tick":self.tick,"frames":self.frames,"scheduler_ns":self.total_ns,"p50_ns":pct(50),"p95_ns":pct(95),"p99_ns":pct(99),"max_ns":self.max_ns,"gc_ns":self.gc_ns,"resumes":self.resumes,"diagnostic_write_ns":self.write_ns,"diagnostic_report_ns":self.report_ns,"input_bridge_ns":self.bridge_ns,"dropped_records":self.dropped,"io_errors":self.io_errors,"functions":functions,"natives":natives,"source_lines":self.lines.iter().map(|((id,line),count)|json!({"function":id,"line":line,"instructions":count})).collect::<Vec<_>>(),"allocations":self.allocations,"performance":{"settle_runs":self.perf.settle_runs,"settle_skipped":self.perf.settle_skipped,"visibility_calls":self.perf.visibility_calls,"sight_avoided":self.perf.sight_avoided,"visibility_cached":self.perf.visibility_cached,"settle_ns":self.perf.settle_ns,"tag_builds":self.perf.tag_builds,"tag_hits":self.perf.tag_hits,"accepted_shots":self.perf.accepted_shots,"attack_commands":self.perf.attack_commands}})
    }
    pub(crate) fn finish(&mut self) {
        if self.frames > 0 {
            let record = self.window(true);
            self.queue(record);
        }
        self.queue(json!({"type":"session_end","tick":self.tick,"dropped_records":self.dropped,"io_errors":self.io_errors}));
        self.flush();
    }
    pub(crate) fn flush(&mut self) {
        use std::io::{BufWriter, Write};
        let started = Instant::now();
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let root = self.root();
        let folder = root.join(session);
        let result = (|| -> std::io::Result<()> {
            if !self.directory_ready {
                std::fs::create_dir_all(&folder)?;
                // Remove only our marked sessions, never arbitrary user directories.
                let mut old = std::fs::read_dir(&root)?
                    .filter_map(Result::ok)
                    .filter(|e| e.path() != folder && e.path().join("manifest.json").is_file())
                    .collect::<Vec<_>>();
                old.sort_by_key(|e| e.file_name());
                while old.len() > 2 {
                    std::fs::remove_dir_all(old.remove(0).path())?;
                }
                if let Some(manifest) = &self.manifest {
                    std::fs::write(
                        folder.join("manifest.json"),
                        serde_json::to_vec_pretty(manifest)?,
                    )?;
                }
                self.directory_ready = true;
            }
            std::fs::write(
                folder.join("errors.json"),
                serde_json::to_vec_pretty(&self.error_records)?,
            )?;
            let mut consumed = 0;
            let write_result = (|| -> std::io::Result<()> {
                while consumed < self.pending.len() {
                    let path = folder.join(format!("trace-{:02}.jsonl", self.segment));
                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(self.bytes == 0)
                        .append(self.bytes != 0)
                        .open(path)?;
                    let mut file = BufWriter::new(file);
                    while consumed < self.pending.len() {
                        let mut bytes = serde_json::to_vec(&self.pending[consumed])?;
                        bytes.push(b'\n');
                        if self.bytes > 0 && self.bytes + bytes.len() as u64 > 4 * 1024 * 1024 {
                            break;
                        }
                        file.write_all(&bytes)?;
                        self.bytes += bytes.len() as u64;
                        consumed += 1;
                    }
                    file.flush()?;
                    if consumed < self.pending.len() {
                        self.segment = (self.segment + 1) % 4;
                        self.bytes = 0;
                    }
                }
                Ok(())
            })();
            if write_result.is_err() {
                // Buffered writes may be partial. Record the ambiguity explicitly.
                self.dropped += self.pending.len() as u64;
                self.pending.clear();
            } else {
                self.pending.drain(..consumed);
            }
            self.pending_bytes = 0;
            write_result
        })();
        if let Err(e) = result {
            self.io_errors += 1;
            if self.io_errors <= 3 {
                diag::warn!(Sim, "bw diagnostics write error: {e}");
            }
        }
        self.write_ns += started.elapsed().as_nanos() as u64;
    }
}
pub(crate) fn value(v: &Value) -> Json {
    match v {
        Value::Undefined => Json::Null,
        Value::Int(n) => json!(n),
        Value::Float(n) => json!(n),
        Value::Vector(v) => json!(v),
        Value::String(s) => json!(s.to_string().chars().take(1024).collect::<String>()),
        Value::Object(n) => json!({"object":n}),
        Value::Array(n) => json!({"array":n}),
        other => json!(format!("{other:?}")),
    }
}
pub(crate) fn event(w: &mut World, kind: &str, text: &str) {
    let mut r = w.resource_mut::<Runtime>();
    let tick = r.benchmark.tick;
    r.benchmark
        .queue(json!({"type":kind,"tick":tick,"message":text}));
    if kind == "runtime_error" && r.errors.len() < 3 {
        r.benchmark.flush();
    }
}
fn field(r: &mut Runtime, v: &Value, key: &str) -> Value {
    match v {
        Value::Object(id) => r.object_field(*id, key),
        Value::Array(id) => r
            .arrays
            .get(id)
            .and_then(|a| a.get(&crate::script::ArrayKey::String(key.into())))
            .cloned()
            .unwrap_or(Value::Undefined),
        _ => Value::Undefined,
    }
}
fn snapshot(r: &Runtime, v: &Value, depth: u8, budget: &mut usize) -> Json {
    if *budget == 0 {
        return json!({"truncated":true});
    }
    *budget -= 1;
    if depth == 0 {
        return value(v);
    }
    match v {
        Value::Array(id) => {
            json!({"array":id,"entries":r.arrays.get(id).map(|a|a.iter().take(32).map(|(k,v)|json!({"key":format!("{k:?}"),"value":snapshot(r,v,depth-1,budget)})).collect::<Vec<_>>())})
        }
        Value::Object(id) => {
            let fields = r.objects.get(id).map(|o| {
                o.iter()
                    .take(32)
                    .map(|(key, v)| {
                        let name = r
                            .program
                            .as_ref()
                            .and_then(|p| p.symbols.get(*key as usize))
                            .map(|s| s.to_string())
                            .or_else(|| {
                                r.dynamic_symbols
                                    .iter()
                                    .find(|(_, id)| **id == *key)
                                    .map(|(s, _)| s.to_string())
                            })
                            .unwrap_or_else(|| key.to_string());
                        (name, snapshot(r, v, depth - 1, budget))
                    })
                    .collect::<serde_json::Map<_, _>>()
            });
            json!({"object":id,"fields":fields})
        }
        _ => value(v),
    }
}
pub(crate) fn native(
    w: &mut World,
    id: usize,
    function: usize,
    line: usize,
    ns: u64,
    args: &[Value],
    result: &Result<Value, String>,
    receiver: &Value,
    script_self: &Value,
    thread: u64,
) {
    let mut r = w.resource_mut::<Runtime>();
    if !r.benchmark.enabled {
        return;
    }
    r.benchmark.native_total_ns += ns;
    if let Some(s) = r.benchmark.natives.get_mut(id) {
        s.calls += 1;
        s.ns += ns;
        s.max_ns = s.max_ns.max(ns);
        s.errors += u64::from(result.is_err());
    }
    if r.benchmark.detailed || result.is_err() {
        let tick = r.benchmark.tick;
        let record = json!({"type":"native","tick":tick,"native":id,"function":function,"line":line,"duration_ns":ns,"receiver":value(receiver),"script_self":value(script_self),"client":r.player_client_of(script_self),"thread":thread,"args":args.iter().map(value).collect::<Vec<_>>(),"result":result.as_ref().map(|v|snapshot(&r,v,2,&mut 64)).ok(),"error":result.as_ref().err()});
        r.benchmark.queue(record);
    }
}
pub(crate) fn frame_end(w: &mut World, p: &Program, elapsed: u64, gc: u64) {
    let report_started = Instant::now();
    let write_before = w.resource::<Runtime>().benchmark.write_ns;
    let tick = w.resource::<Runtime>().last_tick.map_or(0, |t| t.0);
    let mut r = w.resource_mut::<Runtime>();
    let b = &mut r.benchmark;
    b.tick = tick;
    if !b.enabled {
        return;
    }
    b.frames += 1;
    b.total_ns += elapsed;
    b.gc_ns += gc;
    b.max_ns = b.max_ns.max(elapsed);
    b.samples.push(elapsed);
    if b.detailed {
        b.queue(json!({"type":"frame","tick":tick,"scheduler_ns":elapsed,"gc_ns":gc}));
    }
    if tick % 20 == 0 {
        let settings = r
            .dvars
            .iter()
            .filter(|(k, _)| {
                k.starts_with("bots_") || ["mapname", "g_gametype"].contains(&k.as_str())
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>();
        let waypoints = r.object_field(0, "waypoints");
        let waypoint_count = if let Value::Array(id) = waypoints {
            r.arrays.get(&id).map_or(0, BTreeMap::len)
        } else {
            0
        };
        let flags = r.object_field(0, "flags");
        let flag_objects = if let Value::Array(id) = &flags {
            r.arrays
                .get(id)
                .map(|a| a.values().take(16).cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut objective_details = Vec::new();
        for flag in flag_objects {
            let origin = field(&mut r, &flag, "origin");
            let useobj = field(&mut r, &flag, "useobj");
            let owner = field(&mut r, &useobj, "ownerteam");
            let label = field(&mut r, &useobj, "label");
            objective_details.push(json!({"entity":value(&flag),"position":value(&origin),"owner_team":value(&owner),"label":value(&label)}));
        }
        let flags = snapshot(&r, &flags, 2, &mut 128);
        let script_players = r
            .players
            .iter()
            .map(|(client, slot)| json!({"client":client,"object":slot.object}))
            .collect::<Vec<_>>();
        let rng = r.rng;
        let objects = r.objects.len();
        let arrays = r.arrays.len();
        let entries = r.arrays.values().map(BTreeMap::len).sum::<usize>();
        let waiters = r.waiters.len();
        let threads = r.next_serial;
        let errors = r
            .errors
            .iter()
            .map(|((site, message), count)| json!({"site":site,"message":message,"count":count}))
            .collect::<Vec<_>>();
        let ids = r.bw.inputs.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let Some(slot) = r.players.get(&id) else {
                continue;
            };
            let object = slot.object;
            let bot = r.object_field(object, "bot");
            let data = if let Value::Object(bot) = bot {
                let mut fields = serde_json::Map::new();
                let path = r.object_field(bot, "astar");
                fields.insert("path".into(), snapshot(&r, &path, 1, &mut 40));
                fields.insert(
                    "goal_locked".into(),
                    value(&r.object_field(object, "bot_lock_goal")),
                );
                for key in [
                    "script_goal",
                    "moveto",
                    "towards_goal",
                    "next_wp",
                    "second_next_wp",
                    "prio_objective",
                    "climbing",
                    "stop_move",
                    "isfrozen",
                    "isreloading",
                    "last_fire_time",
                    "fire_attempts",
                    "fire_presses",
                    "fire_reason",
                    "target",
                ] {
                    fields.insert(key.into(), value(&r.object_field(bot, key)));
                }
                if let Value::Object(target) = r.object_field(bot, "target") {
                    let mut targetfields = serde_json::Map::new();
                    for key in [
                        "entity",
                        "last_seen_pos",
                        "trace_time",
                        "no_trace_time",
                        "dist",
                        "time",
                        "trace_time_time",
                        "bone",
                        "offset",
                        "aim_offset_base",
                        "aim_offset",
                    ] {
                        targetfields.insert(key.into(), value(&r.object_field(target, key)));
                    }
                    fields.insert("target_detail".into(), Json::Object(targetfields));
                }
                Json::Object(fields)
            } else {
                Json::Null
            };
            let pers = r.object_field(object, "pers");
            let bots = field(&mut r, &pers, "bots");
            let skill = field(&mut r, &bots, "skill");
            let skill = snapshot(&r, &skill, 2, &mut 64);
            let input=r.bw.inputs.get(&id).map(|b|json!({"buttons":b.buttons,"forward":b.forward,"right":b.right,"angles":b.angles,"weapon":b.weapon,"life":b.life.map(|l|l.0),"consumed_buttons":b.consumed_buttons,"consumed_attack_ticks":b.consumed_attacks,"accepted_shots":b.accepted_shots}));
            r.benchmark.queue(json!({"type":"bot","tick":tick,"client":id,"object":object,"state":data,"input":input,"skill":skill}));
        }
        let frame = crate::frame::FrameWorld::from_world(w);
        let positions=frame.client_ids_sorted().into_iter().filter_map(|id|frame.player(id).map(|ps|json!({"client":id.0,"position":ps.origin,"velocity":ps.velocity,"angles":ps.viewangles,"weapon":ps.weapon,"health":ps.health,"team":frame.client_meta(id).map(|m|m.client_state_team),"lifecycle":frame.client_meta(id).map(|m|format!("{:?}",m.lifecycle)),"kills":frame.client_meta(id).map(|m|m.kills),"deaths":frame.client_meta(id).map(|m|m.deaths)}))).collect::<Vec<_>>();
        let mut r = w.resource_mut::<Runtime>();
        let b = &mut r.benchmark;
        b.queue(json!({"type":"world","tick":tick,"players":positions,"script_players":script_players,"settings":settings,"rng_state":rng,"waypoint_count":waypoint_count,"objectives":flags,"objective_details":objective_details,"live_objects":objects,"live_arrays":arrays,"array_entries":entries,"waiters":waiters,"created_threads":threads,"errors":errors}));
    }
    let mut r = w.resource_mut::<Runtime>();
    let b = &mut r.benchmark;
    if b.frames >= 100 {
        b.samples.sort_unstable();
        let pct = |n: usize| b.samples[(b.samples.len() - 1) * n / 100];
        let record = b.window(false);
        diag::info!(
            Sim,
            "bw: benchmark tick={tick} p50_us={} p95_us={} max_us={} dropped={} errors={}",
            pct(50) / 1000,
            pct(95) / 1000,
            b.max_ns / 1000,
            b.dropped,
            b.io_errors
        );
        b.queue(record);
        b.frames = 0;
        b.total_ns = 0;
        b.max_ns = 0;
        b.gc_ns = 0;
        b.resumes = 0;
        b.write_ns = 0;
        b.report_ns = 0;
        b.bridge_ns = 0;
        b.samples.clear();
        b.allocations = 0;
        b.perf = PerfCounters::default();
        b.lines.clear();
        b.functions.fill(Stat::default());
        b.natives.fill(Stat::default());
    }
    if tick % 20 == 0 || r.benchmark.pending.len() > 2048 {
        r.benchmark.flush();
    }
    r.benchmark.report_ns += (report_started.elapsed().as_nanos() as u64)
        .saturating_sub(r.benchmark.write_ns.saturating_sub(write_before));
    let _ = p;
}
