use vary::run;
use std::process::exit;

fn main() {
    // Install a panic hook that swallows the benign "Broken pipe" panic that
    // occurs when a downstream consumer of our stdout closes the pipe early
    // (e.g. `vary -Ss foo | head`, `vary -Si x | less`, `vary ... | true`).
    // Without this, vary aborts with exit code 101 and a scary backtrace.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let is_broken_pipe = match info.payload().downcast_ref::<String>() {
            Some(s) => s.contains("Broken pipe"),
            None => match info.payload().downcast_ref::<&str>() {
                Some(s) => s.contains("Broken pipe"),
                None => false,
            },
        };
        if is_broken_pipe {
            // The reader went away; this is expected, not an error.
            exit(0);
        }
        default_hook(info);
    }));

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    exit(run(&args));
}
