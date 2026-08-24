use vary::run;
use std::process::exit;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    exit(run(&args));
}
