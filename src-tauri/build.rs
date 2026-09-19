use std::env;
#[cfg(target_os = "linux")]
use std::fs::Permissions;
use std::fs::{self, create_dir_all, File};
use std::io::{self, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tar::Archive;
use xz2::read::XzDecoder;

const LINUX_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux";
const WINDOWS_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";

// ffmpeg is bundled and never updated at runtime (unlike yt-dlp, which self-updates),
// so it is pinned to a dated build to keep releases reproducible.
const FFMPEG_TAG: &str = "autobuild-2026-09-19-17-14";
const FFMPEG_BUILD: &str = "N-126658-g6397b2b5b6";

const DOWNLOAD_DIR: &str = "./binaries";
const FFMPEG_BINARIES: [&str; 2] = ["ffmpeg", "ffprobe"];

fn main() -> io::Result<()> {
    let os = env::consts::OS;
    let target_triple = get_target_triple();
    let binaries = vec!["yt-dlp", "ffmpeg", "ffprobe"];
    let suffix = if os == "windows" { ".exe" } else { "" };

    for binary in &binaries {
        println!("cargo:rerun-if-changed={DOWNLOAD_DIR}/{binary}-{target_triple}{suffix}");
    }

    create_dir_all(DOWNLOAD_DIR)?;
    if !check_if_binaries_exist(&binaries, &target_triple, suffix)? {
        let (yt_dlp_url, ffmpeg_url, ffmpeg_archive_name) = match os {
            "linux" => (
                LINUX_URL.to_string(),
                ffmpeg_url("linux64-gpl.tar.xz"),
                "ffmpeg-archive.tar.xz",
            ),
            "windows" => (
                WINDOWS_URL.to_string(),
                ffmpeg_url("win64-gpl.zip"),
                "ffmpeg-archive.zip",
            ),
            other => panic!(
                "Unsupported operating system '{other}': yaydl builds only on linux and windows"
            ),
        };

        let yt_dlp_path = PathBuf::from(DOWNLOAD_DIR).join("yt-dlp");
        download_and_save(&yt_dlp_url, &yt_dlp_path)?;
        rename_with_target_triple(&yt_dlp_path, &target_triple, os)?;

        let archive_path = PathBuf::from(DOWNLOAD_DIR).join(ffmpeg_archive_name);
        download_and_save(&ffmpeg_url, &archive_path)?;
        extract_and_rename_ffmpeg_binaries(&archive_path, &target_triple, os)?;
    }

    tauri_build::build();
    Ok(())
}

fn ffmpeg_url(asset_suffix: &str) -> String {
    format!(
        "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/{FFMPEG_TAG}/ffmpeg-{FFMPEG_BUILD}-{asset_suffix}"
    )
}

fn check_if_binaries_exist(
    binaries: &[&str],
    target_triple: &str,
    suffix: &str,
) -> io::Result<bool> {
    let binaries_triple: Vec<String> = binaries
        .iter()
        .map(|b| format!("{b}-{target_triple}{suffix}"))
        .collect();

    let existing_files: std::collections::HashSet<String> = fs::read_dir(DOWNLOAD_DIR)?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.path().file_name()?.to_str().map(String::from))
        .collect();

    let all_exist = binaries_triple
        .iter()
        .all(|binary| existing_files.contains(binary));
    Ok(all_exist)
}

fn extract_and_rename_ffmpeg_binaries(
    archive_path: &PathBuf,
    target_triple: &str,
    os: &str,
) -> io::Result<()> {
    let file = File::open(archive_path)?;
    match os {
        "linux" => {
            let xz_decoder = XzDecoder::new(file);
            Archive::new(xz_decoder).unpack(DOWNLOAD_DIR)?;
        }
        "windows" => zip::ZipArchive::new(file)?.extract(DOWNLOAD_DIR)?,
        other => {
            panic!("Unsupported operating system '{other}': yaydl builds only on linux and windows")
        }
    }

    let mut extracted_root = None;
    for entry in fs::read_dir(DOWNLOAD_DIR)? {
        let path = entry?.path();
        if path.is_dir() && path.join("bin").exists() {
            extracted_root = Some(path);
            break;
        }
    }

    let extracted_root = extracted_root.ok_or_else(|| {
        io::Error::other(format!(
            "no directory containing a 'bin' folder found after extracting {}",
            archive_path.display()
        ))
    })?;

    let extension = if os == "windows" { "exe" } else { "" };
    for name in FFMPEG_BINARIES {
        let source_path = extracted_root
            .join("bin")
            .join(name)
            .with_extension(extension);
        if !source_path.exists() {
            return Err(io::Error::other(format!(
                "expected {} in the extracted ffmpeg archive, but it is missing",
                source_path.display()
            )));
        }
        let target_path = Path::new(DOWNLOAD_DIR).join(source_path.file_name().unwrap());
        fs::rename(&source_path, &target_path)?;
        rename_with_target_triple(&target_path, target_triple, os)?;
    }

    fs::remove_dir_all(&extracted_root)?;
    fs::remove_file(archive_path)?;

    Ok(())
}

fn download_and_save(url: &str, dest_path: &Path) -> io::Result<()> {
    println!("Downloading {} to {}...", url, dest_path.display());
    let response =
        reqwest::blocking::get(url).unwrap_or_else(|e| panic!("Failed to request {url}: {e}"));
    let status = response.status();
    if !status.is_success() {
        panic!("Failed to download {url}: HTTP {status}");
    }
    let content = response
        .bytes()
        .unwrap_or_else(|e| panic!("Failed to read response body from {url}: {e}"));

    let mut file = File::create(dest_path)?;
    file.write_all(&content)?;

    Ok(())
}

fn get_target_triple() -> String {
    let output = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .unwrap();
    let rustc_output = std::str::from_utf8(&output.stdout).unwrap();
    let host = rustc_output
        .split("\n")
        .nth(4)
        .unwrap()
        .split_once("host: ");
    let (_, target_triple) = host.unwrap();
    target_triple.to_string()
}

fn rename_with_target_triple(binary_path: &Path, target_triple: &str, os: &str) -> io::Result<()> {
    let extension = if os == "windows" { ".exe" } else { "" };
    let new_path = format!(
        "{}-{}{}",
        binary_path.with_extension("").display(),
        target_triple,
        extension
    );

    fs::rename(binary_path, &new_path)?;
    println!("File renamed to {}", new_path);

    #[cfg(target_os = "linux")]
    {
        let perms = Permissions::from_mode(0o755);
        fs::set_permissions(&new_path, perms)?;
        println!("Set executable permissions on {}", new_path);
    }

    Ok(())
}
