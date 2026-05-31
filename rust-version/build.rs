use std::process::Command;

fn main() {
    let git_dir = "../.git";
    let head_path = format!("{git_dir}/HEAD");

    println!("cargo:rerun-if-changed={head_path}");
    println!("cargo:rerun-if-changed={git_dir}/packed-refs");

    if let Ok(head) = std::fs::read_to_string(&head_path) {
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed={git_dir}/{reference}");
        }
    }

    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if hash.is_empty() {
                    None
                } else {
                    Some(hash)
                }
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=LLAMA_LAUNCHER_GIT_COMMIT={commit}");
}
