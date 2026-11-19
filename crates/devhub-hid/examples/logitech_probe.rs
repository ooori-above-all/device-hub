use devhub_core::model::{ButtonAction, Change, DriverKind, KeyStroke, SettingValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--link") {
        return check_lightspeed_link();
    }
    let hub = devhub_hid::connect()?;
    let info = hub
        .scan()?
        .into_iter()
        .filter(|info| info.driver == DriverKind::LogitechHidpp)
        .min_by_key(|info| info.pid != 0xc539)
        .ok_or("PRO Wireless receiver not found")?;
    println!(
        "Receiver: {} ({:04x}:{:04x})",
        info.name, info.vid, info.pid
    );
    let driver = hub.driver(&info.id)?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [arg] if arg == "--onboard-on" => {
            driver.run_action("onboard_enable", None)?;
        }
        [arg] if arg == "--onboard-off" => {
            driver.run_action("onboard_disable", None)?;
        }
        _ => {}
    }
    let state = driver.read()?;
    let mouse = state.mouse.ok_or("Missing mouse settings")?;
    let polling = state
        .settings
        .get("polling_wireless")
        .ok_or("Missing polling rate")?;
    if let [arg, value] = args.as_slice() {
        if arg == "--dpi" {
            let mut stage = mouse.stages[0];
            stage.x = value.parse()?;
            driver.apply(&Change::DpiStages {
                stages: vec![stage],
                active: 0,
            })?;
            println!(
                "Applied DPI: {}",
                driver.read()?.mouse.ok_or("Missing mouse settings")?.stages[0].x
            );
            return Ok(());
        }
    }
    println!("DPI: {}, polling: {:?}", mouse.stages[0].x, polling);

    println!("Stored buttons: {:?}", mouse.buttons);
    if std::env::args().any(|arg| arg == "--button-round-trip") {
        let slot = 7;
        let initial = mouse
            .buttons
            .get(slot)
            .ok_or("Missing right side button")?
            .clone();
        let temporary = ButtonAction::Key(KeyStroke {
            mods: 0,
            usage: if initial
                == ButtonAction::Key(KeyStroke {
                    mods: 0,
                    usage: 0x45,
                }) {
                0x44
            } else {
                0x45
            },
        });
        let set = |action| driver.apply(&Change::Button { slot, action });
        let verify = |expected: &ButtonAction| -> Result<(), Box<dyn std::error::Error>> {
            let actual = driver
                .read()?
                .mouse
                .ok_or("Missing mouse settings")?
                .buttons;
            let mut expected_buttons = mouse.buttons.clone();
            expected_buttons[slot] = expected.clone();
            if actual != expected_buttons {
                return Err("Stored button assignments did not match".into());
            }
            Ok(())
        };
        let change = set(temporary.clone())
            .map_err(Into::into)
            .and_then(|_| verify(&temporary));
        set(initial.clone())?;
        verify(&initial)?;
        change?;
        println!("Button round trip passed");
    }

    if std::env::args().any(|arg| arg == "--round-trip") {
        let mut stage = mouse.stages[0];
        stage.x = if stage.x < 25600 {
            stage.x + 50
        } else {
            stage.x - 50
        };
        let dpi_change = |stage| Change::DpiStages {
            stages: vec![stage],
            active: 0,
        };
        let change = driver.apply(&dpi_change(stage));
        let restore = driver.apply(&dpi_change(mouse.stages[0]));
        restore?;
        change?;
        println!(
            "DPI round trip passed: {} -> {} -> {}",
            mouse.stages[0].x, stage.x, mouse.stages[0].x
        );

        let rate_change = |value| Change::Setting {
            id: "polling_wireless".into(),
            value,
        };
        let test_rate = if polling.as_choice() == 500 {
            1000
        } else {
            500
        };
        let change = driver.apply(&rate_change(SettingValue::Choice(test_rate)));
        let restore = driver.apply(&rate_change(polling.clone()));
        restore?;
        change?;
        println!(
            "Polling round trip passed: {} -> {} -> {} Hz",
            polling.as_choice(),
            test_rate,
            polling.as_choice()
        );
    }
    Ok(())
}

fn check_lightspeed_link() -> Result<(), Box<dyn std::error::Error>> {
    let api = hidapi::HidApi::new()?;
    let mut receiver_seen = false;
    for info in api.device_list().filter(|info| {
        info.vendor_id() == 0x046d
            && info.product_id() == 0xc539
            && info.usage_page() == 0xff00
            && matches!(info.usage(), 1 | 2)
    }) {
        receiver_seen = true;
        let device = info.open_device(&api)?;
        if info.usage() == 1 {
            let query = [0x10, 0xff, 0x81, 0x02, 0, 0, 0];
            print_link_response(&device, "receiver connection register", &query)?;
        } else {
            for app_id in [1, 13] {
                let mut ping = [0; 20];
                ping[..7].copy_from_slice(&[0x11, 1, 0, 0x10 | app_id, 0, 0, 0xaa]);
                print_link_response(&device, "paired mouse ping", &ping)?;
            }
        }
    }
    if !receiver_seen {
        return Err("PRO Wireless LIGHTSPEED USB collections not found".into());
    }
    Ok(())
}

fn print_link_response(
    device: &hidapi::HidDevice,
    label: &str,
    request: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    device.write(request)?;
    let until = std::time::Instant::now() + std::time::Duration::from_millis(800);
    let mut reply = [0; 64];
    while let Some(remaining) = until.checked_duration_since(std::time::Instant::now()) {
        let count = device.read_timeout(&mut reply, remaining.as_millis().max(1) as i32)?;
        if count == 0 {
            break;
        }
        if count >= 7 && reply[1..4] == request[1..4] {
            println!("{label}: response {:02x?}", &reply[4..7]);
            return Ok(());
        }
        if count >= 6
            && reply[1] == request[1]
            && matches!(reply[2], 0x8f | 0xff)
            && reply[3..5] == request[2..4]
        {
            println!("{label}: HID error 0x{:02x}", reply[5]);
            return Ok(());
        }
    }
    println!("{label}: no matching response");
    Ok(())
}
