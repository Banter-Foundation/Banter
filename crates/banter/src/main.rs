fn main() {
    if let Err(err) = banter::run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
