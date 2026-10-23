use super::protocol;
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
        if owner && !result.contains("device_owner=true") {
            let owner_result=self.command("dpm set-device-owner org.nostdrs.timetable/.AdminReceiver").await.map_err(|e|error(format!("App installed and configured, but device-owner setup failed. This may require a device without accounts or an existing owner. No reset was performed. {e:?}")))?;
            if !owner_result
                .lines()
                .any(|line| line.trim().starts_with("Success:"))
            {
                return Err(error(format!(
                    "App installed and configured, but Android did not confirm device-owner setup: {owner_result}"
                )));
            }
        }
        self.command("am start -W -n org.nostdrs.timetable/.MainActivity")
            .await?;
        let mut home = "unchanged";
        if make_home && !owner {
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
        } else if owner {
            home = "device_owner";
        }
        Ok(serde_json::json!({"installed":true,"configured":true,"home":home,"device_owner":owner}).to_string())
    }
}
