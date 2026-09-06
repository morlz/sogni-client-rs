use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn strict_media_requires_known_https_origin_without_credentials() -> TestResult {
    for value in [
        "https://media.sogni.ai/image?signature=fixture",
        "https://sogni.io/image",
        "https://fixture.s3.amazonaws.com/image",
    ] {
        assert!(safe_host(&Url::parse(value)?).is_ok());
    }
    for value in [
        "http://media.sogni.ai/image",
        "https://media.sogni.ai:444/image",
        "https://user:secret-marker@media.sogni.ai/image",
        "https://media.sogni.ai/image#secret-marker",
        "https://media.sogni.ai.invalid/image",
        "https://evilsogni.ai/image",
        "https://127.0.0.1/image",
        "https://[::1]/image",
    ] {
        let url = Url::parse(value)?;
        let result = safe_host(&url);
        assert!(result.is_err());
        assert!(!format!("{result:?}").contains("secret-marker"));
    }

    Ok(())
}

#[test]
fn strict_media_rejects_private_mixed_and_unbounded_dns_answers() -> TestResult {
    let public = "8.8.8.8:443".parse()?;
    validate_addresses(&[public, "[2606:4700:4700::1111]:443".parse()?])?;
    assert!(validate_addresses(&[]).is_err());
    assert!(validate_addresses(&vec![public; MAX_DNS_ADDRESSES + 1]).is_err());
    for value in [
        "0.0.0.0",
        "10.1.2.3",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.168.1.1",
        "192.0.2.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "2001:db8::1",
        "2002::1",
        "3fff::1",
        "64:ff9b::a00:1",
    ] {
        let address = SocketAddr::new(value.parse()?, 443);
        assert!(!public_address(address.ip()));
        assert!(validate_addresses(&[public, address]).is_err());
    }

    Ok(())
}

#[tokio::test]
async fn strict_media_socks_uses_pinned_ip_instead_of_proxy_dns() -> TestResult {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let proxy = SocksProxy::parse(&format!("socks5h://{}", listener.local_addr()?))?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut greeting = [0_u8; 2];
        socket.read_exact(&mut greeting).await?;
        assert_eq!(greeting[0], 5);
        let mut methods = vec![0_u8; usize::from(greeting[1])];
        socket.read_exact(&mut methods).await?;
        socket.write_all(&[5, 0]).await?;
        let mut command = [0_u8; 10];
        socket.read_exact(&mut command).await?;
        assert_eq!(&command[..8], &[5, 1, 0, 1, 8, 8, 8, 8]);
        assert_eq!(u16::from_be_bytes([command[8], command[9]]), 443);
        socket.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;

        Ok::<(), std::io::Error>(())
    });
    let client = pinned_client(
        "media.sogni.ai",
        &["8.8.8.8:443".parse()?],
        Duration::from_secs(2),
        Some(&proxy),
    )?;
    assert!(
        client
            .put("https://media.sogni.ai/fixture")
            .send()
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(3), server).await???;

    Ok(())
}

#[tokio::test]
async fn pinned_media_preserves_host_and_does_not_follow_redirect() -> TestResult {
    // Only the private builder bypasses HTTPS/public validation for this local
    // deterministic HTTP fixture. Production enters through client() above.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 4_096);
            socket.read_exact(&mut byte).await?;
            request.push(byte[0]);
        }
        let headers = String::from_utf8_lossy(&request).to_ascii_lowercase();
        assert!(headers.contains(&format!("host: media.sogni.ai:{}\r\n", address.port())));
        assert!(!headers.contains("authorization:"));
        assert!(!headers.contains("x-api-key:"));
        assert!(!headers.contains("cookie:"));
        socket.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1/untrusted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;

        Ok::<(), std::io::Error>(())
    });
    let client = pinned_client("media.sogni.ai", &[address], Duration::from_secs(2), None)?;
    let response = client
        .put(format!("http://media.sogni.ai:{}/fixture", address.port()))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
    tokio::time::timeout(Duration::from_secs(3), server).await???;

    Ok(())
}
