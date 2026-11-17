
use hidapi::{BusType, DeviceInfo, HidApi, HidDevice};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

const LOGITECH: u16 = 0x046d;
const DIRECT_DEVICE: u8 = 0xff;
const FN_INVERSION: u16 = 0x40a0;
const FN_WITH_DEFAULT: u16 = 0x40a2;
const FN_MULTI_HOST: u16 = 0x40a3;
const HOSTS_INFO: u16 = 0x1815;
const REQUEST_TIMEOUT: Duration = Duration::from_millis(900);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(9);

static DEVICE_GATE: Mutex<()> = Mutex::new(());
static SOFTWARE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardSettings {
    pub device_id: String,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fn_inversion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_id: Option<String>,
}

impl KeyboardSettings {
    fn unavailable(device_id: &str) -> Self {
        Self {
            device_id: device_id.into(),
            supported: false,
            fn_inversion: None,
            reason: None,
            protocol: None,
            feature_id: None,
        }
    }
}

pub fn get_keyboard_settings(device_id: &str) -> Result<KeyboardSettings, String> {
    let device_id = device_id.to_owned();
    crate::hid_thread::run(move || operate(&device_id, None))?
}

pub fn set_fn_inversion(device_id: &str, enabled: bool) -> Result<KeyboardSettings, String> {
    let device_id = device_id.to_owned();
    crate::hid_thread::run(move || operate(&device_id, Some(enabled)))?
}

fn operate(device_id: &str, desired: Option<bool>) -> Result<KeyboardSettings, String> {
    let mut settings = KeyboardSettings::unavailable(device_id);
    let outcome = with_session(device_id, |session| {
        let feature = inspect(session, &mut settings, rand::random::<u8>())?;
        if let Some(enabled) = desired {
            apply_inversion(session, &mut settings, feature, enabled)?;
        }
        Ok(())
    })?;
    match outcome {
        Ok(()) => Ok(settings),
        Err(error) if desired.is_some() => Err(error.to_string()),
        Err(error) => {
            settings.supported = false;
            settings.fn_inversion = None;
            settings.reason = Some(error.to_string());
            Ok(settings)
        }
    }
}

fn with_session<T>(
    device_id: &str,
    run: impl FnOnce(&mut Session) -> Result<T, ProtocolError>,
) -> Result<Result<T, ProtocolError>, String> {
    if device_id.len() != 24 || !device_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid HID device ID. Refresh the device list.".into());
    }
    let _guard = DEVICE_GATE
        .try_lock()
        .map_err(|_| "Another keyboard settings request is in progress. Try again shortly.")?;
    let api = HidApi::new().map_err(|_| "HID access is unavailable on this computer.")?;
    let matches = api
        .device_list()
        .filter(|info| path_id(info.path().to_bytes()) == device_id);
    let info = prefer_valid_collection(matches, |info| validate_target(info).is_ok())
        .ok_or("The selected HID device is no longer present. Refresh the device list.")?;
    Ok((|| {
        validate_target(info)?;
        let device = info.open_device(&api).map_err(|_| {
            ProtocolError::Invalid(
                "Cannot open this HID++ collection. Check OS device permissions or close another device manager.",
            )
        })?;
        let mut descriptor = [0; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
        let size = device.get_report_descriptor(&mut descriptor).map_err(|_| {
            ProtocolError::Invalid(
                "The operating system cannot read this collection's HID report descriptor.",
            )
        })?;
        if size == descriptor.len() {
            return Err(ProtocolError::Invalid(
                "The HID report descriptor is too large to verify safely.",
            ));
        }
        let reports = ReportSupport::parse(&descriptor[..size])?;
        let mut session = Session {
            device,
            reports,
            allow_zero_index: matches!(info.bus_type(), BusType::Bluetooth),
            deadline: Instant::now() + OPERATION_TIMEOUT,
        };
        run(&mut session)
    })())
}

fn path_id(path: &[u8]) -> String {
    Sha256::digest(path)[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn prefer_valid_collection<'a, T: 'a>(
    mut candidates: impl Iterator<Item = &'a T>,
    valid: impl Fn(&T) -> bool,
) -> Option<&'a T> {
    let first = candidates.next()?;
    Some(
        std::iter::once(first)
            .chain(candidates)
            .find(|item| valid(item))
            .unwrap_or(first),
    )
}

fn validate_target(info: &DeviceInfo) -> Result<(), ProtocolError> {
    validate_target_metadata(
        info.vendor_id(),
        info.product_id(),
        info.usage_page(),
        info.usage(),
        info.bus_type(),
    )
}

fn validate_target_metadata(
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    usage: u16,
    bus: BusType,
) -> Result<(), ProtocolError> {
    if vendor_id != LOGITECH {
        return Err(ProtocolError::Invalid(
            "This control supports Logitech HID++ keyboards only.",
        ));
    }
    if (0xc500..=0xc5ff).contains(&product_id) {
        return Err(ProtocolError::Invalid(
            "Receiver-paired keyboard settings are not supported yet. Connect the keyboard directly by Bluetooth or USB; TrapLink does not guess receiver slots.",
        ));
    }
    if !matches!(
        (usage_page, usage),
        (0xff00, 0x0001 | 0x0002) | (0xff43, 0x0202)
    ) {
        return Err(ProtocolError::Invalid(
            "Select this keyboard's vendor HID++ collection (FF00:0001/0002 or FF43:0202). Normal keyboard collections cannot provide this control.",
        ));
    }
    if !matches!(bus, BusType::Usb | BusType::Bluetooth) {
        return Err(ProtocolError::Invalid(
            "The connection type cannot be verified as direct USB or Bluetooth.",
        ));
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
enum ProtocolError {
    Invalid(&'static str),
    Timeout,
    Transport,
    Device { legacy: bool, code: u8 },
    Unconfirmed(String),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Timeout => f.write_str("The keyboard did not answer HID++ in time. Wake it and read settings again."),
            Self::Transport => f.write_str("HID communication failed. Reconnect the keyboard and read settings again."),
            Self::Device { legacy: true, code: 1 } => f.write_str("This device reports HID++ 1.0; this control requires HID++ 2.0 or later."),
            Self::Device { legacy, code } => write!(f, "The keyboard rejected the HID++ {} request (error 0x{code:02X}).", if *legacy { "1.0" } else { "2.0" }),
            Self::Unconfirmed(message) => write!(f, "The Fn setting change could not be confirmed: {message} The change may have applied; read settings again before retrying."),
        }
    }
}

trait Calls {
    fn call(&mut self, feature: u8, function: u8, params: &[u8]) -> Result<Vec<u8>, ProtocolError>;
    fn has_long_input(&self) -> bool;
}

struct Session {
    device: HidDevice,
    reports: ReportSupport,
    allow_zero_index: bool,
    deadline: Instant,
}

impl Calls for Session {
    fn call(&mut self, feature: u8, function: u8, params: &[u8]) -> Result<Vec<u8>, ProtocolError> {
        let now = Instant::now();
        if now >= self.deadline {
            return Err(ProtocolError::Timeout);
        }
        let deadline = self.deadline.min(now + REQUEST_TIMEOUT);
        let report_id = self.reports.output_report()?;
        let software_id = (SOFTWARE_SEQUENCE.fetch_add(1, Ordering::Relaxed) % 15 + 1) as u8;
        let request = Request::new(report_id, feature, function, software_id, params)?;
        let bytes = request.encode();
        let written = self
            .device
            .write(&bytes)
            .map_err(|_| ProtocolError::Transport)?;
        if written != bytes.len() && !(cfg!(target_os = "windows") && written == 0) {
            return Err(ProtocolError::Transport);
        }
        let mut buffer = [0; 64];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ProtocolError::Timeout);
            }
            let millis = remaining.as_millis().clamp(1, 100) as i32;
            let count = self
                .device
                .read_timeout(&mut buffer, millis)
                .map_err(|_| ProtocolError::Transport)?;
            if count == 0 {
                continue;
            }
            match request.decode(&buffer[..count], self.allow_zero_index)? {
                Some(payload) => return Ok(payload.to_vec()),
                None => continue,
            }
        }
    }

    fn has_long_input(&self) -> bool {
        self.reports.long_input
    }
}

#[derive(Clone, Copy, Debug)]
struct Host {
    feature_index: u8,
    index: u8,
}

#[derive(Clone, Copy, Debug)]
struct FnFeature {
    index: u8,
    host: Option<Host>,
}

fn inspect(
    calls: &mut impl Calls,
    settings: &mut KeyboardSettings,
    nonce: u8,
) -> Result<FnFeature, ProtocolError> {
    let (major, minor) = ping(calls, nonce)?;
    settings.protocol = Some(format!("{major}.{minor}"));
    if major < 2 {
        return Err(ProtocolError::Invalid(
            "This keyboard requires a different protocol; HID++ 2.0 or later is needed.",
        ));
    }
    for id in [FN_MULTI_HOST, FN_WITH_DEFAULT, FN_INVERSION] {
        let Some(index) = find_feature(calls, id)? else {
            continue;
        };
        settings.feature_id = Some(format!("0x{id:04X}"));
        let host = if id == FN_MULTI_HOST {
            if !calls.has_long_input() {
                return Err(ProtocolError::Invalid("Multi-host Fn control needs long HID++ replies. Select the keyboard's FF00:0002 or Bluetooth HID++ collection."));
            }
            let feature_index = find_feature(calls, HOSTS_INFO)?.ok_or(ProtocolError::Invalid(
                "The keyboard does not expose HOSTS_INFO, so its current host cannot be selected safely.",
            ))?;
            Some(Host {
                feature_index,
                index: read_current_host(calls, feature_index)?,
            })
        } else {
            None
        };
        let feature = FnFeature { index, host };
        settings.fn_inversion = Some(read_inversion(calls, feature)?);
        settings.supported = true;
        return Ok(feature);
    }
    Err(ProtocolError::Invalid(
        "This device does not expose a supported Fn inversion feature (40A0, 40A2 or 40A3).",
    ))
}

fn ping(calls: &mut impl Calls, nonce: u8) -> Result<(u8, u8), ProtocolError> {
    let ping = calls.call(0, 1, &[0, 0, nonce])?;
    if ping.len() < 3 || ping[2] != nonce {
        return Err(ProtocolError::Invalid(
            "The HID++ protocol ping returned an invalid response.",
        ));
    }
    Ok((ping[0], ping[1]))
}

fn find_feature(calls: &mut impl Calls, id: u16) -> Result<Option<u8>, ProtocolError> {
    let response = calls.call(0, 0, &id.to_be_bytes())?;
    if response.len() < 3 {
        return Err(ProtocolError::Invalid(
            "The keyboard returned an incomplete feature descriptor.",
        ));
    }
    if response[0] == 0 || response[1] & 0xc0 != 0 {
        return Ok(None);
    }
    if response[0] >= 0x80 {
        return Err(ProtocolError::Invalid(
            "The keyboard returned an invalid HID++ feature index.",
        ));
    }
    Ok(Some(response[0]))
}

fn read_current_host(calls: &mut impl Calls, index: u8) -> Result<u8, ProtocolError> {
    let response = calls.call(index, 0, &[])?;
    if response.len() < 4 || response[2] == 0 || response[3] >= response[2] {
        return Err(ProtocolError::Invalid(
            "The keyboard returned an invalid current host. No host slot was guessed.",
        ));
    }
    Ok(response[3])
}

fn read_inversion(calls: &mut impl Calls, feature: FnFeature) -> Result<bool, ProtocolError> {
    let params = feature
        .host
        .map(|host| vec![host.index])
        .unwrap_or_default();
    let response = calls.call(feature.index, 0, &params)?;
    let value = if let Some(host) = feature.host {
        if response.len() < 4 {
            return Err(ProtocolError::Invalid(
                "The keyboard returned an incomplete multi-host Fn response.",
            ));
        }
        if response.first() != Some(&host.index) {
            return Err(ProtocolError::Invalid(
                "The Fn reply belongs to a different host.",
            ));
        }
        response.get(1)
    } else {
        response.first()
    };
    match value {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(ProtocolError::Invalid(
            "The keyboard returned an invalid Fn inversion state.",
        )),
    }
}

fn apply_inversion(
    calls: &mut impl Calls,
    settings: &mut KeyboardSettings,
    feature: FnFeature,
    enabled: bool,
) -> Result<(), ProtocolError> {
    if settings.fn_inversion == Some(enabled) {
        return Ok(());
    }
    if let Some(host) = feature.host {
        if read_current_host(calls, host.feature_index)? != host.index {
            return Err(ProtocolError::Invalid(
                "The keyboard switched hosts. Read settings again before changing Fn mode.",
            ));
        }
    }
    let mut params = Vec::with_capacity(2);
    if let Some(host) = feature.host {
        params.push(host.index);
    }
    params.push(u8::from(enabled));
    let confirm = (|| {
        let acknowledged = calls.call(feature.index, 1, &params)?;
        if feature.host.is_some() && acknowledged.len() < 4 {
            return Err(ProtocolError::Invalid(
                "The keyboard returned an incomplete multi-host Fn acknowledgement.",
            ));
        }
        let actual = read_inversion(calls, feature)?;
        if actual != enabled {
            return Err(ProtocolError::Invalid(
                "The keyboard reported a different Fn state.",
            ));
        }
        if let Some(host) = feature.host {
            if read_current_host(calls, host.feature_index)? != host.index {
                return Err(ProtocolError::Invalid(
                    "The keyboard switched hosts during the change.",
                ));
            }
        }
        settings.fn_inversion = Some(actual);
        Ok(())
    })();
    confirm.map_err(|error: ProtocolError| ProtocolError::Unconfirmed(error.to_string()))
}

#[derive(Debug)]
struct Request {
    report_id: u8,
    feature: u8,
    function_software: u8,
    params: Vec<u8>,
}

impl Request {
    fn new(
        report_id: u8,
        feature: u8,
        function: u8,
        software_id: u8,
        params: &[u8],
    ) -> Result<Self, ProtocolError> {
        let capacity = match report_id {
            0x10 => 3,
            0x11 => 16,
            _ => return Err(ProtocolError::Invalid("Unsupported HID++ report ID.")),
        };
        if function > 15
            || software_id == 0
            || software_id > 15
            || feature >= 0x80
            || params.len() > capacity
        {
            return Err(ProtocolError::Invalid("Invalid HID++ request parameters."));
        }
        Ok(Self {
            report_id,
            feature,
            function_software: (function << 4) | software_id,
            params: params.to_vec(),
        })
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![0; if self.report_id == 0x10 { 7 } else { 20 }];
        bytes[..4].copy_from_slice(&[
            self.report_id,
            DIRECT_DEVICE,
            self.feature,
            self.function_software,
        ]);
        bytes[4..4 + self.params.len()].copy_from_slice(&self.params);
        bytes
    }

    fn decode<'a>(
        &self,
        bytes: &'a [u8],
        allow_zero_index: bool,
    ) -> Result<Option<&'a [u8]>, ProtocolError> {
        let minimum = match bytes.first() {
            Some(0x10) => 7,
            Some(0x11) => 20,
            _ => return Ok(None),
        };
        if bytes.len() < minimum {
            return Err(ProtocolError::Invalid(
                "The keyboard returned a truncated HID++ report.",
            ));
        }
        if bytes[1] != DIRECT_DEVICE && !(allow_zero_index && bytes[1] == 0) {
            return Ok(None);
        }
        if matches!(bytes[2], 0x8f | 0xff)
            && bytes[3] == self.feature
            && bytes[4] == self.function_software
        {
            return Err(ProtocolError::Device {
                legacy: bytes[2] == 0x8f,
                code: bytes[5],
            });
        }
        if bytes[2] != self.feature || bytes[3] != self.function_software {
            return Ok(None);
        }
        Ok(Some(&bytes[4..minimum]))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ReportSupport {
    short_input: bool,
    short_output: bool,
    long_input: bool,
    long_output: bool,
}

impl ReportSupport {
    fn output_report(self) -> Result<u8, ProtocolError> {
        if self.long_output && self.long_input {
            Ok(0x11)
        } else if self.short_output && (self.short_input || self.long_input) {
            Ok(0x10)
        } else {
            Err(ProtocolError::Invalid(
                "This HID collection does not expose compatible HID++ input and output reports.",
            ))
        }
    }

    fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        #[derive(Clone, Copy, Default)]
        struct Globals {
            id: u8,
            size: u32,
            count: u32,
        }
        let invalid =
            || ProtocolError::Invalid("The HID report descriptor is malformed or unsupported.");
        let mut globals = Globals::default();
        let mut stack = Vec::new();
        let mut bits = [[0u32; 256]; 2];
        let mut offset = 0;
        while offset < bytes.len() {
            let prefix = bytes[offset];
            offset += 1;
            if prefix == 0xfe {
                let size = *bytes.get(offset).ok_or_else(invalid)? as usize;
                offset = offset.checked_add(2 + size).ok_or_else(invalid)?;
                if offset > bytes.len() {
                    return Err(invalid());
                }
                continue;
            }
            let size = match prefix & 3 {
                3 => 4,
                n => n as usize,
            };
            let end = offset.checked_add(size).ok_or_else(invalid)?;
            let data = bytes.get(offset..end).ok_or_else(invalid)?;
            let value = data
                .iter()
                .enumerate()
                .fold(0u32, |value, (i, byte)| value | ((*byte as u32) << (8 * i)));
            offset = end;
            match ((prefix >> 2) & 3, prefix >> 4) {
                (1, 7) => globals.size = value,
                (1, 8) if value > 0 && value < 256 => globals.id = value as u8,
                (1, 8) => return Err(invalid()),
                (1, 9) => globals.count = value,
                (1, 10) if stack.len() < 16 => stack.push(globals),
                (1, 10) => return Err(invalid()),
                (1, 11) => globals = stack.pop().ok_or_else(invalid)?,
                (0, tag @ (8 | 9)) => {
                    let length = globals
                        .size
                        .checked_mul(globals.count)
                        .ok_or_else(invalid)?;
                    let total = &mut bits[(tag == 9) as usize][globals.id as usize];
                    *total = total.checked_add(length).ok_or_else(invalid)?;
                    if *total > 32768 {
                        return Err(invalid());
                    }
                }
                _ => {}
            }
        }
        if !stack.is_empty() {
            return Err(invalid());
        }
        let reports = Self {
            short_input: bits[0][0x10] == 48,
            short_output: bits[1][0x10] == 48,
            long_input: bits[0][0x11] == 152,
            long_output: bits[1][0x11] == 152,
        };
        reports.output_report()?;
        Ok(reports)
    }
}

const BATTERY_STATUS: u16 = 0x1000;
const UNIFIED_BATTERY: u16 = 0x1004;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatteryStatus {
    pub device_id: String,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_power: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl BatteryStatus {
    fn unavailable(device_id: &str) -> Self {
        Self {
            device_id: device_id.into(),
            supported: false,
            percent: None,
            level: None,
            charging: None,
            external_power: None,
            feature_id: None,
            reason: None,
        }
    }
}

pub fn battery_status(device_id: &str) -> Result<BatteryStatus, String> {
    let device_id = device_id.to_owned();
    crate::hid_thread::run(move || battery_status_now(&device_id))?
}

fn battery_status_now(device_id: &str) -> Result<BatteryStatus, String> {
    let mut status = BatteryStatus::unavailable(device_id);
    let outcome = with_session(device_id, |session| {
        read_battery(session, &mut status, rand::random::<u8>())
    })?;
    if let Err(error) = outcome {
        let feature_id = status.feature_id.take();
        status = BatteryStatus::unavailable(device_id);
        status.feature_id = feature_id;
        status.reason = Some(error.to_string());
    }
    Ok(status)
}

#[tauri::command]
pub async fn get_battery_status(device_id: String) -> Result<BatteryStatus, String> {
    tokio::task::spawn_blocking(move || battery_status(&device_id))
        .await
        .map_err(|_| "Battery query failed.".to_string())?
}

fn read_battery(
    calls: &mut impl Calls,
    status: &mut BatteryStatus,
    nonce: u8,
) -> Result<(), ProtocolError> {
    let (major, _) = ping(calls, nonce)?;
    if major < 2 {
        return Err(ProtocolError::Invalid(
            "This device reports HID++ 1.0; battery reading requires HID++ 2.0 or later.",
        ));
    }
    if let Some(index) = find_feature(calls, UNIFIED_BATTERY)? {
        status.feature_id = Some(format!("0x{UNIFIED_BATTERY:04X}"));
        read_unified_battery(calls, status, index)?;
    } else if let Some(index) = find_feature(calls, BATTERY_STATUS)? {
        status.feature_id = Some(format!("0x{BATTERY_STATUS:04X}"));
        read_battery_status(calls, status, index)?;
    } else {
        return Err(ProtocolError::Invalid(
            "This device does not expose a supported battery feature (1004 or 1000).",
        ));
    }
    if status.percent.is_none() && status.level.is_none() {
        return Err(ProtocolError::Invalid(
            "The battery feature did not report a level.",
        ));
    }
    status.supported = true;
    Ok(())
}

fn read_unified_battery(
    calls: &mut impl Calls,
    status: &mut BatteryStatus,
    index: u8,
) -> Result<(), ProtocolError> {
    let capabilities = calls.call(index, 0, &[])?;
    if capabilities.len() < 2 {
        return Err(ProtocolError::Invalid(
            "The battery feature returned incomplete capabilities.",
        ));
    }
    let reports_percent = capabilities[1] & 0x02 != 0;
    let reply = calls.call(index, 1, &[])?;
    if reply.len() < 4 {
        return Err(ProtocolError::Invalid(
            "The battery feature returned an incomplete status.",
        ));
    }
    if reports_percent {
        if reply[0] > 100 {
            return Err(ProtocolError::Invalid(
                "The battery feature returned an out-of-range percentage.",
            ));
        }
        status.percent = Some(reply[0]);
    }
    if reply[1] & 0xf0 != 0 {
        return Err(ProtocolError::Invalid(
            "The battery feature returned unknown level flags.",
        ));
    }
    status.level = match reply[1] {
        flags if flags & 0x08 != 0 => Some("full"),
        flags if flags & 0x04 != 0 => Some("good"),
        flags if flags & 0x02 != 0 => Some("low"),
        flags if flags & 0x01 != 0 => Some("critical"),
        _ => None,
    };
    status.charging = Some(match reply[2] {
        0 | 3 => false,
        1 | 2 => true,
        4 => {
            return Err(ProtocolError::Invalid(
                "The battery feature reports a charging error.",
            ))
        }
        _ => {
            return Err(ProtocolError::Invalid(
                "The battery feature returned an unknown charging status.",
            ))
        }
    });
    status.external_power = Some(match reply[3] {
        0 => false,
        1 | 2 => true,
        _ => {
            return Err(ProtocolError::Invalid(
                "The battery feature returned an unknown external power status.",
            ))
        }
    });
    Ok(())
}

fn read_battery_status(
    calls: &mut impl Calls,
    status: &mut BatteryStatus,
    index: u8,
) -> Result<(), ProtocolError> {
    let reply = calls.call(index, 0, &[])?;
    if reply.len() < 3 {
        return Err(ProtocolError::Invalid(
            "The battery feature returned an incomplete status.",
        ));
    }
    if reply[0] > 100 || reply[1] > 100 {
        return Err(ProtocolError::Invalid(
            "The battery feature returned an out-of-range percentage.",
        ));
    }
    status.percent = (reply[0] != 0).then_some(reply[0]);
    let (charging, external_power) = match reply[2] {
        0 => (false, false),
        1 | 2 | 4 => (true, true),
        3 => {
            status.level = Some("full");
            (false, true)
        }
        5 => {
            return Err(ProtocolError::Invalid(
                "The battery feature reports an invalid battery.",
            ))
        }
        6 => {
            return Err(ProtocolError::Invalid(
                "The battery feature reports a thermal error.",
            ))
        }
        7 => {
            return Err(ProtocolError::Invalid(
                "The battery feature reports a charging error.",
            ))
        }
        _ => {
            return Err(ProtocolError::Invalid(
                "The battery feature returned an unknown charging status.",
            ))
        }
    };
    status.charging = Some(charging);
    status.external_power = Some(external_power);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(
        script: Vec<(u8, u8, Vec<u8>, Result<Vec<u8>, ProtocolError>)>,
    ) -> (BatteryStatus, Result<(), ProtocolError>, bool) {
        let mut calls = fake(script);
        let mut status = BatteryStatus::unavailable("test");
        let result = read_battery(&mut calls, &mut status, 42);
        (status, result, calls.script.is_empty())
    }

    const PING: (u8, u8, [u8; 3], [u8; 3]) = (0, 1, [0, 0, 42], [4, 2, 42]);
    fn ping_step() -> (u8, u8, Vec<u8>, Result<Vec<u8>, ProtocolError>) {
        (PING.0, PING.1, PING.2.to_vec(), Ok(PING.3.to_vec()))
    }

    #[test]
    fn unified_battery_reads_percent_level_and_power() {
        let (status, result, done) = battery(vec![
            ping_step(),
            (0, 0, vec![0x10, 0x04], Ok(vec![7, 0, 0])),
            (7, 0, vec![], Ok(vec![0x0f, 0x02])),
            (7, 1, vec![], Ok(vec![64, 0x04, 1, 1])),
        ]);
        assert!(result.is_ok() && done);
        assert!(status.supported);
        assert_eq!(status.feature_id.as_deref(), Some("0x1004"));
        assert_eq!(status.percent, Some(64));
        assert_eq!(status.level, Some("good"));
        assert_eq!(status.charging, Some(true));
        assert_eq!(status.external_power, Some(true));
    }

    #[test]
    fn unified_battery_without_percent_capability_reports_only_level() {
        let (status, result, done) = battery(vec![
            ping_step(),
            (0, 0, vec![0x10, 0x04], Ok(vec![7, 0, 0])),
            (7, 0, vec![], Ok(vec![0x0f, 0x00])),
            (7, 1, vec![], Ok(vec![99, 0x02, 0, 0])),
        ]);
        assert!(result.is_ok() && done);
        assert_eq!(status.percent, None);
        assert_eq!(status.level, Some("low"));
        assert_eq!(status.charging, Some(false));
    }

    #[test]
    fn unified_battery_rejects_malformed_and_unknown_values() {
        for (caps, reply) in [
            (vec![0x0f], vec![50, 4, 0, 0]),
            (vec![0x0f, 0x02], vec![50, 4, 0]),
            (vec![0x0f, 0x02], vec![101, 4, 0, 0]),
            (vec![0x0f, 0x02], vec![50, 0x10, 0, 0]),
            (vec![0x0f, 0x02], vec![50, 4, 4, 0]),
            (vec![0x0f, 0x02], vec![50, 4, 9, 0]),
            (vec![0x0f, 0x02], vec![50, 4, 0, 7]),
            (vec![0x0f, 0x00], vec![50, 0, 0, 0]),
        ] {
            let mut script = vec![
                ping_step(),
                (0, 0, vec![0x10, 0x04], Ok(vec![7, 0, 0])),
                (7, 0, vec![], Ok(caps.clone())),
            ];
            if caps.len() >= 2 {
                script.push((7, 1, vec![], Ok(reply.clone())));
            }
            let (status, result, _) = battery(script);
            assert!(result.is_err(), "{caps:?} {reply:?}");
            assert!(!status.supported);
        }
    }

    #[test]
    fn battery_status_fallback_reads_level_and_charging() {
        let (status, result, done) = battery(vec![
            ping_step(),
            (0, 0, vec![0x10, 0x04], Ok(vec![0, 0, 0])),
            (0, 0, vec![0x10, 0x00], Ok(vec![3, 0, 0])),
            (3, 0, vec![], Ok(vec![90, 50, 3])),
        ]);
        assert!(result.is_ok() && done);
        assert_eq!(status.feature_id.as_deref(), Some("0x1000"));
        assert_eq!(status.percent, Some(90));
        assert_eq!(status.level, Some("full"));
        assert_eq!(status.charging, Some(false));
        assert_eq!(status.external_power, Some(true));
    }

    #[test]
    fn battery_status_rejects_malformed_errors_and_unknown_level() {
        for reply in [
            vec![90, 50],
            vec![120, 50, 0],
            vec![90, 50, 5],
            vec![90, 50, 6],
            vec![90, 50, 7],
            vec![90, 50, 8],
            vec![0, 0, 0],
        ] {
            let (status, result, _) = battery(vec![
                ping_step(),
                (0, 0, vec![0x10, 0x04], Ok(vec![0, 0, 0])),
                (0, 0, vec![0x10, 0x00], Ok(vec![3, 0, 0])),
                (3, 0, vec![], Ok(reply.clone())),
            ]);
            assert!(result.is_err(), "{reply:?}");
            assert!(!status.supported);
        }
    }

    #[test]
    fn battery_unsupported_paths_never_report_support() {
        let (status, result, done) = battery(vec![
            ping_step(),
            (0, 0, vec![0x10, 0x04], Ok(vec![7, 0x40, 0])),
            (0, 0, vec![0x10, 0x00], Ok(vec![0, 0, 0])),
        ]);
        assert!(result.is_err() && done && !status.supported);
        let (status, result, done) = battery(vec![(0, 1, vec![0, 0, 42], Ok(vec![1, 0, 42]))]);
        assert!(result.is_err() && done && !status.supported);
        let (status, result, _) = battery(vec![(0, 1, vec![0, 0, 42], Ok(vec![4, 2, 41]))]);
        assert!(result.is_err() && !status.supported);
        let (status, result, _) = battery(vec![
            ping_step(),
            (0, 0, vec![0x10, 0x04], Ok(vec![7, 0, 0])),
            (7, 0, vec![], Err(ProtocolError::Timeout)),
        ]);
        assert!(matches!(result, Err(ProtocolError::Timeout)) && !status.supported);
    }

    #[test]
    fn battery_status_serializes_camel_case() {
        let mut status = BatteryStatus::unavailable("abc");
        status.external_power = Some(true);
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["deviceId"], "abc");
        assert_eq!(json["externalPower"], true);
        assert!(json.get("percent").is_none());
    }
    #[test]
    fn shared_path_prefers_vendor_usage_over_primary_keyboard_usage() {
        let collections = [(1, 6), (0xff00, 2)];
        let chosen = prefer_valid_collection(collections.iter(), |&(page, usage)| {
            validate_target_metadata(LOGITECH, 0xb123, page, usage, BusType::Usb).is_ok()
        });
        assert_eq!(chosen, Some(&(0xff00, 2)));
        let unsupported = [(1, 6), (0x0c, 1)];
        assert_eq!(
            prefer_valid_collection(unsupported.iter(), |_| false),
            Some(&(1, 6))
        );
        assert_eq!(
            prefer_valid_collection(collections[..0].iter(), |_| true),
            None
        );
    }
    #[test]
    fn target_validation_uses_identifiers_collections_and_transport() {
        assert!(validate_target_metadata(LOGITECH, 0xb123, 0xff00, 2, BusType::Usb).is_ok());
        assert!(
            validate_target_metadata(LOGITECH, 0xb123, 0xff43, 0x0202, BusType::Bluetooth).is_ok()
        );
        assert!(validate_target_metadata(LOGITECH, 0xc547, 0xff00, 2, BusType::Usb).is_err());
        assert!(validate_target_metadata(LOGITECH, 0xb123, 1, 6, BusType::Usb).is_err());
        assert!(validate_target_metadata(LOGITECH, 0xb123, 0x0c, 1, BusType::Usb).is_err());
        assert!(validate_target_metadata(0x3142, 0xb123, 0xff00, 2, BusType::Usb).is_err());
        assert!(validate_target_metadata(LOGITECH, 0xb123, 0xff00, 2, BusType::Unknown).is_err());
    }
    use std::collections::VecDeque;

    struct Fake {
        script: VecDeque<(u8, u8, Vec<u8>, Result<Vec<u8>, ProtocolError>)>,
        long: bool,
    }
    impl Calls for Fake {
        fn call(
            &mut self,
            feature: u8,
            function: u8,
            params: &[u8],
        ) -> Result<Vec<u8>, ProtocolError> {
            let (expected_feature, expected_function, expected_params, response) =
                self.script.pop_front().expect("unexpected HID request");
            assert_eq!(
                (feature, function, params),
                (
                    expected_feature,
                    expected_function,
                    expected_params.as_slice()
                )
            );
            response
        }
        fn has_long_input(&self) -> bool {
            self.long
        }
    }
    fn fake(script: Vec<(u8, u8, Vec<u8>, Result<Vec<u8>, ProtocolError>)>) -> Fake {
        Fake {
            script: script.into(),
            long: true,
        }
    }

    #[test]
    fn request_bytes_and_validation() {
        let request = Request::new(0x10, 0x12, 1, 0x0a, &[1]).unwrap();
        assert_eq!(request.encode(), [0x10, 0xff, 0x12, 0x1a, 1, 0, 0]);
        assert_eq!(
            Request::new(0x11, 0, 0, 8, &[0x40, 0xa0])
                .unwrap()
                .encode()
                .len(),
            20
        );
        for (report, feature, function, software, params) in [
            (0x12, 0, 0, 8, vec![]),
            (0x10, 0, 16, 8, vec![]),
            (0x10, 0, 0, 0, vec![]),
            (0x10, 0x80, 0, 8, vec![]),
            (0x10, 0, 0, 8, vec![0; 4]),
        ] {
            assert!(Request::new(report, feature, function, software, &params).is_err());
        }
    }

    #[test]
    fn replies_must_match_device_feature_function_and_software_id() {
        let request = Request::new(0x10, 0x12, 0, 8, &[]).unwrap();
        assert_eq!(
            request
                .decode(&[0x10, 0xff, 0x12, 8, 1, 0, 0], false)
                .unwrap(),
            Some(&[1, 0, 0][..])
        );
        for bytes in [
            [0x10, 1, 0x12, 8, 1, 0, 0],
            [0x10, 0xff, 0x13, 8, 1, 0, 0],
            [0x10, 0xff, 0x12, 0, 1, 0, 0],
            [0x10, 0xff, 0x12, 0x18, 1, 0, 0],
        ] {
            assert_eq!(request.decode(&bytes, false).unwrap(), None);
        }
        assert!(request.decode(&[0x10, 0xff], false).is_err());
        assert_eq!(
            request
                .decode(&[2, 0xff, 0xff, 0x12, 8, 0, 0], false)
                .unwrap(),
            None
        );
        assert_eq!(
            request.decode(&[0x10, 0, 0x12, 8, 1, 0, 0], true).unwrap(),
            Some(&[1, 0, 0][..])
        );
        assert_eq!(
            request.decode(&[0x10, 0, 0x12, 8, 1, 0, 0], false).unwrap(),
            None
        );
    }

    #[test]
    fn device_errors_are_never_treated_as_success() {
        let request = Request::new(0x11, 0x12, 1, 8, &[1]).unwrap();
        assert_eq!(
            request.decode(&[0x10, 0xff, 0xff, 0x12, 0x18, 5, 0], false),
            Err(ProtocolError::Device {
                legacy: false,
                code: 5
            })
        );
        assert_eq!(
            request.decode(&[0x10, 0xff, 0x8f, 0x12, 0x18, 1, 0], false),
            Err(ProtocolError::Device {
                legacy: true,
                code: 1
            })
        );
    }

    #[test]
    fn descriptor_requires_real_input_and_output_lengths() {
        let descriptor = [0x85, 0x11, 0x75, 8, 0x95, 19, 0x81, 2, 0x91, 2];
        let reports = ReportSupport::parse(&descriptor).unwrap();
        assert_eq!(reports.output_report().unwrap(), 0x11);
        assert!(reports.long_input);
        assert!(ReportSupport::parse(&descriptor[..8]).is_err());
        assert!(ReportSupport::parse(&[0x85, 0x11, 0x75, 8, 0x95, 18, 0x81, 2, 0x91, 2]).is_err());
        assert!(ReportSupport::parse(&[0x76]).is_err());
        assert!(ReportSupport::parse(&[0xb4]).is_err());
    }

    #[test]
    fn host_read_does_not_guess_slots() {
        let mut calls = fake(vec![(7, 0, vec![], Ok(vec![1, 0, 3, 2]))]);
        assert_eq!(read_current_host(&mut calls, 7).unwrap(), 2);
        let mut calls = fake(vec![(7, 0, vec![], Ok(vec![1, 0, 3, 3]))]);
        assert!(read_current_host(&mut calls, 7).is_err());
    }

    #[test]
    fn setting_requires_ack_and_matching_readback() {
        let feature = FnFeature {
            index: 5,
            host: None,
        };
        let mut settings = KeyboardSettings::unavailable("test");
        settings.fn_inversion = Some(false);
        let mut calls = fake(vec![
            (5, 1, vec![1], Ok(vec![0, 0, 0])),
            (5, 0, vec![], Ok(vec![1, 0, 0])),
        ]);
        apply_inversion(&mut calls, &mut settings, feature, true).unwrap();
        assert_eq!(settings.fn_inversion, Some(true));
        assert!(calls.script.is_empty());
        settings.fn_inversion = Some(false);
        let mut calls = fake(vec![
            (5, 1, vec![1], Ok(vec![0, 0, 0])),
            (5, 0, vec![], Ok(vec![0, 0, 0])),
        ]);
        assert!(matches!(
            apply_inversion(&mut calls, &mut settings, feature, true),
            Err(ProtocolError::Unconfirmed(_))
        ));
        assert_eq!(settings.fn_inversion, Some(false));
    }

    #[test]
    fn unchanged_setting_never_writes() {
        let mut calls = fake(vec![]);
        let mut settings = KeyboardSettings::unavailable("test");
        settings.fn_inversion = Some(true);
        apply_inversion(
            &mut calls,
            &mut settings,
            FnFeature {
                index: 5,
                host: None,
            },
            true,
        )
        .unwrap();
    }

    #[test]
    fn multi_host_write_uses_verified_host_and_checks_it_afterward() {
        let mut calls = fake(vec![
            (7, 0, vec![], Ok(vec![1, 0, 3, 2])),
            (5, 1, vec![2, 1], Ok(vec![2, 1, 0, 1])),
            (5, 0, vec![2], Ok(vec![2, 1, 0, 1])),
            (7, 0, vec![], Ok(vec![1, 0, 3, 2])),
        ]);
        let mut settings = KeyboardSettings::unavailable("test");
        settings.fn_inversion = Some(false);
        let feature = FnFeature {
            index: 5,
            host: Some(Host {
                feature_index: 7,
                index: 2,
            }),
        };
        apply_inversion(&mut calls, &mut settings, feature, true).unwrap();
        assert_eq!(settings.fn_inversion, Some(true));
        assert!(calls.script.is_empty());
    }

    #[test]
    fn changed_host_prevents_write_and_invalid_state_is_rejected() {
        let mut calls = fake(vec![(7, 0, vec![], Ok(vec![1, 0, 3, 1]))]);
        let mut settings = KeyboardSettings::unavailable("test");
        settings.fn_inversion = Some(false);
        let feature = FnFeature {
            index: 5,
            host: Some(Host {
                feature_index: 7,
                index: 2,
            }),
        };
        assert!(apply_inversion(&mut calls, &mut settings, feature, true).is_err());
        assert!(calls.script.is_empty());
        let mut calls = fake(vec![(5, 0, vec![2], Ok(vec![2, 2, 0]))]);
        assert!(read_inversion(&mut calls, feature).is_err());
    }

    #[test]
    fn hash_matches_inventory_contract() {
        assert_eq!(path_id(b"abc"), "ba7816bf8f01cfea414140de");
    }

    #[test]
    fn discovery_reports_only_a_confirmed_feature_and_reads_without_writing() {
        let mut calls = fake(vec![
            (0, 1, vec![0, 0, 42], Ok(vec![2, 0, 42])),
            (0, 0, vec![0x40, 0xa3], Ok(vec![0, 0, 0])),
            (0, 0, vec![0x40, 0xa2], Ok(vec![0, 0, 0])),
            (0, 0, vec![0x40, 0xa0], Ok(vec![5, 0, 0])),
            (5, 0, vec![], Ok(vec![1, 0, 0])),
        ]);
        let mut settings = KeyboardSettings::unavailable("test");
        inspect(&mut calls, &mut settings, 42).unwrap();
        assert!(settings.supported);
        assert_eq!(settings.feature_id.as_deref(), Some("0x40A0"));
        assert_eq!(settings.protocol.as_deref(), Some("2.0"));
        assert_eq!(settings.fn_inversion, Some(true));
        assert!(calls.script.is_empty());
    }

    #[test]
    fn absent_feature_and_wrong_ping_never_report_support() {
        let mut calls = fake(vec![
            (0, 1, vec![0, 0, 42], Ok(vec![2, 0, 42])),
            (0, 0, vec![0x40, 0xa3], Ok(vec![0, 0, 0])),
            (0, 0, vec![0x40, 0xa2], Ok(vec![0, 0, 0])),
            (0, 0, vec![0x40, 0xa0], Ok(vec![0, 0, 0])),
        ]);
        let mut settings = KeyboardSettings::unavailable("test");
        assert!(inspect(&mut calls, &mut settings, 42).is_err());
        assert!(!settings.supported);
        let mut calls = fake(vec![(0, 1, vec![0, 0, 42], Ok(vec![2, 0, 41]))]);
        assert!(inspect(&mut calls, &mut settings, 42).is_err());
        assert!(!settings.supported);
    }

    #[test]
    fn rejected_write_never_updates_cached_state() {
        let mut calls = fake(vec![(
            5,
            1,
            vec![1],
            Err(ProtocolError::Device {
                legacy: false,
                code: 5,
            }),
        )]);
        let mut settings = KeyboardSettings::unavailable("test");
        settings.fn_inversion = Some(false);
        assert!(apply_inversion(
            &mut calls,
            &mut settings,
            FnFeature {
                index: 5,
                host: None
            },
            true
        )
        .is_err());
        assert_eq!(settings.fn_inversion, Some(false));
        assert!(calls.script.is_empty());
    }

    #[test]
    fn multi_host_short_read_and_acknowledgement_are_rejected() {
        let feature = FnFeature {
            index: 5,
            host: Some(Host {
                feature_index: 7,
                index: 2,
            }),
        };
        let mut calls = fake(vec![(5, 0, vec![2], Ok(vec![2, 1, 0]))]);
        assert!(read_inversion(&mut calls, feature).is_err());
        let mut settings = KeyboardSettings::unavailable("test");
        settings.fn_inversion = Some(false);
        let mut calls = fake(vec![
            (7, 0, vec![], Ok(vec![1, 0, 3, 2])),
            (5, 1, vec![2, 1], Ok(vec![2, 1, 0])),
        ]);
        assert!(apply_inversion(&mut calls, &mut settings, feature, true).is_err());
        assert_eq!(settings.fn_inversion, Some(false));
        assert!(calls.script.is_empty());
    }
}
