use devhub_core::model::{Change, DriverKind, LightingState, Rgb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--features") {
        return probe_keyboard_features();
    }
    let color = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => None,
        [flag, hex] if flag == "--color" && hex.len() == 6 => {
            Some(Rgb::hex(u32::from_str_radix(hex, 16)?))
        }
        _ => return Err("usage: pro_x_probe [--features | --color RRGGBB]".into()),
    };
    let hub = devhub_hid::connect()?;
    let info = hub
        .scan()?
        .into_iter()
        .find(|info| info.driver == DriverKind::LogitechProKeyboard)
        .ok_or("PRO X Gaming Keyboard not found")?;
    let driver = hub.driver(&info.id)?;
    let caps = driver.caps();
    let area = caps.lighting.first().ok_or("Missing RGB capability")?;
    println!("{}: both RGB collections opened", info.name);
    if let Some(color) = color {
        driver.apply(&Change::Lighting {
            area: area.id,
            state: LightingState {
                on: true,
                effect: area.effects.first().ok_or("Missing RGB effect")?.id,
                color,
                brightness: area.brightness_max,
                speed: 0,
                direction: 0,
                per_key: Default::default(),
                zone_on: vec![],
                palette: vec![],
                palette_sel: 0,
                color_correction: None,
            },
        })?;
        println!("RGB reports sent for #{:06x}", color.to_hex());
    }
    Ok(())
}

fn probe_keyboard_features() -> Result<(), Box<dyn std::error::Error>> {
    let api = hidapi::HidApi::new()?;
    let descriptor = api
        .device_list()
        .find(|info| {
            info.vendor_id() == 0x046d
                && info.product_id() == 0xc339
                && info.usage_page() == 0xff43
                && info.usage() == 0x0602
        })
        .ok_or("PRO X HID++ collection not found")?;
    let channel = devhub_hid::transport::Channel::open(descriptor.open_device(&api)?, 20, 20);
    for (number, (feature, name)) in [
        (0x0005u16, "Device name"),
        (0x1b04, "Reprogrammable controls"),
        (0x8010, "G-keys"),
        (0x8100, "Onboard profiles"),
    ]
    .into_iter()
    .enumerate()
    {
        let client = (number + 1) as u8;
        let [hi, lo] = feature.to_be_bytes();
        channel.send_report(0x11, &[0xff, 0, client, hi, lo])?;
        let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
        let mut index = None;
        while let Some(remaining) = until.checked_duration_since(std::time::Instant::now()) {
            let Ok(reply) = channel.recv(remaining.as_millis().max(1) as i32) else {
                break;
            };
            if reply.len() >= 7 && reply[..4] == [0x11, 0xff, 0, client] {
                index = Some(reply[4]);
                break;
            }
        }
        match index {
            Some(0) => println!("{feature:04x} {name}: unavailable"),
            Some(index) => println!("{feature:04x} {name}: index {index:02x}"),
            None => println!("{feature:04x} {name}: no matching response"),
        }
    }
    Ok(())
}
