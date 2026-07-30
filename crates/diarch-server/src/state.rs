use diarch_core::Config;
use diarch_db::Db;

pub struct AppState {
    pub db: Db,
    pub config: Config,
    pub http: reqwest::Client,
}
