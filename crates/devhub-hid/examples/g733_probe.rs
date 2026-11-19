use devhub_core::backend::{DeviceDriver, HubError, HubResult};
use devhub_core::model::{Change, DriverKind, SettingValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let round_trip = match args.as_slice() {
        [] => false,
        [flag] if flag == "--sidetone-round-trip" => true,
        _ => return Err("usage: g733_probe [--sidetone-round-trip]".into()),
    };
    let hub = devhub_hid::connect()?;
    let info = hub
        .scan()?
        .into_iter()
        .find(|info| info.driver == DriverKind::LogitechG733)
        .ok_or("G733 receiver not found")?;
    println!(
        "Receiver: {} ({:04x}:{:04x})",
        info.name, info.vid, info.pid
    );
    let driver = hub.driver(&info.id)?;
    println!("Battery: {:?}", driver.battery()?);
    if let Some(sidetone) = driver.read()?.settings.get("sidetone") {
        println!("Sidetone: {}%", sidetone.as_num());
    }
    if round_trip {
        let original = read_sidetone(driver.as_ref())?;
        let temporary = if original <= 95.0 {
            original + 5.0
        } else {
            original - 5.0
        };
        let change = apply_sidetone(driver.as_ref(), temporary);
        apply_sidetone(driver.as_ref(), original).map_err(|error| {
            format!("Could not restore original sidetone ({original}%): {error}")
        })?;
        change?;
        println!("Sidetone round trip passed: {original}% -> {temporary}% -> {original}%");
    }
    Ok(())
}

fn read_sidetone(driver: &dyn DeviceDriver) -> HubResult<f32> {
    match driver.read()?.settings.get("sidetone") {
        Some(SettingValue::Num(value)) => Ok(*value),
        _ => Err(HubError::Unsupported("G733 sidetone")),
    }
}

fn apply_sidetone(driver: &dyn DeviceDriver, value: f32) -> HubResult<()> {
    driver.apply(&Change::Setting {
        id: "sidetone".into(),
        value: SettingValue::Num(value),
    })?;
    if read_sidetone(driver)? != value {
        return Err(HubError::Protocol(format!(
            "G733 sidetone did not reach {value}%"
        )));
    }
    Ok(())
}
