const DEFAULT_SERVER_URL: &str = "http://localhost:3000";

pub fn server_url() -> String {
    std::env::var("REPLAY_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string())
}
