mod support;

use lodestone_core::catalog::Provider;
use lodestone_core::catalog::types::{Artifact, Checksum, HashAlgo, ServerType};
use lodestone_core::catalog::vanilla::{
    VANILLA_MANIFEST_URL, VanillaProvider, parse_manifest, parse_version_json,
};
use lodestone_core::error::CatalogError;

use support::FixtureHttpClient;

const MANIFEST_JSON: &str = r#"{
  "latest": { "release": "1.21.9", "snapshot": "1.21.10-rc1" },
  "versions": [
    { "id": "1.21.10-rc1", "type": "snapshot", "url": "https://piston-meta.mojang.com/v1/packages/rc/1.21.10-rc1.json" },
    { "id": "1.21.9", "type": "release", "url": "https://piston-meta.mojang.com/v1/packages/def/1.21.9.json" },
    { "id": "1.20.4", "type": "release", "url": "https://piston-meta.mojang.com/v1/packages/ghi/1.20.4.json" }
  ]
}"#;

const VERSION_JSON_TEMPLATE: &str = r#"{
  "javaVersion": { "component": "java-runtime-delta", "majorVersion": 21 },
  "downloads": {
    "server": {
      "url": "https://piston-data.mojang.com/v1/objects/def/server.jar",
      "size": 8,
      "sha1": "__SHA1__"
    }
  }
}"#;

fn sha1_hex(data: &[u8]) -> String {
    use sha1::Digest;
    let mut h = sha1::Sha1::new();
    h.update(data);
    let bytes = h.finalize();
    let mut out = String::new();
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn fixture_client() -> FixtureHttpClient {
    FixtureHttpClient::new()
        .route_json(VANILLA_MANIFEST_URL, MANIFEST_JSON)
        .route_json(
            "https://piston-meta.mojang.com/v1/packages/def/1.21.9.json",
            &VERSION_JSON_TEMPLATE.replace("__SHA1__", &sha1_hex(b"jarbytes")),
        )
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lode-vanilla-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[tokio::test]
async fn versions_lists_mojang_data() {
    let provider = VanillaProvider::new(fixture_client());
    let versions = provider.versions().await.unwrap();
    let ids: Vec<&str> = versions.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec!["1.21.10-rc1", "1.21.9", "1.20.4"]);
    assert!(versions.iter().find(|v| v.id == "1.20.4").unwrap().release);
    assert!(
        !versions
            .iter()
            .find(|v| v.id == "1.21.10-rc1")
            .unwrap()
            .release
    );
}

#[tokio::test]
async fn resolve_reads_downloads_and_java_from_version_json() {
    let provider = VanillaProvider::new(fixture_client());
    let artifact = provider.resolve("1.21.9").await.unwrap();

    assert_eq!(artifact.server_type, ServerType::Vanilla);
    assert_eq!(artifact.java_major, Some(21));
    assert_eq!(
        artifact.url,
        "https://piston-data.mojang.com/v1/objects/def/server.jar"
    );
    let checksum = artifact.checksum.as_ref().unwrap();
    assert_eq!(checksum.algo, HashAlgo::Sha1);
    assert_eq!(checksum.hex, sha1_hex(b"jarbytes"));
}

#[tokio::test]
async fn resolve_unknown_version_is_version_not_found() {
    let provider = VanillaProvider::new(fixture_client());
    assert!(matches!(
        provider.resolve("1.12.2").await,
        Err(CatalogError::VersionNotFound { .. })
    ));
}

#[test]
fn malformed_input_handling() {
    assert!(parse_manifest("not json").is_err());
    let vj = parse_version_json("{\"nope\": true}").unwrap();
    assert!(vj.java_version.is_none());
    assert!(vj.downloads.is_none());
}

#[tokio::test]
async fn resolve_rejects_version_json_without_server_download() {
    let client = FixtureHttpClient::new()
        .route_json(VANILLA_MANIFEST_URL, MANIFEST_JSON)
        .route_json(
            "https://piston-meta.mojang.com/v1/packages/def/1.21.9.json",
            "{\"javaVersion\":{\"majorVersion\":21}}",
        );
    let provider = VanillaProvider::new(client);
    let err = provider.resolve("1.21.9").await.unwrap_err();
    assert!(matches!(err, CatalogError::Source(msg) if msg.contains("no server download")));
    let _ = client;
}

#[tokio::test]
async fn install_downloads_and_verifies() {
    let client = fixture_client();
    client.route(
        "https://piston-data.mojang.com/v1/objects/def/server.jar",
        b"jarbytes".to_vec(),
    );
    let provider = VanillaProvider::new(client);

    let artifact = Artifact {
        server_type: ServerType::Vanilla,
        version: "1.21.9".into(),
        file_name: "vanilla-1.21.9-server.jar".into(),
        url: "https://piston-data.mojang.com/v1/objects/def/server.jar".into(),
        checksum: Some(Checksum {
            algo: HashAlgo::Sha1,
            hex: sha1_hex(b"jarbytes"),
        }),
        size_bytes: Some(8),
        java_major: Some(21),
    };

    let dir = tmp_dir("ok");
    provider.install(&artifact, &dir).await.unwrap();

    let written = std::fs::read(dir.join("vanilla-1.21.9-server.jar")).unwrap();
    assert_eq!(written, b"jarbytes".to_vec());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn install_rejects_corrupt_download() {
    let client = fixture_client();
    client.route(
        "https://piston-data.mojang.com/v1/objects/def/server.jar",
        b"tampered!".to_vec(),
    );
    let provider = VanillaProvider::new(client);

    let artifact = Artifact {
        server_type: ServerType::Vanilla,
        version: "1.21.9".into(),
        file_name: "vanilla-1.21.9-server.jar".into(),
        url: "https://piston-data.mojang.com/v1/objects/def/server.jar".into(),
        checksum: Some(Checksum {
            algo: HashAlgo::Sha1,
            hex: sha1_hex(b"jarbytes"),
        }),
        size_bytes: Some(9),
        java_major: Some(21),
    };

    let dir = tmp_dir("corrupt");
    let result = provider.install(&artifact, &dir).await;
    assert!(matches!(result, Err(CatalogError::ChecksumMismatch { .. })));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn install_refuses_artifact_without_checksum() {
    let provider = VanillaProvider::new(fixture_client());
    let artifact = Artifact {
        server_type: ServerType::Vanilla,
        version: "1.21.9".into(),
        file_name: "vanilla.jar".into(),
        url: "https://piston-data.mojang.com/v1/objects/def/server.jar".into(),
        checksum: None,
        size_bytes: None,
        java_major: Some(21),
    };
    let dir = tmp_dir("nochecksum");
    assert!(provider.install(&artifact, &dir).await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
