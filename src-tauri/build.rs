#[path = "build_support.rs"]
mod build_support;

fn main() {
    // Google OAuth client settings are read here, at build time, from the environment or from the untracked
    // file src-tauri/google_oauth.json ({"client_id": "...", "client_secret": "..."}). Nothing is stored in git.
    for var in ["GRAPHITE_GOOGLE_CLIENT_ID", "GRAPHITE_GOOGLE_CLIENT_SECRET"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=google_oauth.json");
    let file = std::fs::read_to_string("google_oauth.json").ok();
    for (var, key) in [("GRAPHITE_GOOGLE_CLIENT_ID", "client_id"), ("GRAPHITE_GOOGLE_CLIENT_SECRET", "client_secret")] {
        if let Some(v) = build_support::pick(std::env::var(var).ok(), file.as_deref(), key) {
            println!("cargo:rustc-env={var}={v}");
        }
    }
    if build_support::pick(std::env::var("GRAPHITE_GOOGLE_CLIENT_SECRET").ok(), file.as_deref(), "client_secret").is_none() {
        println!("cargo:warning=No Google client secret configured: Gmail sign-in will not work in this build (see src-tauri/google_oauth.example.json).");
    }
    tauri_build::build()
}
