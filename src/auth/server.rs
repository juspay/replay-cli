use anyhow::{anyhow, Result};
use std::sync::mpsc;
use std::thread;
use tiny_http::{Response, Server};

/// Start a local HTTP server on a random port to receive the auth callback.
/// Returns the port and a receiver that will yield the session token.
pub fn start_callback_server(expected_state: &str) -> Result<(u16, mpsc::Receiver<String>)> {
    let server =
        Server::http("127.0.0.1:0").map_err(|e| anyhow!("Failed to start callback server: {e}"))?;

    let port = match server.server_addr().to_ip() {
        Some(addr) => addr.port(),
        None => return Err(anyhow!("Callback server did not bind to an IP address")),
    };

    let (tx, rx) = mpsc::channel();
    let state = expected_state.to_string();

    thread::spawn(move || {
        // Handle requests until we get a valid callback or the server is dropped
        for request in server.incoming_requests() {
            let url = request.url().to_string();

            let params = match url.split_once('?') {
                Some((_, query)) => query,
                None => {
                    let _ = request.respond(
                        Response::from_string("Missing query parameters").with_status_code(400),
                    );
                    continue;
                }
            };

            let mut token = None;
            let mut callback_state = None;

            for pair in params.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    match key {
                        "token" => token = Some(value.to_string()),
                        "state" => callback_state = Some(value.to_string()),
                        _ => {}
                    }
                }
            }

            // Verify state parameter
            match callback_state.as_deref() {
                Some(s) if s == state => {}
                _ => {
                    let _ = request.respond(
                        Response::from_string("Invalid state parameter").with_status_code(400),
                    );
                    continue;
                }
            }

            let token = match token {
                Some(t) => t,
                None => {
                    let _ = request
                        .respond(Response::from_string("Missing token").with_status_code(400));
                    continue;
                }
            };

            let html = r#"<html><body style="font-family:system-ui;display:flex;justify-content:center;align-items:center;height:100vh;margin:0"><h2>Authentication successful! You can close this tab.</h2></body></html>"#;
            let response = Response::from_string(html).with_header(
                "Content-Type: text/html"
                    .parse::<tiny_http::Header>()
                    .unwrap(),
            );
            let _ = request.respond(response);

            let _ = tx.send(token);
            break;
        }
    });

    Ok((port, rx))
}
