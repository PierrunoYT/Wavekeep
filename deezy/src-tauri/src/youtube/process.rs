//! Bounded, cancellable child-process execution for yt-dlp and ffmpeg.

use std::process::ExitStatus;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::audio::until_cancelled;

const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(120);

/// Bound concurrent extractor work without serializing downloads.
static PROCESS_SLOTS: Semaphore = Semaphore::const_new(8);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct ToolOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ToolOutput {
    pub fn success(self, program: &str) -> Result<Self, String> {
        if self.status.success() {
            return Ok(self);
        }
        Err(process_error(program, &self.stderr))
    }
}

fn process_error(program: &str, stderr: &[u8]) -> String {
    static URL: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"https?://\S+").expect("URL pattern is valid"));
    static SEARCH: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"ytsearch\d*:[^\r\n]+").expect("search pattern is valid")
    });
    let text = String::from_utf8_lossy(stderr);
    let detail = text
        .lines()
        .rev()
        .find(|line| line.contains("ERROR:"))
        .or_else(|| text.lines().rev().find(|line| !line.trim().is_empty()))
        .unwrap_or("unknown error");
    let detail = URL.replace_all(detail, "[url]");
    let detail = SEARCH.replace_all(&detail, "ytsearch:[hidden]");
    let detail: String = detail.chars().take(500).collect();
    format!("{} failed: {}", program, detail)
}

fn spawn_error(program: &str, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        return format!("YouTube needs {} on PATH. Install yt-dlp and ffmpeg, then restart Wavekeep", program);
    }
    format!("Could not start {}: {}", program, error)
}

async fn read_bounded<R>(mut reader: R, cap: usize) -> Result<(Vec<u8>, bool), String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let mut overflow = false;
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > cap {
            overflow = true;
        } else if !overflow {
            bytes.extend_from_slice(&buffer[..read]);
        }
    }
    Ok((bytes, overflow))
}

/// Runs one executable and drains both output streams. On cancellation or
/// timeout the future drops the child, and `kill_on_drop` terminates it.
pub async fn run_tool(
    program: &str,
    args: &[String],
    cancel_flag: Option<&AtomicBool>,
) -> Result<Option<ToolOutput>, String> {
    let run = async {
        let _slot = PROCESS_SLOTS.acquire().await.map_err(|_| "Process queue closed".to_string())?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // A release build has no console, so Windows would open one per child.
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().map_err(|e| spawn_error(program, e))?;
        let stdout = child.stdout.take().ok_or("Could not read child output")?;
        let stderr = child.stderr.take().ok_or("Could not read child errors")?;
        let output = tokio::time::timeout(PROCESS_TIMEOUT, async move {
            let (status, stdout, stderr) = tokio::join!(
                child.wait(),
                read_bounded(stdout, MAX_JSON_BYTES),
                read_bounded(stderr, MAX_STDERR_BYTES)
            );
            let status = status.map_err(|e| format!("Could not wait for {}: {}", program, e))?;
            let (stdout, stdout_overflow) = stdout?;
            let (stderr, _) = stderr?;
            if stdout_overflow {
                return Err(format!("{} returned too much data", program));
            }
            Ok::<ToolOutput, String>(ToolOutput { status, stdout, stderr })
        })
        .await
        .map_err(|_| format!("{} timed out", program))??;
        Ok::<ToolOutput, String>(output)
    };
    match cancel_flag {
        Some(flag) => until_cancelled(run, flag).await.transpose(),
        None => run.await.map(Some),
    }
}

pub async fn ytdl_json(
    target: &str,
    flat: bool,
    limit: usize,
    single_video: bool,
    cancel_flag: Option<&AtomicBool>,
) -> Result<Option<Value>, String> {
    let mut args = vec![
        "--ignore-config".to_string(),
        "--no-update".to_string(),
        "--dump-single-json".to_string(),
        "--playlist-end".to_string(),
        limit.max(1).to_string(),
    ];
    if flat {
        args.push("--flat-playlist".to_string());
    }
    if single_video {
        args.push("--no-playlist".to_string());
        args.push("--format".to_string());
        args.push(
            "bestaudio[ext=webm][protocol=https]/bestaudio[ext=m4a][protocol=https]/bestaudio[ext=mp3][protocol=https]/bestaudio[protocol=https]"
                .to_string(),
        );
    }
    args.push("--".to_string());
    args.push(target.to_string());
    let output = run_tool("yt-dlp", &args, cancel_flag)
        .await?
        .map(|output| output.success("yt-dlp"));
    let Some(output) = output else {
        return Ok(None);
    };
    let output = output?;
    serde_json::from_slice(&output.stdout)
        .map(Some)
        .map_err(|e| format!("yt-dlp returned invalid metadata: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_errors_redact_urls_and_search_text() {
        let error = process_error(
            "yt-dlp",
            b"ERROR: ytsearch20:private words https://youtube.com/watch?v=secret&sig=abc",
        );
        assert!(!error.contains("private words"));
        assert!(!error.contains("sig=abc"));
        assert!(error.contains("yt-dlp failed"));
    }
}
