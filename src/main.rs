fn main() {
    if let Err(e) = agent_vocal_studio::run_cli() {
        eprintln!("{e:#}");
        std::process::exit(1)
    }
}
