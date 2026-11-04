
use trainer::{CliIo, Deps, main_with};

fn run(args: &[&str], deps: &Deps) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = main_with(args, &mut CliIo { out: &mut out, err: &mut err }, deps);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn every_command_prints_help_and_no_args_fail_like_python() {
    let deps = Deps::system();
    for command in trainer::commands() {
        let (code, out, err) = run(&[command, "--help"], &deps);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains(command));
    }
    assert_eq!(run(&[], &deps).0, 1);
    assert_eq!(run(&["-v"], &deps).0, 1);
}
