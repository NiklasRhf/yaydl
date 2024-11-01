#!/bin/bash

# TODO: add macos maybe

LINUX_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
WINDOWS_URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
FFMPEG_WIN_URL="https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win32-gpl.zip"
FFMPEG_LINUX_URL="https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
DOWNLOAD_DIR="src-tauri/binaries"
DOWNLOAD_PATH="$DOWNLOAD_DIR/yt-dlp"
FFMPEG_DOWNLOAD_PATH_WIN="$DOWNLOAD_DIR/ffmpeg.zip"
FFMPEG_DOWNLOAD_PATH_LINUX="$DOWNLOAD_DIR/ffmpeg.tar.xz"

# Determine platform and set executable extension if Windows
EXTENSION=""
URL="$LINUX_URL"
FFMPEG_URL="$FFMPEG_LINUX_URL"
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
  EXTENSION=".exe"
  URL="$WINDOWS_URL"
  FFMPEG_URL="$FFMPEG_WIN_URL"
fi

mkdir -p "$DOWNLOAD_DIR"

download_file() {
  local url=$1
  local output_path=$2
  echo "Downloading $url to $output_path..."
  curl -L "$url" -o "$output_path"
  if [[ $? -ne 0 ]]; then
    echo "Error downloading file."
    exit 1
  fi
}

rename_binary() {
  local binary_path=$1
  local rust_info
  rust_info=$(rustc -vV 2>/dev/null | grep 'host' | awk '{print $2}')
  if [[ -z "$rust_info" ]]; then
    echo "Failed to determine platform target triple."
    exit 1
  fi
  local new_path="${binary_path}-${rust_info}${EXTENSION}"
  mv "$binary_path" "$new_path"
  echo "File renamed to $new_path"
  if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    chmod 755 "$new_path"
    echo "$new_path is now executable"
  fi
}

# Extract and rename binaries in the bin directory of ffmpeg archive
extract_and_rename_ffmpeg_binaries() {
  local archive_path=$1
  local extract_dir="$DOWNLOAD_DIR/extracted_ffmpeg"

  mkdir -p "$extract_dir"
  if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    unzip -d "$extract_dir" "$archive_path"
  else
    tar -xJf "$archive_path" -C "$extract_dir"
  fi

  local bin_dir
  bin_dir=$(find "$extract_dir" -type d -name "bin" | head -n 1)

  if [[ -d "$bin_dir" ]]; then
    for file in "$bin_dir"/*; do
      if [[ -f "$file" ]]; then
        local base_name=$(basename "${file%.*}")
        local dest_path="$DOWNLOAD_DIR/$base_name"
        mv "$file" "$dest_path"
        rename_binary "$dest_path"
      fi
    done
  else
    echo "No 'bin' directory found in the extracted ffmpeg archive."
  fi

  rm -rf "$extract_dir"
}

main() {
  # Download yt-dlp
  download_file "$URL" "$DOWNLOAD_PATH"
  # Rename to target triple
  rename_binary "$DOWNLOAD_PATH"

  # Download and extract ffmpeg
  local ffmpeg_archive_path="$FFMPEG_DOWNLOAD_PATH_LINUX"
  if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    ffmpeg_archive_path="$FFMPEG_DOWNLOAD_PATH_WIN"
  fi
  download_file "$FFMPEG_URL" "$ffmpeg_archive_path"
  # extract and rename to target triple
  extract_and_rename_ffmpeg_binaries "$ffmpeg_archive_path"

  ls -l $DOWNLOAD_DIR
}

main
