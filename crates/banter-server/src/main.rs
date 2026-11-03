fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| banter_server::DEFAULT_ADDR.to_string());
    let data = banter_server::default_data_dir();
    eprintln!("Banter server listening on http://{addr}");
    eprintln!("data directory: {}", data.display());
    if let Err(err) = banter_server::serve(&addr, &data) {
        eprintln!("server stopped: {err}");
        std::process::exit(1);
    }
}
