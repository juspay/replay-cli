pub const fn server_url() -> &'static str {
    match option_env!("REPLAY_SERVER_URL") {
        Some(url) => url,
        None => "http://localhost:3000",
    }
}
