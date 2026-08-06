use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub const INSTANCE_ENV: &str = "MOSAICO";
pub const ISOLATED_HOME_ACK_ENV: &str = "MOSAICO_ISOLATED_HOME_OK";
const HOME_ENV: &str = "HOME";
const HOME_OVERRIDE_ENV: &str = "MOSAICO_HOME";
const CONFIG_OVERRIDE_ENV: &str = "MOSAICO_CONFIG";
const DEFAULT_INSTANCE: &str = "default";
const MISSING_HOME_MESSAGE: &str =
    "neither MOSAICO_HOME nor HOME is set: refusing to relocate keystore/config/state.db \
     under ./.mosaico (would mint new agent identities and empty the trust whitelist)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MosaicoHomeSelection {
    pub instance: String,
    pub mosaico_home: PathBuf,
    pub mosaico_home_set: bool,
    pub mosaico_home_is_default: bool,
}

pub fn validate_process_selection() -> Result<(), String> {
    try_mosaico_home_selection().map(|_| ())
}

pub fn selected_instance_env() -> Option<String> {
    std::env::var(INSTANCE_ENV).ok()
}

/// Mosaico's selected writable root. `$MOSAICO=<name>` selects a completely
/// separate named instance; `$MOSAICO_HOME` remains an exact test/lab override.
pub fn mosaico_home() -> PathBuf {
    mosaico_home_selection().mosaico_home
}

pub fn mosaico_home_selection() -> MosaicoHomeSelection {
    try_mosaico_home_selection().unwrap_or_else(|message| panic!("{message}"))
}

fn try_mosaico_home_selection() -> Result<MosaicoHomeSelection, String> {
    select_mosaico_home(
        std::env::var_os(INSTANCE_ENV),
        std::env::var_os(HOME_OVERRIDE_ENV),
        std::env::var_os(CONFIG_OVERRIDE_ENV),
        std::env::var_os(HOME_ENV),
    )
}

pub fn config_path() -> PathBuf {
    select_config_path(
        std::env::var_os(CONFIG_OVERRIDE_ENV),
        mosaico_home_selection(),
    )
}

pub fn isolated_home_acknowledged() -> bool {
    matches!(
        std::env::var(ISOLATED_HOME_ACK_ENV)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

fn select_config_path(
    mosaico_config: Option<OsString>,
    selection: MosaicoHomeSelection,
) -> PathBuf {
    mosaico_config
        .map(PathBuf::from)
        .unwrap_or_else(|| selection.mosaico_home.join("config.json"))
}

fn select_mosaico_home(
    instance: Option<OsString>,
    mosaico_home: Option<OsString>,
    mosaico_config: Option<OsString>,
    home: Option<OsString>,
) -> Result<MosaicoHomeSelection, String> {
    if instance.is_some() && mosaico_home.is_some() {
        return Err("MOSAICO cannot be combined with MOSAICO_HOME".into());
    }
    if instance.is_some() && mosaico_config.is_some() {
        return Err("MOSAICO cannot be combined with MOSAICO_CONFIG".into());
    }

    let default_mosaico_home = nonempty(home)
        .map(PathBuf::from)
        .map(|h| h.join(".mosaico"));
    if let Some(instance) = instance {
        let instance = validate_instance_name(&instance)?;
        let default = default_mosaico_home
            .clone()
            .ok_or_else(|| "HOME must be set when MOSAICO selects an instance".to_string())?;
        if !default.is_absolute() {
            return Err("HOME must be an absolute path when MOSAICO selects an instance".into());
        }
        let mosaico_home = if instance == DEFAULT_INSTANCE {
            default.clone()
        } else {
            default
                .parent()
                .expect("default Mosaico home has a parent")
                .join(".mosaico-instances")
                .join(&instance)
        };
        return Ok(MosaicoHomeSelection {
            mosaico_home_is_default: instance == DEFAULT_INSTANCE,
            instance,
            mosaico_home,
            mosaico_home_set: false,
        });
    }

    if let Some(mosaico_home) = mosaico_home {
        let mosaico_home = PathBuf::from(mosaico_home);
        if mosaico_home.as_os_str().is_empty() {
            return Err("MOSAICO_HOME cannot be empty".into());
        }
        let mosaico_home_is_default = default_mosaico_home.as_ref() == Some(&mosaico_home);
        return Ok(MosaicoHomeSelection {
            instance: DEFAULT_INSTANCE.into(),
            mosaico_home,
            mosaico_home_set: true,
            mosaico_home_is_default,
        });
    }

    let mosaico_home = default_mosaico_home
        .clone()
        .ok_or_else(|| MISSING_HOME_MESSAGE.to_string())?;
    Ok(MosaicoHomeSelection {
        instance: DEFAULT_INSTANCE.into(),
        mosaico_home,
        mosaico_home_set: false,
        mosaico_home_is_default: true,
    })
}

fn nonempty(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}

fn validate_instance_name(value: &OsStr) -> Result<String, String> {
    let value = value
        .to_str()
        .ok_or_else(|| "MOSAICO must be valid UTF-8".to_string())?;
    let valid = (1..=63).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (index > 0 && matches!(byte, b'-' | b'_'))
        });
    if !valid {
        return Err(
            "invalid MOSAICO instance name: use 1-63 lowercase letters, digits, '-' or '_'; \
             the first character must be a letter or digit"
                .into(),
        );
    }
    Ok(value.to_string())
}

#[cfg(test)]
#[path = "home/tests.rs"]
mod tests;
