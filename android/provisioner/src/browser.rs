use super::{device_tools, ownership, protocol};
use adb_client::webusb::{ADBWebUSBDevice, WebUSBKey};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use wasm_bindgen::{JsCast, prelude::*};

fn error(value: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&value.to_string())
}
#[wasm_bindgen]
pub fn request() -> Result<js_sys::Promise<web_sys::UsbDevice>, JsValue> {
    adb_client::webusb::request_device().map_err(error)
}
#[wasm_bindgen]
pub struct Client {
    device: Option<ADBWebUSBDevice>,
    sdk: u32,
}
#[derive(Serialize)]
struct Capabilities {
    sdk: u32,
    model: String,
    abi: String,
    charging_limit: &'static str,
}
#[wasm_bindgen]
impl Client {
    pub async fn connect(device: JsValue, pem: String) -> Result<Client, JsValue> {
        let key = WebUSBKey::from_pkcs8_pem(&pem).map_err(error)?;
        let device = ADBWebUSBDevice::connect(device.unchecked_into(), &key)
            .await
            .map_err(error)?;
        Ok(Self {
            device: Some(device),
            sdk: 0,
        })
    }
    pub async fn inspect(&mut self) -> Result<String, JsValue> {
        let sdk = self
            .command("getprop ro.build.version.sdk")
            .await?
            .trim()
            .parse::<u32>()
            .map_err(error)?;
        let model = self.command("getprop ro.product.model").await?;
        let abi = self.command("getprop ro.product.cpu.abi").await?;
        self.sdk = sdk;
        serde_json::to_string(&Capabilities {
            sdk,
            model,
            abi,
            charging_limit: "unavailable",
        })
        .map_err(error)
    }
    /// Execute an explicit command over this browser's authorized USB session.
    pub async fn shell(&mut self, command: String, root: bool) -> Result<String, JsValue> {
        let command = device_tools::shell_command(&command, root).map_err(error)?;
        self.command(&command).await
    }
    pub async fn ownership_report(&mut self) -> Result<String, JsValue> {
        let report = self.command("dumpsys device_policy").await?;
        serde_json::to_string(
            &serde_json::json!({"report":report,"owners":ownership::owners(&report),"can_remove_owner":self.sdk >= 24}),
        )
        .map_err(error)
    }
    pub async fn take_ownership(
        &mut self,
        previous: String,
        root: bool,
    ) -> Result<String, JsValue> {
        // Refresh immediately before changing policy; never act on stale browser selection.
        let before = self.command("dumpsys device_policy").await?;
        let owners = ownership::owners(&before);
        if !previous.is_empty() {
            let selected = owners
                .iter()
                .find(|owner| owner.user == 0 && owner.component == previous)
                .ok_or_else(|| error("The selected owner changed. Inspect ownership again."))?;
            if selected.kind != "device"
                || !selected.component.starts_with("org.nostdrs.timetable/")
            {
                let remove = ownership::removal(selected, self.sdk).map_err(error)?;
                // Validate the target receiver with Android before removing its predecessor.
                // A missing/disabled APK or invalid receiver must leave the old owner intact.
                let registered = self
                    .shell(
                        format!("dpm set-active-admin --user 0 {}", ownership::ADMIN),
                        root,
                    )
                    .await
                    .map_err(|e| {
                        error(format!(
                            "Install the current Timetable app before replacing the owner: {}",
                            e.as_string().unwrap_or_default()
                        ))
                    })?;
                if !registered
                    .lines()
                    .any(|line| line.trim().starts_with("Success: Active admin set"))
                {
                    return Err(error(
                        "Android did not validate Timetable's admin receiver; the previous owner was kept",
                    ));
                }
                self.shell(remove, root).await.map_err(|e| {
                    error(format!(
                        "Android refused to remove {previous}: {}",
                        e.as_string().unwrap_or_default()
                    ))
                })?;
                let report = self.command("dumpsys device_policy").await?;
                if ownership::owners(&report)
                    .iter()
                    .any(|owner| owner.user == 0 && owner.component == previous)
                {
                    return Err(error(
                        "Android did not confirm removal of the previous owner",
                    ));
                }
            }
        } else if owners.iter().any(|owner| owner.user == 0) {
            return Err(error(
                "An owner already exists. Inspect ownership and select it first.",
            ));
        }
        let report = self.command("dumpsys device_policy").await?;
        if !ownership::owners(&report).iter().any(|owner| {
            owner.kind == "device"
                && owner.user == 0
                && owner.component.starts_with("org.nostdrs.timetable/")
        }) {
            self.shell(format!("dpm set-device-owner {}", ownership::ADMIN), root).await
                .map_err(|e| error(format!("Android refused Timetable ownership. Inspect current ownership before retrying: {}",e.as_string().unwrap_or_default())))?;
        }
        let report = self.command("dumpsys device_policy").await?;
        if !ownership::owners(&report).iter().any(|owner| {
            owner.kind == "device"
                && owner.user == 0
                && owner.component.starts_with("org.nostdrs.timetable/")
        }) {
            return Err(error("Android did not confirm Timetable as device owner"));
        }
        self.command("am start -W -n org.nostdrs.timetable/.MainActivity")
            .await?;
        Ok(report)
    }
    pub async fn battery_report(&mut self) -> Result<String, JsValue> {
        let battery = self
            .command("dumpsys battery")
            .await
            .map_err(|_| error("Android did not provide battery status"))?;
        let power = self
            .command("dumpsys power")
            .await
            .unwrap_or_else(|_| "Power report unavailable".into());
        let charging = self
            .command(device_tools::CHARGING_REPORT)
            .await
            .unwrap_or_else(|_| "Charging nodes unavailable".into());
        let identity = self.command("id").await.unwrap_or_default();
        serde_json::to_string(&serde_json::json!({"battery":battery,"power":power,"charging":charging,"identity":identity,"limits":device_tools::limits(&charging)})).map_err(error)
    }
    pub async fn set_charge_limit(&mut self, path: String, value: u8) -> Result<u8, JsValue> {
        let command = device_tools::set_limit(&path, value).map_err(error)?;
        let actual=self.command(&command).await.map_err(|_|error("Android refused the real charging threshold. Root or vendor support may be required."))?;
        let actual = actual
            .trim()
            .parse::<u8>()
            .map_err(|_| error("Cannot verify the actual charging threshold"))?;
        if actual > 100 {
            return Err(error("Invalid charging threshold returned by Android"));
        }
        Ok(actual)
    }
    pub async fn apps(&mut self) -> Result<String, JsValue> {
        serde_json::to_string(&self.app_report(None).await?).map_err(error)
    }
    pub async fn set_app_enabled(&mut self, package: String, enabled: bool) -> Result<(), JsValue> {
        if !device_tools::package_name(&package) {
            return Err(error("Invalid application identity"));
        }
        let report = self.app_report(Some(&package)).await?;
        let app = report
            .apps
            .iter()
            .find(|a| a.package == package)
            .ok_or_else(|| error("Application is not installed"))?;
        let command = device_tools::package_command(app, report.user, enabled).map_err(error)?;
        self.command(&command).await?;
        let updated = self.app_report(Some(&package)).await?;
        if !updated
            .apps
            .iter()
            .any(|a| a.package == package && a.enabled == enabled)
        {
            return Err(error(
                "Android did not confirm the requested application state",
            ));
        }
        Ok(())
    }
    pub async fn configure_wifi(
        &mut self,
        ssid: String,
        password: String,
    ) -> Result<String, JsValue> {
        device_tools::wifi(&ssid, &password).map_err(error)?;
        let payload = STANDARD.encode(
            serde_json::to_vec(&serde_json::json!({"ssid":ssid,"password":password}))
                .map_err(error)?,
        );
        let command = format!(
            "am broadcast --receiver-foreground -a org.nostdrs.timetable.WIFI -n org.nostdrs.timetable/.ProvisionReceiver --es payload {}",
            protocol::shell_quote(&payload)
        );
        let result = self.command(&command).await.map_err(|_| {
            error("Android refused Wi-Fi setup; use system settings or device-owner mode")
        })?;
        let bytes = device_tools::report(&result).map_err(|_| {
            error("Wi-Fi setup needs the latest app and permission from the tablet firmware.")
        })?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(error)?;
        if value["saved"] != true || value["connection_requested"] != true {
            return Err(error("Android did not confirm Wi-Fi setup"));
        }
        Ok("Wi-Fi saved and connection requested. Check the tablet for connection status.".into())
    }
    pub async fn provision(
        &mut self,
        apk: js_sys::Uint8Array,
        sha256: String,
        settings: js_sys::Uint8Array,
        make_home: bool,
        device_owner: bool,
        progress: js_sys::Function,
    ) -> Result<String, JsValue> {
        if self.sdk < 21 {
            return Err(error("Android 5.0/API21 or newer is required"));
        }
        if apk.length() == 0 || apk.length() > 32 * 1024 * 1024 || settings.length() > 16384 {
            return Err(error("Invalid installation size"));
        }
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(error("Invalid APK digest"));
        }
        let apk = apk.to_vec();
        if format!("{:x}", Sha256::digest(&apk)) != sha256 {
            return Err(error("APK checksum mismatch"));
        }
        let settings = settings.to_vec();
        timetable_tablet::Settings::decode(&settings).map_err(error)?;
        let path = format!("/data/local/tmp/timetable-{}.apk", &sha256[..16]);
        let outcome = self
            .install(&path, &apk, &settings, make_home, device_owner, &progress)
            .await;
        // Do not uninstall an existing app or reset a tablet after a partial failure.
        // Cleanup is best-effort if unplugging poisoned the ADB connection.
        let _ = self
            .command(&format!("rm -f {}", protocol::shell_quote(&path)))
            .await;
        outcome
    }
    pub async fn disconnect(&mut self) -> Result<(), JsValue> {
        if let Some(device) = self.device.take() {
            device.disconnect().await.map_err(error)?;
        }
        Ok(())
    }
}
impl Client {
    async fn app_report(&mut self, package: Option<&str>) -> Result<device_tools::Apps, JsValue> {
        let mut command="am broadcast --receiver-foreground -a org.nostdrs.timetable.APPS -n org.nostdrs.timetable/.ProvisionReceiver".to_string();
        if let Some(package) = package {
            command.push_str(" --es package ");
            command.push_str(&protocol::shell_quote(package));
        }
        let result = self.command(&command).await?;
        let bytes = device_tools::report(&result).map_err(error)?;
        serde_json::from_slice(&bytes).map_err(error)
    }
    async fn command(&mut self, command: &str) -> Result<String, JsValue> {
        let device = self.device.as_mut().ok_or_else(|| error("Not connected"))?;
        let mut output = Vec::new();
        let wrapped = format!("{command}; result=$?; printf '\\nTT_EXIT:%s\\n' \"$result\"");
        device
            .shell_command(&wrapped, |bytes| {
                if output.len() + bytes.len() > 65536 {
                    return Err(adb_client::RustADBError::ADBRequestFailed(
                        "Device response is too large".into(),
                    ));
                }
                output.extend_from_slice(bytes);
                Ok(())
            })
            .await
            .map_err(error)?;
        protocol::exit_result(&output).map_err(error)
    }
    async fn upload(
        &mut self,
        path: &str,
        apk: &[u8],
        progress: &js_sys::Function,
    ) -> Result<(), JsValue> {
        let device = self.device.as_mut().ok_or_else(|| error("Not connected"))?;
        let mut stream = device.open("sync:").await.map_err(error)?;
        let name = format!("{path},33188");
        stream
            .write(&protocol::packet(b"SEND", name.len() as u32))
            .await
            .map_err(error)?;
        stream.write(name.as_bytes()).await.map_err(error)?;
        for (i, chunk) in apk.chunks(32768).enumerate() {
            stream
                .write(&protocol::packet(b"DATA", chunk.len() as u32))
                .await
                .map_err(error)?;
            stream.write(chunk).await.map_err(error)?;
            progress.call1(
                &JsValue::NULL,
                &JsValue::from_f64(((i * 32768 + chunk.len()) * 100 / apk.len()) as f64),
            )?;
        }
        stream
            .write(&protocol::packet(
                b"DONE",
                (js_sys::Date::now() / 1000.0) as u32,
            ))
            .await
            .map_err(error)?;
        let mut response = Vec::new();
        loop {
            let bytes = stream
                .read()
                .await
                .map_err(error)?
                .ok_or_else(|| error("APK transfer closed before confirmation"))?;
            if response.len() + bytes.len() > 4104 {
                return Err(error("Oversized APK transfer response"));
            }
            response.extend_from_slice(&bytes);
            if protocol::sync_result(&response).map_err(error)?.is_some() {
                break;
            }
        }
        stream.close().await.map_err(error)
    }
    async fn install(
        &mut self,
        path: &str,
        apk: &[u8],
        settings: &[u8],
        make_home: bool,
        owner: bool,
        progress: &js_sys::Function,
    ) -> Result<String, JsValue> {
        self.upload(path, apk, progress).await?;
        let installed = self
            .command(&format!("pm install -r {}", protocol::shell_quote(path)))
            .await?;
        if !installed.lines().any(|line| line.trim() == "Success") {
            return Err(error(format!(
                "Android did not confirm installation: {installed}"
            )));
        }
        let payload = STANDARD.encode(settings);
        let result=self.command(&format!("am broadcast --receiver-foreground -a org.nostdrs.timetable.PROVISION -n org.nostdrs.timetable/.ProvisionReceiver --es settings {}",protocol::shell_quote(&payload))).await?;
        let confirmed = result
            .rsplit_once("result=")
            .and_then(|(_, s)| s.split(|c: char| !c.is_ascii_digit()).next())
            .is_some_and(|s| s == "1");
        if !confirmed {
            return Err(error(format!(
                "The app did not confirm its settings: {result}"
            )));
        }
        let mut actual_owner = result.contains("device_owner=true");
        let mut owner_error = None;
        let mut owner_report = None;
        if owner && !actual_owner {
            match self
                .command("dpm set-device-owner org.nostdrs.timetable/.AdminReceiver")
                .await
            {
                Ok(output)
                    if output
                        .lines()
                        .any(|line| line.trim().starts_with("Success:")) =>
                {
                    // Verify the OS state through the installed app rather than trusting dpm text.
                    match self.command(&format!("am broadcast --receiver-foreground -a org.nostdrs.timetable.PROVISION -n org.nostdrs.timetable/.ProvisionReceiver --es settings {}", protocol::shell_quote(&payload))).await {
                        Ok(report) if report.contains("device_owner=true") => actual_owner = true,
                        _ => owner_error = Some("Android did not confirm device ownership".to_string()),
                    }
                }
                Ok(output) => owner_error = Some(output),
                Err(e) => {
                    owner_error = Some(
                        e.as_string()
                            .unwrap_or_else(|| "Android refused device ownership".into()),
                    )
                }
            }
            if owner_error.is_some() {
                owner_report = self.command("dumpsys device_policy").await.ok();
            }
        }
        self.command("am start -W -n org.nostdrs.timetable/.MainActivity")
            .await?;
        let mut home = "unchanged";
        if (make_home || owner) && !actual_owner {
            if self.sdk >= 24 {
                let _ = self
                    .command("cmd package set-home-activity org.nostdrs.timetable/.MainActivity")
                    .await;
                if self.command("cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME").await.unwrap_or_default().contains("org.nostdrs.timetable/"){home="selected";}
            }
            if home != "selected" {
                let _ = self
                    .command("am start -a android.settings.HOME_SETTINGS")
                    .await;
                home = "choose_on_tablet";
            }
        } else if actual_owner {
            home = "device_owner";
        }
        Ok(serde_json::json!({"installed":true,"configured":true,"home":home,"device_owner":actual_owner,"owner_error":owner_error,"owner_report":owner_report}).to_string())
    }
}
