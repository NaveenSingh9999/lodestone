const ALLOWED_HOSTS: &[&str] = &[
    "piston-meta.mojang.com",
    "piston-data.mojang.com",
    "resources.download.mojang.com",
    "launcher.mojang.com",
    "sessionserver.mojang.com",
    "api.minecraftservices.com",
    "api.papermc.io",
    "fill.papermc.io",
    "fill-data.papermc.io",
    "repo.papermc.io",
    "api.purpurmc.org",
    "meta.fabricmc.net",
    "maven.fabricmc.net",
    "meta.quiltmc.org",
    "maven.quiltmc.org",
    "maven.minecraftforge.net",
    "files.minecraftforge.net",
    "maven.neoforged.net",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "raw.githubusercontent.com",
    "packages.termux.org",
    "packages.termux.dev",
    "grimler.se",
    "mirror.mwt.me",
];

#[derive(Debug, Clone, Default)]
pub struct HostAllowlist {
    hosts: Vec<String>,
}

impl HostAllowlist {
    pub fn standard() -> Self {
        HostAllowlist {
            hosts: ALLOWED_HOSTS.iter().map(|h| h.to_string()).collect(),
        }
    }

    pub fn empty() -> Self {
        HostAllowlist { hosts: Vec::new() }
    }

    pub fn contains(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.');
        self.hosts.iter().any(|allowed| allowed == host)
    }

    pub fn merge(&mut self, hosts: impl IntoIterator<Item = impl AsRef<str>>) {
        for host in hosts {
            let host = host.as_ref().to_string();
            if !self.hosts.contains(&host) {
                self.hosts.push(host);
            }
        }
    }

    pub fn is_allowed(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.');
        self.hosts.iter().any(|allowed| host == *allowed)
    }
}
