use std::env;
use std::fs::{self, create_dir_all, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::fs::Permissions;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;

use tar::Archive;
use xz2::read::XzDecoder;

const LINUX_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux";
const WINDOWS_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
const FFMPEG_WIN_URL: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win32-gpl.zip";
const FFMPEG_LINUX_URL: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz";
const DOWNLOAD_DIR: &str = "./binaries";

fn main() -> io::Result<()> {
    let os = env::consts::OS;
    let target_triple = get_target_triple();
    let binaries = vec!["yt-dlp", "ffmpeg", "ffprobe", "ffplay"];

    println!("cargo:rerun-if-changed=./binaries/{}{}", binaries[0], target_triple);
    println!("cargo:rerun-if-changed=./binaries/{}{}", binaries[1], target_triple);
    println!("cargo:rerun-if-changed=./binaries/{}{}", binaries[2], target_triple);
    println!("cargo:rerun-if-changed=./binaries/{}{}", binaries[3], target_triple);

    create_dir_all(DOWNLOAD_DIR)?;
    if !check_if_binaries_exist(&binaries, &target_triple)? {
        let (yt_dlp_url, ffmpeg_url) = match os {
            "linux" => (LINUX_URL, FFMPEG_LINUX_URL),
            "windows" => (WINDOWS_URL, FFMPEG_WIN_URL),
            _ => panic!("Unsupported operating system"),
        };

        let yt_dlp_path = PathBuf::from(DOWNLOAD_DIR).join("yt-dlp");
        download_and_save(yt_dlp_url, &yt_dlp_path)?;
        rename_with_target_triple(&yt_dlp_path, &target_triple, os)?;

        let ffmpeg_path = PathBuf::from(DOWNLOAD_DIR).join("ffmpeg");
        download_and_save(ffmpeg_url, &ffmpeg_path)?;
        extract_and_rename_ffmpeg_binaries(&ffmpeg_path, &target_triple, os)?;
    }

    tauri_build::build();
    Ok(())
}

fn check_if_binaries_exist(binaries: &[&str], target_triple: &str) -> io::Result<bool> {
    let binaries_triple: Vec<String> = binaries
        .iter()
        .map(|b| format!("{b}-{target_triple}"))
        .collect();

    let existing_files: std::collections::HashSet<String> = fs::read_dir(DOWNLOAD_DIR)?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.path().file_name()?.to_str().map(String::from))
        .collect();

    let all_exist = binaries_triple.iter().all(|binary| existing_files.contains(binary));
    Ok(all_exist)
}

 fn extract_and_rename_ffmpeg_binaries(path: &PathBuf, target_triple: &str, os: &str) -> io::Result<()> {
    let file = File::open(path)?;
    if os == "linux" {
        let xz_decoder = XzDecoder::new(file);
        let mut archive = Archive::new(xz_decoder);
        archive.unpack(DOWNLOAD_DIR)?;
    } else if os == "windows" {
        zip::ZipArchive::new(file)?.extract(DOWNLOAD_DIR)?;
    }
    let mut bin_path = None;
    for entry in fs::read_dir(DOWNLOAD_DIR)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("bin").exists() {
            bin_path = Some(path.join("bin"));
            break;
        }
    }
    if let Some(bin_dir) = bin_path {
        for entry in fs::read_dir(bin_dir)? {
            let source_path = entry?.path();
            let file_name = source_path.file_name().unwrap();
            let target_path = Path::new(DOWNLOAD_DIR).join(file_name);
            fs::rename(source_path, &target_path)?;
            rename_with_target_triple(&target_path, target_triple, os)?;
        }
    } else {
        eprintln!("Could not find the 'bin' directory in the extracted archive.");
    }
    Ok(())
}

fn download_and_save(url: &str, dest_path: &Path) -> io::Result<()> {
    println!("Downloading {} to {}...", url, dest_path.display());
    let response = reqwest::blocking::get(url).expect("Failed to make request");
    let mut file = File::create(dest_path)?;
    let content = response.bytes().expect("Failed to read response bytes");
    file.write_all(&content)?;

    Ok(())
}

fn get_target_triple() -> String {
    let output = std::process::Command::new("rustc").arg("-vV").output().unwrap();
    let rustc_output = std::str::from_utf8(&output.stdout).unwrap();
    let host = rustc_output.split("\n").nth(4).unwrap().split_once("host: ");
    let (_, target_triple) = host.unwrap();
    target_triple.to_string()
}

fn rename_with_target_triple(binary_path: &Path, target_triple: &str, os: &str) -> io::Result<()> {
    let extension = if os.contains("windows") { ".exe" } else { "" };
    let new_path = format!("{}-{}{}", binary_path.with_extension("").display(), target_triple, extension);

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
