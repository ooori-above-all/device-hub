use devhub_core::model::DriverKind;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    Ok(())
}
