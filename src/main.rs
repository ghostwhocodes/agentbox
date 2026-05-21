#![forbid(unsafe_code)]

fn main() {
    if let Err(error) = agentbox::try_main() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
