//! Android settings provider — reads config from IPC.

use parking_lot::Mutex;
use player_ui_traits::{PlayerSettings, SandboxMode, SettingsProvider};

pub struct AndroidSettingsProvider {
    /// Cached settings, updated from Android via IPC.
    settings: Mutex<PlayerSettings>,
}

impl AndroidSettingsProvider {
    pub fn new() -> Self {
        Self {
            settings: Mutex::new(PlayerSettings::default()),
        }
    }

    /// Update settings from a JSON blob (called from command dispatcher).
    pub fn update_from_json(&self, json: &serde_json::Value) {
        let mut s = self.settings.lock();

        if let Some(v) = json.get("disableCrossdomainHttp").and_then(|v| v.as_bool()) {
            s.disable_crossdomain_http = v;
        }
        if let Some(v) = json.get("disableCrossdomainSockets").and_then(|v| v.as_bool()) {
            s.disable_crossdomain_sockets = v;
        }
        if let Some(v) = json.get("hardwareAcceleration").and_then(|v| v.as_bool()) {
            s.hardware_acceleration = v;
        }
        if let Some(v) = json.get("disableGeolocation").and_then(|v| v.as_bool()) {
            s.disable_geolocation = v;
        }
        if let Some(v) = json.get("spoofHardwareId").and_then(|v| v.as_bool()) {
            s.spoof_hardware_id = v;
        }
        if let Some(v) = json.get("disableMicrophone").and_then(|v| v.as_bool()) {
            s.disable_microphone = v;
        }
        if let Some(v) = json.get("disableWebcam").and_then(|v| v.as_bool()) {
            s.disable_webcam = v;
        }
        if let Some(v) = json.get("httpSandboxMode").and_then(|v| v.as_str()) {
            s.http_sandbox_mode = SandboxMode::from_str(v);
        }
        if let Some(v) = json.get("tcpUdpSandboxMode").and_then(|v| v.as_str()) {
            s.tcp_udp_sandbox_mode = SandboxMode::from_str(v);
        }
    }
}

impl SettingsProvider for AndroidSettingsProvider {
    fn get_settings(&self) -> PlayerSettings {
        self.settings.lock().clone()
    }
}
