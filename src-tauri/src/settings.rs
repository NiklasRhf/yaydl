use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Deserialize;
use tracing::{info, warn};
use yaydl_shared::{
    AudioCodec, Notice, NoticeLevel, OutputFormat, Settings, Theme, YaydlError, YtDlpChannel,
    SETTINGS_VERSION,
};

use crate::{
    notices::notice,
    persist::{quarantine, read_optional, write_toml_atomic},
};

pub const SETTINGS_FILE: &str = "settings.toml";

/// The schema written by yaydl up to 0.3, before `version` existed.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SettingsV1 {
    output_dir: PathBuf,
    output_format: String,
    dark_theme: bool,
    yt_dlp_channel: Option<YtDlpChannel>,
}

pub struct LoadedSettings {
    pub settings: Settings,
    /// For the user, raised by the caller once notices can be routed.
    pub notices: Vec<Notice>,
}

pub fn default_settings(output_dir: PathBuf) -> Settings {
    Settings {
        version: SETTINGS_VERSION,
        output_dir,
        default_format: OutputFormat::Audio {
            codec: AudioCodec::Mp3,
        },
        theme: Theme::System,
        yt_dlp_channel: YtDlpChannel::Nightly,
        max_concurrent_downloads: 2,
        embed_metadata: true,
        notify_on_finish: true,
        cookies_from_browser: None,
    }
}

/// The first existing directory of the platform's music, downloads and home
/// directories.
pub fn default_output_dir() -> Result<PathBuf, YaydlError> {
    let candidates = [
        ("audio", dirs::audio_dir()),
        ("download", dirs::download_dir()),
        ("home", dirs::home_dir()),
    ];
    for (kind, dir) in &candidates {
        match dir {
            Some(dir) if dir.is_dir() => {
                info!(kind, dir = %dir.display(), "picked the default output folder");
                return Ok(dir.clone());
            }
            Some(dir) => {
                info!(kind, dir = %dir.display(), "default output folder candidate does not exist")
            }
            None => info!(kind, "the platform reports no such directory"),
        }
    }
    Err(YaydlError::InvalidSettings(
        "none of the music, downloads or home directories exists, so there is no default output folder"
            .to_string(),
    ))
}

/// Loads `path`, creating it with defaults when missing, migrating a v1 file,
/// and quarantining an invalid one. `default_output_dir` is only called when
/// defaults are needed.
pub fn load(
    path: &Path,
    default_output_dir: impl FnOnce() -> Result<PathBuf, YaydlError>,
) -> Result<LoadedSettings, YaydlError> {
    let mut notices = Vec::new();
    let settings = match read_optional(path)? {
        None => {
            let settings = default_settings(default_output_dir()?);
            info!(path = %path.display(), ?settings, "no settings file, writing defaults");
            write_toml_atomic(path, &settings)?;
            settings
        }
        Some(raw) => match parse(&raw) {
            Ok(Parsed::Current(settings)) => {
                info!(path = %path.display(), ?settings, "loaded settings");
                settings
            }
            Ok(Parsed::MigratedFromV1(settings)) => {
                write_toml_atomic(path, &settings)?;
                info!(path = %path.display(), "wrote migrated v2 settings");
                settings
            }
            Err(reason) => {
                warn!(path = %path.display(), %reason, "settings file is invalid");
                let moved = quarantine(path)?;
                let settings = default_settings(default_output_dir()?);
                write_toml_atomic(path, &settings)?;
                notices.push(notice(
                    NoticeLevel::Warning,
                    format!(
                        "Your settings file was invalid ({reason}) and was moved to {}. yaydl started with default settings.",
                        moved.display()
                    ),
                ));
                settings
            }
        },
    };
    if !settings.output_dir.is_dir() {
        warn!(output_dir = %settings.output_dir.display(), "the output folder does not exist");
        notices.push(notice(
            NoticeLevel::Warning,
            format!(
                "The output folder {} does not exist. Downloads will fail until it is back or you choose another folder in Settings.",
                settings.output_dir.display()
            ),
        ));
    }
    Ok(LoadedSettings { settings, notices })
}

enum Parsed {
    Current(Settings),
    MigratedFromV1(Settings),
}

fn parse(raw: &str) -> Result<Parsed, String> {
    let table: toml::Table = toml::from_str(raw).map_err(|e| format!("invalid TOML: {e}"))?;
    if table.contains_key("version") {
        let settings: Settings = toml::from_str(raw).map_err(|e| e.message().to_string())?;
        settings.validate()?;
        return Ok(Parsed::Current(settings));
    }
    let v1: SettingsV1 =
        toml::from_str(raw).map_err(|e| format!("version 1 settings: {}", e.message()))?;
    info!(before = ?v1, "migrating version 1 settings");
    let codec = AudioCodec::parse(&v1.output_format).ok_or_else(|| {
        format!(
            "version 1 settings: output_format \"{}\" is not one of mp3, m4a, opus, flac",
            v1.output_format
        )
    })?;
    let settings = Settings {
        output_dir: v1.output_dir,
        default_format: OutputFormat::Audio { codec },
        theme: if v1.dark_theme {
            Theme::Dark
        } else {
            Theme::Light
        },
        yt_dlp_channel: v1.yt_dlp_channel.unwrap_or_default(),
        ..default_settings(PathBuf::new())
    };
    settings.validate()?;
    info!(after = ?settings, "migrated version 1 settings");
    Ok(Parsed::MigratedFromV1(settings))
}

/// `Settings::validate` plus the filesystem check it leaves to the backend.
pub fn validate_for_update(settings: &Settings) -> Result<(), YaydlError> {
    settings.validate().map_err(YaydlError::InvalidSettings)?;
    if !settings.output_dir.is_dir() {
        return Err(YaydlError::InvalidSettings(format!(
            "the output folder {} does not exist or is not a folder",
            settings.output_dir.display()
        )));
    }
    Ok(())
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    pub fn new(path: PathBuf, settings: Settings) -> Self {
        Self {
            path,
            current: Mutex::new(settings),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().expect("settings lock poisoned").clone()
    }

    pub fn update(&self, settings: Settings) -> Result<Settings, YaydlError> {
        validate_for_update(&settings)?;
        // Held across the write so concurrent updates cannot leave the file and
        // the in-memory value describing different settings.
        let mut current = self.current.lock().expect("settings lock poisoned");
        write_toml_atomic(&self.path, &settings)?;
        info!(before = ?*current, after = ?settings, "settings updated");
        *current = settings.clone();
        Ok(settings)
    }
}

// AGENT CODE: claude-opus-5
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixed_dir(dir: &Path) -> impl FnOnce() -> Result<PathBuf, YaydlError> {
        let dir = dir.to_path_buf();
        move || Ok(dir)
    }

    fn quarantined_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("settings.toml.invalid-")
            })
            .collect()
    }

    #[test]
    fn missing_file_writes_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);

        let loaded = load(&path, fixed_dir(dir.path())).unwrap();

        assert_eq!(loaded.settings, default_settings(dir.path().to_path_buf()));
        assert!(loaded.notices.is_empty());
        let on_disk: Settings = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, loaded.settings);
        assert_eq!(on_disk.max_concurrent_downloads, 2);
        assert_eq!(on_disk.theme, Theme::System);
        assert_eq!(on_disk.yt_dlp_channel, YtDlpChannel::Nightly);
    }

    #[test]
    fn v1_file_is_migrated_and_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        fs::write(
            &path,
            format!(
                "output_dir = {:?}\noutput_format = \"mp3\"\ndark_theme = true\n",
                dir.path().display().to_string()
            ),
        )
        .unwrap();

        let loaded = load(&path, || panic!("defaults are not needed")).unwrap();

        let expected = Settings {
            output_dir: dir.path().to_path_buf(),
            default_format: OutputFormat::Audio {
                codec: AudioCodec::Mp3,
            },
            theme: Theme::Dark,
            ..default_settings(PathBuf::new())
        };
        assert_eq!(loaded.settings, expected);
        assert!(loaded.notices.is_empty());
        let on_disk: Settings = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, expected);
        assert_eq!(on_disk.version, SETTINGS_VERSION);
    }

    #[test]
    fn v1_light_theme_and_channel_are_kept() {
        let parsed = parse("output_dir = \"/music\"\noutput_format = \"flac\"\ndark_theme = false\nyt_dlp_channel = \"stable\"\n");
        let Ok(Parsed::MigratedFromV1(settings)) = parsed else {
            panic!("expected a migration");
        };
        assert_eq!(settings.theme, Theme::Light);
        assert_eq!(settings.yt_dlp_channel, YtDlpChannel::Stable);
        assert_eq!(
            settings.default_format,
            OutputFormat::Audio {
                codec: AudioCodec::Flac
            }
        );
    }

    #[test]
    fn v1_unknown_format_is_rejected() {
        let result = parse("output_dir = \"/music\"\noutput_format = \"wav\"\ndark_theme = true\n");
        assert!(matches!(result, Err(reason) if reason.contains("wav")));
    }

    #[test]
    fn v1_unknown_key_is_quarantined_with_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let original =
            "output_dir = \"/music\"\noutput_format = \"mp3\"\ndark_theme = true\nvolume = 3\n";
        fs::write(&path, original).unwrap();

        let loaded = load(&path, fixed_dir(dir.path())).unwrap();

        assert_eq!(loaded.settings, default_settings(dir.path().to_path_buf()));
        let moved = quarantined_files(dir.path());
        assert_eq!(moved.len(), 1);
        assert_eq!(fs::read_to_string(&moved[0]).unwrap(), original);
        assert_eq!(loaded.notices.len(), 1);
        let n = &loaded.notices[0];
        assert_eq!(n.level, NoticeLevel::Warning);
        assert!(n.text.contains("volume"), "{}", n.text);
        assert!(
            n.text.contains(&moved[0].display().to_string()),
            "{}",
            n.text
        );
    }

    #[test]
    fn invalid_v2_is_quarantined_with_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let mut bad = default_settings(dir.path().to_path_buf());
        bad.max_concurrent_downloads = 9;
        fs::write(&path, toml::to_string(&bad).unwrap()).unwrap();

        let loaded = load(&path, fixed_dir(dir.path())).unwrap();

        assert_eq!(loaded.settings, default_settings(dir.path().to_path_buf()));
        assert_eq!(quarantined_files(dir.path()).len(), 1);
        assert_eq!(loaded.notices.len(), 1);
        assert!(loaded.notices[0].text.contains("max_concurrent_downloads"));
    }

    #[test]
    fn missing_output_dir_is_kept_with_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let gone = dir.path().join("gone");
        fs::write(
            &path,
            toml::to_string(&default_settings(gone.clone())).unwrap(),
        )
        .unwrap();

        let loaded = load(&path, || panic!("defaults are not needed")).unwrap();

        assert_eq!(loaded.settings.output_dir, gone);
        assert_eq!(loaded.notices.len(), 1);
        assert_eq!(loaded.notices[0].level, NoticeLevel::Warning);
    }

    #[test]
    fn update_rejects_a_missing_dir_and_zero_concurrency() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let initial = default_settings(dir.path().to_path_buf());
        let store = SettingsStore::new(path.clone(), initial.clone());

        let missing = Settings {
            output_dir: dir.path().join("nope"),
            ..initial.clone()
        };
        assert!(matches!(
            store.update(missing),
            Err(YaydlError::InvalidSettings(_))
        ));

        let zero = Settings {
            max_concurrent_downloads: 0,
            ..initial.clone()
        };
        assert!(matches!(
            store.update(zero),
            Err(YaydlError::InvalidSettings(_))
        ));
        assert_eq!(store.get(), initial);
        assert!(!path.exists());

        let more = Settings {
            max_concurrent_downloads: 4,
            ..initial
        };
        assert_eq!(store.update(more.clone()).unwrap(), more);
        assert_eq!(store.get(), more);
        let on_disk: Settings = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, more);
    }
}
