use crate::logitech::{feature_at, request_at};
use crate::transport::Channel;
use devhub_core::backend::{DeviceDriver, HubError, HubResult};
use devhub_core::model::*;
use std::sync::Mutex;

const DIRECT: u8 = 0xff;
const ADC: u16 = 0x1f20;
const SIDETONE: u16 = 0x8300;
const SIDETONE_SETTING: &str = "sidetone";
const VOLTAGE_CURVE: [(u16, u16); 8] = [
    (3150, 0),
    (3300, 5),
    (3500, 10),
    (3650, 20),
    (3750, 40),
    (3850, 60),
    (3950, 80),
    (4100, 100),
];

struct G733 {
    info: DeviceInfo,
    channel: Mutex<Channel>,
    battery_feature: u8,
    sidetone_feature: Option<u8>,
}

pub fn open(device: hidapi::HidDevice, info: DeviceInfo) -> HubResult<impl DeviceDriver> {
    let channel = Channel::open(device, 20, 20);
    let battery_feature = feature_at(&channel, DIRECT, ADC)?;
    let sidetone_feature = match feature_at(&channel, DIRECT, SIDETONE) {
        Ok(feature) => Some(feature),
        Err(HubError::Unsupported(_)) => None,
        Err(error) => return Err(error),
    };
    Ok(G733 {
        info,
        channel: Mutex::new(channel),
        battery_feature,
        sidetone_feature,
    })
}

impl DeviceDriver for G733 {
    fn info(&self) -> DeviceInfo {
        self.info.clone()
    }

    fn caps(&self) -> Capabilities {
        let mut caps = Capabilities::empty();
        caps.battery = true;
        if self.sidetone_feature.is_some() {
            caps.settings.push(SettingSpec::slider(
                SIDETONE_SETTING,
                section::AUDIO,
                "サイドトーン",
                0.,
                100.,
                1.,
                "%",
            ));
        }
        caps
    }

    fn read(&self) -> HubResult<DeviceState> {
        let channel = self
            .channel
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut settings = std::collections::BTreeMap::new();
        if let Some(feature) = self.sidetone_feature {
            let value = f32::from(request_at(&channel, DIRECT, feature, 0, &[])?[0]);
            settings.insert(SIDETONE_SETTING.into(), SettingValue::Num(value));
        }
        Ok(DeviceState {
            profile: ProfileState {
                active: 0,
                names: vec![],
            },
            settings,
            mouse: None,
            keyboard: None,
            lighting: Default::default(),
            peq: None,
        })
    }

    fn apply(&self, change: &Change) -> HubResult<()> {
        let Change::Setting {
            id,
            value: SettingValue::Num(value),
        } = change
        else {
            return Err(HubError::Unsupported("G733 setting"));
        };
        if id != SIDETONE_SETTING {
            return Err(HubError::Unsupported("G733 setting"));
        }
        if !(0.0..=100.0).contains(value) || value.fract() != 0.0 {
            return Err(HubError::Invalid(
                "G733 sidetone must be an integer from 0 to 100".into(),
            ));
        }
        let feature = self
            .sidetone_feature
            .ok_or(HubError::Unsupported("G733 sidetone"))?;
        let channel = self
            .channel
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let level = *value as u8;
        request_at(&channel, DIRECT, feature, 1, &[level])?;
        if request_at(&channel, DIRECT, feature, 0, &[])?[0] != level {
            return Err(HubError::Protocol(
                "G733 sidetone readback did not match".into(),
            ));
        }
        Ok(())
    }

    fn battery(&self) -> HubResult<Battery> {
        let channel = self
            .channel
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let reply = request_at(&channel, DIRECT, self.battery_feature, 0, &[])?;
        let voltage = u16::from_be_bytes([reply[0], reply[1]]);
        let flags = reply[2];
        Ok(Battery {
            percent: (flags & 1 != 0 && voltage != 0).then(|| estimate_percent(voltage)),
            charging: flags & 0x06 == 0x02,
        })
    }
}

// G733 ADC returns voltage, so the percentage is an estimate rather than a device-reported value.
fn estimate_percent(voltage: u16) -> u8 {
    for pair in VOLTAGE_CURVE.windows(2) {
        let (low, low_percent) = pair[0];
        let (high, high_percent) = pair[1];
        if voltage <= high {
            return (low_percent
                + (voltage.saturating_sub(low) * (high_percent - low_percent)) / (high - low))
                as u8;
        }
    }
    100
}
