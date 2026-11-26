use crate::model::Device;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long periodic snapshots reuse the last device list before enumerating again.
const INVENTORY_TTL: Duration = Duration::from_secs(10);

type InventoryCache = Mutex<Option<(Instant, Vec<Device>)>>;
static INVENTORY_CACHE: InventoryCache = Mutex::new(None);

#[derive(Clone)]
struct Collection {
    id: String,
    physical_id: String,
    name: String,
    vendor_id: u16,
    product_id: u16,
    transport: String,
    serial: Option<String>,
    manufacturer: Option<String>,
    usage_page: u16,
    usage: u16,
}
fn path_id(path: &[u8]) -> String {
    Sha256::digest(path)[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn hidpp_candidate(c: &Collection) -> bool {
    c.vendor_id == 0x046d
        && !(0xc500..=0xc5ff).contains(&c.product_id)
        && ((c.usage_page == 0xff00 && [1, 2].contains(&c.usage))
            || (c.usage_page == 0xff43 && c.usage == 0x0202))
}
fn input_kind(c: &Collection) -> Option<&'static str> {
    match (c.usage_page, c.usage) {
        (1, 2) => Some("mouse"),
        (1, 6) => Some("keyboard"),
        _ => None,
    }
}
fn grouped_inventory(collections: Vec<Collection>) -> Vec<Device> {
    let mut groups: BTreeMap<String, Vec<Collection>> = BTreeMap::new();
    for c in collections {
        groups
            .entry(format!(
                "{}:{:04x}:{:04x}",
                c.physical_id, c.vendor_id, c.product_id
            ))
            .or_default()
            .push(c);
    }
    let mut devices = vec![];
    for mut group in groups.into_values() {
        let kinds: BTreeSet<String> = group
            .iter()
            .filter_map(input_kind)
            .map(str::to_owned)
            .collect();
        if kinds.is_empty() {
            continue;
        }
        group.sort_by_key(|c| {
            (
                if hidpp_candidate(c) {
                    if c.usage == 1 {
                        1
                    } else {
                        0
                    }
                } else if input_kind(c).is_some() {
                    2
                } else {
                    3
                },
                c.id.clone(),
            )
        });
        let canonical = &group[0];
        let display = group
            .iter()
            .find(|c| input_kind(c).is_some())
            .unwrap_or(canonical);
        let mut capabilities = vec!["read-only-inventory".into()];
        capabilities.extend(kinds.iter().cloned());
        if group.iter().any(hidpp_candidate) {
            capabilities.push("hidpp-settings-candidate".into());
        }
        devices.push(Device {
            id: canonical.id.clone(),
            name: display.name.clone(),
            vendor_id: display.vendor_id,
            product_id: display.product_id,
            transport: display.transport.clone(),
            serial_number: display.serial.clone(),
            manufacturer: display.manufacturer.clone(),
            is_logitech: display.vendor_id == 0x046d,
            capabilities,
            input_kinds: kinds.into_iter().collect(),
        });
    }
    devices.sort_by(|a, b| {
        b.is_logitech
            .cmp(&a.is_logitech)
            .then(a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    });
    devices
}
pub fn inventory() -> Result<Vec<Device>, String> {
    crate::hid_thread::run(inventory_now)?
}

/// Device list for snapshots. Enumerating HID devices is slow and shares the
/// HID thread with HID++ requests, so periodic refreshes reuse a recent list.
/// `force` always enumerates (explicit refresh from the Devices page).
pub fn inventory_cached(force: bool) -> Result<Vec<Device>, String> {
    cached(
        &INVENTORY_CACHE,
        Instant::now(),
        force,
        crate::hid_thread::busy(),
        inventory,
    )
}

fn cached(
    cache: &InventoryCache,
    now: Instant,
    force: bool,
    hid_busy: bool,
    enumerate: impl FnOnce() -> Result<Vec<Device>, String>,
) -> Result<Vec<Device>, String> {
    if !force {
        let cache = cache
            .lock()
            .map_err(|_| "Device list lock is unavailable.")?;
        if let Some((at, devices)) = cache.as_ref() {
            // While another HID job is queued or running, the last list is better
            // than blocking the whole snapshot behind it.
            if hid_busy || now.saturating_duration_since(*at) < INVENTORY_TTL {
                return Ok(devices.clone());
            }
        }
    }
    let devices = enumerate()?;
    if let Ok(mut cache) = cache.lock() {
        if !matches!(cache.as_ref(), Some((at, _)) if *at > now) {
            *cache = Some((now, devices.clone()));
        }
    }
    Ok(devices)
}

fn inventory_now() -> Result<Vec<Device>, String> {
    let api = hidapi::HidApi::new().map_err(|e| format!("HID inventory unavailable: {e}"))?;
    let collections = api
        .device_list()
        .map(|info| {
            let path = info.path().to_string_lossy();
            Collection {
                id: path_id(info.path().to_bytes()),
                physical_id: physical_identity(&path),
                name: info
                    .product_string()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Connected input device")
                    .into(),
                vendor_id: info.vendor_id(),
                product_id: info.product_id(),
                transport: format!("{:?}", info.bus_type()).to_lowercase(),
                serial: info
                    .serial_number()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                manufacturer: info.manufacturer_string().map(str::to_owned),
                usage_page: info.usage_page(),
                usage: info.usage(),
            }
        })
        .collect();
    Ok(grouped_inventory(collections))
}
fn physical_identity(path: &str) -> String {
    #[cfg(target_os = "windows")]
    if let Some(container) = windows_container(path) {
        return format!("container:{container}");
    }
    #[cfg(target_os = "macos")]
    if let Some(identity) = macos_physical_identity(path) {
        return identity;
    }
    unresolved_identity(path)
}

fn unresolved_identity(path: &str) -> String {
    format!("path:{path}")
}

#[cfg(target_os = "macos")]
fn macos_physical_identity(path: &str) -> Option<String> {
    use std::ffi::{c_char, c_void};
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IORegistryEntryIDMatching(entry_id: u64) -> *mut c_void;
        fn IOServiceGetMatchingService(port: u32, matching: *mut c_void) -> u32;
        fn IORegistryEntryGetParentEntry(entry: u32, plane: *const c_char, parent: *mut u32)
            -> i32;
        fn IORegistryEntryGetRegistryEntryID(entry: u32, id: *mut u64) -> i32;
        fn IOObjectConformsTo(entry: u32, class: *const c_char) -> u32;
        fn IOObjectRelease(entry: u32) -> i32;
    }
    let id = path.strip_prefix("DevSrvsID:")?.parse::<u64>().ok()?;
    let matching = unsafe { IORegistryEntryIDMatching(id) };
    if matching.is_null() {
        return None;
    }
    let mut entry = unsafe { IOServiceGetMatchingService(0, matching) };
    if entry == 0 {
        return None;
    }
    for _ in 0..12 {
        let physical = unsafe {
            IOObjectConformsTo(entry, c"IOUSBHostDevice".as_ptr()) != 0
                || IOObjectConformsTo(entry, c"IOUSBDevice".as_ptr()) != 0
                || IOObjectConformsTo(entry, c"IOBluetoothHIDDriver".as_ptr()) != 0
        };
        if physical {
            let mut physical_id = 0;
            let result = unsafe { IORegistryEntryGetRegistryEntryID(entry, &mut physical_id) };
            unsafe { IOObjectRelease(entry) };
            return (result == 0).then(|| format!("ioreg:{physical_id}"));
        }
        let mut parent = 0;
        let result =
            unsafe { IORegistryEntryGetParentEntry(entry, c"IOService".as_ptr(), &mut parent) };
        unsafe { IOObjectRelease(entry) };
        if result != 0 || parent == 0 {
            return None;
        }
        entry = parent;
    }
    unsafe { IOObjectRelease(entry) };
    None
}
#[cfg(target_os = "windows")]
fn windows_container(path: &str) -> Option<String> {
    use windows_sys::Win32::Devices::{DeviceAndDriverInstallation::*, Properties::*};
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut instance = [0u16; 1024];
    let mut size = (instance.len() * 2) as u32;
    let mut kind = 0;
    let result = unsafe {
        CM_Get_Device_Interface_PropertyW(
            path.as_ptr(),
            &DEVPKEY_Device_InstanceId,
            &mut kind,
            instance.as_mut_ptr().cast(),
            &mut size,
            0,
        )
    };
    if result != CR_SUCCESS || kind != DEVPROP_TYPE_STRING {
        return None;
    }
    let mut node = 0;
    if unsafe { CM_Locate_DevNodeW(&mut node, instance.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) }
        != CR_SUCCESS
    {
        return None;
    }
    for _ in 0..4 {
        let mut bytes = [0u8; 16];
        let mut len = 16;
        let mut ty = 0;
        let result = unsafe {
            CM_Get_DevNode_PropertyW(
                node,
                &DEVPKEY_Device_ContainerId,
                &mut ty,
                bytes.as_mut_ptr(),
                &mut len,
                0,
            )
        };
        if result == CR_SUCCESS
            && ty == DEVPROP_TYPE_GUID
            && len == 16
            && bytes.iter().any(|b| *b != 0)
        {
            return Some(bytes.iter().map(|b| format!("{b:02x}")).collect());
        }
        let mut parent = 0;
        if unsafe { CM_Get_Parent(&mut parent, node, 0) } != CR_SUCCESS {
            return None;
        }
        node = parent;
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    fn collection(physical: &str, id: &str, name: &str, usage_page: u16, usage: u16) -> Collection {
        Collection {
            id: id.into(),
            physical_id: physical.into(),
            name: name.into(),
            vendor_id: 0x046d,
            product_id: 0xb123,
            transport: "usb".into(),
            serial: None,
            manufacturer: None,
            usage_page,
            usage,
        }
    }
    #[test]
    fn input_only_and_vendor_canonical() {
        let devices = grouped_inventory(vec![
            collection("a", "keys", "Keyboard", 1, 6),
            collection("a", "vendor", "Keyboard", 0xff00, 2),
            collection("a", "media", "Keyboard", 0x0c, 1),
            collection("mic", "microphone", "Microphone", 0x0c, 1),
            collection("b", "mouse", "Mouse", 1, 2),
        ]);
        assert_eq!(devices.len(), 2);
        let keyboard = devices.iter().find(|d| d.name == "Keyboard").unwrap();
        assert_eq!(keyboard.id, "vendor");
        assert_eq!(keyboard.input_kinds, vec!["keyboard"]);
    }
    fn device(id: &str) -> Device {
        grouped_inventory(vec![collection(id, id, id, 1, 2)])
            .pop()
            .unwrap()
    }

    #[test]
    fn snapshot_inventory_is_cached_until_ttl_or_forced() {
        fn counted<'a>(
            calls: &'a std::cell::Cell<u32>,
            id: &'static str,
        ) -> impl FnOnce() -> Result<Vec<Device>, String> + 'a {
            move || {
                calls.set(calls.get() + 1);
                Ok(vec![device(id)])
            }
        }
        let cache: InventoryCache = Mutex::new(None);
        let calls = std::cell::Cell::new(0);
        let enumerate = |id| counted(&calls, id);
        let start = Instant::now();
        let first = cached(&cache, start, false, false, enumerate("a")).unwrap();
        let second = cached(
            &cache,
            start + Duration::from_secs(5),
            false,
            false,
            enumerate("b"),
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(first[0].id, second.unwrap()[0].id);
        let later = start + INVENTORY_TTL + Duration::from_secs(1);
        let refreshed = cached(&cache, later, false, false, enumerate("c")).unwrap();
        assert_eq!((calls.get(), refreshed[0].name.as_str()), (2, "c"));
        let forced = cached(&cache, later, true, false, enumerate("d")).unwrap();
        assert_eq!((calls.get(), forced[0].name.as_str()), (3, "d"));
        let busy = cached(&cache, later + INVENTORY_TTL * 3, false, true, enumerate("e"));
        assert_eq!((calls.get(), busy.unwrap()[0].name.as_str()), (3, "d"));
    }

    #[test]
    fn snapshot_inventory_errors_are_not_cached() {
        let cache: InventoryCache = Mutex::new(None);
        let start = Instant::now();
        assert!(cached(&cache, start, false, false, || Err("x".into())).is_err());
        assert!(cache.lock().unwrap().is_none());
        let devices = cached(&cache, start, false, true, || Ok(vec![device("a")])).unwrap();
        assert_eq!(devices.len(), 1);
    }

    #[test]
    fn raw_present_inventory_smoke() {
        let inventory = inventory().unwrap();
        assert!(inventory.iter().all(|d| !d.input_kinds.is_empty()));
        assert!(inventory.iter().all(|d| d
            .input_kinds
            .iter()
            .all(|k| k == "keyboard" || k == "mouse")));
        for device in inventory {
            println!(
                "{} | {:?} | {:04x}:{:04x}",
                device.name, device.input_kinds, device.vendor_id, device.product_id
            );
        }
    }
    #[test]
    fn identical_devices_remain_separate() {
        let devices = grouped_inventory(vec![
            collection("first-port", "one", "Same keyboard", 1, 6),
            collection("second-port", "two", "Same keyboard", 1, 6),
        ]);
        assert_eq!(devices.len(), 2);
        assert_ne!(devices[0].id, devices[1].id);
    }
    #[test]
    fn input_detection_ignores_product_and_manufacturer_names() {
        for (name, manufacturer) in [
            ("Fifine Keyboard", "Fifine"),
            ("Microphone", "Audio company"),
            ("Arctis Headset", "SteelSeries"),
            ("", ""),
        ] {
            let mut keyboard = collection("keyboard", "keys", name, 1, 6);
            keyboard.vendor_id = 0x3142;
            keyboard.manufacturer = Some(manufacturer.into());
            let mut mouse = collection("mouse", "pointer", name, 1, 2);
            mouse.vendor_id = 0x3142;
            mouse.manufacturer = Some(manufacturer.into());
            let mut media = collection("media", "media", "Keyboard Mouse", 0x0c, 1);
            media.manufacturer = Some("Fifine".into());
            let devices = grouped_inventory(vec![keyboard, mouse, media]);
            assert_eq!(devices.len(), 2);
            assert_eq!(
                devices.iter().find(|d| d.id == "keys").unwrap().input_kinds,
                vec!["keyboard"]
            );
            assert_eq!(
                devices
                    .iter()
                    .find(|d| d.id == "pointer")
                    .unwrap()
                    .input_kinds,
                vec!["mouse"]
            );
        }
    }
    #[test]
    fn one_physical_device_groups_interfaces_even_with_different_names() {
        let mut keyboard = collection("one-container", "keys", "Fifine Keyboard", 1, 6);
        keyboard.manufacturer = Some("Fifine".into());
        let mut mouse = collection("one-container", "pointer", "Generic HID interface", 1, 2);
        mouse.manufacturer = None;
        let devices = grouped_inventory(vec![keyboard, mouse]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].input_kinds, vec!["keyboard", "mouse"]);
    }
    #[test]
    fn shared_container_does_not_mix_vendor_controls_from_other_products() {
        let keyboard = collection("dock", "keyboard-a", "Keyboard", 1, 6);
        let vendor = collection("dock", "vendor-a", "Keyboard", 0xff00, 2);
        let mut other_product = collection("dock", "keyboard-b", "Keyboard", 1, 6);
        other_product.product_id += 1;
        let mut other_vendor = collection("dock", "keyboard-c", "Keyboard", 1, 6);
        other_vendor.vendor_id = 0x3142;
        let devices = grouped_inventory(vec![keyboard, vendor, other_product, other_vendor]);
        assert_eq!(devices.len(), 3);
        assert_eq!(
            devices
                .iter()
                .find(|d| d.product_id == 0xb123 && d.is_logitech)
                .unwrap()
                .id,
            "vendor-a"
        );
        assert_eq!(
            devices.iter().find(|d| d.product_id == 0xb124).unwrap().id,
            "keyboard-b"
        );
        assert_eq!(
            devices.iter().find(|d| d.vendor_id == 0x3142).unwrap().id,
            "keyboard-c"
        );
    }
    #[test]
    fn long_vendor_collection_is_preferred_to_short_for_fn_queries() {
        let devices = grouped_inventory(vec![
            collection("same", "keyboard", "Keyboard", 1, 6),
            collection("same", "a-short", "Keyboard", 0xff00, 1),
            collection("same", "z-long", "Keyboard", 0xff00, 2),
        ]);
        assert_eq!(devices[0].id, "z-long");
    }
    #[test]
    fn combo_is_one_group() {
        let devices = grouped_inventory(vec![
            collection("receiver", "keyboard", "Receiver", 1, 6),
            collection("receiver", "mouse", "Receiver", 1, 2),
        ]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].input_kinds, vec!["keyboard", "mouse"]);
    }
    #[test]
    fn fallback_preserves_every_unresolved_device_path() {
        let paths = [
            r"\\?\hid#vid_1234&pid_5678#7&aaaa&0&0000#{guid}",
            r"\\?\hid#vid_1234&pid_5678#7&aaaa&0&0001#{guid}",
            r"\\?\hid#vid_1234&pid_5678&mi_01&col01#7&aaaa&0&0000#{guid}",
            r"\\?\hid#vid_1234&pid_5678&mi_02&col02#7&aaaa&0&0001#{guid}",
            "DevSrvsID:100",
            "DevSrvsID:101",
        ];
        let identities: BTreeSet<_> = paths.iter().map(|path| unresolved_identity(path)).collect();
        assert_eq!(identities.len(), paths.len());
        for path in paths {
            assert_eq!(unresolved_identity(path), format!("path:{path}"));
        }
    }
}
