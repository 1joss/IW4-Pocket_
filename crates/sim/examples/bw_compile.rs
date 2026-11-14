//! Compile the complete Bot Warfare dependency closure against supplied stock
//! game sources. Does not substitute successful stubs for missing dependencies.
use sim::script::{Catalog, Program, SourceResolver};
use std::sync::atomic::{AtomicUsize, Ordering};
static CHECKS: AtomicUsize = AtomicUsize::new(0);
static FAILURES: AtomicUsize = AtomicUsize::new(0);
struct Sources(Vec<std::path::PathBuf>);
impl SourceResolver for Sources {
    fn read(&self, module: &str) -> Result<String, String> {
        self.read_bytes(module)
            .map(|b| sim::script::decode_source(&b))
    }
    fn read_bytes(&self, module: &str) -> Result<Vec<u8>, String> {
        if module == "maps/mp/bots/iw4pocket_probe" {
            return Ok(include_bytes!("bw_navigation_probe.gsc").to_vec());
        }
        sim::bot_warfare::source(module)
            .map(|s| s.as_bytes().to_vec())
            .map(Ok)
            .unwrap_or_else(|| {
                self.0
                    .iter()
                    .find_map(|root| {
                        std::fs::read(root.join(format!("{module}.gsc")))
                            .ok()
                            .or_else(|| {
                                module.strip_prefix("character/").and_then(|name| {
                                    std::fs::read(
                                        root.join(format!("character/character{name}.gsc")),
                                    )
                                    .ok()
                                })
                            })
                    })
                    .ok_or_else(|| format!("missing stock module {module}"))
            })
    }
}
fn main() {
    let stress = std::env::args().any(|arg| arg == "--stress-diagnostics");
    let roots = std::env::args()
        .skip(1)
        .filter(|arg| arg != "--stress-diagnostics")
        .map(Into::into)
        .collect();
    let mut modules = sim::bot_warfare::ENTRY_MODULES.to_vec();
    modules.push("maps/mp/bots/iw4pocket_probe");
    match Program::load(&Sources(roots), &modules, &Catalog::iw4()) {
        Ok(p) => {
            println!(
                "Bot Warfare compile: {} modules, {} functions, {} natives",
                p.modules().len(),
                p.function_count(),
                p.native_count()
            );
            let mut world = sim::SimWorld::new();
            let mut natives = sim::script::NativeRegistry::default();
            natives.register(
                sim::script::Namespace::Function,
                "bullettracepassed",
                |_, _, _| Ok(sim::script::Value::Int(1)),
            );
            natives.register(
                sim::script::Namespace::Function,
                "bw_event",
                |_, _, args| {
                    CHECKS.fetch_add(1, Ordering::Relaxed);
                    if args.first() != Some(&sim::script::Value::Int(1)) {
                        FAILURES.fetch_add(1, Ordering::Relaxed);
                        eprintln!("FAILED: {args:?}");
                    }
                    Ok(sim::script::Value::Undefined)
                },
            );
            world
                .install_gsc_program(p, natives, Default::default())
                .expect("all natives bound");
            world
                .start_gsc(
                    "scripts/mp/bots_adapter_iw4pocket::init",
                    sim::script::Value::Object(0),
                    Vec::new(),
                )
                .expect("adapter initialization");
            world
                .start_gsc(
                    "maps/mp/bots/iw4pocket_probe::main",
                    sim::script::Value::level(),
                    Vec::new(),
                )
                .unwrap();
            sim::step(
                &mut world,
                sim::Tick(1),
                &Default::default(),
                50,
                sim::StepReason::AuthorityFrame,
            );
            if stress {
                world
                    .start_gsc(
                        "maps/mp/bots/iw4pocket_probe::diagnostic_flood",
                        sim::script::Value::level(),
                        Vec::new(),
                    )
                    .unwrap();
            }
            for tick in 2..=if stress { 453 } else { 100 } {
                sim::step(
                    &mut world,
                    sim::Tick(tick),
                    &Default::default(),
                    50,
                    sim::StepReason::AuthorityFrame,
                );
            }
            world.shutdown_game();
            assert_eq!(
                CHECKS.load(Ordering::Relaxed),
                8,
                "all navigation checks executed"
            );
            assert_eq!(
                FAILURES.load(Ordering::Relaxed),
                0,
                "navigation expectations"
            );
            println!(
                "Bot Warfare native installation, adapter execution and 8 navigation/CSV checks passed"
            );
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    }
}
