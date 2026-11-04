fn main() {
    let cli = std::env::args().nth(1);
    let bind = std::env::var("BANTER_BIND").ok();
    let port = std::env::var("BANTER_PORT").ok();
    let addr = banter_server::listen_addr(cli.as_deref(), bind.as_deref(), port.as_deref());
    let data = banter_server::default_data_dir();
    let database_url = banter_server::database_url();
    eprintln!("Banter server listening on http://{addr}");
    eprintln!("data directory: {}", data.display());
    eprintln!("database: {database_url}");
    if let Err(err) = banter_server::serve(&addr, &data, &database_url) {
        eprintln!("server stopped: {err}");
        std::process::exit(1);
    }
}
