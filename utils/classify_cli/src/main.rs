struct CliLogger;

impl log::Log for CliLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() % 86_400;
        eprintln!("{:02}:{:02}:{:02} {} {}", seconds / 3600, (seconds / 60) % 60, seconds % 60, record.level(), record.args());
    }

    fn flush(&self) {}
}

static LOGGER: CliLogger = CliLogger;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().take_while(|arg| matches!(arg.as_str(), "-v" | "--verbose")).any(|_| true);
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(if verbose { log::LevelFilter::Debug } else { log::LevelFilter::Info });
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = classify_cli::main_with(&refs, &mut classify_cli::CliIo { out: &mut std::io::stdout(), err: &mut std::io::stderr() }, &classify_cli::Deps::system());
    std::process::exit(if code < 0 { 1 } else { code });
}
