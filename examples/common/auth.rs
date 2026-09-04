use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use sogni_client::{ClientBuilder, Network, SogniClient};
use url::Url;

/// Credentials loaded without ever exposing secrets through `Debug` output.
pub enum Credentials {
    ApiKey(String),
    UsernamePassword { username: String, password: String },
}

/// Build and authenticate a client from `SOGNI_*` environment variables.
///
/// `SOGNI_API_KEY` is preferred. Username/password login is supported only
/// when the crate's default `wallet` feature is enabled.
pub async fn connect(app_id: impl Into<String>, network: Network) -> Result<SogniClient> {
    let credentials = load_credentials()?;
    connect_with_credentials(app_id, network, credentials).await
}

/// Authenticate with credentials already selected by the caller.
///
/// API keys authenticate during client construction. Username/password uses the
/// socket-backed account login flow and therefore requires the `wallet` feature.
pub async fn connect_with_credentials(
    app_id: impl Into<String>,
    network: Network,
    credentials: Credentials,
) -> Result<SogniClient> {
    match credentials {
        Credentials::ApiKey(api_key) => configured_builder(app_id.into(), network)?
            .api_key(api_key)
            .build()
            .await
            .context("create API-key Sogni client"),
        Credentials::UsernamePassword { username, password } => {
            connect_username_password(app_id.into(), network, username, password).await
        }
    }
}

/// Create the REST-only client required by hosted chat and durable workflow APIs.
///
/// Those surfaces require API-key authentication and do not need a WebSocket.
pub async fn connect_api_key_rest_only(
    app_id: impl Into<String>,
    credentials: Credentials,
) -> Result<SogniClient> {
    let Credentials::ApiKey(api_key) = credentials else {
        bail!(
            "this REST-only hosted example requires SOGNI_API_KEY; username/password login needs a socket"
        );
    };
    configured_builder(app_id.into(), Network::Fast)?
        .disable_socket(true)
        .api_key(api_key)
        .build()
        .await
        .context("create REST-only API-key Sogni client")
}

#[cfg(feature = "wallet")]
async fn connect_username_password(
    app_id: String,
    network: Network,
    username: String,
    password: String,
) -> Result<SogniClient> {
    let client = configured_builder(app_id, network)?.build().await?;
    client
        .account
        .login(&username, &password)
        .await
        .context("Sogni username/password login")?;
    Ok(client)
}

#[cfg(not(feature = "wallet"))]
async fn connect_username_password(
    _app_id: String,
    _network: Network,
    _username: String,
    _password: String,
) -> Result<SogniClient> {
    bail!(
        "username/password authentication requires the `wallet` feature; \
         rebuild with default features or set SOGNI_API_KEY"
    )
}

/// Best-effort logout followed by deterministic socket shutdown.
pub async fn close(client: &SogniClient) -> Result<()> {
    let _ = client.account.logout().await;
    client.close().await.context("close Sogni client")
}

/// Load credentials without logging or returning a debuggable secret value.
///
/// Process environment values take precedence over `examples/.env` and `.env`,
/// and an API key takes precedence over username/password. A terminal may prompt
/// for missing username/password credentials; non-interactive callers fail.
pub fn load_credentials() -> Result<Credentials> {
    let dotenv = dotenv_values();
    if let Some(api_key) = credential_value("SOGNI_API_KEY", &dotenv) {
        return Ok(Credentials::ApiKey(api_key));
    }
    if let (Some(username), Some(password)) = (
        credential_value("SOGNI_USERNAME", &dotenv),
        credential_value("SOGNI_PASSWORD", &dotenv),
    ) {
        return Ok(Credentials::UsernamePassword { username, password });
    }
    prompt_credentials()
}

fn prompt_credentials() -> Result<Credentials> {
    if !io::stdin().is_terminal() {
        bail!(
            "credentials are required: set SOGNI_API_KEY, or set both \
             SOGNI_USERNAME and SOGNI_PASSWORD"
        );
    }
    println!("Sogni credentials were not found in the environment or examples/.env.");
    println!("API keys are available from the Sogni dashboard username menu.\n");
    let username = prompt("Username: ")?;
    let password = prompt("Password (input is visible): ")?;
    if username.is_empty() || password.is_empty() {
        bail!("username and password are required");
    }
    let save = prompt("Save these credentials to examples/.env? [y/N]: ")?;
    if matches!(save.to_ascii_lowercase().as_str(), "y" | "yes") {
        save_credentials(&username, &password)?;
    }
    Ok(Credentials::UsernamePassword { username, password })
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_owned())
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Read a setting from the process environment, then `examples/.env` or `.env`.
pub fn setting(name: &str) -> Option<String> {
    nonempty_env(name).or_else(|| dotenv_values().get(name).cloned())
}

fn credential_value(name: &str, dotenv: &BTreeMap<String, String>) -> Option<String> {
    nonempty_env(name).or_else(|| dotenv.get(name).cloned())
}

fn configured_builder(app_id: String, network: Network) -> Result<ClientBuilder> {
    let mut builder = SogniClient::builder().app_id(app_id).network(network);
    if setting("SOGNI_TESTNET").is_some_and(|value| value.eq_ignore_ascii_case("true")) {
        builder = builder.testnet(true);
    }
    if let Some(endpoint) = setting("SOGNI_REST_ENDPOINT") {
        builder = builder
            .rest_endpoint(Url::parse(&endpoint).with_context(|| "parse SOGNI_REST_ENDPOINT")?);
    }
    if let Some(endpoint) = setting("SOGNI_SOCKET_ENDPOINT") {
        builder = builder
            .socket_endpoint(Url::parse(&endpoint).with_context(|| "parse SOGNI_SOCKET_ENDPOINT")?);
    }
    Ok(builder)
}

fn dotenv_candidates() -> [PathBuf; 2] {
    [PathBuf::from("examples/.env"), PathBuf::from(".env")]
}

fn dotenv_values() -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let Some(path) = dotenv_candidates().into_iter().find(|path| path.is_file()) else {
        return values;
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return values;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim().trim_matches(['\'', '"']);
        if !name.is_empty() && !value.is_empty() {
            values.insert(name.to_owned(), value.to_owned());
        }
    }
    values
}

fn save_credentials(username: &str, password: &str) -> Result<()> {
    let path = Path::new("examples/.env");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!(
        "env.tmp-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    writeln!(file, "# Sogni example credentials")?;
    writeln!(file, "SOGNI_USERNAME={username}")?;
    writeln!(file, "SOGNI_PASSWORD={password}")?;
    file.sync_all()?;
    fs::rename(&temporary, path).with_context(|| format!("write {}", path.display()))?;
    println!("Saved credentials to {}", path.display());
    Ok(())
}

/// Make an application id unique enough to keep concurrent example runs separate.
pub fn unique_app_id(prefix: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{prefix}-{timestamp}")
}
