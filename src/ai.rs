//! Optional: ask Claude to explain a LaTeX error. Off by default, and it
//! needs the user's own Anthropic API key. Only the error message and the
//! lines around it are sent, and only when the user presses the button.
//!
//! Rust has no official Anthropic SDK, so this is a plain HTTPS request with
//! curl (already used for DOI lookups). The key never appears on a command
//! line: curl reads that header from stdin.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use gtk::glib;
use serde_json::{Value, json};

use crate::i18n::{tr, trf};

const MODEL: &str = "claude-opus-5-5";
const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";

fn key_path() -> PathBuf {
    glib::user_config_dir().join("rutex").join("anthropic-key")
}

/// The key from ANTHROPIC_API_KEY, or the one saved in Preferences.
pub fn api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .or_else(|| fs::read_to_string(key_path()).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

pub fn key_from_env() -> bool {
    std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.trim().is_empty())
}

pub fn saved_key() -> String {
    fs::read_to_string(key_path()).unwrap_or_default().trim().to_string()
}

/// Saves the key readable only by the user (an empty key deletes it).
pub fn save_key(key: &str) -> std::io::Result<()> {
    let path = key_path();
    if key.trim().is_empty() {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    f.write_all(key.trim().as_bytes())
}

/// Lines around `line` (1-based), numbered, the error line marked with ">>".
pub fn excerpt(src: &str, line: u32, radius: u32) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let at = line.max(1) as usize;
    let from = at.saturating_sub(radius as usize).max(1);
    let to = (at + radius as usize).min(lines.len());
    (from..=to).map(|n| format!("{}{n:>5} | {}", if n == at { ">>" } else { "  " }, lines[n - 1])).collect::<Vec<_>>().join("\n")
}

pub fn request_body(message: &str, file: &str, line: u32, excerpt: &str, spanish: bool) -> Value {
    let language = if spanish { "Spanish" } else { "English" };
    let system = format!(
        "You help someone who is writing a LaTeX document (often a thesis) and is not a LaTeX expert. \
         Explain the compile error they got: what it means in their code, and the exact change that fixes it. \
         Show the corrected line when that helps. Be brief: a short paragraph or a few bullet points, plain text. \
         Answer in {language}."
    );
    let user = format!("Error: {message}\nFile: {file}, line {line}\n\nSource around the error (the error line is marked with >>):\n{excerpt}");
    json!({
        "model": MODEL,
        "max_tokens": 16000,
        "output_config": {"effort": "low"},
        // If a safety classifier declines, the API retries on the model it
        // recommends instead of returning a refusal.
        "fallbacks": "default",
        "system": system,
        "messages": [{"role": "user", "content": user}],
    })
}

/// Text of a Messages API response, or a message for the user.
pub fn parse_response(status: u16, body: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let api_message = v["error"]["message"].as_str().unwrap_or("").to_string();
    match status {
        200 => {
            if v["stop_reason"] == "refusal" {
                return Err(tr("Claude declined to answer this one.").into());
            }
            let text: Vec<&str> = v["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            let text = text.join("\n").trim().to_string();
            if text.is_empty() { Err(tr("The answer was empty. Try again.").into()) } else { Ok(text) }
        }
        401 => Err(tr("The API key was rejected. Check it in Preferences.").into()),
        429 => Err(tr("Too many requests right now. Try again in a moment.").into()),
        500..=599 => Err(tr("Claude is busy right now. Try again in a moment.").into()),
        0 => Err(tr("Could not connect. Check your internet connection.").into()),
        _ if !api_message.is_empty() => Err(api_message),
        _ => Err(trf("Request failed (HTTP {})", &[&status])),
    }
}

/// Asks Claude (blocking). `scratch` is a private folder for the request body.
pub fn explain(key: &str, message: &str, file: &str, line: u32, excerpt: &str, spanish: bool, scratch: &Path) -> Result<String, String> {
    fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    let body_path = scratch.join("ai-request.json");
    {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&body_path).map_err(|e| e.to_string())?;
        f.write_all(request_body(message, file, line, excerpt, spanish).to_string().as_bytes()).map_err(|e| e.to_string())?;
    }
    let mut child = crate::compiler::command("curl")
        .args(["-sS", "--max-time", "120", "-H", "@-"])
        .args(["-H", "content-type: application/json", "-H", "anthropic-version: 2026-10-21", "-H", "anthropic-beta: server-side-fallback-2026-07-01"])
        .args(["-w", "\n%{http_code}", "--data-binary"])
        .arg(format!("@{}", body_path.display()))
        .arg(ENDPOINT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "x-api-key: {key}");
    }
    let out = child.wait_with_output().map_err(|e| e.to_string());
    let _ = fs::remove_file(&body_path);
    let out = String::from_utf8_lossy(&out?.stdout).into_owned();
    let (body, status) = out.rsplit_once('\n').unwrap_or(("", &out));
    parse_response(status.trim().parse().unwrap_or(0), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_request() {
        let ex = excerpt("a\nb\nc\nd\ne", 3, 1);
        assert_eq!(ex, "      2 | b\n>>    3 | c\n      4 | d");
        let body = request_body("Undefined control sequence.", "main.tex", 3, &ex, true);
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["fallbacks"], "default");
        assert!(body["system"].as_str().unwrap().contains("Spanish"));
        assert!(body["messages"][0]["content"].as_str().unwrap().contains(">>    3 | c"));
    }

    #[test]
    fn reads_responses() {
        let ok = r#"{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"Load amssymb."}],"stop_reason":"end_turn"}"#;
        assert_eq!(parse_response(200, ok).unwrap(), "Load amssymb.");
        assert!(parse_response(200, r#"{"content":[],"stop_reason":"refusal"}"#).is_err());
        assert!(parse_response(401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#).unwrap_err().contains("Preferences"));
        assert_eq!(parse_response(400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#).unwrap_err(), "bad");
        assert!(parse_response(529, "").is_err());
    }

    /// Real request with a fake key (free): the API must answer 401, which
    /// means the request itself is well formed. Needs internet.
    #[test]
    #[ignore]
    fn reaches_the_api() {
        let dir = std::env::temp_dir().join(format!("rutex-ai-{}", std::process::id()));
        let err = explain("sk-ant-invalid", "Undefined control sequence.", "main.tex", 1, ">>    1 | \\foo", false, &dir).unwrap_err();
        assert_eq!(err, tr("The API key was rejected. Check it in Preferences."));
        assert!(!dir.join("ai-request.json").exists(), "the request body is removed");
        let _ = fs::remove_dir_all(dir);
    }
}
