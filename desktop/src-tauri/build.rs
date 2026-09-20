fn main() {
    // Bake AGENT_DOCTOR_EDITION into agent-doctor-core via option_env! when set.
    println!("cargo:rerun-if-env-changed=AGENT_DOCTOR_EDITION");
    tauri_build::build()
}
