use std::env;
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tar::Archive;
use xz2::read::XzDecoder;

// yt-dlp is only a bootstrap here: the app copies it to app data and self-updates it at runtime.
// It is still pinned so a given commit always bundles the same, verifiable binary.
const YT_DLP_TAG: &str = "2026.08.19";
const YT_DLP_LINUX_SHA256: &str =
    "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a";
const YT_DLP_WINDOWS_SHA256: &str =
    "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a";

// ffmpeg is bundled and never updated at runtime (unlike yt-dlp, which self-updates),
// so it is pinned to a dated build to keep releases reproducible.
const FFMPEG_TAG: &str = "autobuild-2026-09-19-17-14";
const FFMPEG_BUILD: &str = "N-126658-g6397b2b5b6";
const FFMPEG_LINUX_SHA256: &str =
    "310e6190212e347ed02a7cf53d485022fba4083b4aab3660ab954d8a145eb044";
const FFMPEG_WINDOWS_SHA256: &str =
    "433ae9993f6d135e2a226390724b83bf732ef4edc4c795f0d5501aba054b1d2f";

const SIDECARS: [&str; 3] = ["yt-dlp", "ffmpeg", "ffprobe"];
const FFMPEG_BINARIES: [&str; 2] = ["ffmpeg", "ffprobe"];
const VERSIONS_FILE: &str = ".versions";
const EXTRACT_DIR: &str = ".ffmpeg-extract";

enum ArchiveKind {
    TarXz,
    Zip,
}

struct Platform {
    exe_suffix: &'static str,
    yt_dlp_asset: &'static str,
    yt_dlp_sha256: &'static str,
    ffmpeg_variant: &'static str,
    ffmpeg_extension: &'static str,
    ffmpeg_sha256: &'static str,
    ffmpeg_archive: ArchiveKind,
}

impl Platform {
    fn for_target_os(target_os: &str) -> Self {
        match target_os {
            "linux" => Platform {
                exe_suffix: "",
                yt_dlp_asset: "yt-dlp_linux",
                yt_dlp_sha256: YT_DLP_LINUX_SHA256,
                ffmpeg_variant: "linux64-gpl",
                ffmpeg_extension: "tar.xz",
                ffmpeg_sha256: FFMPEG_LINUX_SHA256,
                ffmpeg_archive: ArchiveKind::TarXz,
            },
            "windows" => Platform {
                exe_suffix: ".exe",
                yt_dlp_asset: "yt-dlp.exe",
                yt_dlp_sha256: YT_DLP_WINDOWS_SHA256,
                ffmpeg_variant: "win64-gpl",
                ffmpeg_extension: "zip",
                ffmpeg_sha256: FFMPEG_WINDOWS_SHA256,
                ffmpeg_archive: ArchiveKind::Zip,
            },
            other => panic!(
                "unsupported target OS '{other}': yaydl bundles sidecars only for linux and windows"
            ),
        }
    }

    fn yt_dlp_url(&self) -> String {
        format!(
            "https://github.com/yt-dlp/yt-dlp/releases/download/{YT_DLP_TAG}/{}",
            self.yt_dlp_asset
        )
    }

    fn ffmpeg_archive_root(&self) -> String {
        format!("ffmpeg-{FFMPEG_BUILD}-{}", self.ffmpeg_variant)
    }

    fn ffmpeg_url(&self) -> String {
        format!(
            "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/{FFMPEG_TAG}/{}.{}",
            self.ffmpeg_archive_root(),
            self.ffmpeg_extension
        )
    }
}

fn main() {
    let target_os = required_env("CARGO_CFG_TARGET_OS");
    let target_triple = required_env("TARGET");
    let platform = Platform::for_target_os(&target_os);
    let binaries_dir = PathBuf::from(required_env("CARGO_MANIFEST_DIR")).join("binaries");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=binaries/{VERSIONS_FILE}");

    fs::create_dir_all(&binaries_dir)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", binaries_dir.display()));

    let sidecar_path =
        |name: &str| binaries_dir.join(format!("{name}-{target_triple}{}", platform.exe_suffix));
    let versions_path = binaries_dir.join(VERSIONS_FILE);
    let expected_versions = format!("yt-dlp {YT_DLP_TAG}\nffmpeg {FFMPEG_TAG} {FFMPEG_BUILD}\n");

    let versions_current = match fs::read_to_string(&versions_path) {
        Ok(recorded) => recorded == expected_versions,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => panic!("failed to read {}: {e}", versions_path.display()),
    };
    let sidecars_present = SIDECARS.iter().all(|name| sidecar_path(name).is_file());

    if !(versions_current && sidecars_present) {
        let yt_dlp = download_verified(&platform.yt_dlp_url(), platform.yt_dlp_sha256);
        install_file(&yt_dlp, &sidecar_path("yt-dlp"), true);

        let archive = download_verified(&platform.ffmpeg_url(), platform.ffmpeg_sha256);
        let extract_dir = binaries_dir.join(EXTRACT_DIR);
        // A previous run that panicked mid-extraction leaves this behind, and unpacking over it
        // would mix files from two archives.
        if extract_dir.exists() {
            fs::remove_dir_all(&extract_dir).unwrap_or_else(|e| {
                panic!("failed to remove stale {}: {e}", extract_dir.display())
            });
        }
        extract(
            &platform.ffmpeg_archive,
            archive,
            &extract_dir,
            &platform.ffmpeg_url(),
        );

        let bin_dir = extract_dir.join(platform.ffmpeg_archive_root()).join("bin");
        for name in FFMPEG_BINARIES {
            let source = bin_dir.join(format!("{name}{}", platform.exe_suffix));
            if !source.is_file() {
                panic!(
                    "expected {} in the ffmpeg archive from {}, but it is missing",
                    source.display(),
                    platform.ffmpeg_url()
                );
            }
            let target = sidecar_path(name);
            make_executable(&source);
            fs::rename(&source, &target).unwrap_or_else(|e| {
                panic!(
                    "failed to move {} to {}: {e}",
                    source.display(),
                    target.display()
                )
            });
            println!("installed {}", target.display());
        }
        fs::remove_dir_all(&extract_dir)
            .unwrap_or_else(|e| panic!("failed to remove {}: {e}", extract_dir.display()));

        // Written last so an interrupted download never records versions it did not install.
        install_file(expected_versions.as_bytes(), &versions_path, false);
    }

    tauri_build::build();
}

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|e| panic!("cargo did not provide {name} to build.rs: {e}"))
}

fn download_verified(url: &str, expected_sha256: &str) -> Vec<u8> {
    println!("Downloading {url}");
    // The ffmpeg archives are large, so reqwest's 30s default total timeout is too tight.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .unwrap_or_else(|e| panic!("failed to build HTTP client: {e}"));
    let response = client
        .get(url)
        .send()
        .unwrap_or_else(|e| panic!("failed to request {url}: {e}"));
    let status = response.status();
    if !status.is_success() {
        panic!("failed to download {url}: HTTP {status}");
    }
    let content = response
        .bytes()
        .unwrap_or_else(|e| panic!("failed to read response body from {url}: {e}"))
        .to_vec();

    let actual_sha256 = sha256_hex(&content);
    if actual_sha256 != expected_sha256 {
        panic!(
            "SHA256 mismatch for {url}: expected {expected_sha256}, got {actual_sha256}. \
             Nothing was written."
        );
    }
    content
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn install_file(content: &[u8], target: &Path, executable: bool) {
    let mut temp_name = target
        .file_name()
        .unwrap_or_else(|| panic!("install target {} has no file name", target.display()))
        .to_os_string();
    temp_name.push(".part");
    let temp = target.with_file_name(temp_name);

    fs::write(&temp, content).unwrap_or_else(|e| panic!("failed to write {}: {e}", temp.display()));
    if executable {
        make_executable(&temp);
    }
    fs::rename(&temp, target).unwrap_or_else(|e| {
        panic!(
            "failed to move {} to {}: {e}",
            temp.display(),
            target.display()
        )
    });
    println!("installed {}", target.display());
}

fn extract(kind: &ArchiveKind, archive: Vec<u8>, dest: &Path, url: &str) {
    let reader = Cursor::new(archive);
    let result = match kind {
        ArchiveKind::TarXz => Archive::new(XzDecoder::new(reader))
            .unpack(dest)
            .map_err(|e| e.to_string()),
        ArchiveKind::Zip => zip::ZipArchive::new(reader)
            .and_then(|mut zip| zip.extract(dest))
            .map_err(|e| e.to_string()),
    };
    result.unwrap_or_else(|e| panic!("failed to extract {url} into {}: {e}", dest.display()));
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|e| panic!("failed to chmod 755 {}: {e}", path.display()));
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
