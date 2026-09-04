use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Expose le commit et l'horodatage de build à /v1/health : avec une instance
/// notifyd par entreprise, c'est ce qui permet de voir laquelle a dérivé.
/// Ordre : variable `GIT_COMMIT_SHA` (CI ou build arg), puis `git rev-parse`
/// (le `.git` est dans le contexte Docker), sinon « unknown ».
fn main() {
    let commit = std::env::var("GIT_COMMIT_SHA")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string());

    let built_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    println!("cargo:rustc-env=NOTIFYD_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=NOTIFYD_BUILD_EPOCH={built_at}");
    println!("cargo:rerun-if-env-changed=GIT_COMMIT_SHA");
    println!("cargo:rerun-if-changed=.git/HEAD");
}
