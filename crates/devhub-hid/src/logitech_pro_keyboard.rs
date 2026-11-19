use devhub_core::backend::{DeviceDriver, HubError, HubResult};
use devhub_core::model::*;
use hidapi::{HidApi, HidDevice};
use std::sync::Mutex;

const MAIN_KEYS: &[u8] = &[
    0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13,
    0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23,
    0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33,
    0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43,
    0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x64,
    0x65, 0x87, 0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7,
];
const LIGHTING_AREA: u8 = 0;
const STATIC_EFFECT: u8 = 0;
const MAX_BRIGHTNESS: u8 = 100;

struct Keyboard {
    info: DeviceInfo,
    lighting: Mutex<Lighting>,
}

struct Lighting {
    short: HidDevice,
    long: HidDevice,
    last_applied: Option<LightingState>,
}

pub fn open(api: &HidApi, short: HidDevice, info: DeviceInfo) -> HubResult<impl DeviceDriver> {
    let descriptor = short
        .get_device_info()
        .map_err(|error| HubError::Io(format!("PRO X short report descriptor: {error}")))?;
    let serial = descriptor
        .serial_number()
        .filter(|value| !value.is_empty())
        .ok_or(HubError::Unsupported(
            "PRO X RGB requires a USB serial number",
        ))?;
    // Windows exposes report 0x11 and report 0x12 on separate HID collections.
    let mut matches = api.device_list().filter(|candidate| {
        candidate.vendor_id() == info.vid
            && candidate.product_id() == info.pid
            && candidate.usage_page() == 0xff43
            && candidate.usage() == 0x0604
            && candidate.serial_number() == Some(serial)
    });
    let descriptor = matches
        .next()
        .filter(|_| matches.next().is_none())
        .ok_or(HubError::Unsupported("PRO X RGB long report collection"))?;
    let long = descriptor
        .open_device(api)
        .map_err(|error| HubError::Io(format!("PRO X RGB long report collection: {error}")))?;
    Ok(Keyboard {
        info,
        lighting: Mutex::new(Lighting {
            short,
            long,
            last_applied: None,
        }),
    })
}

impl DeviceDriver for Keyboard {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn caps(&self) -> Capabilities {
        let mut caps = Capabilities::empty();
        caps.lighting.push(LightArea {
            id: LIGHTING_AREA,
            name: "固定色（この起動中に適用した設定）".into(),
            effects: vec![LightEffect::new(STATIC_EFFECT, "固定色").color().still()],
            brightness_max: MAX_BRIGHTNESS,
            speed_max: 0,
            directions: vec![],
            switchable: true,
            zones: vec![],
            per_key: false,
            direct_drive: false,
            live_readback: false,
        });
        caps
    }

    fn read(&self) -> HubResult<DeviceState> {
        let lighting = self
            .lighting
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Ok(DeviceState {
            profile: ProfileState {
                active: 0,
                names: vec![],
            },
            settings: Default::default(),
            mouse: None,
            keyboard: None,
            lighting: lighting
                .last_applied
                .clone()
                .map(|state| (LIGHTING_AREA, state))
                .into_iter()
                .collect(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        let Change::Lighting {
            area: LIGHTING_AREA,
            state,
        } = change
        else {
            return Err(HubError::Unsupported("PRO X supports static RGB only"));
        };
        if state.effect != STATIC_EFFECT || state.brightness > MAX_BRIGHTNESS {
            return Err(HubError::Invalid("PRO X static RGB settings".into()));
        }
        let scale = |component| {
            if state.on {
                (u16::from(component) * u16::from(state.brightness) / u16::from(MAX_BRIGHTNESS))
                    as u8
            } else {
                0
            }
        };
        let color = Rgb::new(
            scale(state.color.r),
            scale(state.color.g),
            scale(state.color.b),
        );
        let mut lighting = self
            .lighting
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        lighting.set_static(color)?;
        lighting.last_applied = Some(state.clone());
        Ok(())
    }
}

impl Lighting {
    fn set_static(&self, color: Rgb) -> HubResult<()> {
        // Report fields from the C339 G Pro packets documented by g810-led and logitech-colors.
        let logo = color_report::<20>(0x11, 0x10, 1, &[1], color);
        send(&self.short, &logo)?;
        let indicators = color_report::<64>(0x12, 0x40, 5, &[3, 5, 4, 2, 1], color);
        send(&self.long, &indicators)?;
        for keys in MAIN_KEYS.chunks(14) {
            // The protocol reserves 14 slots even in the last, zero-padded report.
            let report = color_report::<64>(0x12, 1, 14, keys, color);
            send(&self.long, &report)?;
        }
        let mut commit = [0; 20];
        commit[..4].copy_from_slice(&[0x11, 0xff, 0x0c, 0x5a]);
        send(&self.short, &commit)
    }
}

fn color_report<const N: usize>(id: u8, group: u8, slots: u8, keys: &[u8], color: Rgb) -> [u8; N] {
    let mut report = [0; N];
    report[..8].copy_from_slice(&[id, 0xff, 0x0c, 0x3a, 0, group, 0, slots]);
    for (entry, key) in report[8..].chunks_exact_mut(4).zip(keys) {
        entry.copy_from_slice(&[*key, color.r, color.g, color.b]);
    }
    report
}

fn send(device: &HidDevice, report: &[u8]) -> HubResult<()> {
    device.send_output_report(report).map_err(|error| {
        HubError::Io(format!(
            "PRO X RGB output report {:02x}: {error}",
            report[0]
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_logo_report_keeps_report_id_and_padding() {
        let report = color_report::<20>(0x11, 0x10, 1, &[1], Rgb::new(0x12, 0x34, 0x56));
        assert_eq!(
            &report[..12],
            &[0x11, 0xff, 0x0c, 0x3a, 0, 0x10, 0, 1, 1, 0x12, 0x34, 0x56]
        );
        assert_eq!(&report[12..], &[0; 8]);
    }

    #[test]
    fn partial_key_report_preserves_the_fourteen_slot_header() {
        let report = color_report::<64>(0x12, 1, 14, &[0x04, 0x65], Rgb::new(1, 2, 3));
        assert_eq!(
            &report[..16],
            &[0x12, 0xff, 0x0c, 0x3a, 0, 1, 0, 14, 0x04, 1, 2, 3, 0x65, 1, 2, 3]
        );
        assert_eq!(&report[16..], &[0; 48]);
    }
}
