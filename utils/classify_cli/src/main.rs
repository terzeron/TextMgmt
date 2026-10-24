fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = classify_cli::main_with(&refs, &mut classify_cli::CliIo { out: &mut std::io::stdout(), err: &mut std::io::stderr() }, &classify_cli::Deps::system());
    std::process::exit(if code < 0 { 1 } else { code });
}
