use std::sync::Arc;

use rustls::{ClientConfig, RootCertStore};
use tokio_tungstenite::Connector;

use crate::{Error, Result};

pub(super) fn native_connector() -> Result<Connector> {
    let certificates = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(certificates.certs);
    if roots.is_empty() {
        return Err(Error::Transport(
            "native TLS trust store is unavailable".into(),
        ));
    }
    Ok(Connector::Rustls(Arc::new(verified_config(roots)?)))
}

fn verified_config(roots: RootCertStore) -> Result<ClientConfig> {
    // Applications can link ring and aws-lc together. An explicit provider avoids
    // rustls's ambiguous global-provider panic without changing host TLS policy.
    ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Transport("TLS protocol configuration failed".into()))
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_socket_crypto_works_with_both_crypto_backends_linked() {
        // The dev dependency enables aws-lc as well as production ring, matching
        // consumers that use another TLS stack. Default automatic selection panics.
        let _other_provider = rustls::crypto::aws_lc_rs::default_provider();
        let config = verified_config(RootCertStore::empty()).expect("explicit TLS provider");
        assert!(config.enable_sni);
        assert!(!config.crypto_provider().cipher_suites.is_empty());
    }
}
