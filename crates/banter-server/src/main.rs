fn main() {
    let cli = std::env::args().nth(1);
    let bind = std::env::var("BANTER_BIND").ok();
    let port = std::env::var("BANTER_PORT").ok();
    let addr = banter_server::listen_addr(cli.as_deref(), bind.as_deref(), port.as_deref());
    let data = banter_server::default_data_dir();
    eprintln!("Banter server listening on http://{addr}");
    eprintln!("data directory: {}", data.display());
    if let Err(err) = banter_server::serve(&addr, &data) {
        eprintln!("server stopped: {err}");
        std::process::exit(1);
    }
}
