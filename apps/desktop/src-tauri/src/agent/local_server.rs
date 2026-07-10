//! Optional dev-server auto-launch. When the agent is pointed at a project
//! directory, we detect the dev command (currently `package.json` →
//! `scripts.dev`), spawn it, and poll the target URL until it responds.
//! The server is killed when the LocalServer is dropped at end of run.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

pub struct LocalServer {
    /// Held so Drop's kill_on_drop fires when the agent run ends.
    _child: Child,
}

impl LocalServer {
    /// Spawn the project's dev command, then block until `target_url`
    /// responds or `ready_timeout` elapses. Returns Err if either step fails.
    pub async fn start(
        project_dir: &Path,
        target_url: &str,
        ready_timeout: Duration,
    ) -> Result<Self, String> {
        let dev_command = detect_dev_command(project_dir)?;

        // `exec` so sh hands its PID directly to the dev tool rather than
        // forking, which keeps kill_on_drop pointing at the real process
        // and limits grandchild leakage.
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("exec {dev_command}"))
            .current_dir(project_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| format!("spawn dev command `{dev_command}` in {project_dir:?}: {e}"))?;

        let server = Self { _child: child };
        server.wait_ready(target_url, ready_timeout).await?;
        Ok(server)
    }

    async fn wait_ready(&self, target_url: &str, timeout: Duration) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("reqwest client: {e}"))?;
        let started = Instant::now();
        loop {
            if let Ok(resp) = client.get(target_url).send().await {
                let s = resp.status();
                if s.is_success() || s.is_redirection() {
                    return Ok(());
                }
            }
            if started.elapsed() > timeout {
                return Err(format!(
                    "dev server did not become ready within {:?} at {target_url}",
                    timeout
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

pub fn detect_dev_command(project_dir: &Path) -> Result<String, String> {
    let pkg_path = project_dir.join("package.json");
    if pkg_path.exists() {
        let pkg_text =
            std::fs::read_to_string(&pkg_path).map_err(|e| format!("read {pkg_path:?}: {e}"))?;
        let pkg: serde_json::Value =
            serde_json::from_str(&pkg_text).map_err(|e| format!("parse package.json: {e}"))?;
        let scripts = pkg.get("scripts");
        if scripts.and_then(|s| s.get("dev")).is_some() {
            return Ok("npm run dev".into());
        }
        if scripts.and_then(|s| s.get("start")).is_some() {
            return Ok("npm start".into());
        }
        return Err(format!(
            "package.json at {pkg_path:?} has no `dev` or `start` script"
        ));
    }
    Err(format!(
        "no recognized project layout in {project_dir:?} (looked for package.json)"
    ))
}

#[cfg(test)]
mod tests {
    use super::detect_dev_command;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "codevetter-agent-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn detects_npm_run_dev() {
        let dir = tmp_dir("dev");
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"dev":"vite","build":"vite build"}}"#,
        )
        .unwrap();
        assert_eq!(detect_dev_command(&dir).unwrap(), "npm run dev");
    }

    #[test]
    fn falls_back_to_npm_start() {
        let dir = tmp_dir("start");
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"start":"node server.js"}}"#,
        )
        .unwrap();
        assert_eq!(detect_dev_command(&dir).unwrap(), "npm start");
    }

    #[test]
    fn errors_when_package_json_lacks_dev_and_start() {
        let dir = tmp_dir("none");
        fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"build":"vite build"}}"#,
        )
        .unwrap();
        let err = detect_dev_command(&dir).unwrap_err();
        assert!(err.contains("no `dev` or `start`"), "{err}");
    }

    #[test]
    fn errors_when_no_package_json() {
        let dir = tmp_dir("empty");
        let err = detect_dev_command(&dir).unwrap_err();
        assert!(err.contains("no recognized project layout"), "{err}");
    }
}
