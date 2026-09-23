use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Deserialize;
use tracing::{info, warn};
use yaydl_shared::{
    AudioCodec, Browser, Language, Notice, NoticeLevel, OutputFormat, Settings, Theme, YaydlError,
    YtDlpChannel, SETTINGS_VERSION,
};

use crate::{
    i18n,
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

/// The schema written after 0.3 and before `language` existed.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct SettingsV2 {
    version: u32,
    output_dir: PathBuf,
    default_format: OutputFormat,
    theme: Theme,
    yt_dlp_channel: YtDlpChannel,
    max_concurrent_downloads: u8,
    embed_metadata: bool,
    notify_on_finish: bool,
    cookies_from_browser: Option<Browser>,
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
        language: Language::System,
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

/// Loads `path`, creating it with defaults when missing, migrating a v1 or v2
/// file, and quarantining an invalid one. `default_output_dir` is only called
/// when defaults are needed. Notices are rendered in the language of the
/// settings that end up in effect.
pub fn load(
    path: &Path,
    default_output_dir: impl FnOnce() -> Result<PathBuf, YaydlError>,
) -> Result<LoadedSettings, YaydlError> {
    let mut quarantined = None;
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
            Ok(Parsed::Migrated { from, settings }) => {
                write_toml_atomic(path, &settings)?;
                info!(path = %path.display(), from, to = SETTINGS_VERSION, "wrote migrated settings");
                settings
            }
            Err(reason) => {
                warn!(path = %path.display(), %reason, "settings file is invalid");
                let moved = quarantine(path)?;
                let settings = default_settings(default_output_dir()?);
                write_toml_atomic(path, &settings)?;
                quarantined = Some((reason, moved));
                settings
            }
        },
    };
    let texts = i18n::texts_for(&settings);
    let mut notices = Vec::new();
    if let Some((reason, moved)) = quarantined {
        notices.push(notice(
            NoticeLevel::Warning,
            (texts.settings_invalid)(&reason, &moved),
        ));
    }
    if !settings.output_dir.is_dir() {
        warn!(output_dir = %settings.output_dir.display(), "the output folder does not exist");
        notices.push(notice(
            NoticeLevel::Warning,
            (texts.output_folder_missing)(&settings.output_dir),
        ));
    }
    Ok(LoadedSettings { settings, notices })
}

#[derive(Debug)]
enum Parsed {
    Current(Settings),
    Migrated { from: u32, settings: Settings },
}

fn parse(raw: &str) -> Result<Parsed, String> {
    let table: toml::Table = toml::from_str(raw).map_err(|e| format!("invalid TOML: {e}"))?;
    match table.get("version") {
        None => migrate_v1(raw),
        Some(toml::Value::Integer(2)) => migrate_v2(raw),
        // Every other version, including one written by a newer yaydl, goes
        // through the strict current schema and `validate`, which names the
        // version it expected.
        Some(_) => {
            let settings: Settings = toml::from_str(raw).map_err(|e| e.message().to_string())?;
            settings.validate()?;
            Ok(Parsed::Current(settings))
        }
    }
}

fn migrate_v2(raw: &str) -> Result<Parsed, String> {
    let v2: SettingsV2 =
        toml::from_str(raw).map_err(|e| format!("version 2 settings: {}", e.message()))?;
    info!(before = ?v2, "migrating version 2 settings");
    let settings = Settings {
        version: SETTINGS_VERSION,
        output_dir: v2.output_dir,
        default_format: v2.default_format,
        theme: v2.theme,
        yt_dlp_channel: v2.yt_dlp_channel,
        max_concurrent_downloads: v2.max_concurrent_downloads,
        embed_metadata: v2.embed_metadata,
        notify_on_finish: v2.notify_on_finish,
        cookies_from_browser: v2.cookies_from_browser,
        language: Language::System,
    };
    settings.validate()?;
    info!(after = ?settings, "migrated version 2 settings");
    Ok(Parsed::Migrated {
        from: v2.version,
        settings,
    })
}

fn migrate_v1(raw: &str) -> Result<Parsed, String> {
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
    Ok(Parsed::Migrated { from: 1, settings })
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
        assert_eq!(on_disk.language, Language::System);
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
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("version = 3"), "{raw}");
        assert!(raw.contains("language = \"system\""), "{raw}");
        let on_disk: Settings = toml::from_str(&raw).unwrap();
        assert_eq!(on_disk, expected);
    }

    fn v2_toml(output_dir: &Path, extra: &str) -> String {
        format!(
            "version = 2\n\
             output_dir = {:?}\n\
             theme = \"dark\"\n\
             yt_dlp_channel = \"stable\"\n\
             max_concurrent_downloads = 3\n\
             embed_metadata = false\n\
             notify_on_finish = true\n\
             cookies_from_browser = \"firefox\"\n\
             {extra}\
             \n\
             [default_format]\n\
             kind = \"video\"\n\
             quality = \"720p\"\n",
            output_dir.display().to_string()
        )
    }

    #[test]
    fn v2_file_is_migrated_to_v3_and_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        fs::write(&path, v2_toml(dir.path(), "")).unwrap();

        let loaded = load(&path, || panic!("defaults are not needed")).unwrap();

        let expected = Settings {
            version: 3,
            output_dir: dir.path().to_path_buf(),
            default_format: OutputFormat::Video {
                quality: yaydl_shared::VideoQuality::P720,
            },
            theme: Theme::Dark,
            yt_dlp_channel: YtDlpChannel::Stable,
            max_concurrent_downloads: 3,
            embed_metadata: false,
            notify_on_finish: true,
            cookies_from_browser: Some(Browser::Firefox),
            language: Language::System,
        };
        assert_eq!(loaded.settings, expected);
        assert!(loaded.notices.is_empty());
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("version = 3"), "{raw}");
        assert!(raw.contains("language = \"system\""), "{raw}");
        let on_disk: Settings = toml::from_str(&raw).unwrap();
        assert_eq!(on_disk, expected);
    }

    #[test]
    fn v2_with_a_language_key_is_rejected() {
        let result = parse(&v2_toml(Path::new("/music"), "language = \"german\"\n"));
        assert!(
            matches!(&result, Err(reason) if reason.contains("language")),
            "{result:?}"
        );
    }

    #[test]
    fn v3_round_trips_without_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let settings = Settings {
            language: Language::German,
            cookies_from_browser: Some(Browser::Brave),
            ..default_settings(dir.path().to_path_buf())
        };
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("language = \"german\""), "{written}");
        fs::write(&path, &written).unwrap();

        let loaded = load(&path, || panic!("defaults are not needed")).unwrap();

        assert_eq!(loaded.settings, settings);
        assert!(loaded.notices.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), written);
    }

    #[test]
    fn v3_without_language_is_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let full = toml::to_string(&default_settings(dir.path().to_path_buf())).unwrap();
        let without: String = full
            .lines()
            .filter(|l| !l.starts_with("language"))
            .map(|l| format!("{l}\n"))
            .collect();
        fs::write(&path, &without).unwrap();

        let loaded = load(&path, fixed_dir(dir.path())).unwrap();

        assert_eq!(quarantined_files(dir.path()).len(), 1);
        assert_eq!(loaded.notices.len(), 1);
        assert!(
            loaded.notices[0].text.contains("language"),
            "{}",
            loaded.notices[0].text
        );
    }

    #[test]
    fn unknown_language_is_quarantined_with_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let original = toml::to_string(&default_settings(dir.path().to_path_buf()))
            .unwrap()
            .replace("language = \"system\"", "language = \"klingon\"");
        assert!(original.contains("klingon"));
        fs::write(&path, &original).unwrap();

        let loaded = load(&path, fixed_dir(dir.path())).unwrap();

        assert_eq!(loaded.settings, default_settings(dir.path().to_path_buf()));
        let moved = quarantined_files(dir.path());
        assert_eq!(moved.len(), 1);
        assert_eq!(fs::read_to_string(&moved[0]).unwrap(), original);
        assert_eq!(loaded.notices.len(), 1);
        assert_eq!(loaded.notices[0].level, NoticeLevel::Warning);
        assert!(
            loaded.notices[0].text.contains("klingon"),
            "{}",
            loaded.notices[0].text
        );
    }

    #[test]
    fn load_notices_use_the_language_in_effect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        let gone = dir.path().join("gone");
        let german = Settings {
            language: Language::German,
            ..default_settings(gone.clone())
        };
        fs::write(&path, toml::to_string(&german).unwrap()).unwrap();

        let loaded = load(&path, || panic!("defaults are not needed")).unwrap();

        assert_eq!(loaded.notices.len(), 1);
        assert_eq!(
            loaded.notices[0].text,
            format!(
                "Der Zielordner {} existiert nicht. Downloads schlagen fehl, bis er wieder da ist oder du in den Einstellungen einen anderen Ordner wählst.",
                gone.display()
            )
        );
    }

    #[test]
    fn v1_light_theme_and_channel_are_kept() {
        let parsed = parse("output_dir = \"/music\"\noutput_format = \"flac\"\ndark_theme = false\nyt_dlp_channel = \"stable\"\n");
        let Ok(Parsed::Migrated { from: 1, settings }) = parsed else {
            panic!("expected a migration from version 1, got {parsed:?}");
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
    fn invalid_v3_is_quarantined_with_a_notice() {
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
