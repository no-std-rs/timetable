//! Bounded, host-testable policy for explicit device maintenance actions.
use serde::{Deserialize, Serialize};
#[derive(Deserialize, Serialize)]
pub struct Apps {
    pub user: u32,
    pub apps: Vec<App>,
}
#[derive(Deserialize, Serialize)]
pub struct App {
    #[serde(rename = "package")]
    pub package: String,
    pub label: String,
    pub enabled: bool,
    pub system: bool,
    pub blocked: bool,
    pub reason: String,
}
/// Keep explicit shell commands isolated so `exit` cannot suppress the ADB status.
/// Root uses the tablet's existing `su`; it does not change adbd or install root.
pub fn shell_command(command: &str, root: bool) -> Result<String, &'static str> {
    if command.trim().is_empty() || command.len() > 4096 || command.contains('\0') {
        return Err("Enter a command of at most 4096 bytes without NUL characters");
    }
    let command = format!("(\n{command}\n)");
    Ok(if root {
        format!("su -c {}", super::protocol::shell_quote(&command))
    } else {
        command
    })
}

pub fn package_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .split('.')
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}
pub fn protected(value: &str) -> bool {
    value == "android" || value == "org.nostdrs.timetable" || value.starts_with("com.android.")
}
pub fn package_command(app: &App, user: u32, enabled: bool) -> Result<String, &'static str> {
    if !package_name(&app.package) || user > 21474 {
        return Err("Invalid application identity");
    }
    if !enabled && (app.blocked || protected(&app.package)) {
        return Err("This application is protected because Android or Timetable needs it");
    }
    Ok(format!(
        "pm {} --user {user} {}",
        if enabled { "enable" } else { "disable-user" },
        super::protocol::shell_quote(&app.package)
    ))
}
pub fn report(output: &str) -> Result<Vec<u8>, &'static str> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let confirmed = output
        .rsplit_once("result=")
        .and_then(|(_, s)| s.split(|c: char| !c.is_ascii_digit()).next())
        == Some("1");
    if !confirmed {
        return Err("Install the latest Timetable app before using device tools");
    }
    let (_, payload) = output
        .rsplit_once("TT_REPORT:")
        .ok_or("Install the latest Timetable app before using application controls")?;
    let encoded = payload
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '/' && c != '=')
        .next()
        .ok_or("Invalid device report")?;
    if encoded.len() > 60000 {
        return Err("Device report is too large");
    }
    STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid device report")
}
pub fn wifi(ssid: &str, password: &str) -> Result<(), &'static str> {
    if ssid.is_empty()
        || ssid.len() > 32
        || ssid.chars().chain(password.chars()).any(char::is_control)
        || !(password.is_empty()
            || (8..=63).contains(&password.len())
            || password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("Check the Wi-Fi name and password");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_shell_keeps_exit_and_root_arguments_inside_the_command() {
        assert_eq!(shell_command("exit 7", false).unwrap(), "(\nexit 7\n)");
        assert_eq!(
            shell_command("printf '%s' done", true).unwrap(),
            "su -c '(\nprintf '\"'\"'%s'\"'\"' done\n)'"
        );
        assert!(shell_command(" \n", false).is_err());
        assert!(shell_command("id\0; reboot", false).is_err());
        assert!(shell_command(&"x".repeat(4097), true).is_err());
    }
    #[test]
    fn protected_packages_and_shell_arguments_are_checked() {
        let mut app = App {
            package: "com.example.background".into(),
            label: "App".into(),
            enabled: true,
            system: false,
            blocked: false,
            reason: String::new(),
        };
        assert_eq!(
            package_command(&app, 0, false).unwrap(),
            "pm disable-user --user 0 'com.example.background'"
        );
        app.package = "com.android.settings".into();
        assert!(package_command(&app, 0, false).is_err());
        assert!(package_command(&app, 0, true).is_ok());
        app.package = "org.nostdrs.timetable".into();
        assert!(package_command(&app, 0, false).is_err());
        app.package = "evil'; reboot".into();
        assert!(package_command(&app, 0, true).is_err());
        app.package = "com.vendor.home".into();
        app.blocked = true;
        assert!(package_command(&app, 0, false).is_err());
    }
    #[test]
    fn broadcast_reports_and_wifi_inputs_are_bounded() {
        assert_eq!(
            report("Broadcast completed: result=1, data=\"TT_REPORT:e30=\"").unwrap(),
            b"{}"
        );
        assert!(report("Broadcast completed: result=0").is_err());
        assert!(wifi("House's Wi-Fi", "correct pass").is_ok());
        assert!(wifi("House", "").is_ok());
        assert!(wifi("House", "short").is_err());
        assert!(wifi("bad\nSSID", "correct pass").is_err());
    }
}

#[derive(Serialize)]
pub struct Limit {
    pub path: String,
    pub value: u8,
    pub writable: bool,
}
pub const CHARGING_REPORT: &str = r#"for supply in /sys/class/power_supply/*; do
 [ -d "$supply" ] || continue
 printf '\n%s\n' "$supply"
 for field in type status health capacity temp voltage_now current_now charge_control_start_threshold charge_control_end_threshold charging_enabled battery_charging_enabled input_suspend; do
  node="$supply/$field"
  [ -e "$node" ] || continue
  printf '%s: ' "$field"
  if [ -r "$node" ]; then cat "$node"; else printf 'not readable\n'; fi
 done
 node="$supply/charge_control_end_threshold"
 if [ -r "$node" ] && [ "$(cat "$supply/type" 2>/dev/null)" = Battery ]; then
  writable=no; [ -w "$node" ] && writable=yes
  printf 'TT_LIMIT %s %s %s\n' "$node" "$(cat "$node")" "$writable"
 fi
done"#;
pub fn limit_path(value: &str) -> bool {
    value
        .strip_prefix("/sys/class/power_supply/")
        .and_then(|s| s.strip_suffix("/charge_control_end_threshold"))
        .is_some_and(|name| {
            !name.is_empty()
                && name != "."
                && name != ".."
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        })
}
pub fn limits(report: &str) -> Vec<Limit> {
    report
        .lines()
        .filter_map(|line| {
            let mut fields = line.strip_prefix("TT_LIMIT ")?.split_whitespace();
            let path = fields.next()?;
            let value = fields.next()?.parse::<u8>().ok()?;
            let writable = fields.next()?;
            (limit_path(path)
                && value <= 100
                && fields.next().is_none()
                && matches!(writable, "yes" | "no"))
            .then(|| Limit {
                path: path.into(),
                value,
                writable: writable == "yes",
            })
        })
        .collect()
}
pub fn set_limit(path: &str, value: u8) -> Result<String, &'static str> {
    if !limit_path(path) || !(50..=100).contains(&value) {
        return Err("Invalid charging threshold");
    }
    let base = path.strip_suffix("/charge_control_end_threshold").unwrap();
    let node = super::protocol::shell_quote(path);
    let kind = super::protocol::shell_quote(&format!("{base}/type"));
    Ok(format!(
        "[ -w {node} ] && [ \"$(cat {kind})\" = Battery ] && printf '%s' {value} > {node} && cat {node}"
    ))
}
#[cfg(test)]
mod charging_tests {
    use super::*;
    #[test]
    fn only_standard_real_battery_thresholds_can_be_written() {
        let path = "/sys/class/power_supply/battery/charge_control_end_threshold";
        let values = limits(&format!(
            "TT_LIMIT {path} 100 yes\nTT_LIMIT {path} bad yes\nTT_LIMIT /tmp/fake 80 yes"
        ));
        assert_eq!(values.len(), 1);
        assert!(values[0].writable);
        let command = set_limit(path, 80).unwrap();
        assert!(command.contains("printf '%s' 80"));
        assert!(!command.contains("dumpsys"));
        for bad in [
            "/tmp/fake",
            "/sys/class/power_supply/../charge_control_end_threshold",
            "/sys/class/power_supply/battery/input_suspend",
        ] {
            assert!(set_limit(bad, 80).is_err());
        }
        assert!(set_limit(path, 0).is_err());
    }
}
