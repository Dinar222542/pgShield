use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub server_url: String,
    pub listen_addr: String,
    pub listen_port: u16,
    pub backup_dir: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            server_url: "http://localhost:8080".into(),
            listen_addr: "0.0.0.0".into(),
            listen_port: 9443,
            backup_dir: "/var/lib/pgshield/backups".into(),
            tls_cert: None,
            tls_key: None,
        }
    }
}

impl AgentConfig {
    pub fn load(path: &str) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_yaml::from_str(&contents).ok())
            .unwrap_or_default()
    }
}
