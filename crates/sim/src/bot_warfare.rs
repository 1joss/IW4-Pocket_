//! Pinned upstream Bot Warfare source/data overlay. The game still supplies its
//! stock scripts. No Recompiled controller or generated navigation is used.
include!(concat!(env!("OUT_DIR"), "/bot_warfare_files.rs"));
pub const REVISION: &str = "31415e49e5baf160bd369c9ef4d61295018ffef2";
pub const ENTRY_MODULES: &[&str] = &[
    "scripts/mp/bots_adapter_iw4pocket",
    "scripts/mp/bots",
    "scripts/mp/bots_chat",
    "scripts/mp/bots_menu",
    "scripts/mp/bots_wp_editor",
];
pub fn source(module: &str) -> Option<&'static str> {
    let path = format!("{}.gsc", module.replace('\\', "/").to_ascii_lowercase());
    file(&path).and_then(|b| std::str::from_utf8(b).ok())
}
pub fn file(path: &str) -> Option<&'static [u8]> {
    FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| *bytes)
}
pub(crate) fn builtins() -> Vec<crate::script::Builtin> {
    use crate::script::{Builtin, Namespace::*, Owner::*};
    let functions = [
        "printconsole",
        "filewrite",
        "fileread",
        "fileexists",
        "bw_event",
        "bw_profile",
        "kick",
    ];
    let methods = [
        "botaction",
        "botstop",
        "botmovement",
        "botmeleeparams",
        "botremoteangles",
        "botangles",
        "botweapon",
        "isbot",
        "istestclient",
        "getping",
    ];
    functions
        .into_iter()
        .map(|s| Builtin::new(Function, s, Script, false))
        .chain(
            methods
                .into_iter()
                .map(|s| Builtin::new(Method, s, Player, false)),
        )
        .collect()
}

/// Keep the latest loader/install error outside the rotating runtime trace.
pub fn report_load_error(map: &str, gametype: &str, stage: &str, fault: &crate::script::Fault) {
    let root = if cfg!(target_os = "ios") {
        std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Documents/BotWarfareDiagnostics")
    } else {
        std::env::var_os("IW4L_BW_DIAGNOSTICS")
            .map(Into::into)
            .unwrap_or_else(|| std::path::PathBuf::from("iw4l-artifacts/bot-warfare-diagnostics"))
    };
    let record = serde_json::json!({"schema":1,"controller":"bot-warfare-gsc","revision":REVISION,"type":"load_error","map":map,"gametype":gametype,"stage":stage,"message":fault.to_string(),"callers":format!("{:?}",fault.callers)});
    if let Err(e) = std::fs::create_dir_all(&root).and_then(|_| {
        std::fs::write(
            root.join("latest-load-error.json"),
            serde_json::to_vec_pretty(&record)?,
        )
    }) {
        diag::warn!(Sim, "bw diagnostics load-error write failed: {e}");
    }
}
