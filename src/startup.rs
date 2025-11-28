use std::{
    collections::{HashMap, HashSet},
    fs::create_dir_all,
    path::Path,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use amplify::s;
use biscuit_auth::PublicKey;
use clap::Parser;
use migration::{Migrator, MigratorTrait};
use sea_orm::{ActiveValue, ConnectOptions, Database};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    auth::check_auth_args,
    database::{
        AppDatabase,
        entities::{config, cosigner, next_address_index},
    },
    error::AppError,
    utils::check_port_is_available,
};

const CONFIG_NAME: &str = "config.toml";
const MIN_COSIGNERS: usize = 2;

pub(crate) const LOGS_DIR: &str = "logs";
pub(crate) const FILES_DIR: &str = "files";

// rgb-lib version compatibility range for this hub version
pub(crate) const MIN_RGB_LIB_VERSION: &str = "0.3";
pub(crate) const MAX_RGB_LIB_VERSION: &str = "0.3";

pub(crate) const DB_MIN_CONNECTIONS: u32 = 0;
pub(crate) const DB_TIMEOUT: Duration = Duration::from_secs(8);
pub(crate) const DB_NAME: &str = "rgb_multisig_hub_db";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub(crate) struct AppArgs {
    /// Path for the daemon directory
    pub(crate) app_directory_path: PathBuf,

    /// Listening port of the daemon
    #[arg(long, default_value_t = 3001)]
    pub(crate) daemon_listening_port: u16,
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct AppConfig {
    pub(crate) cosigner_xpubs: Vec<String>,
    pub(crate) threshold_colored: u8,
    pub(crate) threshold_vanilla: u8,
    pub(crate) root_public_key: String,
    pub(crate) rgb_lib_version: String,
}

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub(crate) struct AppParams {
    pub(crate) app_dir: PathBuf,
    pub(crate) daemon_listening_port: u16,
    pub(crate) cosigner_xpubs: Vec<String>,
    pub(crate) threshold_colored: u8,
    pub(crate) threshold_vanilla: u8,
    pub(crate) root_public_key: PublicKey,
    pub(crate) rgb_lib_version: String,
}

pub(crate) struct AppState {
    pub(crate) files_dir: PathBuf,
    pub(crate) database: AppDatabase,
    pub(crate) cancel_token: CancellationToken,
    pub(crate) root_public_key: PublicKey,
    pub(crate) cosigners_by_xpub: HashMap<String, i32>,
    pub(crate) cosigners_by_idx: HashMap<i32, String>,
    pub(crate) threshold_colored: u8,
    pub(crate) threshold_vanilla: u8,
    pub(crate) rgb_lib_version: String,
    pub(crate) write_lock: Arc<Mutex<()>>,
}

pub(crate) fn parse_startup_args_and_config() -> Result<AppParams, AppError> {
    let args = AppArgs::parse();

    create_dir_all(&args.app_directory_path)?;
    let cfg_path = args.app_directory_path.join(CONFIG_NAME);
    if !cfg_path.exists() {
        return Err(AppError::MissingConfigFile(
            cfg_path.to_string_lossy().to_string(),
        ));
    }
    let cfg: AppConfig = confy::load_path(cfg_path)?;

    parse_args_and_config_internal(args, cfg)
}

fn parse_args_and_config_internal(args: AppArgs, cfg: AppConfig) -> Result<AppParams, AppError> {
    let daemon_listening_port = args.daemon_listening_port;
    check_port_is_available(daemon_listening_port)?;

    let num_cosigners = cfg.cosigner_xpubs.len();
    if num_cosigners < MIN_COSIGNERS {
        return Err(AppError::InvalidCosignerNumber(num_cosigners));
    }
    if cfg.threshold_colored == 0 || cfg.threshold_vanilla == 0 {
        return Err(AppError::InvalidThreshold(s!("must be a positive value")));
    }
    if cfg.threshold_colored as usize > num_cosigners
        || cfg.threshold_vanilla as usize > num_cosigners
    {
        return Err(AppError::InvalidThreshold(s!(
            "cannot be higher than number of cosigners"
        )));
    }

    let root_public_key = check_auth_args(&cfg.root_public_key)?;

    // validate rgb-lib version is within supported range
    validate_rgb_lib_version(
        &cfg.rgb_lib_version,
        MIN_RGB_LIB_VERSION,
        MAX_RGB_LIB_VERSION,
    )?;

    Ok(AppParams {
        app_dir: args.app_directory_path,
        daemon_listening_port,
        cosigner_xpubs: cfg.cosigner_xpubs,
        threshold_colored: cfg.threshold_colored,
        threshold_vanilla: cfg.threshold_vanilla,
        root_public_key,
        rgb_lib_version: cfg.rgb_lib_version,
    })
}

fn validate_rgb_lib_version(
    version: &str,
    min_version: &str,
    max_version: &str,
) -> Result<(), AppError> {
    let (major, minor) = parse_version(version)?;
    let (min_major, min_minor) = parse_version(min_version)?;
    let (max_major, max_minor) = parse_version(max_version)?;
    if major < min_major || (major == min_major && minor < min_minor) {
        return Err(AppError::InvalidRgbLibVersion(format!(
            "rgb-lib version {} is below minimum supported version {}",
            version, MIN_RGB_LIB_VERSION
        )));
    }
    if major > max_major || (major == max_major && minor > max_minor) {
        return Err(AppError::InvalidRgbLibVersion(format!(
            "rgb-lib version {} is above maximum supported version {}",
            version, MAX_RGB_LIB_VERSION
        )));
    }
    Ok(())
}

fn parse_version(version: &str) -> Result<(u32, u32), AppError> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 2 {
        return Err(AppError::InvalidRgbLibVersion(format!(
            "version must be in format 'major.minor', got: {}",
            version
        )));
    }
    let major: u32 = parts[0].parse().map_err(|_| {
        AppError::InvalidRgbLibVersion(format!("invalid major version: {}", parts[0]))
    })?;
    let minor: u32 = parts[1].parse().map_err(|_| {
        AppError::InvalidRgbLibVersion(format!("invalid minor version: {}", parts[1]))
    })?;
    Ok((major, minor))
}

#[cfg(not(target_os = "windows"))]
fn adjust_canonicalization<P: AsRef<Path>>(p: P) -> String {
    p.as_ref().display().to_string()
}

#[cfg(target_os = "windows")]
pub(crate) fn adjust_canonicalization<P: AsRef<Path>>(p: P) -> String {
    const VERBATIM_PREFIX: &str = r#"\\?\"#;
    let p = p.as_ref().display().to_string();
    if p.starts_with(VERBATIM_PREFIX) {
        p[VERBATIM_PREFIX.len()..].to_string()
    } else {
        p
    }
}

pub(crate) async fn start_daemon(app_params: &AppParams) -> Result<Arc<AppState>, AppError> {
    let files_dir = app_params.app_dir.join(FILES_DIR);
    create_dir_all(&files_dir)?;
    let logs_dir = app_params.app_dir.join(LOGS_DIR);
    create_dir_all(&logs_dir)?;
    let db_path = app_params.app_dir.join(DB_NAME);
    let display_db_path = adjust_canonicalization(db_path);
    let connection_string = format!("sqlite:{display_db_path}?mode=rwc");
    let mut opt = ConnectOptions::new(connection_string);
    opt.max_connections(app_params.cosigner_xpubs.len() as u32)
        .min_connections(DB_MIN_CONNECTIONS)
        .connect_timeout(DB_TIMEOUT)
        .idle_timeout(DB_TIMEOUT)
        .max_lifetime(DB_TIMEOUT);
    let connection = Database::connect(opt).await?;
    Migrator::up(&connection, None).await?;
    let database = AppDatabase::new(connection);

    let db_cosigners = if let Some(db_config) = database.get_config().await? {
        // already started at least once
        if db_config.threshold_colored != app_params.threshold_colored
            || db_config.threshold_vanilla != app_params.threshold_vanilla
        {
            return Err(AppError::InvalidThreshold(s!(
                "cannot change threshold on already configured service"
            )));
        }
        let db_cosigners = database.iter_cosigners::<AppError>().await?;
        let db_xpubs: HashSet<&String> = db_cosigners.iter().map(|c| &c.xpub).collect();
        let cfg_xpubs: HashSet<&String> = app_params.cosigner_xpubs.iter().collect();
        if db_xpubs != cfg_xpubs {
            return Err(AppError::CannotChangeCosigners);
        }
        db_cosigners
    } else {
        // first start
        // - set DB config
        let config = config::ActiveModel {
            threshold_colored: ActiveValue::Set(app_params.threshold_colored),
            threshold_vanilla: ActiveValue::Set(app_params.threshold_vanilla),
            ..Default::default()
        };
        let idx = database.set_config(config).await?;
        if idx != 1 {
            return Err(AppError::InconsistentState(s!(
                "there should not be a config entry"
            )));
        }
        // - set DB cosigners
        let cosigners = app_params
            .cosigner_xpubs
            .iter()
            .map(|xpub| cosigner::ActiveModel {
                xpub: ActiveValue::Set(xpub.clone()),
                ..Default::default()
            })
            .collect();
        let idx = database.set_cosigners(cosigners).await?;
        if idx as usize != app_params.cosigner_xpubs.len() {
            return Err(AppError::InconsistentState(s!(
                "there should not be cosigners entries"
            )));
        }
        let db_cosigners = database.iter_cosigners::<AppError>().await?;
        // - initialize DB next address
        let index = next_address_index::ActiveModel {
            internal: ActiveValue::Set(0),
            external: ActiveValue::Set(0),
            ..Default::default()
        };
        let idx = database.set_next_address_index(index).await?;
        if idx != 1 {
            return Err(AppError::InconsistentState(s!(
                "there should not be a next address entry"
            )));
        }
        db_cosigners
    };
    let cosigners_by_xpub = db_cosigners
        .iter()
        .map(|c| (c.xpub.clone(), c.idx))
        .collect();
    let cosigners_by_idx = db_cosigners.into_iter().map(|c| (c.idx, c.xpub)).collect();

    let cancel_token = CancellationToken::new();

    Ok(Arc::new(AppState {
        files_dir,
        database,
        cancel_token,
        root_public_key: app_params.root_public_key,
        cosigners_by_xpub,
        cosigners_by_idx,
        threshold_colored: app_params.threshold_colored,
        threshold_vanilla: app_params.threshold_vanilla,
        rgb_lib_version: app_params.rgb_lib_version.clone(),
        write_lock: Arc::new(Mutex::new(())),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn test_parse_version() {
        // valid versions
        assert_eq!(parse_version("1.0").unwrap(), (1, 0));
        assert_eq!(parse_version("1.1").unwrap(), (1, 1));
        assert_eq!(parse_version("2.1").unwrap(), (2, 1));
        assert_eq!(parse_version("0.3").unwrap(), (0, 3));
        assert_eq!(parse_version("10.25").unwrap(), (10, 25));
        assert_eq!(parse_version("01.25").unwrap(), (1, 25));
        assert_eq!(parse_version("1.025").unwrap(), (1, 25));

        // invalid versions
        assert!(matches!(
            parse_version("1").unwrap_err(),
            AppError::InvalidRgbLibVersion(e) if e == "version must be in format 'major.minor', got: 1"
        ));
        assert!(matches!(
            parse_version("1.1.0").unwrap_err(),
            AppError::InvalidRgbLibVersion(e) if e == "version must be in format 'major.minor', got: 1.1.0"
        ));
        assert!(matches!(
            parse_version("abc.def").unwrap_err(),
            AppError::InvalidRgbLibVersion(e) if e.contains("invalid major version")
        ));
        assert!(matches!(
            parse_version("abc.1").unwrap_err(),
            AppError::InvalidRgbLibVersion(e) if e.contains("invalid major version")
        ));
        assert!(matches!(
            parse_version("1.def").unwrap_err(),
            AppError::InvalidRgbLibVersion(e) if e.contains("invalid minor version")
        ));
    }

    #[test]
    fn test_validate_rgb_lib_version() {
        // valid version
        assert!(validate_rgb_lib_version("0.3", "0.3", "0.3").is_ok());
        assert!(validate_rgb_lib_version("0.3", "0.2", "0.4").is_ok());
        assert!(validate_rgb_lib_version("1.2", "0.2", "2.4").is_ok());
        assert!(validate_rgb_lib_version("1.6", "0.2", "2.4").is_ok());
        assert!(validate_rgb_lib_version("1.2", "0.2", "2.0").is_ok());

        // below minimum
        assert!(matches!(
            validate_rgb_lib_version("0.2", "0.3", "0.3").unwrap_err(),
            AppError::InvalidRgbLibVersion(e) if e.contains("below minimum")
        ));

        // above maximum
        assert!(matches!(
            validate_rgb_lib_version("0.4", "0.3", "0.3").unwrap_err(),
            AppError::InvalidRgbLibVersion(e) if e.contains("above maximum")
        ));
    }

    #[test]
    fn test_parse_args_and_config_internal() {
        // valid
        let args = AppArgs {
            app_directory_path: PathBuf::from("test"),
            daemon_listening_port: 3333,
        };
        let config = AppConfig {
            cosigner_xpubs: vec![s!("xpub1"), s!("xpub2")],
            threshold_colored: 2,
            threshold_vanilla: 2,
            root_public_key: s!("0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112ca"),
            rgb_lib_version: s!("0.3"),
        };
        let params = parse_args_and_config_internal(args, config).unwrap();
        assert_eq!(params.app_dir, PathBuf::from("test"));
        assert_eq!(params.daemon_listening_port, 3333);
        assert_eq!(params.cosigner_xpubs, vec![s!("xpub1"), s!("xpub2")]);
        assert_eq!(params.threshold_colored, 2);
        assert_eq!(params.threshold_vanilla, 2);
        assert_eq!(params.rgb_lib_version, s!("0.3"));

        // insufficient cosigners
        let args = AppArgs {
            app_directory_path: PathBuf::from("test"),
            daemon_listening_port: 3333,
        };
        let config = AppConfig {
            cosigner_xpubs: vec![s!("xpub1")],
            threshold_colored: 1,
            threshold_vanilla: 1,
            root_public_key: s!("0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112ca"),
            rgb_lib_version: s!("0.3"),
        };
        let result = parse_args_and_config_internal(args, config);
        assert!(matches!(
            result.unwrap_err(),
            AppError::InvalidCosignerNumber(1)
        ));

        // zero threshold
        let args = AppArgs {
            app_directory_path: PathBuf::from("test"),
            daemon_listening_port: 3333,
        };
        let config = AppConfig {
            cosigner_xpubs: vec![s!("xpub1"), s!("xpub2")],
            threshold_colored: 0,
            threshold_vanilla: 2,
            root_public_key: s!("0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112ca"),
            rgb_lib_version: s!("0.3"),
        };
        let result = parse_args_and_config_internal(args, config);
        assert!(matches!(
            result.unwrap_err(),
            AppError::InvalidThreshold(e) if e == "must be a positive value"
        ));

        // threshold exceeds cosigners
        let args = AppArgs {
            app_directory_path: PathBuf::from("test"),
            daemon_listening_port: 3333,
        };
        let config = AppConfig {
            cosigner_xpubs: vec![s!("xpub1"), s!("xpub2")],
            threshold_colored: 3,
            threshold_vanilla: 2,
            root_public_key: s!("0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112ca"),
            rgb_lib_version: s!("0.3"),
        };
        let result = parse_args_and_config_internal(args, config);
        assert!(matches!(
            result.unwrap_err(),
            AppError::InvalidThreshold(e) if e == "cannot be higher than number of cosigners"
        ));

        // invalid public key
        let args = AppArgs {
            app_directory_path: PathBuf::from("test"),
            daemon_listening_port: 3333,
        };
        let config = AppConfig {
            cosigner_xpubs: vec![s!("xpub1"), s!("xpub2")],
            threshold_colored: 2,
            threshold_vanilla: 2,
            root_public_key: s!("invalid_key"),
            rgb_lib_version: s!("0.3"),
        };
        let result = parse_args_and_config_internal(args, config);
        assert!(matches!(result.unwrap_err(), AppError::InvalidRootKey));

        // invalid rgb-lib version
        let args = AppArgs {
            app_directory_path: PathBuf::from("test"),
            daemon_listening_port: 3333,
        };
        let config = AppConfig {
            cosigner_xpubs: vec![s!("xpub1"), s!("xpub2")],
            threshold_colored: 2,
            threshold_vanilla: 2,
            root_public_key: s!("0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112ca"),
            rgb_lib_version: s!("0.2"),
        };
        let result = parse_args_and_config_internal(args, config);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::InvalidRgbLibVersion(msg) => {
                assert!(
                    msg.contains("below minimum"),
                    "Expected 'below minimum' but got: {}",
                    msg
                );
            }
            e => panic!("Expected InvalidRgbLibVersion error, got: {:?}", e),
        }

        // port unavailable
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let args = AppArgs {
            app_directory_path: PathBuf::from("test"),
            daemon_listening_port: port,
        };
        let config = AppConfig {
            cosigner_xpubs: vec!["xpub1".to_string(), "xpub2".to_string()],
            threshold_colored: 2,
            threshold_vanilla: 2,
            root_public_key: "0606bc5f1e32cb636c96911fc3e97174609d51ee5304a319610f451e8b1112ca"
                .to_string(),
            rgb_lib_version: "0.3".to_string(),
        };
        let result = parse_args_and_config_internal(args, config);
        assert!(matches!(result.unwrap_err(), AppError::UnavailablePort(p) if p == port));
        drop(listener);
    }
}
