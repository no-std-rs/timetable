//! Parse only explicit owner sections, never ordinary active administrators.
use serde::Serialize;

pub const ADMIN: &str = "org.nostdrs.timetable/.AdminReceiver";
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Owner {
    pub kind: &'static str,
    pub user: u32,
    pub component: String,
}

pub fn component(value: &str) -> bool {
    value.split_once('/').is_some_and(|(package, class)| {
        super::device_tools::package_name(package)
            && !class.is_empty()
            && class.len() <= 255
            && class
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'$'))
    })
}

pub fn owners(report: &str) -> Vec<Owner> {
    let mut found = Vec::new();
    let mut kind = None;
    let mut user = None;
    let mut admin = None;
    for line in report.lines().chain(std::iter::once("")) {
        let line = line.trim();
        if line.is_empty() || line.ends_with(':') {
            if let (Some(kind), Some(user), Some(component)) =
                (kind.take(), user.take(), admin.take())
            {
                found.push(Owner {
                    kind,
                    user,
                    component,
                });
            }
            if line == "Device Owner:" {
                kind = Some("device");
            } else if let Some(id) = line
                .strip_prefix("Profile Owner (User ")
                .and_then(|s| s.strip_suffix("):"))
            {
                kind = Some("profile");
                user = id.parse::<u32>().ok();
            }
        } else if kind.is_some() {
            if let Some(id) = line.strip_prefix("User ID:") {
                user = id.trim().parse::<u32>().ok();
            } else if let Some(value) = line
                .strip_prefix("admin=ComponentInfo{")
                .and_then(|s| s.strip_suffix('}'))
                && component(value)
            {
                admin = Some(value.to_string());
            }
        }
    }
    found
}

pub fn removal(owner: &Owner, sdk: u32) -> Result<String, &'static str> {
    if sdk < 24 {
        return Err("Replacing an existing owner requires Android 7.0 or newer");
    }
    if owner.user != 0 || !component(&owner.component) {
        return Err("Select an owner on the main user (0)");
    }
    Ok(format!(
        "dpm remove-active-admin --user 0 {}",
        super::protocol::shell_quote(&owner.component)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separates_real_owners_from_admins_and_other_users() {
        let report = "Device Policy Manager state:\n  Enabled Device Admins (User 0):\n    admin=ComponentInfo{com.other/.Admin}\n\n  Device Owner:\n    admin=ComponentInfo{com.example/.Admin}\n    package=com.example\n    User ID: 0\n\n  Profile Owner (User 10):\n    admin=ComponentInfo{com.work/com.work.Owner}\n\n  Profile Owner (User 0):\n    admin=ComponentInfo{com.profile/.Owner}\n";
        let rows = owners(report);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0].kind, rows[0].user, rows[0].component.as_str()),
            ("device", 0, "com.example/.Admin")
        );
        assert!(removal(&rows[1], 24).is_err());
        for sdk in 21..24 {
            assert_eq!(
                removal(&rows[2], sdk).unwrap_err(),
                "Replacing an existing owner requires Android 7.0 or newer"
            );
        }
        assert_eq!(
            removal(&rows[2], 24).unwrap(),
            "dpm remove-active-admin --user 0 'com.profile/.Owner'"
        );
        assert!(
            owners("Profile Owner (User x):\n admin=ComponentInfo{com.bad/.Owner}\n").is_empty()
        );
        assert!(owners("Device Owner:\n admin=ComponentInfo{com.bad/.Owner}\n").is_empty());
        assert!(!component("com.bad/.Owner; reboot"));
        assert!(!component("com.bad/.Owner/other"));
    }
}
