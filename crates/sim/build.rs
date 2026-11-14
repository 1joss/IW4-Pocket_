use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output.status.success().then_some(())?;
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn main() {
    let root =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("bot_warfare");
    fn scan(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                scan(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    scan(&root, &mut files);
    files.sort();
    let mut generated = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for file in files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        generated.push_str(&format!(
            "({rel:?}, include_bytes!({:?})),\n",
            file.to_str().unwrap()
        ));
    }
    generated.push_str("];\n");
    std::fs::write(
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bot_warfare_files.rs"),
        generated,
    )
    .unwrap();
    println!("cargo::rerun-if-changed=bot_warfare");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=IW4L_BUILD_NUMBER");
    let tracked = git(&["ls-files", "--error-unmatch", "Cargo.toml"]).is_some();
    if tracked {
        for reference in
            std::iter::once("HEAD".to_owned()).chain(git(&["symbolic-ref", "--quiet", "HEAD"]))
        {
            if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
                println!("cargo::rerun-if-changed={path}");
            }
        }
    }
    let number = match std::env::var("IW4L_BUILD_NUMBER") {
        Ok(number) => Some(number),
        Err(_) if tracked => git(&["rev-list", "--count", "HEAD"]),
        Err(_) => None,
    }
    .and_then(|number| number.parse::<i32>().ok())
    .filter(|number| *number >= 0)
    .map_or_else(|| "unavailable".to_owned(), |number| number.to_string());
    println!("cargo::rustc-env=IW4L_BUILD_NUMBER={number}");
}
