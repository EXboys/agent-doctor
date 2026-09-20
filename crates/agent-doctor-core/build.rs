fn main() {
    // `option_env!("AGENT_DOCTOR_EDITION")` in edition.rs — rebuild when it changes.
    println!("cargo:rerun-if-env-changed=AGENT_DOCTOR_EDITION");
}
