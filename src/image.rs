use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAttachment {
    pub name: String,
    pub media_type: String,
    pub data: String,
}

impl ImageAttachment {
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("opening image {}", path.display()))?;
        let mut bytes = Vec::new();
        file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes)?;
        Self::from_bytes(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            &bytes,
        )
    }

    pub fn from_bytes(name: String, bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            bail!("image exceeds the 5 MiB attachment limit");
        }
        let media_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            "image/gif"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "image/webp"
        } else {
            bail!("unsupported image format; use PNG, JPEG, GIF, or WebP");
        };
        Ok(Self {
            name,
            media_type: media_type.to_string(),
            data: STANDARD.encode(bytes),
        })
    }

    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.data)
    }
}

pub enum ClipboardContent {
    Image(ImageAttachment),
    Text(String),
}

pub async fn clipboard_content() -> Result<ClipboardContent> {
    match clipboard_image().await {
        Ok(image) => Ok(ClipboardContent::Image(image)),
        Err(image_error) => {
            #[cfg(target_os = "macos")]
            let candidates: &[(&str, &[&str])] = &[("pbpaste", &[])];
            #[cfg(not(target_os = "macos"))]
            let candidates: &[(&str, &[&str])] = &[
                ("wl-paste", &["--no-newline", "--type", "text"]),
                (
                    "xclip",
                    &["-selection", "clipboard", "-t", "UTF8_STRING", "-o"],
                ),
            ];
            for (program, args) in candidates {
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    Command::new(program)
                        .args(*args)
                        .stdin(Stdio::null())
                        .stderr(Stdio::null())
                        .kill_on_drop(true)
                        .output(),
                )
                .await;
                if let Ok(Ok(output)) = output
                    && output.status.success()
                {
                    return Ok(ClipboardContent::Text(
                        String::from_utf8(output.stdout).context("clipboard text is not UTF-8")?,
                    ));
                }
            }
            Err(image_error)
        }
    }
}

async fn clipboard_image() -> Result<ImageAttachment> {
    #[cfg(target_os = "macos")]
    let candidates: &[(&str, &[&str])] = &[("pngpaste", &["-"])];
    #[cfg(not(target_os = "macos"))]
    let candidates: &[(&str, &[&str])] = &[
        ("wl-paste", &["--no-newline", "--type", "image/png"]),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
        ),
    ];
    let mut failures = Vec::new();
    for (program, args) in candidates {
        let mut child = match Command::new(program)
            .args(*args)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("starting clipboard reader"),
        };
        let mut bytes = Vec::new();
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            child
                .stdout
                .take()
                .unwrap()
                .take(MAX_IMAGE_BYTES + 1)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() as u64 > MAX_IMAGE_BYTES {
                bail!("clipboard image exceeds the 5 MiB attachment limit");
            }
            Ok::<_, anyhow::Error>(child.wait().await?)
        })
        .await
        .context("clipboard reader timed out")??;
        if result.success() {
            return ImageAttachment::from_bytes("clipboard.png".into(), &bytes);
        }
        failures.push(*program);
    }
    if failures.is_empty() {
        bail!(
            "no clipboard image reader found; install wl-clipboard or xclip on Linux, or pngpaste on macOS; paste an image file path into the chatbox instead"
        );
    }
    bail!("clipboard has no readable PNG image; paste an image file path into the chatbox instead")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_detected_from_bytes_and_oversized_inputs_are_rejected() {
        for (bytes, media) in [
            (&b"\x89PNG\r\n\x1a\n"[..], "image/png"),
            (&b"\xff\xd8\xff"[..], "image/jpeg"),
            (&b"GIF89a"[..], "image/gif"),
            (&b"RIFF1234WEBP"[..], "image/webp"),
        ] {
            let image = ImageAttachment::from_bytes("test".into(), bytes).unwrap();
            assert_eq!(image.media_type, media);
            assert_eq!(STANDARD.decode(image.data).unwrap(), bytes);
        }
        assert!(ImageAttachment::from_bytes("bad.png".into(), b"not an image").is_err());
        assert!(
            ImageAttachment::from_bytes("large.png".into(), &vec![0; MAX_IMAGE_BYTES as usize + 1])
                .is_err()
        );
    }
}
