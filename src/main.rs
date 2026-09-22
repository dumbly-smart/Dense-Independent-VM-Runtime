use std::path::PathBuf;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 || args[1] != "probe" || args[2] != "--out" {
        eprintln!("usage: densevm probe --out NEW_DIRECTORY");
        std::process::exit(2);
    }
    let out = PathBuf::from(&args[3]);
    if let Err(error) = densevm::probe::run(&out) {
        eprintln!("densevm probe failed: {error}");
        std::process::exit(1);
    }
}
