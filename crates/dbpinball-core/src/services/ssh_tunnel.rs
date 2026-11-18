use std::borrow::Cow;
use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::domain::connection::{DbConnection, SshTunnelConfig};
use crate::domain::error::{CoreError, ErrorCode};

const TUNNEL_READY_TIMEOUT: Duration = Duration::from_secs(15);
const TUNNEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Active SSH tunnel child process. Terminated cleanly when dropped.
pub struct SshTunnelGuard {
    child: Child,
}

impl Drop for SshTunnelGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Expand a leading `~/` or `~\` in a private key path to the user's home directory.
fn expand_home_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        if let Some(home) = dirs::home_dir() {
            let joined: PathBuf = home.join(rest);
            return joined.to_string_lossy().into_owned();
        }
    }
    trimmed.to_string()
}

/// Build OpenSSH CLI arguments for local port forwarding.
pub fn build_ssh_args(
    tunnel: &SshTunnelConfig,
    local_port: u16,
    target_host: &str,
    target_port: u16,
) -> Result<Vec<String>, CoreError> {
    if tunnel.host.trim().is_empty() {
        return Err(CoreError::new(
            ErrorCode::Usage,
            "SSH tunnel host is required",
        ));
    }
    if tunnel.username.trim().is_empty() {
        return Err(CoreError::new(
            ErrorCode::Usage,
            "SSH tunnel username is required",
        ));
    }
    if tunnel.port == 0 {
        return Err(CoreError::new(
            ErrorCode::Usage,
            "SSH tunnel port must be between 1 and 65535",
        ));
    }
    if target_host.trim().is_empty() || target_port == 0 {
        return Err(CoreError::new(
            ErrorCode::Usage,
            "Target database host and port are required for SSH tunnel forwarding",
        ));
    }

    let host_key_policy = if tunnel.strict_host_key_checking {
        "StrictHostKeyChecking=yes"
    } else {
        "StrictHostKeyChecking=accept-new"
    };

    let mut args = vec![
        "-N".to_string(),
        "-T".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=15".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=30".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
        "-o".to_string(),
        host_key_policy.to_string(),
        "-p".to_string(),
        tunnel.port.to_string(),
    ];

    if let Some(key_path) = tunnel
        .private_key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("-i".to_string());
        args.push(expand_home_path(key_path));
    }

    args.push("-L".to_string());
    args.push(format!(
        "127.0.0.1:{local_port}:{target_host}:{target_port}"
    ));
    args.push(format!("{}@{}", tunnel.username.trim(), tunnel.host.trim()));

    Ok(args)
}

fn reserve_local_port() -> Result<u16, CoreError> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
        CoreError::new(
            ErrorCode::Connection,
            format!("could not allocate local port for SSH tunnel: {error}"),
        )
    })?;
    let port = listener
        .local_addr()
        .map_err(|error| {
            CoreError::new(
                ErrorCode::Connection,
                format!("could not read local SSH tunnel port: {error}"),
            )
        })?
        .port();
    drop(listener);
    Ok(port)
}

fn read_child_stderr(child: &mut Child) -> String {
    let Some(mut stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut buf = String::new();
    let _ = stderr.read_to_string(&mut buf);
    buf.trim().to_string()
}

/// Prepare a connection for SQL access, opening an SSH tunnel if enabled.
///
/// When an SSH tunnel is enabled, the returned `DbConnection` points to
/// `127.0.0.1:<local_port>` while keeping all other metadata intact, and the
/// returned `SshTunnelGuard` must be kept alive until the database session closes.
pub async fn prepare_connection<'a>(
    connection: &'a DbConnection,
) -> Result<(Cow<'a, DbConnection>, Option<SshTunnelGuard>), CoreError> {
    let Some(tunnel) = connection.ssh_tunnel.as_ref().filter(|t| t.enabled) else {
        return Ok((Cow::Borrowed(connection), None));
    };

    let local_port = reserve_local_port()?;
    let args = build_ssh_args(tunnel, local_port, &connection.host, connection.port)?;

    let mut command = Command::new("ssh");
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn().map_err(|error| {
        CoreError::new(
            ErrorCode::Connection,
            format!("failed to start ssh client for tunnel: {error}"),
        )
    })?;

    let deadline = Instant::now() + TUNNEL_READY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            CoreError::new(
                ErrorCode::Connection,
                format!("failed to monitor ssh tunnel process: {error}"),
            )
        })? {
            let detail = read_child_stderr(&mut child);
            let message = if detail.is_empty() {
                format!("ssh tunnel exited with status {status}")
            } else {
                format!("ssh tunnel failed: {detail}")
            };
            return Err(CoreError::redacted(ErrorCode::Connection, message));
        }

        if tokio::net::TcpStream::connect(("127.0.0.1", local_port))
            .await
            .is_ok()
        {
            let mut effective = connection.clone();
            effective.host = "127.0.0.1".to_string();
            effective.port = local_port;
            return Ok((Cow::Owned(effective), Some(SshTunnelGuard { child })));
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CoreError::new(
                ErrorCode::Connection,
                "timed out waiting for SSH tunnel port forwarding to become ready",
            ));
        }

        tokio::time::sleep(TUNNEL_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::connection::{DbEngine, TargetClass};

    #[test]
    fn build_ssh_args_includes_port_forwarding_and_key() {
        let tunnel = SshTunnelConfig {
            enabled: true,
            host: "34.101.87.166".into(),
            port: 2222,
            username: "dbtunnel".into(),
            private_key_path: Some("/tmp/tunnel_id_rsa".into()),
            strict_host_key_checking: false,
        };

        let args = build_ssh_args(&tunnel, 45678, "127.0.0.1", 3306).unwrap();
        assert!(args.contains(&"StrictHostKeyChecking=accept-new".to_string()));
        assert!(args.windows(2).any(|w| w == ["-p", "2222"]));
        assert!(args.windows(2).any(|w| w == ["-i", "/tmp/tunnel_id_rsa"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["-L", "127.0.0.1:45678:127.0.0.1:3306"]));
        assert_eq!(
            args.last().map(String::as_str),
            Some("dbtunnel@34.101.87.166")
        );
    }

    #[test]
    fn build_ssh_args_rejects_empty_host_or_zero_port() {
        let tunnel = SshTunnelConfig {
            enabled: true,
            host: " ".into(),
            port: 22,
            username: "dbtunnel".into(),
            private_key_path: None,
            strict_host_key_checking: true,
        };
        assert!(build_ssh_args(&tunnel, 45678, "127.0.0.1", 3306).is_err());

        let bad_port = SshTunnelConfig {
            host: "bastion.example.com".into(),
            port: 0,
            ..tunnel
        };
        assert!(build_ssh_args(&bad_port, 45678, "127.0.0.1", 3306).is_err());
    }

    #[tokio::test]
    async fn prepare_connection_returns_borrowed_when_tunnel_disabled() {
        let connection = DbConnection {
            id: "c1".into(),
            name: "local".into(),
            engine: DbEngine::Postgres,
            host: "127.0.0.1".into(),
            port: 5432,
            database: "app".into(),
            username: "postgres".into(),
            use_ssl: false,
            target: TargetClass::Local,
            ssh_tunnel: None,
        };

        let (effective, guard) = prepare_connection(&connection).await.unwrap();
        assert!(matches!(effective, Cow::Borrowed(_)));
        assert!(guard.is_none());
    }
}
