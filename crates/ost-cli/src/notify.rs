// SPDX-License-Identifier: Apache-2.0
//! Best-effort desktop notifications for long commands (`--notify`).
//!
//! Optional and never fatal: if the platform mechanism is missing (or locked
//! down) the call is a silent no-op. Auto-disabled when there is no human at the
//! console — over SSH or in CI — since a toast there would land nowhere. We shell
//! out to the OS's own tool rather than pull a notification crate, keeping the
//! dependency surface flat.

use std::process::{Command, Stdio};

/// Whether notifications make sense here: never headless (SSH) or in CI, where a
/// desktop toast has no audience. Callers still gate on the opt-in `--notify`.
pub fn enabled() -> bool {
    let blocked = [
        // CI systems.
        "CI",
        "GITHUB_ACTIONS",
        "GITLAB_CI",
        "BUILDKITE",
        "JENKINS_URL",
        "TEAMCITY_VERSION",
        // Remote sessions with no local desktop.
        "SSH_CONNECTION",
        "SSH_TTY",
        "SSH_CLIENT",
    ];
    !blocked.iter().any(|k| std::env::var_os(k).is_some())
}

/// Fire a notification, best-effort. Failures (missing tool, denied) are ignored.
pub fn send(title: &str, body: &str) {
    let _ = platform_send(title, body);
}

#[cfg(target_os = "macos")]
fn platform_send(title: &str, body: &str) -> Option<()> {
    // osascript ships with macOS.
    let script = format!(
        "display notification {} with title {}",
        applescript_quote(body),
        applescript_quote(title),
    );
    spawn("osascript", &["-e", &script])
}

/// Quote a string as an AppleScript literal (double-quoted, `\` and `"` escaped).
#[cfg(target_os = "macos")]
fn applescript_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

#[cfg(target_os = "linux")]
fn platform_send(title: &str, body: &str) -> Option<()> {
    // notify-send is best-effort; absent on minimal systems → no-op.
    spawn("notify-send", &["--app-name=ost", title, body])
}

#[cfg(target_os = "windows")]
fn platform_send(title: &str, body: &str) -> Option<()> {
    // A WinRT toast driven by PowerShell. Best-effort: older Windows or a
    // locked-down PowerShell simply does nothing.
    let script = windows_toast_script(title, body);
    spawn(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
}

/// Build the PowerShell WinRT toast one-liner with the title/body interpolated as
/// single-quoted literals (`'` doubled per PowerShell quoting).
#[cfg(target_os = "windows")]
fn windows_toast_script(title: &str, body: &str) -> String {
    let q = |s: &str| s.replace('\'', "''");
    format!(
        "$ErrorActionPreference='SilentlyContinue';\
         [Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime]|Out-Null;\
         $t=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02);\
         $x=$t.GetElementsByTagName('text');\
         $x.Item(0).AppendChild($t.CreateTextNode('{title}'))|Out-Null;\
         $x.Item(1).AppendChild($t.CreateTextNode('{body}'))|Out-Null;\
         $n=[Windows.UI.Notifications.ToastNotification]::new($t);\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('OpenStrata').Show($n);",
        title = q(title),
        body = q(body),
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_send(_title: &str, _body: &str) -> Option<()> {
    None
}

/// Spawn the notifier detached with all stdio nulled; never wait on it.
fn spawn(program: &str, args: &[&str]) -> Option<()> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
        .map(|_| ())
}
