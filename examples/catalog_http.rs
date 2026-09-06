//! Read-only API-key catalogue probe with redacted diagnostics and no realtime session.
use sogni_client::{Error, Network, SogniClient};

#[tokio::main]
async fn main() {
    let Ok(key) = std::env::var("SOGNI_API_KEY") else {
        eprintln!("SOGNI_API_KEY is required");
        return;
    };
    let socket_probe = std::env::var("SOGNI_SOCKET_PROBE").as_deref() == Ok("1");
    let proxy = std::env::var("SOGNI_PROXY_URL").ok();
    let mut builder = SogniClient::builder()
        .api_key(key)
        .defer_socket_start(!socket_probe);
    if let Some(proxy) = proxy {
        builder = builder.proxy_url(proxy);
    }
    let client = match builder.build().await {
        Ok(client) => client,
        Err(error) => {
            report("client", &error);
            return;
        }
    };
    let mut events = client.subscribe();
    let monitoring = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if matches!(
                event.name.as_str(),
                "connected" | "authenticated" | "disconnected"
            ) {
                println!(
                    "realtime event: {} code={:?}",
                    event.name,
                    event.data.get("code").and_then(serde_json::Value::as_u64)
                );
                if event.name == "disconnected" {
                    let reason = event
                        .data
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_lowercase();
                    println!(
                        "authentication category: api_key={} token={} protocol={} expired={} invalid={}",
                        reason.contains("api key") || reason.contains("api_key"),
                        reason.contains("token"),
                        reason.contains("protocol") || reason.contains("version"),
                        reason.contains("expired"),
                        reason.contains("invalid")
                    );
                }
            }
        }
    });
    for (kind, result) in [
        (
            "supported",
            client.projects.get_supported_models(true).await,
        ),
        (
            "available",
            client.projects.get_available_models(Network::Fast).await,
        ),
    ] {
        match result {
            Ok(models) => println!("{kind}: {} records", models.len()),
            Err(error) => report(kind, &error),
        }
    }
    match client.account.me().await {
        Ok(_) => println!("authenticated account: available"),
        Err(error) => report("authenticated account", &error),
    }
    if let Ok(options) = client
        .projects
        .get_model_options("flux1-schnell-fp8", false)
        .await
    {
        for field in ["width", "height", "maxPixels"] {
            println!(
                "detail {field}: {}",
                options.raw.get(field).unwrap_or(&serde_json::Value::Null)
            );
        }
    }
    if socket_probe {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while !client.is_socket_authenticated() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        println!("realtime connected: {}", client.is_socket_connected());
        println!(
            "realtime authenticated: {}",
            client.is_socket_authenticated()
        );
    }
    if let Err(error) = client.close().await {
        report("close", &error);
    }
    monitoring.abort();
}

fn report(operation: &str, error: &Error) {
    match error {
        Error::Api(error) => eprintln!("{operation}: HTTP {}", error.status),
        Error::Http(error) => {
            let detail = format!("{error:?}").to_lowercase();
            let category = if detail.contains("certificate") {
                "certificate"
            } else if detail.contains("dns") {
                "dns"
            } else if detail.contains("proxy") {
                "proxy"
            } else {
                "request"
            };
            eprintln!(
                "{operation}: {category} connect={} timeout={} status={:?}",
                error.is_connect(),
                error.is_timeout(),
                error.status().map(|status| status.as_u16())
            );
        }
        _ => eprintln!("{operation}: SDK protocol or configuration error"),
    }
}
