use crate::{
    master_files::MasterFiles,
    transport::{Error, PeerCertificate},
};
use std::sync::Arc;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    listen: std::net::SocketAddr,
    certificate: String,
    private_key: String,
    client_ca: String,
    peers: Vec<PeerCertificate>,
    internal_ca: String,
    internal_identity: String,
    private_url: String,
    version: String,
    pod_uid_env: Option<String>,
    master: MasterFiles,
}
pub async fn run() -> Result<(), Error> {
    let path = std::env::args()
        .nth(1)
        .ok_or("configuration JSON path required")?;
    let config: Config = serde_json::from_slice(&tokio::fs::read(path).await?)?;
    if config.version.is_empty() {
        return Err("version required".into());
    }
    let instance = match config.pod_uid_env {
        Some(name) => std::env::var(name)?,
        None => uuid::Uuid::new_v4().to_string(),
    };
    let registry =
        crate::capacity::Registry::new(instance, std::thread::available_parallelism()?.get())
            .map_err(|_| "invalid GameServer instance configuration")?;
    let arena = config.master.load().await?;
    let client =
        crate::transport::internal_client(&config.internal_ca, &config.internal_identity).await?;
    let app = crate::application::Application {
        arena: Arc::new(arena),
        private_client: client,
        private_url: config.private_url,
        version: config.version,
        registry: Arc::new(tokio::sync::Mutex::new(registry)),
    };
    let tls = crate::transport::tls_config(
        &config.certificate,
        &config.private_key,
        Some(&config.client_ca),
    )
    .await?;
    crate::transport::serve_internal(
        config.listen,
        crate::application::router(app),
        tls,
        config.peers,
    )
    .await
}
