use lodestone_core::net::{HostAllowlist, HttpClient, HttpsClient, ensure_https};

#[test]
fn standard_allowlist_covers_spec_sources() {
    let a = HostAllowlist::standard();
    for host in [
        "piston-meta.mojang.com",
        "piston-data.mojang.com",
        "api.papermc.io",
        "api.purpurmc.org",
        "meta.fabricmc.net",
        "maven.neoforged.net",
        "packages.termux.org",
        "sessionserver.mojang.com",
    ] {
        assert!(a.is_allowed(host), "missing {host}");
    }
    assert!(!a.is_allowed("evil.example.com"));
    assert!(!a.is_allowed("meta.fabricmc.net.evil.com"));
}

#[test]
fn empty_allowlist_accepts_nothing() {
    let a = HostAllowlist::empty();
    assert!(!a.is_allowed("piston-meta.mojang.com"));
}

#[test]
fn merge_adds_without_duplicates() {
    let mut a = HostAllowlist::empty();
    a.merge(["h1.example"]);
    a.merge(["h1.example", "h2.example"]);
    assert!(a.is_allowed("h1.example"));
    assert!(a.is_allowed("h2.example"));
    assert!(a.contains("h2.example"));
}

#[test]
fn https_is_required() {
    assert!(ensure_https("https://piston-meta.mojang.com/x").is_ok());
    assert!(ensure_https("http://piston-meta.mojang.com/x").is_err());
    assert!(ensure_https("ftp://piston-meta.mojang.com/x").is_err());
}

#[tokio::test]
async fn client_refuses_disallowed_hosts_without_network() {
    let client = HttpsClient::new(HostAllowlist::empty()).unwrap();
    let err = client
        .get("https://evil.example.com/steal")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        lodestone_core::error::NetError::DisallowedHost(_)
    ));

    let err = client
        .get("http://piston-meta.mojang.com/x")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        lodestone_core::error::NetError::DisallowedHost(_)
    ));
}
