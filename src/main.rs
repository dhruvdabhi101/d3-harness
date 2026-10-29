fn main() {
    if let Err(error) = oc_harness::parse_and_run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
